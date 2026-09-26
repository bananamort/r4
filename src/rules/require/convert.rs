use std::fmt;

use crate::nodes::{Block, Expression, FunctionCall, Prefix, Statement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;
use crate::utils::expressions_as_statement;

use crate::modules::{classify_require_call, require_call_line, InstanceSegment, RequireCall};

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

    /// Returns the expression that replaces a Roblox/Luau instance `require`.
    ///
    /// The default leaves instance requires untouched; implementations that
    /// understand the project's file layout override this.
    fn convert_instance(&self, _segments: &[InstanceSegment]) -> Option<Expression> {
        None
    }
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
    fn classify(&mut self, call: &FunctionCall) -> Option<Classified> {
        match classify_require_call(call, self.model) {
            RequireCall::NotRequire => None,
            RequireCall::Path { path } => Some(Classified::Path(path)),
            RequireCall::Instance { segments } => Some(Classified::Instance(segments)),
            RequireCall::NonLiteral => {
                self.error
                    .get_or_insert(ConvertRequireError::NonLiteralRequireArgument {
                        line: require_call_line(call),
                    });
                None
            }
        }
    }

    fn replacement(&self, classified: Classified) -> Option<Expression> {
        match classified {
            Classified::Path(path) => self.converter.convert(&path),
            Classified::Instance(segments) => self.converter.convert_instance(&segments),
        }
    }
}

enum Classified {
    Path(String),
    Instance(Vec<InstanceSegment>),
}

impl<C: RequirePathConverter> NodeProcessor for Processor<'_, C> {
    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::Call(call) = expression else {
            return;
        };
        let Some(classified) = self.classify(call) else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        if let Some(replacement) = self.replacement(classified) {
            *expression = replacement;
            self.converted += 1;
        }
    }

    fn process_statement(&mut self, statement: &mut Statement) {
        let Statement::Call(call) = statement else {
            return;
        };
        let Some(classified) = self.classify(call) else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        if let Some(replacement) = self.replacement(classified) {
            *statement = expressions_as_statement(vec![replacement]);
            self.converted += 1;
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        // A call used as the prefix of a field/index/another call
        // (`require('./m').field`, `require('./m')()`) is not an
        // `Expression::Call`, so it must be handled here.
        let Prefix::Call(call) = prefix else {
            return;
        };
        let Some(classified) = self.classify(call) else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        if let Some(replacement) = self.replacement(classified) {
            *prefix = Prefix::from(replacement);
            self.converted += 1;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nodes::Expression;
    use crate::rules::test_support::{analyze, generate};

    struct IdentifierConverter;

    impl RequirePathConverter for IdentifierConverter {
        fn convert(&self, path: &str) -> Option<Expression> {
            let name = path.trim_start_matches("./").replace(['/', '.'], "_");
            Some(Expression::identifier(format!("module_{name}")))
        }
    }

    #[test]
    fn rejects_shadowed_require() {
        for code in [
            "local require = function(path) return path end\nreturn require('./x')",
            "local function f(require) return require('./x') end",
            "local require = require\nrequire = other\nreturn require('./x')",
        ] {
            let (mut block, model) = analyze(code);
            let converted = convert_require(&mut block, &model, &IdentifierConverter)
                .expect("shadowed require calls are never converted");

            assert_eq!(converted, 0, "converted a shadowed require for `{code}`");
            assert!(
                generate(&block).contains("require("),
                "the shadowed call must be left in place for `{code}`"
            );
        }
    }

    #[test]
    fn converts_global_and_immutable_alias() {
        for code in [
            "return require('./x')",
            "local require = require\nreturn require('./x')",
        ] {
            let (mut block, model) = analyze(code);
            let converted = convert_require(&mut block, &model, &IdentifierConverter)
                .expect("global require must convert");

            assert_eq!(converted, 1, "failed to convert `{code}`");
            let output = generate(&block);
            assert!(output.contains("module_x"), "unexpected: {output}");
            assert!(!output.contains("require("), "unexpected: {output}");
        }
    }

    #[test]
    fn converts_statement_calls() {
        let (mut block, model) = analyze("require('./x')");
        let converted = convert_require(&mut block, &model, &IdentifierConverter)
            .expect("global require must convert");

        assert_eq!(converted, 1);
        let output = generate(&block);
        assert!(output.contains("module_x"), "unexpected: {output}");
    }

    #[test]
    fn rejects_non_literal_arguments() {
        let (mut block, model) = analyze("return require(path)");
        let error = convert_require(&mut block, &model, &IdentifierConverter)
            .expect_err("a dynamic require argument must be reported");

        assert!(matches!(
            error,
            ConvertRequireError::NonLiteralRequireArgument { .. }
        ));
    }

    #[test]
    fn converts_calls_in_prefix_position() {
        for code in [
            "return require('./x').field",
            "return require('./x').a.b",
            "return require('./x')()",
            "return (require('./x')).field",
        ] {
            let (mut block, model) = analyze(code);
            let converted = convert_require(&mut block, &model, &IdentifierConverter)
                .expect("global require must convert");

            assert_eq!(converted, 1, "failed to convert `{code}`");
            let output = generate(&block);
            assert!(output.contains("module_x"), "unexpected: {output}");
            assert!(!output.contains("require("), "unexpected: {output}");
        }
    }

    #[test]
    fn rejects_non_literal_require_in_prefix_position() {
        let (mut block, model) = analyze("return require(path).field");
        let error = convert_require(&mut block, &model, &IdentifierConverter)
            .expect_err("a dynamic require argument must be reported");

        assert!(matches!(
            error,
            ConvertRequireError::NonLiteralRequireArgument { .. }
        ));
    }
}
