//! Control flow graph construction and analysis.
//!
//! The entry point is [`build_chunk`], which lowers a parsed
//! [`Block`](crate::nodes::Block) together with its
//! [`SemanticModel`](crate::semantic::SemanticModel) into a
//! [`ControlFlowGraph`]. The [`analysis`] module provides reachability,
//! dominator, liveness and constant propagation passes on top of it, and the
//! provenance helpers link graph contents back to AST nodes through structural
//! origins recorded during lowering.

mod block;
mod builder;
mod graph;
mod provenance;

pub mod analysis;

pub use block::{BasicBlock, BlockId, Terminator};
pub use builder::{build_chunk, build_function, CfgBuilder, CfgError, CfgErrorKind};
pub use graph::{ControlFlowGraph, InvariantViolation};
pub use provenance::{
    validate_block_identities, ConditionKind, ConditionOrigin, StatementOrigin, UnidentifiedBlock,
};
