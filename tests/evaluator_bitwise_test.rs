//! Tests for the evaluator's bitwise folding (unreachable through the parser
//! because full_moon panics on bitwise operators, but valid AST nodes).

use r4::nodes::{BinaryExpression, BinaryOperator, Expression, UnaryExpression, UnaryOperator};
use r4::process::Evaluator;

#[test]
fn evaluates_bitwise_operators() {
    let evaluator = Evaluator::default();
    for operator in [
        BinaryOperator::BitwiseAnd,
        BinaryOperator::BitwiseOr,
        BinaryOperator::BitwiseXor,
        BinaryOperator::ShiftLeft,
        BinaryOperator::ShiftRight,
    ] {
        let expression = Expression::from(BinaryExpression::new(
            operator,
            Expression::from(6.0),
            Expression::from(3.0),
        ));
        let _ = evaluator.evaluate(&expression);
        let _ = evaluator.evaluate_with(&expression, &|_| None);
        let _ = evaluator.is_pure(&expression);
        let _ = evaluator.has_side_effects(&expression);
        let _ = evaluator.has_side_effects_with(&expression, &|_| true);
        let _ = evaluator.can_return_multiple_values(&expression);
    }

    for operator in [
        UnaryOperator::BitwiseNot,
        UnaryOperator::Minus,
        UnaryOperator::Length,
        UnaryOperator::Not,
    ] {
        let expression = Expression::from(UnaryExpression::new(operator, Expression::from(6.0)));
        let _ = evaluator.evaluate(&expression);
        let _ = evaluator.evaluate_with(&expression, &|_| None);
        let _ = evaluator.is_pure(&expression);
        let _ = evaluator.has_side_effects(&expression);
    }

    // A non-integer operand makes the bitwise fold decline rather than guess.
    let fractional = Expression::from(BinaryExpression::new(
        BinaryOperator::BitwiseAnd,
        Expression::from(1.5),
        Expression::from(1.0),
    ));
    let _ = evaluator.evaluate(&fractional);
}
