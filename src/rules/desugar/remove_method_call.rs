use crate::nodes::{Block, Expression, FieldExpression, FunctionCall, Prefix};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

#[derive(Debug, Default)]
struct Processor {}

impl NodeProcessor for Processor {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Call(call) = expression {
            self.process_function_call(call);
        }
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        if !call.has_method() {
            return;
        }

        let replace_with: Option<Prefix> = match call.get_prefix() {
            Prefix::Identifier(identifier) => Some(identifier.clone().into()),
            Prefix::Parenthese(parenthese) => match *parenthese.inner_expression() {
                Expression::If(_)
                | Expression::Index(_)
                | Expression::Field(_)
                | Expression::Function(_)
                | Expression::Binary(_)
                | Expression::Table(_)
                | Expression::Unary(_)
                | Expression::VariableArguments(_)
                | Expression::TypeCast(_)
                | Expression::TypeInstantiation(_)
                | Expression::InterpolatedString(_)
                | Expression::Parenthese(_)
                | Expression::Call(_) => None,

                Expression::Nil(_)
                | Expression::True(_)
                | Expression::False(_)
                | Expression::String(_)
                | Expression::Number(_)
                | Expression::Identifier(_) => Some(parenthese.inner_expression().clone().into()),
            },
            _ => None,
        };

        if let Some(new_prefix) = replace_with {
            let method_name = call
                .take_method()
                .expect("method name is expected to exist");

            *call.mutate_prefix() = FieldExpression::new(new_prefix.clone(), method_name).into();
            call.mutate_arguments()
                .insert(0, Expression::from(new_prefix));
        }
    }
}

/// Converts method calls with identifier prefixes to regular function calls.
///
/// This pass transforms calls like `obj:method()` into `obj.method(obj)`.
pub fn remove_method_call(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor::default();
    DefaultVisitor::visit_block(block, &mut processor);
}
