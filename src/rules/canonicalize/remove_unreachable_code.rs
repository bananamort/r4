use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::cfg::analysis::{
    fold_constant_branches_with, ConstantPropagation, ConstantPropagationError, Reachability,
};
use crate::cfg::{build_chunk, expression_token_key, statement_token_key, CfgError, Terminator};
use crate::nodes::{
    Block, DoStatement, Expression, Identifier, IfStatement, Statement, WhileStatement,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, TokenKey};

/// An error produced while removing unreachable code.
#[derive(Clone, Debug, PartialEq)]
pub enum UnreachableCodeError {
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Constant propagation could not resolve declarations (usually because
    /// the AST has no token data).
    ConstantPropagation(ConstantPropagationError),
}

impl From<CfgError> for UnreachableCodeError {
    fn from(error: CfgError) -> Self {
        Self::Cfg(error)
    }
}

impl From<ConstantPropagationError> for UnreachableCodeError {
    fn from(error: ConstantPropagationError) -> Self {
        Self::ConstantPropagation(error)
    }
}

impl fmt::Display for UnreachableCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnreachableCodeError::Cfg(error) => write!(f, "{}", error),
            UnreachableCodeError::ConstantPropagation(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for UnreachableCodeError {}

/// Removes statements that can never execute, using the control flow graph.
///
/// The pass folds branch conditions with reaching-definition constant
/// propagation, computes reachability, and then:
///
/// * deletes statements that belong to unreachable blocks,
/// * replaces `if` statements whose taken branch is known with a `do` block
///   containing only that branch,
/// * removes `while` loops whose condition is a known false constant.
///
/// Unknown conditions are left untouched. Like the other semantic passes, it
/// resolves declarations through source tokens, so the AST must be parsed with
/// token preservation.
pub fn remove_unreachable_code(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, UnreachableCodeError> {
    let mut cfg = build_chunk(block, model)?;
    let analysis = ConstantPropagation::analyze(&cfg, model)?;
    let evaluator = Evaluator::default();
    let reachability_before_folding = Reachability::analyze(&cfg);

    let mut block_decisions: Vec<Option<bool>> = vec![None; cfg.block_count()];
    let mut condition_decisions: HashMap<TokenKey, bool> = HashMap::new();

    for block_id in cfg.iter_block_ids() {
        if !reachability_before_folding.is_reachable(block_id) {
            continue;
        }

        if let Terminator::Branch { condition, .. } = cfg.terminator(block_id) {
            let constants = analysis.block_exit_constants(block_id);
            let value = evaluator
                .evaluate_with(condition, &|identifier: &Identifier| {
                    model
                        .resolve_identifier(identifier)
                        .and_then(|symbol| constants.get(&symbol).cloned())
                })
                .and_then(|value| value.is_truthy());

            if let Some(truthy) = value {
                block_decisions[block_id.index()] = Some(truthy);
                if let Some(key) = expression_token_key(condition) {
                    condition_decisions.insert(key, truthy);
                }
            }
        }
    }

    fold_constant_branches_with(&mut cfg, |block_id, _| block_decisions[block_id.index()]);

    let reachability = Reachability::analyze(&cfg);
    let mut unreachable_statements: HashSet<TokenKey> = HashSet::new();

    for block_id in cfg.iter_block_ids() {
        if reachability.is_reachable(block_id) {
            continue;
        }
        for statement in &cfg.blocks[block_id.index()].statements {
            if let Some(key) = statement_token_key(statement) {
                unreachable_statements.insert(key);
            }
        }
    }

    if unreachable_statements.is_empty() && condition_decisions.is_empty() {
        return Ok(0);
    }

    let mut rewriter = Rewriter {
        unreachable_statements,
        condition_decisions,
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut rewriter);

    Ok(rewriter.removed)
}

struct Rewriter {
    unreachable_statements: HashSet<TokenKey>,
    condition_decisions: HashMap<TokenKey, bool>,
    removed: usize,
}

enum IfRewrite {
    Unchanged,
    Remove,
    Replace(Box<Statement>),
}

impl Rewriter {
    fn condition_decision(&self, condition: &Expression) -> Option<bool> {
        expression_token_key(condition).and_then(|key| self.condition_decisions.get(&key).copied())
    }

    fn keep_statement(&mut self, statement: &mut Statement) -> bool {
        if let Some(key) = statement_token_key(statement) {
            if self.unreachable_statements.contains(&key) {
                self.removed += 1;
                return false;
            }
        }

        match statement {
            Statement::If(if_statement) => match self.simplify_if(if_statement) {
                IfRewrite::Unchanged => true,
                IfRewrite::Remove => {
                    self.removed += 1;
                    false
                }
                IfRewrite::Replace(replacement) => {
                    self.removed += 1;
                    *statement = *replacement;
                    true
                }
            },
            Statement::While(while_statement) => self.simplify_while(while_statement),
            _ => true,
        }
    }

    fn simplify_while(&mut self, while_statement: &WhileStatement) -> bool {
        if self.condition_decision(while_statement.get_condition()) == Some(false) {
            self.removed += 1;
            false
        } else {
            true
        }
    }

    fn simplify_if(&self, if_statement: &IfStatement) -> IfRewrite {
        let mut skipped_false_branch = false;

        for branch in if_statement.iter_branches() {
            match self.condition_decision(branch.get_condition()) {
                Some(true) => {
                    return IfRewrite::Replace(Box::new(
                        DoStatement::new(branch.get_block().clone()).into(),
                    ));
                }
                Some(false) => {
                    skipped_false_branch = true;
                }
                None => {
                    // Once a condition is unknown the remaining branches are
                    // all live; leave the statement untouched.
                    return IfRewrite::Unchanged;
                }
            }
        }

        if !skipped_false_branch {
            return IfRewrite::Unchanged;
        }

        match if_statement.get_else_block() {
            Some(else_block) if !else_block.is_empty() => {
                IfRewrite::Replace(Box::new(DoStatement::new(else_block.clone()).into()))
            }
            _ => IfRewrite::Remove,
        }
    }
}

impl NodeProcessor for Rewriter {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();
        statements.retain_mut(|statement| self.keep_statement(statement));
        block.set_statements(statements);
    }
}
