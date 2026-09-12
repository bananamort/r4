use crate::nodes::{Block, FunctionAssignment, FunctionExpression, FunctionStatement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl NodeProcessor for Processor {
    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        function.mutate_attributes().clear_attributes();
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        function.mutate_attributes().clear_attributes();
    }

    fn process_function_expression(&mut self, function: &mut FunctionExpression) {
        function.mutate_attributes().clear_attributes();
    }
}

/// Removes every Luau attribute (`@native`, `@checked`, ...) from functions.
///
/// Attributes only carry tooling hints, so removing them never changes
/// runtime behavior.
pub fn remove_attribute(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}
