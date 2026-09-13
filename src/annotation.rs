//! Non-intrusive syntax annotations.
//!
//! Annotations are metadata attached to syntax nodes without changing the AST
//! representation: they live in a side table keyed by a structural
//! [`NodeAddress`]. An address refers to a node by the identity the analyzer
//! already stamped on the tree (a [`BlockIdentity`] for blocks) or by a
//! deterministic structural position (a statement index, or the pre-order
//! expression index inside a statement). Resolution never guesses: a stale or
//! out-of-range address is a typed [`AnnotationError`], never a silent miss.
//!
//! Validation reuses the single complete read-only traversal
//! ([`crate::process::ImmutableVisitor`]); it does not hand-write a second walk.
//!
//! This layer is agnostic of parsing, analysis and code generation; it only
//! reads an analyzed [`Block`].

use std::collections::HashMap;
use std::fmt;

use crate::nodes::{BindingId, Block, BlockIdentity, Identifier, Statement};
use crate::process::{ImmutableProcessor, ImmutableVisitor, Walk};

/// A metadata tag attached to a syntax node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxAnnotation {
    /// The annotation kind, for example `"keep"` or `"hoisted"`.
    pub kind: Option<String>,
    /// Free-form payload associated with the annotation.
    pub data: Option<String>,
}

impl SyntaxAnnotation {
    /// Creates an annotation with a kind and no payload.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: Some(kind.into()),
            data: None,
        }
    }

    /// Creates an annotation with a kind and a payload.
    pub fn with_data(kind: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: Some(kind.into()),
            data: Some(data.into()),
        }
    }

    /// Creates an annotation with neither kind nor payload.
    pub fn empty() -> Self {
        Self {
            kind: None,
            data: None,
        }
    }
}

/// A structural address of a syntax node.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NodeAddress {
    /// A block, addressed by its structural identity.
    Block(BlockIdentity),
    /// A statement in a block, addressed by its index.
    Statement {
        /// The containing block.
        block: BlockIdentity,
        /// The statement index inside the block.
        index: u32,
    },
    /// An expression, addressed by its pre-order index inside a statement.
    Expression {
        /// The containing block.
        block: BlockIdentity,
        /// The statement index inside the block.
        statement: u32,
        /// The pre-order expression index inside the statement.
        index: u32,
    },
    /// The condition of an `if`, `while` or `repeat` statement.
    Condition {
        /// The containing block.
        block: BlockIdentity,
        /// The statement index inside the block.
        statement: u32,
    },
    /// An identifier occurrence, addressed by the binding it resolves to.
    Identifier(BindingId),
}

/// An error produced while validating an annotation address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnotationError {
    /// The addressed block does not exist in this tree.
    UnknownBlock {
        /// The missing block identity.
        identity: BlockIdentity,
    },
    /// The addressed statement does not exist in its block.
    UnknownStatement {
        /// The containing block.
        block: BlockIdentity,
        /// The out-of-range statement index.
        index: u32,
    },
    /// The addressed expression does not exist in its statement.
    UnknownExpression {
        /// The containing block.
        block: BlockIdentity,
        /// The containing statement.
        statement: u32,
        /// The out-of-range expression index.
        index: u32,
    },
    /// The addressed statement has no condition.
    MissingCondition {
        /// The containing block.
        block: BlockIdentity,
        /// The statement without a condition.
        statement: u32,
    },
    /// The addressed binding has no identifier occurrence in this tree.
    UnknownIdentifier {
        /// The missing binding.
        binding: BindingId,
    },
}

impl fmt::Display for AnnotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnnotationError::UnknownBlock { identity } => {
                write!(f, "annotation block {:?} does not exist", identity)
            }
            AnnotationError::UnknownStatement { block, index } => write!(
                f,
                "annotation statement {} does not exist in block {:?}",
                index, block
            ),
            AnnotationError::UnknownExpression {
                block,
                statement,
                index,
            } => write!(
                f,
                "annotation expression {} does not exist in statement {} of block {:?}",
                index, statement, block
            ),
            AnnotationError::MissingCondition { block, statement } => write!(
                f,
                "annotation condition does not exist on statement {} of block {:?}",
                statement, block
            ),
            AnnotationError::UnknownIdentifier { binding } => {
                write!(f, "annotation identifier {:?} does not exist", binding)
            }
        }
    }
}

impl std::error::Error for AnnotationError {}

impl AnnotationError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            AnnotationError::UnknownBlock { .. } => "annotation/unknown-block",
            AnnotationError::UnknownStatement { .. } => "annotation/unknown-statement",
            AnnotationError::UnknownExpression { .. } => "annotation/unknown-expression",
            AnnotationError::MissingCondition { .. } => "annotation/missing-condition",
            AnnotationError::UnknownIdentifier { .. } => "annotation/unknown-identifier",
        }
    }
}

/// A side table of annotations keyed by structural node address.
#[derive(Clone, Debug, Default)]
pub struct SyntaxAnnotations {
    entries: HashMap<NodeAddress, Vec<SyntaxAnnotation>>,
}

impl SyntaxAnnotations {
    /// Creates an empty annotation table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an annotation to an address.
    pub fn insert(&mut self, address: NodeAddress, annotation: SyntaxAnnotation) {
        self.entries.entry(address).or_default().push(annotation);
    }

    /// Returns the annotations attached to an address.
    pub fn get(&self, address: &NodeAddress) -> &[SyntaxAnnotation] {
        self.entries.get(address).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Returns whether the address has any annotation.
    pub fn contains(&self, address: &NodeAddress) -> bool {
        self.entries.contains_key(address)
    }

    /// Removes and returns the annotations attached to an address.
    pub fn remove(&mut self, address: &NodeAddress) -> Vec<SyntaxAnnotation> {
        self.entries.remove(address).unwrap_or_default()
    }

    /// Returns the number of annotated addresses.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether there are no annotations.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over the annotated addresses and their annotations.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeAddress, &[SyntaxAnnotation])> {
        self.entries
            .iter()
            .map(|(address, values)| (address, values.as_slice()))
    }

    /// Removes every annotation.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Resolves every address against `root`, failing on the first stale one.
    ///
    /// A pass that changed the tree must call this instead of assuming its
    /// annotations still point at the nodes it tagged. Addresses are structural,
    /// so re-analyzing an unchanged tree preserves them; a structural change is
    /// reported as a typed error instead of a silent miss.
    pub fn validate(&self, root: &Block) -> Result<(), AnnotationError> {
        for address in self.entries.keys() {
            self.validate_address(root, address)?;
        }
        Ok(())
    }

    fn validate_address(&self, root: &Block, address: &NodeAddress) -> Result<(), AnnotationError> {
        match address {
            NodeAddress::Block(identity) => {
                let mut checker = BlockChecker {
                    target: *identity,
                    found: false,
                };
                Walk::visit_block(root, &mut checker);
                if checker.found {
                    Ok(())
                } else {
                    Err(AnnotationError::UnknownBlock {
                        identity: *identity,
                    })
                }
            }
            NodeAddress::Statement { block, index } => {
                let mut checker = StatementChecker {
                    block: *block,
                    index: *index,
                    block_seen: false,
                    found: false,
                };
                Walk::visit_block(root, &mut checker);
                match (checker.block_seen, checker.found) {
                    (false, _) => Err(AnnotationError::UnknownBlock { identity: *block }),
                    (true, false) => Err(AnnotationError::UnknownStatement {
                        block: *block,
                        index: *index,
                    }),
                    (true, true) => Ok(()),
                }
            }
            NodeAddress::Expression {
                block,
                statement,
                index,
            } => {
                let mut checker = ExpressionChecker {
                    block: *block,
                    statement: *statement,
                    index: *index,
                    block_seen: false,
                    found: false,
                };
                Walk::visit_block(root, &mut checker);
                match (checker.block_seen, checker.found) {
                    (false, _) => Err(AnnotationError::UnknownBlock { identity: *block }),
                    (true, false) => Err(AnnotationError::UnknownExpression {
                        block: *block,
                        statement: *statement,
                        index: *index,
                    }),
                    (true, true) => Ok(()),
                }
            }
            NodeAddress::Condition { block, statement } => {
                let mut checker = ConditionChecker {
                    block: *block,
                    statement: *statement,
                    block_seen: false,
                    found: false,
                };
                Walk::visit_block(root, &mut checker);
                match (checker.block_seen, checker.found) {
                    (false, _) => Err(AnnotationError::UnknownBlock { identity: *block }),
                    (true, false) => Err(AnnotationError::MissingCondition {
                        block: *block,
                        statement: *statement,
                    }),
                    (true, true) => Ok(()),
                }
            }
            NodeAddress::Identifier(binding) => {
                let mut finder = IdentifierFinder {
                    binding: *binding,
                    found: false,
                };
                Walk::visit_block(root, &mut finder);
                if finder.found {
                    Ok(())
                } else {
                    Err(AnnotationError::UnknownIdentifier { binding: *binding })
                }
            }
        }
    }
}

fn has_condition(statement: &Statement) -> bool {
    match statement {
        Statement::If(statement) => statement.iter_branches().next().is_some(),
        Statement::While(_) | Statement::Repeat(_) => true,
        _ => false,
    }
}

struct BlockChecker {
    target: BlockIdentity,
    found: bool,
}

impl ImmutableProcessor for BlockChecker {
    fn process_block(&mut self, block: &Block) {
        if block.get_identity() == Some(self.target) {
            self.found = true;
        }
    }
}

struct StatementChecker {
    block: BlockIdentity,
    index: u32,
    block_seen: bool,
    found: bool,
}

impl ImmutableProcessor for StatementChecker {
    fn process_block(&mut self, block: &Block) {
        if block.get_identity() != Some(self.block) {
            return;
        }
        self.block_seen = true;
        if (self.index as usize) < block.iter_statements().count() {
            self.found = true;
        }
    }
}

struct ExpressionChecker {
    block: BlockIdentity,
    statement: u32,
    index: u32,
    block_seen: bool,
    found: bool,
}

impl ImmutableProcessor for ExpressionChecker {
    fn process_block(&mut self, block: &Block) {
        if block.get_identity() != Some(self.block) {
            return;
        }
        self.block_seen = true;
        let Some(statement) = block.iter_statements().nth(self.statement as usize) else {
            return;
        };
        let mut counter = ExpressionCounter { count: 0 };
        Walk::visit_statement(statement, &mut counter);
        if counter.count > self.index {
            self.found = true;
        }
    }
}

struct ConditionChecker {
    block: BlockIdentity,
    statement: u32,
    block_seen: bool,
    found: bool,
}

impl ImmutableProcessor for ConditionChecker {
    fn process_block(&mut self, block: &Block) {
        if block.get_identity() != Some(self.block) {
            return;
        }
        self.block_seen = true;
        if let Some(statement) = block.iter_statements().nth(self.statement as usize) {
            self.found = has_condition(statement);
        }
    }
}

struct ExpressionCounter {
    count: u32,
}

impl ImmutableProcessor for ExpressionCounter {
    fn process_expression(&mut self, _: &crate::nodes::Expression) {
        self.count += 1;
    }
}

struct IdentifierFinder {
    binding: BindingId,
    found: bool,
}

impl ImmutableProcessor for IdentifierFinder {
    fn process_identifier(&mut self, identifier: &Identifier) {
        if identifier.get_binding() == Some(self.binding) {
            self.found = true;
        }
    }
}
