use std::fmt;

use crate::{
    nodes::{Block, HexNumber, NumberExpression, Position, Token},
    process::{DefaultVisitor, NodeProcessor, NodeVisitor},
    semantic::SemanticModel,
};

/// An error produced by [`convert_luau_number`] or
/// [`convert_luau_number_with_code`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertLuauNumberError {
    /// A number token carries only a source position and the source needed to
    /// read it was not provided (or does not contain the token's range). The
    /// pass cannot know whether the token holds a Luau underscore separator, so
    /// it fails instead of writing a guessed replacement lexeme.
    UnreadableNumberToken {
        /// The source line of the token, when known.
        line: Option<usize>,
    },
}

impl ConvertLuauNumberError {
    /// Returns the stable code of this error.
    pub fn code(&self) -> &'static str {
        match self {
            ConvertLuauNumberError::UnreadableNumberToken { .. } => {
                "desugar/unreadable-number-token"
            }
        }
    }
}

impl fmt::Display for ConvertLuauNumberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertLuauNumberError::UnreadableNumberToken { line } => {
                if let Some(line) = line {
                    write!(f, "line {}: ", line)?;
                }
                f.write_str(
                    "a number token references the source by position but no matching source was \
                     provided; pass the source with `convert_luau_number_with_code`",
                )
            }
        }
    }
}

impl std::error::Error for ConvertLuauNumberError {}

struct Processor<'a> {
    code: Option<&'a str>,
    error: Option<ConvertLuauNumberError>,
}

impl Processor<'_> {
    /// Reads a token's text and removes its Luau underscore separators.
    ///
    /// A token that only carries a source position is read from `code`; when
    /// that is impossible the pass fails instead of guessing.
    fn normalize(&mut self, token: &mut Token) {
        let content = match self.code {
            Some(code) => match token.try_read_with(code) {
                Ok(content) => content.to_owned(),
                Err(_) => {
                    self.fail(token);
                    return;
                }
            },
            None => match token.get_position() {
                Position::LineNumber { content, .. } | Position::Any { content } => {
                    content.to_string()
                }
                Position::LineNumberReference { .. } => {
                    self.fail(token);
                    return;
                }
            },
        };

        if content.contains('_') {
            token.replace_with_content(content.replace('_', ""));
        }
    }

    fn fail(&mut self, token: &Token) {
        if self.error.is_none() {
            self.error = Some(ConvertLuauNumberError::UnreadableNumberToken {
                line: token.get_line_number(),
            });
        }
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_number_expression(&mut self, number: &mut NumberExpression) {
        if self.error.is_some() {
            return;
        }

        match number {
            NumberExpression::Binary(binary) => {
                // A binary literal is Luau-only and is replaced by the exact
                // hexadecimal literal of its value. This is the transformation
                // the pass exists for, not a fallback: the value is exact, and
                // the original token (with its trivia) is reused.
                let value = binary.get_raw_value();
                let token = binary.get_token().cloned();
                let mut hex_number = HexNumber::new(value, false);
                if let Some(mut token) = token {
                    token.replace_with_content(format!("0x{value:x}"));
                    hex_number.set_token(token);
                }
                *number = hex_number.into();
            }
            NumberExpression::Hex(hex_number) => {
                if let Some(token) = hex_number.mutate_token() {
                    self.normalize(token);
                }
            }
            NumberExpression::Decimal(decimal_number) => {
                if let Some(token) = decimal_number.mutate_token() {
                    self.normalize(token);
                }
            }
        }
    }
}

/// Converts Luau number literals (binary numbers and underscore separators) to
/// regular Lua numbers.
///
/// # Errors
///
/// Returns [`ConvertLuauNumberError::UnreadableNumberToken`] when a decimal or
/// hexadecimal token references the source by position and no source is
/// available. Use [`convert_luau_number_with_code`] in that case.
pub fn convert_luau_number(
    block: &mut Block,
    _model: &SemanticModel,
) -> Result<(), ConvertLuauNumberError> {
    run(block, None)
}

/// Converts Luau number literals, reading position-based tokens from `code`.
///
/// # Errors
///
/// Returns [`ConvertLuauNumberError::UnreadableNumberToken`] when `code` does
/// not contain a decimal or hexadecimal token's range.
pub fn convert_luau_number_with_code(
    block: &mut Block,
    code: &str,
    _model: &SemanticModel,
) -> Result<(), ConvertLuauNumberError> {
    run(block, Some(code))
}

fn run(block: &mut Block, code: Option<&str>) -> Result<(), ConvertLuauNumberError> {
    let mut processor = Processor { code, error: None };
    DefaultVisitor::visit_block(block, &mut processor);
    match processor.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, analyze_with_tokens, generate};

    #[test]
    fn converts_binary_literals() {
        let (mut block, model) = analyze("return 0b1010");

        convert_luau_number(&mut block, &model).expect("content tokens are readable");

        insta::assert_snapshot!(generate(&block).trim(), @"return 0xa");
    }

    #[test]
    fn normalizes_hexadecimal_literals() {
        let (mut block, model) = analyze("return 0x12EF");

        convert_luau_number(&mut block, &model).expect("content tokens are readable");

        insta::assert_snapshot!(generate(&block).trim(), @"return 0x12ef");
    }

    /// The recorded content and trivia length of every number token, in order.
    fn number_info(block: &mut Block) -> Vec<(Option<String>, usize)> {
        use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

        #[derive(Default)]
        struct NumberInfo {
            entries: Vec<(Option<String>, usize)>,
        }

        impl NodeProcessor for NumberInfo {
            fn process_number_expression(&mut self, number: &mut NumberExpression) {
                let token = number.get_token();
                let content = token.and_then(|token| match token.get_position() {
                    Position::LineNumber { content, .. } | Position::Any { content } => {
                        Some(content.to_string())
                    }
                    Position::LineNumberReference { .. } => None,
                });
                let trivia = token
                    .map(|token| {
                        token.iter_leading_trivia().count() + token.iter_trailing_trivia().count()
                    })
                    .unwrap_or(0);
                self.entries.push((content, trivia));
            }
        }

        let mut collector = NumberInfo::default();
        DefaultVisitor::visit_block(block, &mut collector);
        collector.entries
    }

    #[test]
    fn errors_on_a_position_token_without_the_source() {
        let (mut block, model) = analyze_with_tokens("return 1_000");

        let error = convert_luau_number(&mut block, &model)
            .expect_err("a position token cannot be read without the source");

        assert_eq!(error.code(), "desugar/unreadable-number-token");
        assert_eq!(
            number_info(&mut block),
            vec![(None, 0)],
            "the block must be left untouched"
        );
    }

    #[test]
    fn errors_on_a_mismatched_source() {
        let (mut block, model) = analyze_with_tokens("return 1_000");

        let error = convert_luau_number_with_code(&mut block, "", &model)
            .expect_err("the source does not contain the token range");

        assert_eq!(error.code(), "desugar/unreadable-number-token");
    }

    #[test]
    fn normalizes_underscores_from_the_source() {
        let (mut block, model) = analyze_with_tokens("return 1_000");

        convert_luau_number_with_code(&mut block, "return 1_000", &model)
            .expect("the source contains the token range");

        assert_eq!(number_info(&mut block), vec![(Some("1000".to_owned()), 0)]);
    }

    #[test]
    fn converts_binary_without_the_source_and_keeps_the_token() {
        let (mut block, model) = analyze_with_tokens("return 0b1010");

        convert_luau_number(&mut block, &model)
            .expect("a binary literal is converted from its exact value");

        assert_eq!(
            number_info(&mut block),
            vec![(Some("0xa".to_owned()), 0)],
            "the binary literal must become hexadecimal"
        );
    }

    #[test]
    fn keeps_number_token_trivia_when_normalizing_with_the_source() {
        let source = "return\n--[[keep]]\n1_000";
        let (mut block, model) = analyze_with_tokens(source);
        let before = number_info(&mut block);
        assert!(
            before.iter().any(|(_, trivia)| *trivia > 0),
            "the comment must be attached to the number token: {before:?}"
        );

        convert_luau_number_with_code(&mut block, source, &model).expect("the source is valid");

        let after = number_info(&mut block);
        assert_eq!(
            before.iter().map(|(_, trivia)| *trivia).collect::<Vec<_>>(),
            after.iter().map(|(_, trivia)| *trivia).collect::<Vec<_>>(),
            "the token's trivia must be preserved"
        );
        assert_eq!(after[0].0.as_deref(), Some("1000"));
    }
}
