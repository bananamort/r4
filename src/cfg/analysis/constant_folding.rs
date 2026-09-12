//! Constant-aware control-flow simplification.

use crate::nodes::{Expression, Identifier};
use crate::process::Evaluator;

use crate::cfg::{BlockId, ControlFlowGraph, Terminator};

/// Evaluates branch conditions that are compile-time constants and rewrites
/// them into unconditional jumps, detaching the impossible edge.
///
/// Folding a condition such as `if false then ... end` or `while true do ... end`
/// makes the corresponding region unreachable, which
/// [`Reachability`](super::Reachability) then reports as dead blocks.
///
/// Returns the number of branches that were folded. The graph invariants hold
/// after folding (see [`ControlFlowGraph::verify_invariants`]).
pub fn fold_constant_branches(cfg: &mut ControlFlowGraph) -> usize {
    let evaluator = Evaluator::default();
    fold_constant_branches_with(cfg, move |_, condition| {
        evaluator
            .evaluate_with(condition, &|_: &Identifier| None)
            .and_then(|value| value.is_truthy())
    })
}

/// Like [`fold_constant_branches`], but lets the caller decide the value of each
/// branch condition. `condition_value` receives the branch block and its
/// condition, and returns `Some(bool)` when the condition is known.
///
/// This is the entry point for constant propagation: callers can resolve
/// identifiers through a reaching-definition environment (see
/// [`Super::fold_constant_branches_with_propagation`]).
pub fn fold_constant_branches_with<F>(cfg: &mut ControlFlowGraph, mut condition_value: F) -> usize
where
    F: FnMut(BlockId, &Expression) -> Option<bool>,
{
    let mut folded = 0;

    for index in 0..cfg.blocks.len() {
        let block = BlockId(index as u32);

        let replacement = match cfg.terminator(block) {
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => match condition_value(block, condition) {
                Some(true) => Some(*then_block),
                Some(false) => Some(*else_block),
                None => None,
            },
            _ => None,
        };

        if let Some(target) = replacement {
            cfg.set_terminator(block, Terminator::Jump(target));
            folded += 1;
        }
    }

    folded
}
