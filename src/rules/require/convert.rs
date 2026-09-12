use std::fmt;

use crate::nodes::{Block, Expression, FunctionCall, Statement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;
use crate::utils::expressions_as_statement;

use super::{classify_require_call, require_call_line, RequireCall};

/// An error produced while converting `require` calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertRequireError {
    /// A call that resolves to the global `require` does not use a single
    /// string-literal argument, so it cannot be converted statically.
    NonLiteralRequireArgument {
        /// The source line of the call, if known.
        line: Option<usize>,
    },
}

impl fmt::Display for ConvertRequireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertRequireError::NonLiteralRequireArgument { line } => match line {
                Some(line) => write!(
                    f,
                    "line {}: require argument is not a single string literal",
                    line
                ),
                None => write!(f, "require argument is not a single string literal"),
            },
        }
    }
}

impl std::error::Error for ConvertRequireError {}

/// Maps a statically resolved `require` path to a replacement expression.
///
/// Returning `None` means this converter does not handle the path; the call is
/// left exactly as it is. A shadowed `require` never reaches the converter.
pub trait RequirePathConverter {
    /// Returns the expression that replaces `require(path)`.
    fn convert(&self, path: &str) -> Option<Expression>;
}

/// Rewrites every `require("<path>")` call whose prefix resolves to the global
/// `require` (directly or through an immutable alias).
///
/// Returns the number of converted calls. Shadowed `require` calls are left
/// untouched; a resolved `require` call without a static string argument is
/// reported as [`ConvertRequireError::NonLiteralRequireArgument`].
pub fn convert_require<C: RequirePathConverter>(
    block: &mut Block,
    model: &SemanticModel,
    converter: &C,
) -> Result<usize, ConvertRequireError> {
    let mut processor = Processor {
        model,
        converter,
        converted: 0,
        error: None,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    match processor.error {
        Some(error) => Err(error),
        None => Ok(processor.converted),
    }
}

struct Processor<'a, C> {
    model: &'a SemanticModel,
    converter: &'a C,
    converted: usize,
    error: Option<ConvertRequireError>,
}

impl<C: RequirePathConverter> Processor<'_, C> {
    /// Classifies a call without holding a borrow across the rewrite.
    fn classify(&mut self, call: &FunctionCall) -> Option<String> {
        match classify_require_call(call, self.model) {
            RequireCall::NotRequire => None,
            RequireCall::Path { path } => Some(path),
            RequireCall::NonLiteral => {
                self.error
                    .get_or_insert(ConvertRequireError::NonLiteralRequireArgument {
                        line: require_call_line(call),
                    });
                None
            }
        }
    }
}

impl<C: RequirePathConverter> NodeProcessor for Processor<'_, C> {
    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::Call(call) = expression else {
            return;
        };
        let Some(path) = self.classify(call) else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        if let Some(replacement) = self.converter.convert(&path) {
            *expression = replacement;
            self.converted += 1;
        }
    }

    fn process_statement(&mut self, statement: &mut Statement) {
        let Statement::Call(call) = statement else {
            return;
        };
        let Some(path) = self.classify(call) else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        if let Some(replacement) = self.converter.convert(&path) {
            *statement = expressions_as_statement(vec![replacement]);
            self.converted += 1;
        }
    }
}
