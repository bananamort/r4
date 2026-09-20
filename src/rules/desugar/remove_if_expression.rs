use crate::nodes::{
    BinaryExpression, BinaryOperator, Block, Expression, IndexExpression, TableEntry,
    TableExpression,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

#[derive(Default)]
struct Processor {
    evaluator: Evaluator,
}

impl Processor {
    fn wrap_in_table(&self, expression: Expression) -> Expression {
        TableExpression::new(vec![TableEntry::from_value({
            if self.evaluator.can_return_multiple_values(&expression) {
                expression.in_parentheses()
            } else {
                expression
            }
        })])
        .into()
    }

    fn convert_if_branch(
        &self,
        condition: Expression,
        result: Expression,
        else_result: Expression,
    ) -> Expression {
        if self
            .evaluator
            .evaluate(&result)
            .is_truthy()
            .unwrap_or(false)
        {
            BinaryExpression::new(
                BinaryOperator::Or,
                BinaryExpression::new(BinaryOperator::And, condition, result),
                else_result,
            )
            .into()
        } else {
            IndexExpression::new(
                Expression::from(BinaryExpression::new(
                    BinaryOperator::Or,
                    BinaryExpression::new(
                        BinaryOperator::And,
                        condition,
                        self.wrap_in_table(result),
                    ),
                    self.wrap_in_table(else_result),
                )),
                Expression::from(1),
            )
            .into()
        }
    }
}

impl NodeProcessor for Processor {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::If(if_expression) = expression {
            let else_result = if_expression.iter_branches().fold(
                if_expression.get_else_result().clone(),
                |else_result, branch| {
                    self.convert_if_branch(
                        branch.get_condition().clone(),
                        branch.get_result().clone(),
                        else_result,
                    )
                },
            );

            *expression = self.convert_if_branch(
                if_expression.get_condition().clone(),
                if_expression.get_result().clone(),
                else_result,
            );
        }
    }
}

/// Removes Luau `if ... then ... else ...` expressions, replacing them with
/// `and`/`or` chains (and table indexing when a result may be falsy).
pub fn remove_if_expression(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor::default();
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nodes::Statement;
    use crate::process::{Evaluator, LuaValue};
    use crate::rules::test_support::{analyze, generate, parse};

    #[test]
    fn lowers_if_expressions() {
        let (mut block, model) = analyze("local x = if true then 1 else 2");

        remove_if_expression(&mut block, &model);

        let output = generate(&block);
        let generated = parse(&output);

        let Statement::LocalAssign(assign) = generated
            .first_statement()
            .expect("the generated block starts with an assignment")
        else {
            panic!("expected a local assignment, got: {output}");
        };

        let value = assign.last_value().expect("the assignment has a value");
        assert_eq!(Evaluator::default().evaluate(value), LuaValue::Number(1.0));
    }
}
