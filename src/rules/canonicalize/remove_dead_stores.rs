use std::collections::HashSet;
use std::fmt;

use bitvec::prelude::*;

use crate::cfg::analysis::{Liveness, LivenessError};
use crate::cfg::{
    build_chunk, validate_block_identities, BlockId, CfgError, StatementOrigin, UnidentifiedBlock,
};
use crate::nodes::{Block, Statement, Variable};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolKind};

/// An error produced while removing dead stores.
#[derive(Clone, Debug, PartialEq)]
pub enum DeadStoreError {
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Liveness could not be computed.
    Liveness(LivenessError),
    /// The AST and the semantic model do not match.
    Provenance(UnidentifiedBlock),
    /// A statement in the graph has no structural origin.
    MissingStatementOrigin {
        /// The graph block containing the statement.
        block: BlockId,
        /// The index of the statement inside the block.
        statement: usize,
    },
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

impl From<UnidentifiedBlock> for DeadStoreError {
    fn from(error: UnidentifiedBlock) -> Self {
        DeadStoreError::Provenance(error)
    }
}

impl fmt::Display for DeadStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeadStoreError::Cfg(error) => write!(f, "{}", error),
            DeadStoreError::Liveness(error) => write!(f, "{}", error),
            DeadStoreError::Provenance(error) => write!(f, "{}", error),
            DeadStoreError::MissingStatementOrigin { block, statement } => write!(
                f,
                "statement {} of graph block {:?} has no structural origin",
                statement, block
            ),
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
/// Statements are matched to the AST through structural origins recorded by
/// semantic analysis, so the pass works on token-less trees.
pub fn remove_dead_stores(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, DeadStoreError> {
    let cfg = build_chunk(block, model)?;
    let liveness = Liveness::analyze(&cfg, model)?;
    validate_block_identities(block)?;

    let evaluator = Evaluator::default();
    let mut dead_statements: HashSet<StatementOrigin> = HashSet::new();

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
                let origin = cfg.blocks[index].statement_origin(statement_index).ok_or(
                    DeadStoreError::MissingStatementOrigin {
                        block: block_id,
                        statement: statement_index,
                    },
                )?;
                dead_statements.insert(origin);
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

    // Liveness resolved every identifier in the graph before this check runs.
    let symbol = model
        .resolve_identifier(identifier)
        .expect("liveness resolved every identifier in the graph");

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
        .all(|value| !crate::semantic::has_side_effects(model, evaluator, value))
}

struct DeadStoreRemover {
    dead_statements: HashSet<StatementOrigin>,
    removed: usize,
}

impl NodeProcessor for DeadStoreRemover {
    fn process_block(&mut self, block: &mut Block) {
        let identity = block
            .get_identity()
            .expect("block identities were validated before the rewrite");
        let mut statements = block.take_statements();
        let mut index = 0;
        statements.retain(|_| {
            let origin = StatementOrigin {
                block: identity,
                index,
            };
            index += 1;
            if self.dead_statements.contains(&origin) {
                self.removed += 1;
                false
            } else {
                true
            }
        });
        block.set_statements(statements);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze_with_tokens, generate, parse};
    use crate::semantic::SemanticAnalyzer;

    #[test]
    fn removes_overwritten_stores() {
        let (mut block, model) = analyze_with_tokens("local x\nx = 1\nx = 2\nreturn x");

        let removed = remove_dead_stores(&mut block, &model)
            .expect("dead store removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("x = 1"), "unexpected: {output}");
        assert!(output.contains("x = 2"), "unexpected: {output}");
        parse(&output);
    }

    #[test]
    fn keeps_conditional_stores() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local x
            if condition then
                x = 1
            else
                x = 2
            end
            return x
            "#,
        );

        let removed = remove_dead_stores(&mut block, &model)
            .expect("dead store removal must succeed with tokens");
        assert_eq!(removed, 0, "both stores reach the return");
    }

    #[test]
    fn keeps_stores_to_captured_locals() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local x
            local function observe()
                return x
            end
            x = 1
            return observe
            "#,
        );

        let removed = remove_dead_stores(&mut block, &model)
            .expect("dead store removal must succeed with tokens");
        assert_eq!(removed, 0, "a nested function can observe the store");
    }

    #[test]
    fn keeps_side_effecting_stores() {
        let (mut block, model) = analyze_with_tokens("local x\nx = compute()\nx = 2\nreturn x");

        let removed = remove_dead_stores(&mut block, &model)
            .expect("dead store removal must succeed with tokens");
        assert_eq!(removed, 0, "the call must keep running");
    }

    #[test]
    fn works_without_token_data() {
        let mut block = parse("local x\nx = 1\nx = 2\nreturn x");
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .expect("semantic analysis must succeed");

        let removed =
            remove_dead_stores(&mut block, &model).expect("token-less stores must be removable");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("x = 1"), "unexpected: {output}");
    }
}
