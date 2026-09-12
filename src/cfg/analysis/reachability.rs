use std::collections::VecDeque;

use crate::cfg::{BlockId, ControlFlowGraph};

/// Reachability analysis over a [`ControlFlowGraph`].
///
/// Performs a breadth-first traversal from the graph entry and records which
/// blocks can be executed. Blocks that cannot be reached are dead code (for
/// example statements after a `return` or `break`).
#[derive(Clone, Debug)]
pub struct Reachability {
    reachable: Vec<bool>,
}

impl Reachability {
    /// Analyzes the graph starting from its entry block.
    pub fn analyze(cfg: &ControlFlowGraph) -> Self {
        let mut reachable = vec![false; cfg.blocks.len()];
        let mut queue = VecDeque::new();

        reachable[cfg.entry.index()] = true;
        queue.push_back(cfg.entry);

        while let Some(block) = queue.pop_front() {
            for &successor in cfg.successors(block) {
                if !reachable[successor.index()] {
                    reachable[successor.index()] = true;
                    queue.push_back(successor);
                }
            }
        }

        Self { reachable }
    }

    /// Returns whether a block can be executed.
    #[inline]
    pub fn is_reachable(&self, block: BlockId) -> bool {
        self.reachable[block.index()]
    }

    /// Returns whether a block is unreachable (dead code).
    #[inline]
    pub fn is_unreachable(&self, block: BlockId) -> bool {
        !self.is_reachable(block)
    }

    /// Iterates over all reachable blocks.
    pub fn reachable_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.reachable
            .iter()
            .enumerate()
            .filter(|(_, reachable)| **reachable)
            .map(|(index, _)| BlockId(index as u32))
    }

    /// Iterates over all unreachable (dead) blocks.
    pub fn unreachable_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.reachable
            .iter()
            .enumerate()
            .filter(|(_, reachable)| !**reachable)
            .map(|(index, _)| BlockId(index as u32))
    }
}
