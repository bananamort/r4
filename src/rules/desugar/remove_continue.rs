use std::fmt;
use std::mem;

use crate::nodes::{
    AssignStatement, Block, GenericForStatement, Identifier, IfStatement, LastStatement,
    NumericForStatement, RepeatStatement, UnaryExpression, UnaryOperator, VariableAssignment,
    WhileStatement,
};
use crate::process::{
    DefaultPostVisitor, DefaultVisitor, NodePostProcessor, NodePostVisitor, NodeProcessor,
    NodeVisitor,
};
use crate::rules::NameGenerator;
use crate::semantic::{ScopeId, ScopeKind, SemanticModel};

/// An error produced by [`remove_continue`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveContinueError {
    /// A `repeat` loop that contains `continue` and whose `until` condition
    /// reads a local declared in the loop body cannot be lowered to Lua 5.1
    /// without changing the lifetime of those locals: the only Lua 5.1 way to
    /// emulate `continue` introduces a nested block, and a `repeat` condition
    /// shares its body's scope. The lowering fails rather than producing a loop
    /// whose condition reads a different binding.
    RepeatConditionReadsBodyLocal {
        /// The source line of the loop, when token data is available.
        line: Option<usize>,
    },
}

impl RemoveContinueError {
    /// Returns the stable code of this error.
    pub fn code(&self) -> &'static str {
        match self {
            RemoveContinueError::RepeatConditionReadsBodyLocal { .. } => {
                "desugar/repeat-continue-body-local"
            }
        }
    }
}

impl fmt::Display for RemoveContinueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoveContinueError::RepeatConditionReadsBodyLocal { line } => {
                if let Some(line) = line {
                    write!(f, "line {}: ", line)?;
                }
                f.write_str(
                    "a `repeat` loop with `continue` cannot have an `until` condition that reads a \
                     local declared in its body when lowering to Lua 5.1",
                )
            }
        }
    }
}

impl std::error::Error for RemoveContinueError {}

/// Collects whether a `repeat` condition reads a local declared in the loop
/// body (including locals of blocks nested in the body, and upvalues captured
/// by functions inside the condition). A local of a function *declared inside
/// the condition* is not a body local.
struct BodyLocalReads<'a> {
    model: &'a SemanticModel,
    body_scope: ScopeId,
    found: bool,
}

impl NodeProcessor for BodyLocalReads<'_> {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        if self.found {
            return;
        }
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            // An unresolvable read could be a body local; refuse to prove the
            // rewrite safe.
            self.found = true;
            return;
        };
        let mut scope = Some(self.model.symbol(symbol).declaring_scope);
        while let Some(current) = scope {
            if current == self.body_scope {
                self.found = true;
                return;
            }
            if matches!(
                self.model.scope(current).kind,
                ScopeKind::File | ScopeKind::Function
            ) {
                // The declaration lives inside a function nested in the
                // condition, not in the loop body.
                return;
            }
            scope = self.model.scope(current).parent;
        }
    }
}

struct Processor<'a> {
    model: &'a SemanticModel,
    loop_stack: Vec<Option<LoopData>>,
    names: NameGenerator,
    error: Option<RemoveContinueError>,
}

struct LoopData {
    has_continue_statement: bool,
    identifier: Identifier,
}

impl LoopData {
    fn new(identifier: Identifier) -> Self {
        Self {
            has_continue_statement: false,
            identifier,
        }
    }

    fn get_identifier(&self) -> &Identifier {
        &self.identifier
    }
}

impl<'a> Processor<'a> {
    fn new(model: &'a SemanticModel) -> Self {
        Self {
            model,
            loop_stack: Vec::new(),
            names: NameGenerator::new(model),
            error: None,
        }
    }

    fn push_loop(&mut self) {
        let identifier = self.names.generate_with_prefix("__R4_CONTINUE");
        self.loop_stack
            .push(Some(LoopData::new(Identifier::new(identifier))));
    }

    fn push_no_loop(&mut self) {
        self.loop_stack.push(None);
    }

    /// Returns whether `statement`'s `until` condition reads a local declared
    /// in the loop body. When the model cannot locate the body block, the
    /// rewrite is refused.
    fn repeat_condition_reads_body_local(&self, statement: &RepeatStatement) -> bool {
        let Ok(body_scope) = self.model.try_scope_of(statement.get_block()) else {
            return true;
        };
        let mut condition = statement.get_condition().clone();
        let mut reads = BodyLocalReads {
            model: self.model,
            body_scope,
            found: false,
        };
        DefaultVisitor::visit_expression(&mut condition, &mut reads);
        reads.found
    }

    fn wrap_loop_block_if_needed(&mut self, block: &mut Block) {
        if let Some(loop_data) = self.loop_stack.pop().flatten() {
            if !loop_data.has_continue_statement {
                return;
            }
            let mut current_loop_block = mem::take(block);

            if current_loop_block.get_last_statement().is_none() {
                current_loop_block.push_statement(AssignStatement::from_variable(
                    loop_data.get_identifier().clone(),
                    true,
                ));
            }

            let new_block = Block::default()
                .with_statement(
                    VariableAssignment::from_variable(loop_data.get_identifier().clone())
                        .with_value(false),
                )
                .with_statement(RepeatStatement::new(current_loop_block, true))
                .with_statement(IfStatement::create(
                    UnaryExpression::new(UnaryOperator::Not, loop_data.get_identifier().clone()),
                    LastStatement::Break(None),
                ));

            *block = new_block;
        }
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_generic_for_statement(&mut self, _: &mut GenericForStatement) {
        self.push_loop();
    }

    fn process_numeric_for_statement(&mut self, _: &mut NumericForStatement) {
        self.push_loop();
    }

    fn process_repeat_statement(&mut self, _: &mut RepeatStatement) {
        self.push_loop();
    }

    fn process_while_statement(&mut self, _: &mut WhileStatement) {
        self.push_loop();
    }

    fn process_function_statement(&mut self, _: &mut crate::nodes::FunctionStatement) {
        self.push_no_loop();
    }

    fn process_function_expression(&mut self, _: &mut crate::nodes::FunctionExpression) {
        self.push_no_loop();
    }

    fn process_block(&mut self, block: &mut Block) {
        let new_statement =
            block
                .mutate_last_statement()
                .and_then(|last_statement| match last_statement {
                    LastStatement::Continue(continue_token) => {
                        if let Some(Some(loop_data)) = self.loop_stack.last_mut() {
                            if !loop_data.has_continue_statement {
                                loop_data.has_continue_statement = true;
                            }

                            *last_statement = LastStatement::Break(continue_token.take().map(
                                |mut continue_token| {
                                    continue_token.replace_with_content("break");
                                    continue_token
                                },
                            ));

                            Some(AssignStatement::from_variable(
                                loop_data.get_identifier().clone(),
                                true,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                });

        if let Some(statement) = new_statement {
            block.push_statement(statement);
        }
    }
}

impl NodePostProcessor for Processor<'_> {
    fn process_after_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_repeat_statement(&mut self, statement: &mut RepeatStatement) {
        if let Some(Some(loop_data)) = self.loop_stack.last() {
            if loop_data.has_continue_statement && self.repeat_condition_reads_body_local(statement)
            {
                self.error =
                    Some(RemoveContinueError::RepeatConditionReadsBodyLocal { line: None });
                self.loop_stack.pop();
                return;
            }
        }
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_while_statement(&mut self, statement: &mut WhileStatement) {
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_function_statement(&mut self, _: &mut crate::nodes::FunctionStatement) {
        self.loop_stack.pop();
    }

    fn process_after_function_expression(&mut self, _: &mut crate::nodes::FunctionExpression) {
        self.loop_stack.pop();
    }
}

/// Removes `continue` statements (and converts them into flagged `break`
/// statements wrapped in a repeated loop).
///
/// # Errors
///
/// Returns [`RemoveContinueError::RepeatConditionReadsBodyLocal`] when a
/// `repeat` loop cannot be lowered without changing the meaning of its `until`
/// condition. See the variant for details.
pub fn remove_continue(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<(), RemoveContinueError> {
    let mut processor = Processor::new(model);
    DefaultPostVisitor::visit_block(block, &mut processor);
    match processor.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate, parse};

    #[test]
    fn desugars_continue_statements() {
        let (mut block, model) = analyze(
            r#"
            for i = 1, 10 do
                if i > 5 then
                    continue
                end
                print(i)
            end
            "#,
        );

        remove_continue(&mut block, &model).expect("the loop lowers");

        let output = generate(&block);
        assert!(!output.contains("continue"), "unexpected: {output}");
        parse(&output);
    }

    #[test]
    fn avoids_existing_aggregate_identifiers() {
        let (mut block, model) = analyze(
            r#"
            for i = 1, 10 do
                if i > 5 then
                    continue
                end
                local __R4_CONTINUE1 = 1
                print(__R4_CONTINUE1)
            end
            "#,
        );

        remove_continue(&mut block, &model).expect("the loop lowers");

        let output = generate(&block);
        assert!(!output.contains("continue"), "unexpected: {output}");
        assert!(
            output.contains("local __R4_CONTINUE1 = 1"),
            "the user local must keep its name: {output}"
        );
        assert!(
            output.contains("local __R4_CONTINUE ="),
            "the generated flag must avoid the user identifier: {output}"
        );
        parse(&output);
    }

    #[test]
    fn repeat_continue_still_lowers_when_the_condition_is_independent() {
        // `done` is an outer local, not declared in the loop body, so wrapping
        // the body does not change what the condition reads.
        let (mut block, model) = analyze(
            r#"
            local done = false
            repeat
                if done then
                    continue
                end
                done = true
            until done
            "#,
        );

        remove_continue(&mut block, &model).expect("the loop lowers");
        assert!(!generate(&block).contains("continue"));
    }

    #[test]
    fn repeat_continue_whose_condition_reads_a_body_local_is_typed_error() {
        let (mut block, model) = analyze(
            r#"
            local i = 0
            repeat
                local done = i >= 3
                i = i + 1
                if not done then
                    continue
                end
                print(i)
            until done
            "#,
        );

        let error = remove_continue(&mut block, &model).expect_err("must fail fast");
        assert_eq!(error.code(), "desugar/repeat-continue-body-local");
    }

    #[test]
    fn the_error_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<RemoveContinueError>();
    }
}
