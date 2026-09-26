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

    /// Converts this error into a structured diagnostic.
    pub fn to_diagnostic(&self) -> crate::diagnostic::Diagnostic {
        crate::diagnostic::Diagnostic::error(self.to_string()).with_code(self.code())
    }
}

impl From<AnnotationError> for crate::diagnostic::Diagnostic {
    fn from(error: AnnotationError) -> Self {
        error.to_diagnostic()
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
    ///
    /// The tree is traversed once to build an address index, so validation is
    /// linear in the tree plus the number of annotations, instead of one full
    /// traversal per annotation.
    pub fn validate(&self, root: &Block) -> Result<(), AnnotationError> {
        let index = AddressIndex::build(root);
        for address in self.entries.keys() {
            index.check(address)?;
        }
        Ok(())
    }
}

fn has_condition(statement: &Statement) -> bool {
    match statement {
        Statement::If(statement) => statement.iter_branches().next().is_some(),
        Statement::While(_) | Statement::Repeat(_) => true,
        _ => false,
    }
}

/// The pre-order expression count and condition shape of one statement.
struct StatementShape {
    expressions: u32,
    has_condition: bool,
}

/// The addresses that exist in a tree, built in a single traversal.
#[derive(Default)]
struct AddressIndex {
    /// Every block identity mapped to the shape of each of its statements.
    statements: HashMap<BlockIdentity, Vec<StatementShape>>,
    /// Every binding carried by an identifier in the tree.
    identifiers: std::collections::HashSet<BindingId>,
}

impl AddressIndex {
    fn build(root: &Block) -> Self {
        #[derive(Default)]
        struct Builder {
            index: AddressIndex,
        }

        impl ImmutableProcessor for Builder {
            fn process_block(&mut self, block: &Block) {
                let Some(identity) = block.get_identity() else {
                    return;
                };
                let shapes = block
                    .iter_statements()
                    .map(|statement| {
                        let mut counter = ExpressionCounter { count: 0 };
                        Walk::visit_statement(statement, &mut counter);
                        StatementShape {
                            expressions: counter.count,
                            has_condition: has_condition(statement),
                        }
                    })
                    .collect();
                self.index.statements.insert(identity, shapes);
            }

            fn process_identifier(&mut self, identifier: &Identifier) {
                if let Some(binding) = identifier.get_binding() {
                    self.index.identifiers.insert(binding);
                }
            }
        }

        let mut builder = Builder::default();
        Walk::visit_block(root, &mut builder);
        builder.index
    }

    fn check(&self, address: &NodeAddress) -> Result<(), AnnotationError> {
        match address {
            NodeAddress::Block(identity) => match self.statements.contains_key(identity) {
                true => Ok(()),
                false => Err(AnnotationError::UnknownBlock {
                    identity: *identity,
                }),
            },
            NodeAddress::Statement { block, index } => {
                let Some(shapes) = self.statements.get(block) else {
                    return Err(AnnotationError::UnknownBlock { identity: *block });
                };
                if (*index as usize) < shapes.len() {
                    Ok(())
                } else {
                    Err(AnnotationError::UnknownStatement {
                        block: *block,
                        index: *index,
                    })
                }
            }
            NodeAddress::Expression {
                block,
                statement,
                index,
            } => {
                let Some(shapes) = self.statements.get(block) else {
                    return Err(AnnotationError::UnknownBlock { identity: *block });
                };
                let Some(shape) = shapes.get(*statement as usize) else {
                    return Err(AnnotationError::UnknownExpression {
                        block: *block,
                        statement: *statement,
                        index: *index,
                    });
                };
                if *index < shape.expressions {
                    Ok(())
                } else {
                    Err(AnnotationError::UnknownExpression {
                        block: *block,
                        statement: *statement,
                        index: *index,
                    })
                }
            }
            NodeAddress::Condition { block, statement } => {
                let Some(shapes) = self.statements.get(block) else {
                    return Err(AnnotationError::UnknownBlock { identity: *block });
                };
                let Some(shape) = shapes.get(*statement as usize) else {
                    return Err(AnnotationError::MissingCondition {
                        block: *block,
                        statement: *statement,
                    });
                };
                if shape.has_condition {
                    Ok(())
                } else {
                    Err(AnnotationError::MissingCondition {
                        block: *block,
                        statement: *statement,
                    })
                }
            }
            NodeAddress::Identifier(binding) => match self.identifiers.contains(binding) {
                true => Ok(()),
                false => Err(AnnotationError::UnknownIdentifier { binding: *binding }),
            },
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
