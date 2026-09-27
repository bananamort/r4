use std::fmt;
use std::iter;

use bstr::ByteSlice;

use crate::nodes::{
    Block, Expression, FieldExpression, FunctionCall, Identifier, InterpolatedStringExpression,
    InterpolationSegment, Prefix, StringExpression, TupleArguments, TypedIdentifier,
    VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::SemanticModel;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReplacementStrategy {
    #[default]
    StringSpecifier,
    ToStringSpecifier,
}

/// An error produced while lowering interpolated strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveInterpolatedStringError {
    /// `tostring` may have been replaced, so the interpolation cannot be
    /// lowered to a call to it.
    UnstableTostring,
    /// `string.format` may have been replaced, so the interpolation cannot be
    /// lowered to a call to it.
    UnstableStringFormat,
}

impl fmt::Display for RemoveInterpolatedStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnstableTostring => write!(
                f,
                "cannot lower an interpolated string: the `tostring` global may have been replaced"
            ),
            Self::UnstableStringFormat => write!(
                f,
                "cannot lower an interpolated string: the `string` global may have been replaced"
            ),
        }
    }
}

impl std::error::Error for RemoveInterpolatedStringError {}

struct RemoveInterpolatedStringProcessor<'a> {
    model: &'a SemanticModel,
    names: NameGenerator,
    string_format_identifier: Option<String>,
    tostring_identifier: Option<String>,
    strategy: ReplacementStrategy,
    tostring_stable: bool,
    string_stable: bool,
    unstable: Option<RemoveInterpolatedStringError>,
}

const DEFAULT_TOSTRING_IDENTIFIER: &str = "tostring";
const DEFAULT_STRING_LIBRARY: &str = "string";
const DEFAULT_STRING_FORMAT_NAME: &str = "format";

impl RemoveInterpolatedStringProcessor<'_> {
    fn tostring_prefix(&mut self) -> Prefix {
        if self
            .model
            .name_is_transparent_global(DEFAULT_TOSTRING_IDENTIFIER)
        {
            Prefix::from_name(DEFAULT_TOSTRING_IDENTIFIER)
        } else {
            let name = self
                .tostring_identifier
                .get_or_insert_with(|| {
                    self.names
                        .generate_with_prefix("__R4_TO_STR")
                        .expect("the reserved prefix is a valid identifier")
                })
                .clone();
            Prefix::from_name(name)
        }
    }

    fn string_format_prefix(&mut self) -> Prefix {
        if self
            .model
            .name_is_transparent_global(DEFAULT_STRING_LIBRARY)
        {
            FieldExpression::new(
                Prefix::from_name(DEFAULT_STRING_LIBRARY),
                DEFAULT_STRING_FORMAT_NAME,
            )
            .into()
        } else {
            let name = self
                .string_format_identifier
                .get_or_insert_with(|| {
                    self.names
                        .generate_with_prefix("__R4_STR_FMT")
                        .expect("the reserved prefix is a valid identifier")
                })
                .clone();
            Prefix::from_name(name)
        }
    }

    /// Returns which standard-library globals the rewrite of `string` calls, as
    /// `(tostring, string.format)`.
    fn globals_used(&self, string: &InterpolatedStringExpression) -> (bool, bool) {
        if string.is_empty() {
            return (false, false);
        }
        if string.len() == 1 {
            return match string.iter_segments().next().expect("a single segment") {
                InterpolationSegment::String(_) => (false, false),
                InterpolationSegment::Value(_) => (true, false),
            };
        }

        let has_value = string
            .iter_segments()
            .any(|segment| matches!(segment, InterpolationSegment::Value(_)));
        let tostring = has_value && self.strategy == ReplacementStrategy::StringSpecifier;
        (tostring, true)
    }

    fn replace_with(&mut self, string: &InterpolatedStringExpression) -> Expression {
        if string.is_empty() {
            StringExpression::from_value("").into()
        } else if string.len() == 1 {
            match string.iter_segments().next().unwrap() {
                InterpolationSegment::String(string_segment) => {
                    StringExpression::from_value(string_segment.get_value()).into()
                }
                InterpolationSegment::Value(value_segment) => {
                    let prefix = self.tostring_prefix();
                    FunctionCall::from_prefix(prefix)
                        .with_argument(value_segment.get_expression().clone())
                        .into()
                }
            }
        } else {
            let arguments = iter::once(
                StringExpression::from_value(string.iter_segments().fold(
                    Vec::new(),
                    |mut format_string, segment| {
                        match segment {
                            InterpolationSegment::String(string_segment) => {
                                format_string.extend_from_slice(
                                    &string_segment.get_value().replace(b"%", b"%%"),
                                );
                            }
                            InterpolationSegment::Value(_) => {
                                format_string.extend_from_slice(match self.strategy {
                                    ReplacementStrategy::StringSpecifier => b"%s",
                                    ReplacementStrategy::ToStringSpecifier => b"%*",
                                });
                            }
                        }
                        format_string
                    },
                ))
                .into(),
            )
            .chain(
                string
                    .iter_segments()
                    .filter_map(|segment| match segment {
                        InterpolationSegment::Value(segment) => {
                            Some(segment.get_expression().clone())
                        }
                        InterpolationSegment::String(_) => None,
                    })
                    .map(|value| match self.strategy {
                        ReplacementStrategy::ToStringSpecifier => value,
                        ReplacementStrategy::StringSpecifier => {
                            let prefix = self.tostring_prefix();
                            FunctionCall::from_prefix(prefix)
                                .with_argument(value)
                                .into()
                        }
                    }),
            )
            .collect::<TupleArguments>();

            let prefix = self.string_format_prefix();
            FunctionCall::from_prefix(prefix)
                .with_arguments(arguments)
                .into()
        }
    }
}

impl NodeProcessor for RemoveInterpolatedStringProcessor<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::InterpolatedString(string) = expression else {
            return;
        };

        let (tostring, string_format) = self.globals_used(string);
        if tostring && !self.tostring_stable {
            self.unstable = Some(RemoveInterpolatedStringError::UnstableTostring);
            return;
        }
        if string_format && !self.string_stable {
            self.unstable = Some(RemoveInterpolatedStringError::UnstableStringFormat);
            return;
        }

        *expression = self.replace_with(string);
    }
}

/// Removes Luau backtick interpolated strings, replacing them with
/// `string.format` calls (using the `%s` specifier).
///
/// `tostring` and `string` resolve through the semantic model. Read-only
/// aliases such as `local tostring = tostring` are transparent, so no alias is
/// introduced for them; otherwise a collision-free alias allocated by
/// [`NameGenerator`] is inserted at the top of the block.
///
/// The lowering calls `tostring` and `string.format`, so it fails with
/// [`RemoveInterpolatedStringError`] when the program can replace either (by
/// writing it, or by reaching globals from code the model cannot see) rather
/// than emitting a call that may not convert a value the way interpolation
/// does.
pub fn remove_interpolated_string(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<(), RemoveInterpolatedStringError> {
    remove_interpolated_string_with_strategy(block, model, ReplacementStrategy::StringSpecifier)
}

/// Removes interpolated strings using the `%*` format specifier, which relies
/// on `string.format` converting its arguments with `tostring`.
pub fn remove_interpolated_string_with_tostring(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<(), RemoveInterpolatedStringError> {
    remove_interpolated_string_with_strategy(block, model, ReplacementStrategy::ToStringSpecifier)
}

fn remove_interpolated_string_with_strategy(
    block: &mut Block,
    model: &SemanticModel,
    strategy: ReplacementStrategy,
) -> Result<(), RemoveInterpolatedStringError> {
    let mut processor = RemoveInterpolatedStringProcessor {
        model,
        names: NameGenerator::new(model),
        string_format_identifier: None,
        tostring_identifier: None,
        strategy,
        tostring_stable: model.global_is_stable(DEFAULT_TOSTRING_IDENTIFIER),
        string_stable: model.global_is_stable(DEFAULT_STRING_LIBRARY),
        unstable: None,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    if let Some(error) = processor.unstable {
        return Err(error);
    }

    let string_format_identifier = processor.string_format_identifier;
    let tostring_identifier = processor.tostring_identifier;

    if string_format_identifier.is_some() || tostring_identifier.is_some() {
        let mut variables = Vec::new();
        let mut values = Vec::new();

        if let Some(identifier) = string_format_identifier {
            variables.push(TypedIdentifier::new(identifier));
            values.push(
                FieldExpression::new(
                    Prefix::from_name(DEFAULT_STRING_LIBRARY),
                    DEFAULT_STRING_FORMAT_NAME,
                )
                .into(),
            );
        }

        if let Some(identifier) = tostring_identifier {
            variables.push(TypedIdentifier::new(identifier));
            values.push(Identifier::new(DEFAULT_TOSTRING_IDENTIFIER).into());
        }

        block.insert_statement(0, VariableAssignment::new(variables, values));
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn lowers_a_plain_interpolated_string() {
        let (mut block, model) = analyze("return `hello`");

        remove_interpolated_string(&mut block, &model).expect("no library call is emitted");

        insta::assert_snapshot!(generate(&block).trim(), @"return 'hello'");
    }

    #[test]
    fn lowers_interpolation_with_tostring() {
        let (mut block, model) = analyze("return `value = {value}`");

        remove_interpolated_string(&mut block, &model).expect("the libraries are stable");

        insta::assert_snapshot!(
            generate(&block).trim(),
            @"return string.format('value = %s', tostring(value))"
        );
    }

    #[test]
    fn rejects_a_replaced_tostring() {
        let (mut block, model) = analyze("tostring = function() end\nreturn `{value}`");

        let error =
            remove_interpolated_string(&mut block, &model).expect_err("`tostring` was replaced");

        assert_eq!(error, RemoveInterpolatedStringError::UnstableTostring);
    }

    #[test]
    fn rejects_a_replaced_string_library() {
        let (mut block, model) = analyze("string.format = function() end\nreturn `{a} {b}`");

        let error =
            remove_interpolated_string(&mut block, &model).expect_err("`string` was replaced");

        assert_eq!(error, RemoveInterpolatedStringError::UnstableStringFormat);
    }

    #[test]
    fn accepts_an_unstable_environment_without_interpolation() {
        let (mut block, model) = analyze("require('./module')\nreturn value");

        remove_interpolated_string(&mut block, &model).expect("there is nothing to lower");
    }
}
