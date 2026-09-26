//! Sound loop-invariant code motion analysis.
//!
//! The analysis consumes [`DominatorTree`] natural loops and proves, for every
//! candidate statement, the three strict preconditions of hoisting:
//!
//! 1. **Strict purity** — the assigned expression contains no calls, no
//!    metamethods and no mutable table access ([`Evaluator::is_pure`]).
//! 2. **Loop invariance** — every symbol the expression reads is declared
//!    outside the natural loop, is not defined anywhere inside the loop (the
//!    loop header's `for` iteration variables included), and is not captured
//!    by a closure that could be invoked from the loop.
//! 3. **Per-iteration freshness** — the assigned local is not captured by any
//!    closure, preserving Lua's per-iteration binding semantics.
//!
//! Additionally, a candidate is only considered when its block dominates every
//! latch of the loop, so hoisting cannot add an evaluation on a path that
//! previously skipped the statement. Anything that cannot be strictly proven
//! is left in place; the analysis never guesses.
//!
//! The analysis only computes *what* can move. The rewrite lives in
//! [`hoist_loop_invariants`](crate::rules::canonicalize::hoist_loop_invariants),
//! which consumes this analysis and moves the statements with fresh names.

use std::collections::{HashMap, HashSet};

use crate::cfg::{BlockId, ControlFlowGraph, StatementOrigin};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId, SymbolKind};

use super::{DominatorTree, Liveness, LivenessError};

/// An error produced while computing loop-invariant code motion candidates.
#[derive(Clone, Debug, PartialEq)]
pub enum LicmError {
    /// Liveness could not be computed.
    Liveness(LivenessError),
    /// A basic block contains a statement but records no structural origin for
    /// it, so the analysis cannot tell which AST statement to hoist. This means
    /// the graph's provenance is incomplete.
    MissingStatementOrigin {
        /// The block that is missing an origin.
        block: BlockId,
        /// The statement index within the block.
        index: usize,
    },
}

impl From<LivenessError> for LicmError {
    fn from(error: LivenessError) -> Self {
        Self::Liveness(error)
    }
}

impl std::fmt::Display for LicmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LicmError::Liveness(error) => write!(f, "{}", error),
            LicmError::MissingStatementOrigin { block, index } => write!(
                f,
                "block {:?} statement {} has no structural origin; the control flow graph provenance is incomplete",
                block, index
            ),
        }
    }
}

impl std::error::Error for LicmError {}

/// A statement proven safe to hoist out of a natural loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HoistedStatement {
    /// The structural origin of the statement in the AST.
    pub origin: StatementOrigin,
    /// The local symbol the statement declares.
    pub symbol: SymbolId,
}

/// The statements that can be hoisted out of one loop, in emission order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopHoist {
    /// The structural origin of the loop statement (the insertion point's
    /// preceding statement in the enclosing block).
    pub loop_statement: StatementOrigin,
    /// The header block of the natural loop.
    pub header: BlockId,
    /// The hoistable statements.
    pub statements: Vec<HoistedStatement>,
}

/// The result of the loop-invariant analysis.
#[derive(Clone, Debug, Default)]
pub struct LoopInvariantAnalysis {
    hoists: Vec<LoopHoist>,
}

impl LoopInvariantAnalysis {
    /// Computes the hoistable statements for every natural loop of `cfg`.
    ///
    /// The analysis is conservative: a statement that cannot be proven to
    /// satisfy every precondition is not reported. Natural loops are processed
    /// innermost first so a statement is only ever assigned to the smallest
    /// loop that contains it.
    pub fn analyze(cfg: &ControlFlowGraph, model: &SemanticModel) -> Result<Self, LicmError> {
        let liveness = Liveness::analyze(cfg, model)?;
        let dominators = DominatorTree::analyze(cfg);
        let evaluator = Evaluator::default();

        let mut latches_by_header: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        for edge in dominators.back_edges() {
            latches_by_header
                .entry(edge.header)
                .or_default()
                .push(edge.latch);
        }

        let mut loops: Vec<(StatementOrigin, BlockId)> = cfg
            .loop_headers
            .iter()
            .map(|(origin, header)| (*origin, *header))
            .collect();
        loops.sort_by_key(|(origin, _)| *origin);

        // Innermost loops have fewer blocks; planning them first assigns each
        // statement to the smallest loop that contains it.
        let mut planned: Vec<(usize, StatementOrigin, Vec<BlockId>, HashSet<BlockId>)> = Vec::new();
        for (origin, header) in loops {
            let Some(latches) = latches_by_header.get(&header) else {
                continue;
            };
            let mut blocks: HashSet<BlockId> = HashSet::new();
            for latch in latches {
                if let Some(blocks_of_latch) = dominators.try_natural_loop_blocks(header, *latch) {
                    blocks.extend(blocks_of_latch.iter().copied());
                }
            }
            planned.push((blocks.len(), origin, latches.clone(), blocks));
        }
        planned.sort_by_key(|(size, origin, ..)| (*size, *origin));

        let mut assigned: HashSet<StatementOrigin> = HashSet::new();
        let mut hoists: Vec<LoopHoist> = Vec::new();

        for (_, loop_origin, latches, blocks) in planned {
            let loop_defs = definitions_in_blocks(&liveness, &blocks);

            let mut block_ids: Vec<BlockId> = blocks.iter().copied().collect();
            block_ids.sort();

            let mut statements = Vec::new();
            for block_id in block_ids {
                if !latches
                    .iter()
                    .all(|latch| dominators.dominates(block_id, *latch))
                {
                    continue;
                }

                let block = cfg.block(block_id);
                for (index, statement) in block.statements.iter().enumerate() {
                    let crate::nodes::Statement::LocalAssign(assignment) = statement else {
                        continue;
                    };

                    // Multi-assignment adjustment is intentionally out of scope:
                    // only the simple single-variable, single-value form is
                    // proven safe here.
                    if assignment.variables_len() != 1 || assignment.values_len() != 1 {
                        continue;
                    }

                    let Some(origin) = block.statement_origin(index) else {
                        return Err(LicmError::MissingStatementOrigin {
                            block: block_id,
                            index,
                        });
                    };
                    if assigned.contains(&origin) {
                        continue;
                    }

                    let identifier = assignment
                        .iter_variables()
                        .next()
                        .expect("the single identifier was checked above");
                    // A `<close>` local closes its value when its scope exits.
                    // Hoisting it out of the loop would close it once, at the
                    // enclosing scope, instead of once per iteration.
                    if identifier.get_attribute().is_some() {
                        continue;
                    }
                    let value = assignment
                        .iter_values()
                        .next()
                        .expect("the single value was checked above");

                    if !evaluator.is_pure(value) {
                        continue;
                    }

                    let symbol = model
                        .resolve_identifier(identifier.get_identifier())
                        .expect("liveness resolved every identifier in the graph");
                    let info = model.symbol(symbol);
                    if info.kind != SymbolKind::Local || info.is_captured() {
                        continue;
                    }

                    if !reads_are_loop_invariant(value, model, &loop_defs) {
                        continue;
                    }

                    statements.push(HoistedStatement { origin, symbol });
                    assigned.insert(origin);
                }
            }

            if !statements.is_empty() {
                let header = cfg
                    .loop_headers
                    .get(&loop_origin)
                    .copied()
                    .expect("the loop origin was collected from the same map");
                hoists.push(LoopHoist {
                    loop_statement: loop_origin,
                    header,
                    statements,
                });
            }
        }

        Ok(Self { hoists })
    }

    /// Returns the loops with at least one hoistable statement.
    #[inline]
    pub fn hoists(&self) -> &[LoopHoist] {
        &self.hoists
    }

    /// Returns whether no statement can be hoisted.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hoists.is_empty()
    }
}

fn definitions_in_blocks(liveness: &Liveness, blocks: &HashSet<BlockId>) -> HashSet<SymbolId> {
    let mut defs = HashSet::new();
    for block in blocks {
        for bit in liveness.defs[block.index()].iter_ones() {
            defs.insert(SymbolId(bit as u32));
        }
    }
    defs
}

fn reads_are_loop_invariant(
    value: &crate::nodes::Expression,
    model: &SemanticModel,
    loop_defs: &HashSet<SymbolId>,
) -> bool {
    struct ReadsCollector<'a> {
        model: &'a SemanticModel,
        symbols: Vec<SymbolId>,
    }

    impl NodeProcessor for ReadsCollector<'_> {
        fn process_variable_expression(&mut self, identifier: &mut crate::nodes::Identifier) {
            let symbol = self
                .model
                .resolve_identifier(identifier)
                .expect("liveness resolved every identifier in the graph");
            self.symbols.push(symbol);
        }
    }

    let mut value = value.clone();
    let mut collector = ReadsCollector {
        model,
        symbols: Vec::new(),
    };
    DefaultVisitor::visit_expression(&mut value, &mut collector);

    collector.symbols.iter().all(|symbol| {
        let info = model.symbol(*symbol);
        info.kind != SymbolKind::Global && !info.is_captured() && !loop_defs.contains(symbol)
    })
}
