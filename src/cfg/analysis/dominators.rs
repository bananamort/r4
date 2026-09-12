use std::collections::{HashMap, HashSet};

use crate::cfg::{BlockId, ControlFlowGraph};

/// A back edge identified by the natural loop analysis.
///
/// `latch -> header` is a back edge exactly when `header` strictly dominates
/// `latch`, which means every path from the entry to the latch passes through
/// the header.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BackEdge {
    /// The block whose terminator jumps back to the header.
    pub latch: BlockId,
    /// The loop header targeted by the back edge.
    pub header: BlockId,
}

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
    back_edges: Vec<BackEdge>,
    loop_headers: Vec<bool>,
    natural_loops: HashMap<(BlockId, BlockId), HashSet<BlockId>>,
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

        let mut tree = Self {
            idom,
            children,
            reverse_postorder_index,
            reverse_postorder,
            back_edges: Vec::new(),
            loop_headers: vec![false; block_count],
            natural_loops: HashMap::new(),
        };
        tree.compute_natural_loops(cfg);
        tree
    }

    /// Identifies back edges (`latch -> header` with `header` strictly
    /// dominating `latch`) and computes the exact natural loop of each.
    fn compute_natural_loops(&mut self, cfg: &ControlFlowGraph) {
        let mut back_edges = Vec::new();

        for block in cfg.iter_block_ids() {
            for &successor in cfg.successors(block) {
                if block != successor && self.strictly_dominates(successor, block) {
                    back_edges.push(BackEdge {
                        latch: block,
                        header: successor,
                    });
                }
            }
        }

        for edge in &back_edges {
            let mut blocks: HashSet<BlockId> = HashSet::new();
            blocks.insert(edge.header);
            blocks.insert(edge.latch);

            // Blocks that can reach the latch without going through the header.
            let mut stack = vec![edge.latch];
            while let Some(block) = stack.pop() {
                for &predecessor in cfg.predecessors(block) {
                    if blocks.insert(predecessor) {
                        stack.push(predecessor);
                    }
                }
            }

            // Exact natural loop: the block must also be dominated by the
            // header, which filters out irreducible-graph nodes.
            blocks.retain(|block| {
                *block == edge.header || self.strictly_dominates(edge.header, *block)
            });

            self.natural_loops.insert((edge.header, edge.latch), blocks);
            self.loop_headers[edge.header.index()] = true;
        }

        self.back_edges = back_edges;
    }

    /// Returns every back edge of the graph, in block order.
    #[inline]
    pub fn back_edges(&self) -> &[BackEdge] {
        &self.back_edges
    }

    /// Returns whether a block is the header of at least one natural loop.
    #[inline]
    pub fn is_loop_header(&self, block: BlockId) -> bool {
        self.loop_headers[block.index()]
    }

    /// Returns the exact natural loop of the back edge `latch -> header`, or
    /// `None` when that edge is not a back edge of the analyzed graph.
    #[inline]
    pub fn try_natural_loop_blocks(
        &self,
        header: BlockId,
        latch: BlockId,
    ) -> Option<&HashSet<BlockId>> {
        self.natural_loops.get(&(header, latch))
    }

    /// Returns the exact natural loop of the back edge `latch -> header`.
    ///
    /// The loop is `{ header } ∪ { w | header strictly dominates w and w can
    /// reach latch without going through header }`.
    ///
    /// # Panics
    ///
    /// Panics when `latch -> header` is not a back edge of the analyzed graph.
    /// Use [`Self::try_natural_loop_blocks`] to query without a precondition.
    pub fn natural_loop_blocks(&self, header: BlockId, latch: BlockId) -> HashSet<BlockId> {
        self.try_natural_loop_blocks(header, latch)
            .cloned()
            .unwrap_or_else(|| {
                panic!("edge {latch:?} -> {header:?} is not a natural loop back edge")
            })
    }

    /// Returns the dominator subtree of a block in pre-order, including the
    /// block itself.
    pub fn dominator_subtree(&self, block: BlockId) -> Vec<BlockId> {
        let mut result = Vec::new();
        let mut stack = vec![block];

        while let Some(current) = stack.pop() {
            result.push(current);
            for &child in self.children[current.index()].iter().rev() {
                stack.push(child);
            }
        }

        result
    }

    /// Returns whether `block` is provably unreachable given a predicate that
    /// marks known-unreachable blocks.
    ///
    /// This is exact: a block is unreachable when it or any strict dominator is
    /// known unreachable, because every path from the entry to the block passes
    /// through each strict dominator.
    pub fn is_provably_unreachable(
        &self,
        block: BlockId,
        is_unreachable: impl Fn(BlockId) -> bool,
    ) -> bool {
        if is_unreachable(block) {
            return true;
        }

        let mut current = self.idom(block);
        while let Some(dominator) = current {
            if is_unreachable(dominator) {
                return true;
            }
            let next = self.idom(dominator);
            if next == Some(dominator) {
                break;
            }
            current = next;
        }

        false
    }

    /// Returns the blocks proven unreachable when the edge
    /// `branch -> removed_target` is known never to be taken.
    ///
    /// The proof is exact and deliberately conservative:
    ///
    /// * the removed target must have no predecessor other than `branch`,
    ///   otherwise another path may reach it and nothing is proven;
    /// * the removal then propagates down the dominator tree, because every
    ///   block strictly dominated by an unreachable block is unreachable.
    ///
    /// # Panics
    ///
    /// Panics when `removed_target` is not a successor of `branch`.
    pub fn prune_after_removed_edge(
        &self,
        cfg: &ControlFlowGraph,
        branch: BlockId,
        removed_target: BlockId,
    ) -> HashSet<BlockId> {
        assert!(
            cfg.successors(branch).contains(&removed_target),
            "{removed_target:?} is not a successor of {branch:?}"
        );

        if cfg
            .predecessors(removed_target)
            .iter()
            .any(|predecessor| *predecessor != branch)
        {
            // Another path can still reach the target; no exact proof is
            // possible from this single edge.
            return HashSet::new();
        }

        let mut pruned = HashSet::new();
        pruned.insert(removed_target);

        let mut changed = true;
        while changed {
            changed = false;
            for block in cfg.iter_block_ids() {
                if pruned.contains(&block) {
                    continue;
                }
                if let Some(dominator) = self.idom(block) {
                    if dominator != block && pruned.contains(&dominator) {
                        pruned.insert(block);
                        changed = true;
                    }
                }
            }
        }

        pruned
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
