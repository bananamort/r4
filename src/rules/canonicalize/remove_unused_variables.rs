use std::fmt;

use crate::nodes::{Block, Expression, FunctionAssignment, Identifier, Statement};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId};
use crate::utils::expressions_as_statement;

/// An error produced while removing unused variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnusedVariableError {
    /// A declaration is not bound in the semantic model. This usually means
    /// the AST does not match the analyzed model.
    UnresolvedDeclaration {
        /// The name of the unresolved declaration.
        name: String,
        /// The source line of the declaration, if known.
        line: Option<usize>,
    },
}

impl fmt::Display for UnusedVariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnusedVariableError::UnresolvedDeclaration { name, line } => match line {
                Some(line) => write!(
                    f,
                    "line {}: declaration of `{}` is not bound in the semantic model",
                    line, name
                ),
                None => write!(
                    f,
                    "declaration of `{}` is not bound in the semantic model",
                    name
                ),
            },
        }
    }
}

impl std::error::Error for UnusedVariableError {}

/// Removes local variables and local functions that are never read.
///
/// Unused detection is `O(1)` per declaration (`Symbol::read_count`); the
/// initializer values are preserved as expression statements when they have
/// side effects, using [`Evaluator::has_side_effects`].
///
/// Only single-variable assignments and assignments whose variables are all
/// unused are rewritten: partially-used multi-assignments are left untouched
/// because removing them requires re-aligning multiple return values.
///
/// Declarations are resolved through the structural bindings recorded by
/// semantic analysis, so the pass works on token-less trees. When any
/// declaration cannot be resolved the pass fails without modifying the block.
pub fn remove_unused_variables(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, UnusedVariableError> {
    validate_declarations(block, model)?;

    let mut processor = Processor {
        model,
        evaluator: Evaluator::default(),
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    Ok(processor.removed)
}

fn validate_declarations(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<(), UnusedVariableError> {
    let mut validator = Validator { model, error: None };
    DefaultVisitor::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Validator<'a> {
    model: &'a SemanticModel,
    error: Option<UnusedVariableError>,
}

impl Validator<'_> {
    fn check(&mut self, identifier: &Identifier) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = resolve_declaration(self.model, identifier) {
            self.error = Some(error);
        }
    }
}

impl NodeProcessor for Validator<'_> {
    fn process_local_assign_statement(
        &mut self,
        assignment: &mut crate::nodes::VariableAssignment,
    ) {
        for variable in assignment.iter_variables() {
            self.check(variable.get_identifier());
        }
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        self.check(function.get_identifier());
    }
}

fn resolve_declaration(
    model: &SemanticModel,
    identifier: &Identifier,
) -> Result<SymbolId, UnusedVariableError> {
    model
        .resolve_identifier(identifier)
        .ok_or_else(|| UnusedVariableError::UnresolvedDeclaration {
            name: identifier.get_name().clone(),
            line: identifier
                .get_token()
                .and_then(|token| token.get_line_number()),
        })
}

struct Processor<'a> {
    model: &'a SemanticModel,
    evaluator: Evaluator,
    removed: usize,
}

impl Processor<'_> {
    /// Returns the symbol of a removable declaration, or `None` when the
    /// declaration is read, written again later, or could not be resolved.
    ///
    /// A declaration may only be removed when it is the symbol's *only* write:
    /// if a later `x = ...` exists, removing `local x` would turn that write
    /// into an assignment to a global.
    fn removable_symbol(&self, identifier: &Identifier) -> Option<SymbolId> {
        let symbol = resolve_declaration(self.model, identifier)
            .expect("declarations were validated before the rewrite");
        (self.model.is_unused(symbol) && self.model.is_immutable(symbol)).then_some(symbol)
    }

    fn side_effects<'a>(&self, values: impl Iterator<Item = &'a Expression>) -> Vec<Expression> {
        values
            .filter(|value| self.evaluator.has_side_effects(value))
            .cloned()
            .collect()
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();

        statements.retain_mut(|statement| self.keep_statement(statement));

        block.set_statements(statements);
    }
}

impl Processor<'_> {
    fn keep_statement(&mut self, statement: &mut Statement) -> bool {
        match statement {
            Statement::LocalAssign(assignment) => {
                // Removing variables from a multi-assignment changes value
                // alignment; only single-variable assignments are rewritten.
                if assignment.variables_len() != 1 {
                    return true;
                }

                let is_unused = {
                    let identifier = assignment
                        .iter_variables()
                        .next()
                        .expect("a local assignment has at least one variable")
                        .get_identifier();
                    self.removable_symbol(identifier).is_some()
                };

                if !is_unused {
                    return true;
                }

                let values = self.side_effects(assignment.iter_values());
                if values.is_empty() {
                    self.removed += 1;
                    false
                } else {
                    *statement = expressions_as_statement(values);
                    self.removed += 1;
                    true
                }
            }
            Statement::LocalFunction(function)
                if self.removable_symbol(function.get_identifier()).is_some() =>
            {
                self.removed += 1;
                false
            }
            _ => true,
        }
    }
}
