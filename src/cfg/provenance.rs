//! Provenance linking control flow graph contents back to AST nodes.
//!
//! Basic blocks store cloned statements and branch conditions. To let
//! reachability-driven passes rewrite the right source nodes without relying
//! on token positions, the builder records the *structural origin* of every
//! lowered statement and branch condition:
//!
//! * semantic analysis stamps every [`Block`](crate::nodes::Block) with a
//!   unique [`BlockIdentity`],
//! * a statement origin is the pair of its block identity and its index inside
//!   that block,
//! * a condition origin additionally records which construct produced the
//!   branch condition (`if`/`elseif` branch, `while` or `repeat`).
//!
//! Origins work on token-less trees: no source position is involved.

use std::fmt;

use crate::nodes::{Block, BlockIdentity};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::utils::lines;

/// The structural position of a statement in the analyzed AST.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatementOrigin {
    /// The block that contains the statement.
    pub block: BlockIdentity,
    /// The index of the statement inside that block.
    pub index: u32,
}

/// The AST construct that produced a branch condition.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConditionKind {
    /// The `index`-th condition of an `if`/`elseif` chain.
    IfBranch(u32),
    /// The condition of a `while` statement.
    While,
    /// The condition of a `repeat ... until` statement.
    Repeat,
}

/// The structural position of a branch terminator's condition.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConditionOrigin {
    /// The block that contains the statement owning the condition.
    pub block: BlockIdentity,
    /// The index of the statement inside that block.
    pub statement: u32,
    /// Which condition of the statement this is.
    pub kind: ConditionKind,
}

impl ConditionOrigin {
    /// Builds a condition origin from the origin of the owning statement.
    #[inline]
    pub fn new(statement: StatementOrigin, kind: ConditionKind) -> Self {
        Self {
            block: statement.block,
            statement: statement.index,
            kind,
        }
    }
}

/// An AST block that has no structural identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnidentifiedBlock {
    /// The source line of the first statement in the block, when token data is
    /// available.
    pub line: Option<usize>,
}

impl fmt::Display for UnidentifiedBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(
                f,
                "line {}: block has no structural identity; the semantic model does not match the AST",
                line
            ),
            None => write!(
                f,
                "block has no structural identity; the semantic model does not match the AST"
            ),
        }
    }
}

impl std::error::Error for UnidentifiedBlock {}

/// Verifies that every block reachable from `block` carries a structural
/// identity.
///
/// Semantic analysis stamps every block it visits; a missing identity means
/// the tree was not produced by the model that is being used, so structural
/// provenance cannot be trusted.
pub fn validate_block_identities(block: &mut Block) -> Result<(), UnidentifiedBlock> {
    struct Validator {
        error: Option<UnidentifiedBlock>,
    }

    impl NodeProcessor for Validator {
        fn process_block(&mut self, block: &mut Block) {
            if self.error.is_some() || block.get_identity().is_some() {
                return;
            }
            let line = block
                .first_statement()
                .and_then(lines::first_statement_token)
                .and_then(|token| token.get_line_number());
            self.error = Some(UnidentifiedBlock { line });
        }
    }

    let mut validator = Validator { error: None };
    DefaultVisitor::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
