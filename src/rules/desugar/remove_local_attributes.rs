use crate::nodes::{
    Block, FunctionAssignment, FunctionExpression, FunctionStatement, GenericForStatement,
    NumericForStatement, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl NodeProcessor for Processor {
    fn process_local_assign_statement(&mut self, assign: &mut VariableAssignment) {
        assign.clear_attributes();
    }

    fn process_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        statement.clear_attributes();
    }

    fn process_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        statement.clear_attributes();
    }

    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        function.clear_attributes();
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        function.clear_attributes();
    }

    fn process_function_expression(&mut self, function: &mut FunctionExpression) {
        function.clear_attributes();
    }
}

/// Removes the Lua 5.4 local attributes (`<const>` and `<close>`).
///
/// This is a lowering pass, not a rewrite: a `<close>` local guarantees that its
/// value is closed when the scope exits, and a `<const>` local is a compile-time
/// constant. Dropping them therefore changes what the program guarantees, and it
/// is only sound when the caller is targeting a dialect that has no local
/// attributes. [`crate::rules::desugar_to_lua51`] is the only in-tree caller, for
/// exactly that reason.
pub fn remove_local_attributes(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn removes_lua54_local_attributes() {
        let (mut block, model) = analyze("local x <const> = 1\nreturn x");

        remove_local_attributes(&mut block, &model);

        let output = generate(&block);
        assert!(!output.contains('<'), "unexpected: {output}");
        assert!(output.contains("local x = 1"), "unexpected: {output}");
    }
}
