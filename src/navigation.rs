//! Position-to-node navigation.
//!
//! [`SyntaxIndex`] turns an analyzed, token-preserving [`Block`] into a
//! queryable set of source spans: `node_at(offset)` answers "which node is at
//! this byte offset", and `ancestors`, `span`, `kind` and `binding` support
//! editor features such as hover and go-to-definition.
//!
//! Building the index clones the tree once and reads spans through the
//! existing token accessors, so the caller's tree is never mutated. A tree
//! without token data or without a semantic analysis is a typed error
//! (navigation would otherwise silently miss nodes).

use std::collections::HashSet;
use std::fmt;

use crate::diagnostic::TextSpan;
use crate::nodes::{
    BindingId, Block, FunctionAssignment, FunctionStatement, GenericForStatement, Identifier,
    NumericForStatement, Statement,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

/// Identifies a node in a [`SyntaxIndex`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The kind of node an index entry describes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A block.
    Block,
    /// A statement.
    Statement,
    /// An identifier occurrence.
    Identifier,
}

/// One indexed node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    /// The node's identifier inside the index.
    pub id: NodeId,
    /// The kind of node.
    pub kind: NodeKind,
    /// The node's byte span.
    pub span: TextSpan,
    /// The binding of an identifier occurrence, when it has one.
    pub binding: Option<BindingId>,
}

/// An error produced while building a [`SyntaxIndex`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationError {
    /// The tree has no token data, so spans are unavailable.
    MissingTokens,
    /// The tree was not analyzed, so block identities are unavailable.
    UnanalyzedBlock,
}

impl fmt::Display for NavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NavigationError::MissingTokens => {
                write!(f, "the tree has no token data; parse with preserve_tokens")
            }
            NavigationError::UnanalyzedBlock => {
                write!(f, "the tree was not analyzed")
            }
        }
    }
}

impl std::error::Error for NavigationError {}

impl NavigationError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            NavigationError::MissingTokens => "navigation/missing-tokens",
            NavigationError::UnanalyzedBlock => "navigation/unanalyzed-block",
        }
    }
}

/// A navigable index of the nodes of an analyzed tree.
#[derive(Clone, Debug, Default)]
pub struct SyntaxIndex {
    entries: Vec<IndexEntry>,
}

impl SyntaxIndex {
    /// Builds an index over `block`.
    pub fn build(block: &Block) -> Result<Self, NavigationError> {
        let mut copy = block.clone();
        let mut builder = IndexBuilder {
            entries: Vec::new(),
            has_tokens: false,
            missing_identity: false,
        };
        DefaultVisitor::visit_block(&mut copy, &mut builder);

        if !builder.has_tokens {
            return Err(NavigationError::MissingTokens);
        }
        if builder.missing_identity {
            return Err(NavigationError::UnanalyzedBlock);
        }

        // Deduplicate entries that describe the same node (a function name is
        // reached both as a name and as an identifier occurrence).
        let mut seen = HashSet::new();
        builder.entries.retain(|entry| {
            seen.insert((entry.kind, entry.span.start, entry.span.end, entry.binding))
        });
        for (index, entry) in builder.entries.iter_mut().enumerate() {
            entry.id = NodeId(index as u32);
        }

        Ok(Self {
            entries: builder.entries,
        })
    }

    /// Returns the number of indexed nodes.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over every indexed node.
    pub fn iter(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.iter()
    }

    /// Returns the entry for a node.
    pub fn entry(&self, id: NodeId) -> Option<&IndexEntry> {
        self.entries.get(id.index())
    }

    /// Returns the span of a node.
    pub fn span(&self, id: NodeId) -> Option<TextSpan> {
        self.entry(id).map(|entry| entry.span)
    }

    /// Returns the kind of a node.
    pub fn kind(&self, id: NodeId) -> Option<NodeKind> {
        self.entry(id).map(|entry| entry.kind)
    }

    /// Returns the binding of an identifier node.
    pub fn binding(&self, id: NodeId) -> Option<BindingId> {
        self.entry(id).and_then(|entry| entry.binding)
    }

    /// Returns the most specific node whose span contains `offset`.
    ///
    /// Ties are broken towards identifiers, then statements, then blocks.
    pub fn node_at(&self, offset: usize) -> Option<NodeId> {
        self.entries
            .iter()
            .filter(|entry| entry.span.contains(offset))
            .min_by_key(|entry| {
                (
                    entry.span.len(),
                    match entry.kind {
                        NodeKind::Identifier => 0,
                        NodeKind::Statement => 1,
                        NodeKind::Block => 2,
                    },
                )
            })
            .map(|entry| entry.id)
    }

    /// Returns the nodes that strictly contain `id`, innermost first.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let Some(target) = self.entry(id) else {
            return Vec::new();
        };
        let mut ancestors: Vec<&IndexEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.id != id
                    && entry.span.start <= target.span.start
                    && entry.span.end >= target.span.end
                    && entry.span.len() > target.span.len()
            })
            .collect();
        ancestors.sort_by_key(|entry| entry.span.len());
        ancestors.into_iter().map(|entry| entry.id).collect()
    }

    /// Returns the identifier occurrence at `offset`, if any.
    pub fn identifier_at(&self, offset: usize) -> Option<&IndexEntry> {
        self.node_at(offset)
            .and_then(|id| self.entry(id))
            .filter(|entry| entry.kind == NodeKind::Identifier)
    }
}

struct IndexBuilder {
    entries: Vec<IndexEntry>,
    has_tokens: bool,
    missing_identity: bool,
}

impl IndexBuilder {
    fn push(&mut self, kind: NodeKind, span: Option<TextSpan>, binding: Option<BindingId>) {
        match span {
            Some(span) => {
                self.has_tokens = true;
                self.entries.push(IndexEntry {
                    id: NodeId(0),
                    kind,
                    span,
                    binding,
                });
            }
            None => {
                // The tree was parsed without token data; the caller decides
                // whether that is fatal.
            }
        }
    }

    fn identifier(&mut self, identifier: &Identifier) {
        let binding = identifier.get_binding();
        let span = identifier.get_token().and_then(|token| token.get_span());
        self.push(NodeKind::Identifier, span, binding);
    }
}

impl NodeProcessor for IndexBuilder {
    fn process_block(&mut self, block: &mut Block) {
        if block.get_identity().is_none() {
            self.missing_identity = true;
        }

        let span = block_span(block);
        self.push(NodeKind::Block, span, None);

        let mut spans = Vec::new();
        for statement in block.iter_mut_statements() {
            spans.push(statement_span(statement));
        }
        for span in spans {
            self.push(NodeKind::Statement, span, None);
        }
    }

    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        self.identifier(identifier);
    }

    fn process_local_assign_statement(&mut self, statement: &mut crate::nodes::VariableAssignment) {
        for variable in statement.iter_variables() {
            self.identifier(variable.get_identifier());
        }
    }

    fn process_local_function_statement(&mut self, statement: &mut FunctionAssignment) {
        self.identifier(statement.get_identifier());
        for parameter in statement.iter_parameters() {
            self.identifier(parameter.get_identifier());
        }
    }

    fn process_function_statement(&mut self, statement: &mut FunctionStatement) {
        let name = statement.get_name();
        self.identifier(name.get_name());
        for field in name.get_field_names() {
            self.identifier(field);
        }
        if let Some(method) = name.get_method() {
            self.identifier(method);
        }
        for parameter in statement.iter_parameters() {
            self.identifier(parameter.get_identifier());
        }
    }

    fn process_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        self.identifier(statement.get_identifier().get_identifier());
    }

    fn process_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        for identifier in statement.iter_identifiers() {
            self.identifier(identifier.get_identifier());
        }
    }
}

fn block_span(block: &mut Block) -> Option<TextSpan> {
    let start = block.mutate_first_token().get_start()?;
    let end = block.mutate_last_token().get_end()?;
    Some(TextSpan::new(start, end))
}

fn statement_span(statement: &mut Statement) -> Option<TextSpan> {
    let start = statement.mutate_first_token().get_start()?;
    let end = statement.mutate_last_token().get_end()?;
    Some(TextSpan::new(start, end))
}
