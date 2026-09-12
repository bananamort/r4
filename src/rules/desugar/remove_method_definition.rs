use crate::nodes::{Block, FunctionStatement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl NodeProcessor for Processor {
    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        function.remove_method();
    }
}

/// Converts method function definitions into regular function definitions.
///
/// `function value:method(...)` becomes `function value.method(self, ...)`,
/// inserting the implicit `self` parameter.
pub fn remove_method_definition(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}
