use std::mem;

use crate::nodes::{
    AssignStatement, Block, GenericForStatement, Identifier, IfStatement, LastStatement,
    NumericForStatement, RepeatStatement, UnaryExpression, UnaryOperator, VariableAssignment,
    WhileStatement,
};
use crate::process::{DefaultPostVisitor, NodePostProcessor, NodePostVisitor, NodeProcessor};
use crate::rules::NameGenerator;
use crate::semantic::SemanticModel;

struct Processor {
    loop_stack: Vec<Option<LoopData>>,
    names: NameGenerator,
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

impl Processor {
    fn new(model: &SemanticModel) -> Self {
        Self {
            loop_stack: Vec::new(),
            names: NameGenerator::new(model),
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

impl NodeProcessor for Processor {
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

impl NodePostProcessor for Processor {
    fn process_after_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        self.wrap_loop_block_if_needed(statement.mutate_block());
    }

    fn process_after_repeat_statement(&mut self, statement: &mut RepeatStatement) {
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
pub fn remove_continue(block: &mut Block, model: &SemanticModel) {
    let mut processor = Processor::new(model);
    DefaultPostVisitor::visit_block(block, &mut processor);
}
