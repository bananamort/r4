use crate::nodes::{
    AssignStatement, BinaryExpression, Block, CompoundAssignStatement, DoStatement, Expression,
    FieldExpression, IndexExpression, Prefix, Statement, Variable, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::SemanticModel;

/// Converts compound assignments (`+=`, `..=`, ...) into regular assignments.
///
/// The variable is evaluated only once: when the left-hand side (or its index)
/// has side effects, it is hoisted into a generated local inside a `do ... end`
/// block. Generated names are allocated by [`NameGenerator`], so they cannot
/// collide with any analyzed symbol.
struct Processor {
    names: NameGenerator,
}

impl Processor {
    fn new(model: &SemanticModel) -> Self {
        Self {
            names: NameGenerator::new(model),
        }
    }

    #[inline]
    fn generate_variable(&mut self) -> String {
        self.names.generate_with_prefix("__R4_VAR")
    }

    fn simplify_prefix(&self, prefix: &Prefix) -> Option<Prefix> {
        match prefix {
            Prefix::Parenthese(parenthese) => {
                if let Expression::Identifier(identifier) = parenthese.inner_expression() {
                    Some(Prefix::from(identifier.clone()))
                } else {
                    None
                }
            }
            Prefix::Identifier(_)
            | Prefix::Call(_)
            | Prefix::Field(_)
            | Prefix::Index(_)
            | Prefix::TypeInstantiation(_) => None,
        }
    }

    fn remove_parentheses(&self, expression: impl Into<Expression>) -> Expression {
        let expression = expression.into();
        if let Expression::Parenthese(parenthese) = expression {
            parenthese.into_inner_expression()
        } else {
            expression
        }
    }

    fn replace_with(&mut self, assignment: &CompoundAssignStatement) -> Option<Statement> {
        match assignment.get_variable() {
            Variable::Index(index) => {
                let prefix_assignment = match index.get_prefix() {
                    Prefix::Identifier(_) => None,
                    Prefix::Parenthese(parenthese)
                        if matches!(
                            parenthese.inner_expression(),
                            Expression::False(_)
                                | Expression::Identifier(_)
                                | Expression::Number(_)
                                | Expression::Nil(_)
                                | Expression::String(_)
                                | Expression::True(_)
                                | Expression::VariableArguments(_)
                        ) =>
                    {
                        None
                    }
                    Prefix::Call(_)
                    | Prefix::Field(_)
                    | Prefix::Index(_)
                    | Prefix::Parenthese(_)
                    | Prefix::TypeInstantiation(_) => Some(self.generate_variable()),
                };
                let index_assignment = match index.get_index() {
                    Expression::False(_)
                    | Expression::Identifier(_)
                    | Expression::Number(_)
                    | Expression::Nil(_)
                    | Expression::InterpolatedString(_)
                    | Expression::String(_)
                    | Expression::True(_)
                    | Expression::VariableArguments(_) => None,
                    Expression::Parenthese(parenthese)
                        if matches!(
                            parenthese.inner_expression(),
                            Expression::False(_)
                                | Expression::Identifier(_)
                                | Expression::Number(_)
                                | Expression::Nil(_)
                                | Expression::String(_)
                                | Expression::True(_)
                                | Expression::VariableArguments(_)
                        ) =>
                    {
                        None
                    }
                    Expression::Binary(_)
                    | Expression::Call(_)
                    | Expression::Field(_)
                    | Expression::Function(_)
                    | Expression::If(_)
                    | Expression::Index(_)
                    | Expression::Parenthese(_)
                    | Expression::Table(_)
                    | Expression::TypeCast(_)
                    | Expression::TypeInstantiation(_)
                    | Expression::Unary(_) => Some(self.generate_variable()),
                };

                match (prefix_assignment, index_assignment) {
                    (None, None) => {
                        let variable = IndexExpression::new(
                            self.simplify_prefix(index.get_prefix())
                                .unwrap_or_else(|| index.get_prefix().clone()),
                            self.remove_parentheses(index.get_index().clone()),
                        );

                        Some(self.create_new_assignment_with_variable(
                            assignment,
                            variable.clone().into(),
                            Some(variable.into()),
                        ))
                    }
                    (None, Some(index_variable)) => {
                        let assign = VariableAssignment::from_variable(index_variable.clone())
                            .with_value(self.remove_parentheses(index.get_index().clone()));
                        let variable = IndexExpression::new(
                            self.simplify_prefix(index.get_prefix())
                                .unwrap_or_else(|| index.get_prefix().clone()),
                            Expression::identifier(index_variable),
                        );
                        Some(self.create_do_assignment(assignment, assign, variable))
                    }
                    (Some(prefix_variable), None) => {
                        let assign = VariableAssignment::from_variable(prefix_variable.clone())
                            .with_value(self.remove_parentheses(index.get_prefix().clone()));
                        let variable = IndexExpression::new(
                            Prefix::from_name(prefix_variable),
                            index.get_index().clone(),
                        );

                        Some(self.create_do_assignment(assignment, assign, variable))
                    }
                    (Some(prefix_variable), Some(index_variable)) => {
                        let assign = VariableAssignment::from_variable(prefix_variable.clone())
                            .with_value(self.remove_parentheses(index.get_prefix().clone()))
                            .with_variable(index_variable.clone())
                            .with_value(self.remove_parentheses(index.get_index().clone()));
                        let variable = IndexExpression::new(
                            Prefix::from_name(prefix_variable),
                            Expression::identifier(index_variable),
                        );
                        Some(self.create_do_assignment(assignment, assign, variable))
                    }
                }
            }
            Variable::Field(field) => match field.get_prefix() {
                Prefix::Identifier(_) => None,
                Prefix::Parenthese(parenthese)
                    if matches!(
                        parenthese.inner_expression(),
                        Expression::False(_)
                            | Expression::Identifier(_)
                            | Expression::Number(_)
                            | Expression::Nil(_)
                            | Expression::String(_)
                            | Expression::True(_)
                            | Expression::VariableArguments(_)
                    ) =>
                {
                    let new_prefix =
                        if let Expression::Identifier(identifier) = parenthese.inner_expression() {
                            Prefix::from(identifier.clone())
                        } else {
                            (*parenthese.clone()).into()
                        };
                    let new_variable = FieldExpression::new(new_prefix, field.get_field().clone());

                    Some(self.create_new_assignment_with_variable(
                        assignment,
                        new_variable.clone().into(),
                        Some(new_variable.into()),
                    ))
                }
                Prefix::Call(_)
                | Prefix::Field(_)
                | Prefix::Index(_)
                | Prefix::Parenthese(_)
                | Prefix::TypeInstantiation(_) => {
                    let identifier = self.generate_variable();
                    let new_variable = FieldExpression::new(
                        Prefix::from_name(&identifier),
                        field.get_field().clone(),
                    );

                    let assign = VariableAssignment::from_variable(identifier).with_value(
                        match field.get_prefix().clone() {
                            Prefix::Parenthese(parenthese) => parenthese.into_inner_expression(),
                            prefix => prefix.into(),
                        },
                    );

                    Some(self.create_do_assignment(assignment, assign, new_variable))
                }
            },
            Variable::Identifier(_) => None,
        }
    }

    fn create_do_assignment(
        &mut self,
        compound_assignment: &CompoundAssignStatement,
        assign: impl Into<Statement>,
        variable: impl Into<Variable>,
    ) -> Statement {
        let variable = variable.into();
        DoStatement::new(
            Block::default()
                .with_statement(assign.into())
                .with_statement(self.create_new_assignment_with_variable(
                    compound_assignment,
                    variable.clone().into(),
                    Some(variable),
                )),
        )
        .into()
    }

    fn create_new_assignment(
        &mut self,
        assignment: &CompoundAssignStatement,
        variable: impl Into<Expression>,
    ) -> Statement {
        self.create_new_assignment_with_variable(assignment, variable.into(), None)
    }

    fn create_new_assignment_with_variable(
        &mut self,
        assignment: &CompoundAssignStatement,
        value: Expression,
        variable: Option<Variable>,
    ) -> Statement {
        let operator = assignment.get_operator().to_binary_operator();

        let mut expression = BinaryExpression::new(operator, value, assignment.get_value().clone());
        if let Some(token) = assignment.get_tokens().map(|tokens| {
            let mut new_token = tokens.operator.clone();
            new_token.replace_with_content(operator.to_str());
            new_token
        }) {
            expression.set_token(token);
        }

        AssignStatement::from_variable(
            variable.unwrap_or_else(|| assignment.get_variable().clone()),
            expression,
        )
        .into()
    }
}

impl NodeProcessor for Processor {
    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::CompoundAssign(assignment) = statement {
            let variable = assignment.get_variable();
            *statement = self
                .replace_with(assignment)
                .unwrap_or_else(|| self.create_new_assignment(assignment, variable.clone()));
        }
    }
}

/// Replaces a single compound assignment statement with its desugared form.
pub(crate) fn replace_compound_assignment(statement: &mut Statement, model: &SemanticModel) {
    let mut processor = Processor::new(model);
    processor.process_statement(statement);
}

/// Removes compound assignments from a block.
pub fn remove_compound_assign(block: &mut Block, model: &SemanticModel) {
    let mut processor = Processor::new(model);
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn removes_compound_assignments() {
        let (mut block, model) = analyze("x += 1");

        remove_compound_assign(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"x = x + 1");
    }

    #[test]
    fn evaluates_the_target_index_once() {
        let (mut block, model) = analyze("t[index()] += 1");

        remove_compound_assign(&mut block, &model);

        let output = generate(&block);
        assert!(output.contains("index()"), "unexpected: {output}");
        assert_eq!(
            output.matches("index()").count(),
            1,
            "the index expression must be evaluated only once: {output}"
        );
    }
}
