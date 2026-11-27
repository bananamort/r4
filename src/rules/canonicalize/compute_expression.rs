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
    evaluator.value_to_expression(value)
}

fn evaluate(evaluator: &Evaluator, expression: &Expression) -> Option<LuaValue> {
    evaluator.evaluate_with(expression, &|_identifier| None)
}

/// Returns whether a folded value can be represented faithfully as a literal.
///
/// Non-finite numbers (`inf`, `nan`) have no Lua literal, and very large
/// strings are left alone to bound the resulting AST. Unknown, function and
/// table values are never constants.
pub(crate) fn is_safe_literal(value: &LuaValue) -> bool {
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
    let evaluator = Evaluator::for_dialect(model.dialect());
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
    fn lua53_folds_an_integral_float_to_a_float_literal() {
        // `2 ^ 10` is a float in Lua 5.3; folding keeps the float subtype by
        // emitting `1024.0`, not the integer-looking `1024`.
        let (mut block, model) = analyze_lua53("return 2 ^ 10");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 1024.0");
    }

    #[test]
    fn lua53_folds_a_float_concat_with_the_float_subtype() {
        // In Lua 5.3 `tostring(1.0)` is `"1.0"`, so the folded string keeps it.
        let (mut block, model) = analyze_lua53("return '' .. 1.0");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return '1.0'");
    }

    #[test]
    fn lua53_string_number_coercion_uses_the_integer_subtype() {
        // `'2' + 3` is the integer 5 in Lua 5.3, so `5 // 2` is the integer 2;
        // coercing the string to a float would make the floor division `2.0`.
        let (mut block, model) = analyze_lua53("return ('2' + 3) // 2");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 2");
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
    fn folds_float_concat_without_the_float_subtype() {
        // Lua 5.1/5.2/Luau have no float subtype, so an integral float prints
        // as `1` rather than `1.0`.
        let (mut block, model) = analyze("return '' .. 1.0");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return '1'");
    }

    #[test]
    fn folds_constant_if_expressions() {
        let (mut block, model) = analyze("return if true then 1 else 2");

        compute_expression(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 1");
    }
}
