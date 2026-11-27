use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::cfg::analysis::{
    fold_constant_branches_with, ConstantPropagation, ConstantPropagationError, Reachability,
};
use crate::cfg::{
    build_chunk, validate_block_identities, CfgError, ConditionKind, ConditionOrigin,
    StatementOrigin, Terminator, UnidentifiedBlock,
};
use crate::nodes::{Block, DoStatement, Identifier, IfStatement, Statement};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

/// An error produced while removing unreachable code.
#[derive(Clone, Debug, PartialEq)]
pub enum UnreachableCodeError {
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Constant propagation could not resolve an identifier.
    ConstantPropagation(ConstantPropagationError),
    /// The AST and the semantic model do not match.
    Provenance(UnidentifiedBlock),
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

impl From<UnidentifiedBlock> for UnreachableCodeError {
    fn from(error: UnidentifiedBlock) -> Self {
        Self::Provenance(error)
    }
}

impl fmt::Display for UnreachableCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnreachableCodeError::Cfg(error) => write!(f, "{}", error),
            UnreachableCodeError::ConstantPropagation(error) => write!(f, "{}", error),
            UnreachableCodeError::Provenance(error) => write!(f, "{}", error),
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
/// Unknown conditions are left untouched. Statements and conditions are
/// matched to the AST through structural origins, so the pass works on
/// token-less trees.
pub fn remove_unreachable_code(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, UnreachableCodeError> {
    validate_block_identities(block)?;

    let mut cfg = build_chunk(block, model)?;
    let analysis = ConstantPropagation::analyze(&cfg, model)?;
    let evaluator = Evaluator::for_dialect(model.dialect());
    let reachability_before_folding = Reachability::analyze(&cfg);

    let mut block_decisions: Vec<Option<bool>> = vec![None; cfg.block_count()];
    let mut condition_decisions: HashMap<ConditionOrigin, bool> = HashMap::new();

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
                let origin = cfg.blocks[block_id.index()]
                    .branch_origin
                    .expect("the CFG records a condition origin for every emitted branch");
                condition_decisions.insert(origin, truthy);
            }
        }
    }

    fold_constant_branches_with(&mut cfg, |block_id, _| block_decisions[block_id.index()]);

    let reachability = Reachability::analyze(&cfg);
    let mut unreachable_statements: HashSet<StatementOrigin> = HashSet::new();

    for block_id in cfg.iter_block_ids() {
        if reachability.is_reachable(block_id) {
            continue;
        }
        let index = block_id.index();
        for statement_index in 0..cfg.blocks[index].statements.len() {
            let origin = cfg.blocks[index]
                .statement_origin(statement_index)
                .expect("the CFG records an origin for every emitted statement");
            unreachable_statements.insert(origin);
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
    unreachable_statements: HashSet<StatementOrigin>,
    condition_decisions: HashMap<ConditionOrigin, bool>,
    removed: usize,
}

enum IfRewrite {
    Unchanged,
    Remove,
    Replace(Box<Statement>),
}

impl Rewriter {
    fn condition_decision(&self, origin: ConditionOrigin) -> Option<bool> {
        self.condition_decisions.get(&origin).copied()
    }

    fn keep_statement(&mut self, statement: &mut Statement, origin: StatementOrigin) -> bool {
        if self.unreachable_statements.contains(&origin) {
            self.removed += 1;
            return false;
        }

        match statement {
            Statement::If(if_statement) => match self.simplify_if(if_statement, origin) {
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
            Statement::While(_) => self.simplify_while(origin),
            _ => true,
        }
    }

    fn simplify_while(&mut self, origin: StatementOrigin) -> bool {
        let condition = ConditionOrigin::new(origin, ConditionKind::While);
        if self.condition_decision(condition) == Some(false) {
            self.removed += 1;
            false
        } else {
            true
        }
    }

    fn simplify_if(&self, if_statement: &IfStatement, origin: StatementOrigin) -> IfRewrite {
        let mut skipped_false_branch = false;

        for (index, branch) in if_statement.iter_branches().enumerate() {
            let condition = ConditionOrigin::new(origin, ConditionKind::IfBranch(index as u32));
            match self.condition_decision(condition) {
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
        let identity = block
            .get_identity()
            .expect("block identities were validated before the rewrite");
        let mut statements = block.take_statements();
        let mut index = 0;
        statements.retain_mut(|statement| {
            let origin = StatementOrigin {
                block: identity,
                index,
            };
            index += 1;
            self.keep_statement(statement, origin)
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
    fn removes_code_after_an_infinite_loop() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            while true do
                work()
            end
            after()
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("work()"), "unexpected: {output}");
        assert!(!output.contains("after()"), "unexpected: {output}");
    }

    #[test]
    fn folds_constant_false_conditionals() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local enabled = false
            if enabled then
                dead()
            else
                live()
            end
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("dead()"), "unexpected: {output}");
        assert!(output.contains("live()"), "unexpected: {output}");
        assert!(!output.contains("if enabled"), "unexpected: {output}");
    }

    #[test]
    fn folds_constant_true_conditionals() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            if true then
                keep()
            else
                drop()
            end
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("keep()"), "unexpected: {output}");
        assert!(!output.contains("drop()"), "unexpected: {output}");
    }

    #[test]
    fn removes_false_while_loops() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local running = false
            while running do
                work()
            end
            after()
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("while"), "unexpected: {output}");
        assert!(!output.contains("work()"), "unexpected: {output}");
        assert!(output.contains("after()"), "unexpected: {output}");
    }

    #[test]
    fn removes_code_after_a_do_return() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            do
                return 1
            end
            after()
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("return 1"), "unexpected: {output}");
        assert!(!output.contains("after()"), "unexpected: {output}");
    }

    #[test]
    fn keeps_unknown_control_flow() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            if condition then
                a()
            else
                b()
            end
            c()
            "#,
        );

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("unreachable code removal must succeed with tokens");

        assert_eq!(removed, 0);
        let output = generate(&block);
        assert!(output.contains("a()"), "unexpected: {output}");
        assert!(output.contains("b()"), "unexpected: {output}");
        assert!(output.contains("c()"), "unexpected: {output}");
    }

    #[test]
    fn works_without_token_data() {
        let mut block = parse("local running = false\nwhile running do work() end\nafter()");
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .expect("semantic analysis must succeed");

        let removed = remove_unreachable_code(&mut block, &model)
            .expect("token-less declarations must be resolved");

        assert!(removed >= 1);
        let output = generate(&block);
        assert!(!output.contains("while"), "unexpected: {output}");
        assert!(output.contains("after()"), "unexpected: {output}");
    }
}
