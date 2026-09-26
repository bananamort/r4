use crate::nodes::{BinaryOperator, Block, Expression};
use crate::process::{DefaultVisitor, Evaluator, LuaValue, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

/// The largest folded string literal. Larger results are left in place so the
/// pass cannot expand the AST without bound.
const MAX_FOLDED_STRING_LENGTH: usize = 4096;

struct Processor {
    evaluator: Evaluator,
}

impl NodeProcessor for Processor {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Some(replacement) = fold(&self.evaluator, expression) {
            *expression = replacement;
        }
    }
}

fn fold(evaluator: &Evaluator, expression: &Expression) -> Option<Expression> {
    // Short-circuit `and`/`or` when the left operand's truthiness is known.
    // This is sound even when the right operand has side effects, because the
    // selected operand is evaluated exactly as before (`true and f()` is `f()`).
    if let Expression::Binary(binary) = expression {
        if matches!(binary.operator(), BinaryOperator::And | BinaryOperator::Or) {
            if let Some(truthy) =
                evaluate(evaluator, binary.left()).and_then(|value| value.is_truthy())
            {
                return match binary.operator() {
                    BinaryOperator::And if truthy => Some(binary.right().clone()),
                    BinaryOperator::And => Some(binary.left().clone()),
                    BinaryOperator::Or if truthy => Some(binary.left().clone()),
                    BinaryOperator::Or => Some(binary.right().clone()),
                    _ => None,
                };
            }
        }
    }

    match expression {
        Expression::Unary(_) | Expression::Binary(_) | Expression::If(_) => {}
        _ => return None,
    }

    // Strict purity: no calls, metamethods or mutable table accesses.
    if !evaluator.is_pure(expression) {
        return None;
    }

    let value = evaluate(evaluator, expression)?;
    if !is_safe_literal(&value) {
        return None;
    }
    // In Lua 5.3/5.4 an integral result of a float operation must not become an
    // integer literal; the generator prints integral floats without a fraction,
    // so leave those expressions in place instead of changing their subtype.
    if evaluator.integer_numbers() {
        if let LuaValue::Number(number) = &value {
            if number.is_finite() && number.fract() == 0.0 {
                return None;
            }
        }
    }
    value.to_expression()
}

fn evaluate(evaluator: &Evaluator, expression: &Expression) -> Option<LuaValue> {
    evaluator.evaluate_with(expression, &|_identifier| None)
}

/// Returns whether a folded value can be represented faithfully as a literal.
///
/// Non-finite numbers (`inf`, `nan`) have no Lua literal, and very large
/// strings are left alone to bound the resulting AST. Unknown, function and
/// table values are never constants.
fn is_safe_literal(value: &LuaValue) -> bool {
    match value {
        LuaValue::Integer(_) => true,
        LuaValue::Number(number) => number.is_finite(),
        LuaValue::String(bytes) => bytes.len() <= MAX_FOLDED_STRING_LENGTH,
        LuaValue::True | LuaValue::False | LuaValue::Nil => true,
        LuaValue::Function | LuaValue::Table | LuaValue::Unknown => false,
    }
}

/// Folds pure constant expressions (`1 + 2`, `if true then a else b`, ...)
/// and short-circuits `and`/`or`.
///
/// Only expressions that are strictly pure and evaluate to a representable
/// literal are folded. Division by zero, non-finite results, metamethod-observing
/// operands and oversized strings are left exactly as they are.
pub fn compute_expression(block: &mut Block, model: &SemanticModel) {
    let mut evaluator = Evaluator::default();
    // Lua 5.3/5.4 have an integer subtype; every other supported dialect folds
    // numbers as floats.
    if matches!(
        model.dialect(),
        crate::parser::Dialect::Lua53 | crate::parser::Dialect::Lua54
    ) {
        evaluator = evaluator.with_integer_numbers();
    }
    let mut processor = Processor { evaluator };
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, analyze_lua53, generate};

    #[test]
    fn folds_constant_arithmetic() {
        let (mut block, model) = analyze("return 1 + 2");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 3");
    }

    #[test]
    fn lua53_folds_integer_arithmetic() {
        let (mut block, model) = analyze_lua53("return 1 + 2");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 3");
    }

    #[test]
    fn lua53_keeps_64_bit_precision() {
        let (mut block, model) = analyze_lua53("return 9223372036854775807 | 0");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 9223372036854775807");
    }

    #[test]
    fn lua53_wraps_integer_overflow() {
        let (mut block, model) = analyze_lua53("return 9223372036854775807 + 1");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return -9223372036854775808");
    }

    #[test]
    fn lua53_uses_floor_division_and_modulo() {
        let (mut block, model) = analyze_lua53("return -7 // 2, -7 % 3");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return -4, 2");
    }

    #[test]
    fn lua53_leaves_an_integral_float_alone() {
        // Folding `2 ^ 10` would print an integral float as an integer literal,
        // changing its subtype, so the expression is left in place.
        let (mut block, model) = analyze_lua53("return 2 ^ 10");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 2 ^ 10");
    }

    #[test]
    fn lua53_compares_across_integer_and_float() {
        let (mut block, model) = analyze_lua53("return 1 == 1.0, 1 < 1.5, 2 > 1");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return true, true, true");
    }

    #[test]
    fn folds_constant_string_concatenation() {
        let (mut block, model) = analyze("return 'a' .. 'b'");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 'ab'");
    }

    #[test]
    fn folds_constant_if_expressions() {
        let (mut block, model) = analyze("return if true then 1 else 2");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 1");
    }
}
