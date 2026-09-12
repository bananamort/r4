use crate::cfg::{BlockId, ControlFlowGraph};

/// The dominator tree of a [`ControlFlowGraph`].
///
/// A block `d` dominates a block `b` when every path from the entry to `b`
/// passes through `d`. Every block is dominated by the entry and by itself.
#[derive(Clone, Debug)]
pub struct DominatorTree {
    idom: Vec<Option<BlockId>>,
    children: Vec<Vec<BlockId>>,
    reverse_postorder_index: Vec<Option<usize>>,
    reverse_postorder: Vec<BlockId>,
}

impl DominatorTree {
    /// Computes the dominator tree of a graph using the Cooper-Harvey-Kennedy
    /// iterative algorithm. Unreachable blocks have no dominator.
    pub fn analyze(cfg: &ControlFlowGraph) -> Self {
        let block_count = cfg.blocks.len();
        let reverse_postorder = reverse_postorder(cfg);

        let mut reverse_postorder_index = vec![None; block_count];
        for (index, block) in reverse_postorder.iter().enumerate() {
            reverse_postorder_index[block.index()] = Some(index);
        }

        let mut idom: Vec<Option<BlockId>> = vec![None; block_count];
        if !reverse_postorder.is_empty() {
            idom[cfg.entry.index()] = Some(cfg.entry);
        }

        let mut changed = true;
        while changed {
            changed = false;

            for &block in reverse_postorder.iter().skip(1) {
                let mut new_idom: Option<BlockId> = None;

                for &predecessor in cfg.predecessors(block) {
                    if idom[predecessor.index()].is_none() {
                        continue;
                    }
                    new_idom = Some(match new_idom {
                        None => predecessor,
                        Some(current) => {
                            intersect(predecessor, current, &idom, &reverse_postorder_index)
                        }
                    });
                }

                if new_idom.is_some() && idom[block.index()] != new_idom {
                    idom[block.index()] = new_idom;
                    changed = true;
                }
            }
        }

        let mut children = vec![Vec::new(); block_count];
        for (index, dominator) in idom.iter().enumerate() {
            if let Some(dominator) = dominator {
                if dominator.index() != index {
                    children[dominator.index()].push(BlockId(index as u32));
                }
            }
        }

        Self {
            idom,
            children,
            reverse_postorder_index,
            reverse_postorder,
        }
    }

    /// Returns the immediate dominator of a block, or `None` for the entry block
    /// and unreachable blocks.
    #[inline]
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.idom[block.index()].filter(|idom| idom.index() != block.index())
    }

    /// Returns the immediate dominator of a block, including the entry block
    /// (which maps to itself).
    #[inline]
    pub fn idom(&self, block: BlockId) -> Option<BlockId> {
        self.idom[block.index()]
    }

    /// Returns the children of a block in the dominator tree.
    #[inline]
    pub fn children_of(&self, block: BlockId) -> &[BlockId] {
        &self.children[block.index()]
    }

    /// Returns whether `dominator` dominates `block`.
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        let mut current = Some(block);
        while let Some(current_block) = current {
            if current_block == dominator {
                return self.idom[block.index()].is_some();
            }
            match self.idom[current_block.index()] {
                Some(idom) if idom != current_block => current = Some(idom),
                _ => break,
            }
        }
        false
    }

    /// Returns whether `dominator` strictly dominates `block` (dominates it and
    /// is not the same block).
    #[inline]
    pub fn strictly_dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        dominator != block && self.dominates(dominator, block)
    }

    /// Returns the blocks in reverse post-order used by the analysis.
    #[inline]
    pub fn reverse_postorder(&self) -> &[BlockId] {
        &self.reverse_postorder
    }

    /// Returns the reverse post-order index of a block, if it is reachable.
    #[inline]
    pub fn reverse_postorder_index(&self, block: BlockId) -> Option<usize> {
        self.reverse_postorder_index[block.index()]
    }
}

fn intersect(
    first: BlockId,
    second: BlockId,
    idom: &[Option<BlockId>],
    reverse_postorder_index: &[Option<usize>],
) -> BlockId {
    let mut finger1 = first;
    let mut finger2 = second;

    while finger1 != finger2 {
        while reverse_postorder_index[finger1.index()].expect("reachable block")
            > reverse_postorder_index[finger2.index()].expect("reachable block")
        {
            finger1 = idom[finger1.index()].expect("reachable block has a dominator");
        }

        while reverse_postorder_index[finger2.index()].expect("reachable block")
            > reverse_postorder_index[finger1.index()].expect("reachable block")
        {
            finger2 = idom[finger2.index()].expect("reachable block has a dominator");
        }
    }

    finger1
}

fn reverse_postorder(cfg: &ControlFlowGraph) -> Vec<BlockId> {
    let mut visited = vec![false; cfg.blocks.len()];
    let mut postorder = Vec::new();
    let mut stack = vec![(cfg.entry, 0usize)];

    visited[cfg.entry.index()] = true;

    while let Some((block, next_successor)) = stack.pop() {
        let successors = cfg.successors(block);
        if next_successor < successors.len() {
            let successor = successors[next_successor];
            stack.push((block, next_successor + 1));
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                stack.push((successor, 0));
            }
        } else {
            postorder.push(block);
        }
    }

    postorder.reverse();
    postorder
}
