use std::mem;

use crate::nodes::{
    AssignStatement, Block, FieldExpression, FunctionExpression, FunctionStatement, Identifier,
    Statement, TypedIdentifier, Variable,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl Processor {
    fn convert(function: &mut FunctionStatement) -> Statement {
        let name = function.get_name();

        let base = name.get_name().clone();
        let fields = name.get_field_names().clone();
        let method = name.get_method().cloned();

        let variable = if fields.is_empty() {
            match &method {
                Some(method) => Variable::from(FieldExpression::new(base, method.clone())),
                None => Variable::from(base),
            }
        } else {
            let mut fields_iter = fields.iter().cloned().chain(method.iter().cloned());
            let mut current = FieldExpression::new(
                base,
                fields_iter
                    .next()
                    .expect("a non-empty field list has a first field"),
            );
            for field in fields_iter {
                current = FieldExpression::new(current, field);
            }
            Variable::from(current)
        };

        let mut expression = FunctionExpression::new(
            mem::take(function.mutate_block()),
            mem::take(function.mutate_parameters()),
            function.is_variadic(),
        );

        if method.is_some() {
            expression
                .mutate_parameters()
                .insert(0, TypedIdentifier::from(Identifier::new("self")));
        }

        AssignStatement::from_variable(variable, expression).into()
    }
}

impl NodeProcessor for Processor {
    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::Function(function) = statement {
            *statement = Self::convert(function);
        }
    }
}

/// Converts function statements into regular assignments.
///
/// `function value.name(...)` becomes `value.name = function(...)`, and a
/// method definition becomes a field assignment with the implicit `self`
/// parameter made explicit.
pub fn convert_function_to_assignment(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn converts_function_statements_to_assignments() {
        let (mut block, model) = analyze("function f(a) return a end");

        convert_function_to_assignment(&mut block, &model);

        let output = generate(&block);
        assert!(output.contains("f = function(a)"), "unexpected: {output}");
    }

    #[test]
    fn keeps_dotted_targets() {
        let (mut block, model) = analyze("function a.b.c(x) end");

        convert_function_to_assignment(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"a.b.c = function(x) end");
    }
}
