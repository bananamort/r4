use std::collections::HashSet;
use std::fmt;

use bitvec::prelude::*;

use crate::cfg::analysis::{Liveness, LivenessError};
use crate::cfg::{build_chunk, statement_token_key, CfgError};
use crate::nodes::{Block, Statement, Variable};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolKind, TokenKey};

/// An error produced while removing dead stores.
#[derive(Clone, Debug, PartialEq)]
pub enum DeadStoreError {
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Liveness could not be computed (usually because the AST has no token
    /// data).
    Liveness(LivenessError),
}

impl From<CfgError> for DeadStoreError {
    fn from(error: CfgError) -> Self {
        DeadStoreError::Cfg(error)
    }
}

impl From<LivenessError> for DeadStoreError {
    fn from(error: LivenessError) -> Self {
        DeadStoreError::Liveness(error)
    }
}

impl fmt::Display for DeadStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeadStoreError::Cfg(error) => write!(f, "{}", error),
            DeadStoreError::Liveness(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for DeadStoreError {}

/// Removes assignments whose value can never be observed.
///
/// An assignment is dead when the written local is not live after the write and
/// the assigned values have no side effects. Liveness is computed over the
/// control flow graph, so stores that are overwritten on every path, or written
/// before an unconditional return, are removed while stores in conditional
/// branches are kept.
///
/// Writes to captured locals are never removed: a nested function may observe
/// the value even when the enclosing function never reads it.
///
/// Like the other semantic passes, it resolves identifiers through source
/// tokens, so the AST must be parsed with token preservation.
pub fn remove_dead_stores(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, DeadStoreError> {
    let cfg = build_chunk(block, model)?;
    let liveness = Liveness::analyze(&cfg, model)?;
    let evaluator = Evaluator::default();
    let mut dead_statements: HashSet<TokenKey> = HashSet::new();

    for block_id in cfg.iter_block_ids() {
        let index = block_id.index();
        let statements = &cfg.blocks[index].statements;

        let mut live = liveness.live_out[index].clone();
        for bit in liveness.terminator_uses[index].iter_ones() {
            live.set(bit, true);
        }

        for statement_index in (0..statements.len()).rev() {
            let defs = &liveness.statement_defs[index][statement_index];
            let uses = &liveness.statement_uses[index][statement_index];

            if is_dead_store(&statements[statement_index], defs, &live, model, &evaluator) {
                if let Some(key) = statement_token_key(&statements[statement_index]) {
                    dead_statements.insert(key);
                }
            }

            // Transfer: the statement's definitions no longer matter, its
            // uses become live.
            for bit in defs.iter_ones() {
                live.set(bit, false);
            }
            for bit in uses.iter_ones() {
                live.set(bit, true);
            }
        }
    }

    if dead_statements.is_empty() {
        return Ok(0);
    }

    let mut remover = DeadStoreRemover {
        dead_statements,
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut remover);

    Ok(remover.removed)
}

fn is_dead_store(
    statement: &Statement,
    defs: &BitVec,
    live: &BitVec,
    model: &SemanticModel,
    evaluator: &Evaluator,
) -> bool {
    let Statement::Assign(assignment) = statement else {
        return false;
    };

    if assignment.variables_len() != 1 {
        return false;
    }

    let Some(Variable::Identifier(identifier)) = assignment.iter_variables().next() else {
        return false;
    };

    let Some(symbol) = model.resolve_identifier(identifier) else {
        return false;
    };

    let symbol_info = model.symbol(symbol);
    if symbol_info.kind != SymbolKind::Local || !symbol_info.capturing_scopes.is_empty() {
        return false;
    }

    // Exactly this symbol must be defined, and it must not be live after.
    if defs.count_ones() != 1 || !defs[symbol.index()] {
        return false;
    }
    if live[symbol.index()] {
        return false;
    }

    assignment
        .iter_values()
        .all(|value| !evaluator.has_side_effects(value))
}

struct DeadStoreRemover {
    dead_statements: HashSet<TokenKey>,
    removed: usize,
}

impl NodeProcessor for DeadStoreRemover {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();
        statements.retain(|statement| {
            let key = statement_token_key(statement);
            if key.is_some_and(|key| self.dead_statements.contains(&key)) {
                self.removed += 1;
                false
            } else {
                true
            }
        });
        block.set_statements(statements);
    }
}
