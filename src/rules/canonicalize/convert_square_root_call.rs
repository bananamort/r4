use crate::nodes::{
    BinaryExpression, BinaryOperator, Block, Expression, FunctionCall, Prefix, Statement,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::rules::shadow::ShadowPolicy;
use crate::semantic::SemanticModel;
use crate::utils::{expressions_as_statement, preserve_arguments_side_effects};

const DEFAULT_MATH_LIBRARY: &str = "math";
const DEFAULT_MATH_SQRT_NAME: &str = "sqrt";

struct Processor<'a> {
    evaluator: Evaluator,
    shadows: ShadowPolicy<'a>,
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
                    && self.shadows.is_transparent(DEFAULT_MATH_LIBRARY)
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
                let values = preserve_arguments_side_effects(&self.evaluator, call.get_arguments());

                *statement = expressions_as_statement(values);
            }
        }
    }
}

/// Converts `math.sqrt(x)` calls into `x ^ 0.5`.
///
/// The name `math` is treated as shadowed when it is declared anywhere in the
/// block.
pub fn convert_square_root_call(block: &mut Block) {
    let shadow_policy = ShadowPolicy::from_block(block);
    let mut processor = Processor {
        evaluator: Evaluator::default(),
        shadows: shadow_policy,
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Like [`convert_square_root_call`], but resolves `math` through the semantic
/// model.
///
/// Read-only aliases such as `local math = math` no longer prevent the
/// optimization: the pass converts through them because the local always holds
/// the global table. Mutable or non-alias shadowing still prevents conversion.
pub fn convert_square_root_call_with_model(block: &mut Block, model: &SemanticModel) {
    let mut processor = Processor {
        evaluator: Evaluator::default(),
        shadows: ShadowPolicy::from_model(model),
    };
    DefaultVisitor::visit_block(block, &mut processor);
}
