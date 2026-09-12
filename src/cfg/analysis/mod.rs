//! Data-flow analyses over a [`ControlFlowGraph`](crate::cfg::ControlFlowGraph).

mod constant_folding;
mod dominators;
mod liveness;
mod reachability;

pub use constant_folding::fold_constant_branches;
pub use dominators::DominatorTree;
pub use liveness::{Liveness, LivenessError};
pub use reachability::Reachability;
