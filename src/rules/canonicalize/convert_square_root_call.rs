use crate::nodes::{
    BinaryExpression, BinaryOperator, Block, Expression, FunctionCall, Prefix, Statement,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;
use crate::utils::{expressions_as_statement, preserve_arguments_side_effects};

const DEFAULT_MATH_LIBRARY: &str = "math";
const DEFAULT_MATH_SQRT_NAME: &str = "sqrt";

struct Processor<'a> {
    evaluator: Evaluator,
    model: &'a SemanticModel,
}

impl Processor<'_> {
    fn is_math_sqrt_call(&self, call: &FunctionCall) -> bool {
        if call.has_method() {
            return false;
        }

        if call.get_arguments().len() != 1 {
            return false;
        }

        if let Prefix::Field(field_expr) = call.get_prefix() {
            if field_expr.get_field().get_name() != DEFAULT_MATH_SQRT_NAME {
                return false;
            }

            if let Prefix::Identifier(identifier) = field_expr.get_prefix() {
                if identifier.get_name() == DEFAULT_MATH_LIBRARY
                    && self.model.name_is_transparent_global(DEFAULT_MATH_LIBRARY)
                {
                    return true;
                }
            }
        }

        false
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Call(call) = expression {
            if self.is_math_sqrt_call(call) {
                let arguments = call.get_arguments();
                let expressions = arguments.clone().to_expressions();
                if let Some(argument) = expressions.first() {
                    *expression = BinaryExpression::new(
                        BinaryOperator::Caret,
                        argument.clone(),
                        Expression::from(0.5),
                    )
                    .into();
                }
            }
        }
    }

    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::Call(call) = statement {
            if self.is_math_sqrt_call(call) {
                let values = preserve_arguments_side_effects(
                    self.model,
                    &self.evaluator,
                    call.get_arguments(),
                );

                *statement = expressions_as_statement(values);
            }
        }
    }
}

/// Converts `math.sqrt(x)` calls into `x ^ 0.5`.
///
/// `math` is resolved through the semantic model: read-only aliases such as
/// `local math = math` no longer prevent the optimization, while mutable or
/// non-alias shadowing does. Name strings alone are never trusted.
pub fn convert_square_root_call(block: &mut Block, model: &SemanticModel) {
    let mut processor = Processor {
        evaluator: Evaluator::default(),
        model,
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn converts_square_root_calls_to_exponentiation() {
        let (mut block, model) = analyze("return math.sqrt(4)");

        convert_square_root_call(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 4 ^ 0.5");
    }

    #[test]
    fn converts_square_root_calls_on_values() {
        let (mut block, model) = analyze("local x = math.sqrt(value)\nreturn x");

        convert_square_root_call(&mut block, &model);

        let output = generate(&block);
        assert!(output.contains("value ^ 0.5"), "unexpected: {output}");
        assert!(!output.contains("math.sqrt"), "unexpected: {output}");
    }
}
