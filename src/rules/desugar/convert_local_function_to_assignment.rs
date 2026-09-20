use std::fmt;
use std::mem;

use crate::nodes::{Block, FunctionExpression, Identifier, Statement, VariableAssignment};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId};

/// An error produced while converting local functions into assignments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertLocalFunctionError {
    /// The local function's name is not bound in the semantic model, so the
    /// rewrite cannot be proven safe. This means the AST does not match the
    /// analyzed model.
    UnresolvedFunction {
        /// The name of the unresolved function.
        name: String,
    },
}

impl fmt::Display for ConvertLocalFunctionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertLocalFunctionError::UnresolvedFunction { name } => write!(
                f,
                "local function `{}` is not bound in the semantic model",
                name
            ),
        }
    }
}

impl std::error::Error for ConvertLocalFunctionError {}

struct Processor<'a> {
    model: &'a SemanticModel,
    converted: usize,
    error: Option<ConvertLocalFunctionError>,
}

struct SymbolFinder<'a> {
    model: &'a SemanticModel,
    symbol: SymbolId,
    found: bool,
}

impl NodeProcessor for SymbolFinder<'_> {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        if !self.found && self.model.resolve_identifier(identifier) == Some(self.symbol) {
            self.found = true;
        }
    }
}

fn references_symbol(block: &mut Block, model: &SemanticModel, symbol: SymbolId) -> bool {
    let mut finder = SymbolFinder {
        model,
        symbol,
        found: false,
    };
    DefaultVisitor::visit_block(block, &mut finder);
    finder.found
}

impl NodeProcessor for Processor<'_> {
    fn process_statement(&mut self, statement: &mut Statement) {
        let Statement::LocalFunction(function) = statement else {
            return;
        };

        let symbol = match self.model.resolve_identifier(function.get_identifier()) {
            Some(symbol) => symbol,
            None => {
                self.error = Some(ConvertLocalFunctionError::UnresolvedFunction {
                    name: function.get_identifier().get_name().clone(),
                });
                return;
            }
        };

        // `local function f() ... end` makes `f` visible inside its own body.
        // The assignment form evaluates the function expression before `f` is
        // declared, so a self-reference would observe a different binding.
        // Every other reference (before or after) is safe.
        let mut body = mem::take(function.mutate_block());
        let recursive = references_symbol(&mut body, self.model, symbol);
        *function.mutate_block() = body;
        if recursive {
            return;
        }

        let kind = function.get_assignment_kind();
        let expression = FunctionExpression::new(
            mem::take(function.mutate_block()),
            mem::take(function.mutate_parameters()),
            function.is_variadic(),
        );

        *statement = VariableAssignment::from_variable(function.get_identifier().clone())
            .with_assignment_kind(kind)
            .with_value(expression)
            .into();
        self.converted += 1;
    }
}

/// Converts non-recursive `local function` statements into local assignments.
///
/// `local function f(...) ... end` becomes `local f = function(...) ... end`
/// when the function body never references the declared symbol. The recursion
/// check resolves identifiers through the semantic model, so a parameter or a
/// nested local with the same name does not count as a self-reference.
///
/// Returns the number of converted declarations. An unresolved function name
/// is a model mismatch and is reported as
/// [`ConvertLocalFunctionError::UnresolvedFunction`] instead of being skipped.
pub fn convert_local_function_to_assignment(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, ConvertLocalFunctionError> {
    let mut processor = Processor {
        model,
        converted: 0,
        error: None,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    match processor.error {
        Some(error) => Err(error),
        None => Ok(processor.converted),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::parse;
    use crate::semantic::SemanticAnalyzer;

    #[test]
    fn reports_a_foreign_model() {
        let mut block = parse("local function f() end");
        let other = parse("local function g() end");
        let model = SemanticAnalyzer::new()
            .analyze(&other)
            .expect("the other block must analyze");

        let error = convert_local_function_to_assignment(&mut block, &model)
            .expect_err("a function that is not bound in the model must be reported");

        assert!(matches!(
            error,
            ConvertLocalFunctionError::UnresolvedFunction { ref name } if name == "f"
        ));
    }
}
