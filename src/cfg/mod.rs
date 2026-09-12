//! Control flow graph construction and analysis.
//!
//! The entry point is [`build_chunk`], which lowers a parsed
//! [`Block`](crate::nodes::Block) together with its
//! [`SemanticModel`](crate::semantic::SemanticModel) into a
//! [`ControlFlowGraph`]. The [`analysis`] module provides reachability,
//! dominator and liveness passes on top of it.

mod block;
mod builder;
mod graph;

pub mod analysis;

pub use block::{BasicBlock, BlockId, Terminator};
pub use builder::{build_chunk, build_function, CfgBuilder, CfgError, CfgErrorKind};
pub use graph::{ControlFlowGraph, InvariantViolation};
