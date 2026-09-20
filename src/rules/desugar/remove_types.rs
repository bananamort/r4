use crate::nodes::*;
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

#[derive(Default)]
struct RemoveTypesProcessor {
    evaluator: Evaluator,
}

impl NodeProcessor for RemoveTypesProcessor {
    fn process_block(&mut self, block: &mut Block) {
        block.filter_statements(|statement| {
            !matches!(
                statement,
                Statement::TypeDeclaration(_) | Statement::TypeFunction(_)
            )
        });
    }

    fn process_local_assign_statement(&mut self, local_assign: &mut VariableAssignment) {
        local_assign.clear_types();
    }

    fn process_numeric_for_statement(&mut self, numeric_for: &mut NumericForStatement) {
        numeric_for.clear_types();
    }

    fn process_generic_for_statement(&mut self, generic_for: &mut GenericForStatement) {
        generic_for.clear_types();
    }

    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        function.clear_types();
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        function.clear_types();
    }

    fn process_function_expression(&mut self, function: &mut FunctionExpression) {
        function.clear_types();
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        call.remove_type_instantiation_from_method();
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        loop {
            match expression {
                Expression::TypeCast(type_cast) => {
                    let value = type_cast.get_expression();
                    if self.evaluator.can_return_multiple_values(value) {
                        *expression = value.clone().in_parentheses();
                    } else {
                        *expression = value.clone();
                    }
                }
                Expression::TypeInstantiation(type_instantiation) => {
                    let prefix: Expression = type_instantiation.get_prefix().clone().into();
                    if self.evaluator.can_return_multiple_values(&prefix) {
                        *expression = prefix.in_parentheses();
                    } else {
                        *expression = prefix;
                    }
                }
                _ => {
                    break;
                }
            }
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        while let Prefix::TypeInstantiation(type_instantiation) = prefix {
            *prefix = type_instantiation.get_prefix().clone();
        }
    }
}

/// Removes all Luau type annotations, type declarations and type-only
/// expressions from a block.
pub fn remove_types(block: &mut Block, _model: &SemanticModel) {
    let mut processor = RemoveTypesProcessor::default();
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn strips_local_type_annotations() {
        let (mut block, model) = analyze("local x: number = 10");

        remove_types(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"local x = 10");
    }

    #[test]
    fn strips_function_parameter_and_return_types() {
        let (mut block, model) =
            analyze("local function add(a: number, b: number): number return a + b end");

        remove_types(&mut block, &model);

        let output = generate(&block);
        assert!(!output.contains(':'), "types must be removed: {output}");
        assert!(
            output.contains("local function add(a, b)"),
            "unexpected: {output}"
        );
    }
}
