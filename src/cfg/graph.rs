use std::collections::HashMap;

use crate::semantic::{LabelId, ScopeId};

use super::block::{BasicBlock, BlockId, Terminator};
use super::provenance::StatementOrigin;

/// A control flow graph for a single function (or the main chunk).
///
/// Blocks are stored in a flat arena and reference each other by [`BlockId`].
/// The `entry` block is where execution starts, and every `Return` terminator
/// is connected to the unique `exit` sink block.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlFlowGraph {
    /// All basic blocks, indexed by [`BlockId`].
    pub blocks: Vec<BasicBlock>,
    /// The unique entry block of the function/chunk.
    pub entry: BlockId,
    /// The unique exit sink of the function/chunk.
    pub exit: BlockId,
    /// Maps semantic labels to the basic block that starts at the label.
    pub label_blocks: HashMap<LabelId, BlockId>,
    /// Maps the structural origin of each loop statement to its header block.
    pub loop_headers: HashMap<StatementOrigin, BlockId>,
    /// The semantic function scope this graph was built for.
    pub function_scope: ScopeId,
}

impl ControlFlowGraph {
    /// Creates an empty graph with the given entry/exit blocks. Used internally
    /// by the builder.
    pub(crate) fn new(entry: BlockId, exit: BlockId, function_scope: ScopeId) -> Self {
        Self {
            blocks: Vec::new(),
            entry,
            exit,
            label_blocks: HashMap::new(),
            loop_headers: HashMap::new(),
            function_scope,
        }
    }

    /// Returns the block with the given identifier.
    #[inline]
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.index()]
    }

    /// Returns the number of blocks in this graph.
    #[inline]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns the successors of a block.
    #[inline]
    pub fn successors(&self, id: BlockId) -> &[BlockId] {
        &self.blocks[id.index()].successors
    }

    /// Returns the predecessors of a block.
    #[inline]
    pub fn predecessors(&self, id: BlockId) -> &[BlockId] {
        &self.blocks[id.index()].predecessors
    }

    /// Returns the terminator of a block.
    #[inline]
    pub fn terminator(&self, id: BlockId) -> &Terminator {
        &self.blocks[id.index()].terminator
    }

    /// Iterates over all block identifiers in creation order.
    pub fn iter_block_ids(&self) -> impl Iterator<Item = BlockId> {
        (0..self.blocks.len() as u32).map(BlockId)
    }

    /// Replaces the terminator of a block and rebuilds its edges, keeping the
    /// predecessor and successor lists consistent.
    ///
    /// This is the supported way for analysis passes to rewrite control flow:
    /// the old edges are detached before the new terminator targets are
    /// connected, so [`Self::verify_invariants`] continues to hold.
    ///
    /// Rewriting control flow invalidates the structural origin of the old
    /// terminator, so `branch_origin` is cleared.
    pub fn set_terminator(&mut self, block: BlockId, terminator: Terminator) {
        let old_successors = std::mem::take(&mut self.blocks[block.index()].successors);
        for successor in old_successors {
            self.blocks[successor.index()]
                .predecessors
                .retain(|predecessor| *predecessor != block);
        }

        let targets = terminator.targets(self.exit);
        self.blocks[block.index()].branch_origin = None;
        self.blocks[block.index()].terminator = terminator;

        for target in targets {
            if !self.blocks[block.index()].successors.contains(&target) {
                self.blocks[block.index()].successors.push(target);
                self.blocks[target.index()].predecessors.push(block);
            }
        }
    }

    /// Verifies the structural invariants of the graph:
    ///
    /// * every successor/predecessor reference points to an existing block,
    /// * edges are not duplicated,
    /// * the terminator targets and the successor list are consistent in both
    ///   directions, so every successor lists this block as a predecessor (and
    ///   vice versa).
    pub fn verify_invariants(&self) -> Result<(), Vec<InvariantViolation>> {
        let mut violations = Vec::new();

        for block in &self.blocks {
            let mut expected = block.terminator.targets(self.exit);
            expected.sort();
            expected.dedup();

            let mut successors = block.successors.clone();
            successors.sort();
            successors.dedup();

            if successors != expected {
                violations.push(InvariantViolation::TerminatorMismatch {
                    block: block.id,
                    terminator_targets: expected,
                    successors,
                });
            }

            if has_duplicates(&block.successors) {
                violations.push(InvariantViolation::DuplicateSuccessor { block: block.id });
            }

            if has_duplicates(&block.predecessors) {
                violations.push(InvariantViolation::DuplicatePredecessor { block: block.id });
            }

            if block.statement_origins.len() != block.statements.len() {
                violations.push(InvariantViolation::StatementOriginMismatch {
                    block: block.id,
                    statements: block.statements.len(),
                    origins: block.statement_origins.len(),
                });
            }

            for &successor in &block.successors {
                if successor.index() >= self.blocks.len() {
                    violations.push(InvariantViolation::DanglingSuccessor {
                        block: block.id,
                        successor,
                    });
                    continue;
                }

                if !self.blocks[successor.index()]
                    .predecessors
                    .contains(&block.id)
                {
                    violations.push(InvariantViolation::MissingPredecessor {
                        block: block.id,
                        successor,
                    });
                }
            }

            for &predecessor in &block.predecessors {
                if predecessor.index() >= self.blocks.len() {
                    violations.push(InvariantViolation::DanglingPredecessor {
                        block: block.id,
                        predecessor,
                    });
                    continue;
                }

                if !self.blocks[predecessor.index()]
                    .successors
                    .contains(&block.id)
                {
                    violations.push(InvariantViolation::MissingSuccessor {
                        block: block.id,
                        predecessor,
                    });
                }
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

fn has_duplicates(blocks: &[BlockId]) -> bool {
    for (index, block) in blocks.iter().enumerate() {
        if blocks[index + 1..].contains(block) {
            return true;
        }
    }
    false
}

/// A structural invariant violation found by
/// [`ControlFlowGraph::verify_invariants`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvariantViolation {
    /// The terminator targets do not match the successor list.
    TerminatorMismatch {
        /// The offending block.
        block: BlockId,
        /// The sorted, deduplicated terminator targets.
        terminator_targets: Vec<BlockId>,
        /// The sorted, deduplicated successors.
        successors: Vec<BlockId>,
    },
    /// A block lists the same successor twice.
    DuplicateSuccessor {
        /// The offending block.
        block: BlockId,
    },
    /// A block lists the same predecessor twice.
    DuplicatePredecessor {
        /// The offending block.
        block: BlockId,
    },
    /// A successor identifier is out of bounds.
    DanglingSuccessor {
        /// The offending block.
        block: BlockId,
        /// The out-of-bounds successor.
        successor: BlockId,
    },
    /// A predecessor identifier is out of bounds.
    DanglingPredecessor {
        /// The offending block.
        block: BlockId,
        /// The out-of-bounds predecessor.
        predecessor: BlockId,
    },
    /// A successor does not list this block as a predecessor.
    MissingPredecessor {
        /// The offending block.
        block: BlockId,
        /// The inconsistent successor.
        successor: BlockId,
    },
    /// A predecessor does not list this block as a successor.
    MissingSuccessor {
        /// The offending block.
        block: BlockId,
        /// The inconsistent predecessor.
        predecessor: BlockId,
    },
    /// The statement origin list is not parallel to the statement list.
    StatementOriginMismatch {
        /// The offending block.
        block: BlockId,
        /// The number of statements.
        statements: usize,
        /// The number of recorded origins.
        origins: usize,
    },
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvariantViolation::TerminatorMismatch {
                block,
                terminator_targets,
                successors,
            } => write!(
                f,
                "block {:?} terminator targets {:?} do not match successors {:?}",
                block, terminator_targets, successors
            ),
            InvariantViolation::DuplicateSuccessor { block } => {
                write!(f, "block {:?} has duplicate successors", block)
            }
            InvariantViolation::DuplicatePredecessor { block } => {
                write!(f, "block {:?} has duplicate predecessors", block)
            }
            InvariantViolation::DanglingSuccessor { block, successor } => {
                write!(
                    f,
                    "block {:?} references missing successor {:?}",
                    block, successor
                )
            }
            InvariantViolation::DanglingPredecessor { block, predecessor } => write!(
                f,
                "block {:?} references missing predecessor {:?}",
                block, predecessor
            ),
            InvariantViolation::MissingPredecessor { block, successor } => write!(
                f,
                "block {:?} has successor {:?} that does not list it as predecessor",
                block, successor
            ),
            InvariantViolation::MissingSuccessor { block, predecessor } => write!(
                f,
                "block {:?} has predecessor {:?} that does not list it as successor",
                block, predecessor
            ),
            InvariantViolation::StatementOriginMismatch {
                block,
                statements,
                origins,
            } => write!(
                f,
                "block {:?} has {} statements but {} statement origins",
                block, statements, origins
            ),
        }
    }
}
