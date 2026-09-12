//! Data-flow analyses over a [`ControlFlowGraph`](crate::cfg::ControlFlowGraph).

mod dominators;
mod liveness;
mod reachability;

pub use dominators::DominatorTree;
pub use liveness::{Liveness, LivenessError};
pub use reachability::Reachability;
