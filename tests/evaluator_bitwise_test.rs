//! Tests for the evaluator's bitwise folding (unreachable through the parser
//! because full_moon panics on bitwise operators, but valid AST nodes).

use r4::nodes::{BinaryExpression, BinaryOperator, Expression, UnaryExpression, UnaryOperator};
use r4::process::{Evaluator, LuaValue};

fn evaluate(operator: BinaryOperator, left: f64, right: f64) -> LuaValue {
    Evaluator::default().evaluate(&Expression::from(BinaryExpression::new(
        operator,
        Expression::from(left),
        Expression::from(right),
    )))
}

#[test]
fn folds_bitwise_operators_to_their_lua_values() {
    assert_eq!(
        evaluate(BinaryOperator::BitwiseAnd, 6.0, 3.0),
        LuaValue::Integer(2)
    );
    assert_eq!(
        evaluate(BinaryOperator::BitwiseOr, 6.0, 3.0),
        LuaValue::Integer(7)
    );
    assert_eq!(
        evaluate(BinaryOperator::BitwiseXor, 6.0, 3.0),
        LuaValue::Integer(5)
    );
    assert_eq!(
        evaluate(BinaryOperator::ShiftLeft, 6.0, 3.0),
        LuaValue::Integer(48)
    );
    assert_eq!(
        evaluate(BinaryOperator::ShiftRight, 6.0, 3.0),
        LuaValue::Integer(0)
    );
}

#[test]
fn bitwise_folding_declines_on_non_integers() {
    // 1.5 is not an integer, so the fold must decline rather than truncate.
    assert_eq!(
        evaluate(BinaryOperator::BitwiseAnd, 1.5, 1.0),
        LuaValue::Unknown
    );
    assert_eq!(
        evaluate(BinaryOperator::ShiftLeft, 1.0, 0.5),
        LuaValue::Unknown
    );
}

#[test]
fn folds_unary_bitwise_not() {
    let evaluated = Evaluator::default().evaluate(&Expression::from(UnaryExpression::new(
        UnaryOperator::BitwiseNot,
        Expression::from(6.0),
    )));
    assert_eq!(evaluated, LuaValue::Integer(-7));

    // A non-integer operand declines.
    let evaluated = Evaluator::default().evaluate(&Expression::from(UnaryExpression::new(
        UnaryOperator::BitwiseNot,
        Expression::from(1.5),
    )));
    assert_eq!(evaluated, LuaValue::Unknown);
}

#[test]
fn bitwise_expressions_are_pure_and_have_no_side_effects() {
    let evaluator = Evaluator::default();
    let expression = Expression::from(BinaryExpression::new(
        BinaryOperator::BitwiseXor,
        Expression::from(6.0),
        Expression::from(3.0),
    ));
    assert!(evaluator.is_pure(&expression));
    assert!(!evaluator.has_side_effects(&expression));
    assert!(!evaluator.can_return_multiple_values(&expression));
    assert_eq!(
        evaluator.evaluate_with(&expression, &|_| None),
        Some(LuaValue::Integer(5))
    );
}
