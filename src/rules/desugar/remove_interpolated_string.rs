use std::iter;

use bstr::ByteSlice;

use crate::nodes::{
    Block, Expression, FieldExpression, FunctionCall, Identifier, InterpolatedStringExpression,
    InterpolationSegment, Prefix, StringExpression, TupleArguments, TypedIdentifier,
    VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::shadow::ShadowPolicy;
use crate::semantic::SemanticModel;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReplacementStrategy {
    #[default]
    StringSpecifier,
    ToStringSpecifier,
}

struct RemoveInterpolatedStringProcessor<'a> {
    string_format_identifier: String,
    tostring_identifier: String,
    define_string_format: bool,
    define_tostring: bool,
    shadows: ShadowPolicy<'a>,
    strategy: ReplacementStrategy,
}

const DEFAULT_TOSTRING_IDENTIFIER: &str = "tostring";
const DEFAULT_STRING_LIBRARY: &str = "string";
const DEFAULT_STRING_FORMAT_NAME: &str = "format";

impl RemoveInterpolatedStringProcessor<'_> {
    fn replace_with(&mut self, string: &InterpolatedStringExpression) -> Expression {
        if string.is_empty() {
            StringExpression::from_value("").into()
        } else if string.len() == 1 {
            match string.iter_segments().next().unwrap() {
                InterpolationSegment::String(string_segment) => {
                    StringExpression::from_value(string_segment.get_value()).into()
                }
                InterpolationSegment::Value(value_segment) => FunctionCall::from_name(
                    if self.shadows.is_transparent(DEFAULT_TOSTRING_IDENTIFIER) {
                        DEFAULT_TOSTRING_IDENTIFIER
                    } else {
                        self.define_tostring = true;
                        &self.tostring_identifier
                    },
                )
                .with_argument(value_segment.get_expression().clone())
                .into(),
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
                        ReplacementStrategy::StringSpecifier => FunctionCall::from_name(
                            if self.shadows.is_transparent(DEFAULT_TOSTRING_IDENTIFIER) {
                                DEFAULT_TOSTRING_IDENTIFIER
                            } else {
                                self.define_tostring = true;
                                &self.tostring_identifier
                            },
                        )
                        .with_argument(value)
                        .into(),
                    }),
            )
            .collect::<TupleArguments>();

            FunctionCall::from_prefix(if self.shadows.is_transparent(DEFAULT_STRING_LIBRARY) {
                FieldExpression::new(
                    Prefix::from_name(DEFAULT_STRING_LIBRARY),
                    DEFAULT_STRING_FORMAT_NAME,
                )
                .into()
            } else {
                self.define_string_format = true;
                Prefix::from_name(&self.string_format_identifier)
            })
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
/// `tostring` and `string` are treated as shadowed when they are declared
/// anywhere in the block; in that case aliases are introduced at the top of the
/// block.
pub fn remove_interpolated_string(block: &mut Block) {
    let shadows = ShadowPolicy::from_block(block);
    remove_interpolated_string_with_strategy(block, ReplacementStrategy::StringSpecifier, shadows);
}

/// Removes interpolated strings using the `%*` format specifier, which relies
/// on `string.format` converting its arguments with `tostring`.
pub fn remove_interpolated_string_with_tostring(block: &mut Block) {
    let shadows = ShadowPolicy::from_block(block);
    remove_interpolated_string_with_strategy(
        block,
        ReplacementStrategy::ToStringSpecifier,
        shadows,
    );
}

/// Like [`remove_interpolated_string`], but resolves `string` and `tostring`
/// through the semantic model.
///
/// Read-only aliases such as `local tostring = tostring` are transparent, so
/// no alias definitions are introduced for them.
pub fn remove_interpolated_string_with_model(block: &mut Block, model: &SemanticModel) {
    remove_interpolated_string_with_strategy(
        block,
        ReplacementStrategy::StringSpecifier,
        ShadowPolicy::from_model(model),
    );
}

/// Like [`remove_interpolated_string_with_tostring`], but resolves `string` and
/// `tostring` through the semantic model.
pub fn remove_interpolated_string_with_tostring_and_model(
    block: &mut Block,
    model: &SemanticModel,
) {
    remove_interpolated_string_with_strategy(
        block,
        ReplacementStrategy::ToStringSpecifier,
        ShadowPolicy::from_model(model),
    );
}

fn remove_interpolated_string_with_strategy<'a>(
    block: &mut Block,
    strategy: ReplacementStrategy,
    shadows: ShadowPolicy<'a>,
) {
    const STRING_FORMAT_IDENTIFIER: &str = "__R4_STR_FMT";
    const TOSTRING_IDENTIFIER: &str = "__R4_TO_STR";

    let mut processor = RemoveInterpolatedStringProcessor {
        string_format_identifier: STRING_FORMAT_IDENTIFIER.to_owned(),
        tostring_identifier: TOSTRING_IDENTIFIER.to_owned(),
        define_string_format: false,
        define_tostring: false,
        shadows,
        strategy,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    if processor.define_string_format || processor.define_tostring {
        let mut variables = Vec::new();
        let mut values = Vec::new();

        if processor.define_string_format {
            variables.push(TypedIdentifier::new(STRING_FORMAT_IDENTIFIER));
            values.push(
                FieldExpression::new(
                    Prefix::from_name(DEFAULT_STRING_LIBRARY),
                    DEFAULT_STRING_FORMAT_NAME,
                )
                .into(),
            );
        }

        if processor.define_tostring {
            variables.push(TypedIdentifier::new(TOSTRING_IDENTIFIER));
            values.push(Identifier::new(DEFAULT_TOSTRING_IDENTIFIER).into());
        }

        block.insert_statement(0, VariableAssignment::new(variables, values));
    }
}
