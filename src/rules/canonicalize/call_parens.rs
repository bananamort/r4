use crate::nodes::{Arguments, Block, Expression, FunctionCall, StringExpression, TableExpression};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

use std::mem;

#[derive(Debug, Clone, Default)]
struct Processor {}

impl NodeProcessor for Processor {
    fn process_function_call(&mut self, call: &mut FunctionCall) {
        let new_arguments = match call.mutate_arguments() {
            Arguments::Tuple(tuple) if tuple.len() == 1 => {
                let expression = tuple
                    .iter_mut_values()
                    .next()
                    .expect("the tuple has one value");

                match expression {
                    Expression::String(string) => {
                        let mut steal_string = StringExpression::empty();
                        mem::swap(string, &mut steal_string);
                        Some(Arguments::String(steal_string))
                    }
                    Expression::Table(table) => {
                        let mut steal_table = TableExpression::default();
                        mem::swap(table, &mut steal_table);
                        Some(Arguments::Table(steal_table))
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        if let Some(new_arguments) = new_arguments {
            *call.mutate_arguments() = new_arguments;
        }
    }
}

/// Removes parentheses when calling functions with a string or a table
/// (`f("x")` becomes `f "x"`).
pub fn remove_call_parens(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor::default();
    DefaultVisitor::visit_block(block, &mut processor);
}
