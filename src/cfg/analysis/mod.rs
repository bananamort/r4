//! Data-flow analyses over a [`ControlFlowGraph`](crate::cfg::ControlFlowGraph).

mod constant_folding;
mod constant_propagation;
mod dominators;
mod liveness;
mod reachability;

pub use constant_folding::{fold_constant_branches, fold_constant_branches_with};
pub use constant_propagation::{
    evaluate_with_environment, fold_constant_branches_with_propagation, ConstantPropagation,
    ConstantPropagationError,
};
pub use dominators::DominatorTree;
pub use liveness::{Liveness, LivenessError};
pub use reachability::Reachability;
