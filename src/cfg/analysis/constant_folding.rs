//! Constant-aware control-flow simplification.

use crate::nodes::Expression;
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
    let mut folded = 0;

    for index in 0..cfg.blocks.len() {
        let block = BlockId(index as u32);

        let replacement = match cfg.terminator(block) {
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => match evaluate_condition(&evaluator, condition) {
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

fn evaluate_condition(evaluator: &Evaluator, condition: &Expression) -> Option<bool> {
    evaluator.evaluate(condition).is_truthy()
}
