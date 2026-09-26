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

struct RemoveInterpolatedStringProcessor<'a> {
    model: &'a SemanticModel,
    names: NameGenerator,
    string_format_identifier: Option<String>,
    tostring_identifier: Option<String>,
    strategy: ReplacementStrategy,
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
        if let Expression::InterpolatedString(string) = expression {
            *expression = self.replace_with(string);
        }
    }
}

/// Removes Luau backtick interpolated strings, replacing them with
/// `string.format` calls (using the `%s` specifier).
///
/// `tostring` and `string` resolve through the semantic model. Read-only
/// aliases such as `local tostring = tostring` are transparent, so no alias is
/// introduced for them; otherwise a collision-free alias allocated by
/// [`NameGenerator`] is inserted at the top of the block.
pub fn remove_interpolated_string(block: &mut Block, model: &SemanticModel) {
    remove_interpolated_string_with_strategy(block, model, ReplacementStrategy::StringSpecifier);
}

/// Removes interpolated strings using the `%*` format specifier, which relies
/// on `string.format` converting its arguments with `tostring`.
pub fn remove_interpolated_string_with_tostring(block: &mut Block, model: &SemanticModel) {
    remove_interpolated_string_with_strategy(block, model, ReplacementStrategy::ToStringSpecifier);
}

fn remove_interpolated_string_with_strategy(
    block: &mut Block,
    model: &SemanticModel,
    strategy: ReplacementStrategy,
) {
    let mut processor = RemoveInterpolatedStringProcessor {
        model,
        names: NameGenerator::new(model),
        string_format_identifier: None,
        tostring_identifier: None,
        strategy,
    };
    DefaultVisitor::visit_block(block, &mut processor);

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
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn lowers_a_plain_interpolated_string() {
        let (mut block, model) = analyze("return `hello`");

        remove_interpolated_string(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 'hello'");
    }

    #[test]
    fn lowers_interpolation_with_tostring() {
        let (mut block, model) = analyze("return `value = {value}`");

        remove_interpolated_string(&mut block, &model);

        insta::assert_snapshot!(
            generate(&block).trim(),
            @"return string.format('value = %s', tostring(value))"
        );
    }
}
