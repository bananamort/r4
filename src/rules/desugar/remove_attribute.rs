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
/// Attributes only carry tooling hints, so removing them never changes runtime
/// behavior. The Lua 5.4 local attributes (`<const>`/`<close>`) are *not*
/// removed here: `<close>` carries a runtime guarantee, so it is only dropped by
/// the explicit lowering pipeline, never by this pass.
pub fn remove_attribute(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn removes_function_attributes() {
        let (mut block, model) = analyze("@native function f() end");

        remove_attribute(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"function f() end");
    }

    #[test]
    fn removes_attributes_from_function_expressions() {
        let (mut block, model) = analyze("local f = @native function() end");

        remove_attribute(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"local f = function() end");
    }
}
