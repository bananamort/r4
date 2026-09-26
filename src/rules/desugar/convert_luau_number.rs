use crate::{
    generator::utils::write_number,
    nodes::{Block, HexNumber, NumberExpression, Position, Token},
    process::{DefaultVisitor, NodeProcessor, NodeVisitor},
    semantic::SemanticModel,
};

#[derive(Default)]
struct Processor<'a> {
    code: Option<&'a str>,
}

impl Processor<'_> {
    /// Normalizes a number token to a Lua 5.1 lexeme.
    ///
    /// When the source is available the token text is read and its underscore
    /// separators removed. Otherwise the lexeme is rebuilt from the AST value
    /// (`canonical`), which never contains an underscore. Either way
    /// [`Token::replace_with_content`] only changes the text, so the token's
    /// leading and trailing trivia are preserved.
    fn normalize(&self, token: &mut Token, canonical: &str) {
        match self.code {
            Some(code) => {
                let content = token.read(code).to_owned();
                if content.contains('_') {
                    token.replace_with_content(content.replace('_', ""));
                }
            }
            None => {
                let existing = match token.get_position() {
                    Position::LineNumber { content, .. } | Position::Any { content } => {
                        Some(content.to_string())
                    }
                    Position::LineNumberReference { .. } => None,
                };
                match existing {
                    // A content token can be normalized without the source.
                    Some(content) if content.contains('_') => {
                        token.replace_with_content(content.replace('_', ""));
                    }
                    Some(_) => {}
                    // A position-only token cannot be read without the source,
                    // so rebuild the lexeme from the AST value.
                    None => token.replace_with_content(canonical.to_owned()),
                }
            }
        }
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_number_expression(&mut self, number: &mut NumberExpression) {
        // The canonical lexeme of the current value, used when the token text
        // cannot be read. It is computed before the borrow on `number` is taken.
        let canonical = write_number(number);

        match number {
            NumberExpression::Binary(binary) => {
                // A binary literal is Luau-only and becomes hexadecimal. The
                // original token (and its trivia) is moved onto the hex literal.
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
                    self.normalize(token, &canonical);
                }
            }
            NumberExpression::Decimal(decimal_number) => {
                if let Some(token) = decimal_number.mutate_token() {
                    self.normalize(token, &canonical);
                }
            }
        }
    }
}

/// Converts Luau number literals (binary numbers and underscore separators) to
/// regular Lua numbers.
///
/// Underscore removal reads the token text when the original source is
/// available. Tokens that only carry a source position (as produced by the
/// parser) are normalized from the AST value instead, so the pass also works
/// when the source is not available; this rebuilds the lexeme but keeps the
/// token's trivia.
pub fn convert_luau_number(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor { code: None };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Converts Luau number literals using the original source `code` to read
/// position-based tokens.
pub fn convert_luau_number_with_code(block: &mut Block, code: &str, _model: &SemanticModel) {
    let mut processor = Processor { code: Some(code) };
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn converts_binary_literals() {
        let (mut block, model) = analyze("return 0b1010");

        convert_luau_number(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"return 0xa");
    }

    #[test]
    fn normalizes_hexadecimal_literals() {
        let (mut block, model) = analyze("return 0x12EF");

        convert_luau_number(&mut block, &model);

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
    fn trims_underscores_without_the_source() {
        let (mut block, model) = crate::rules::test_support::analyze_with_tokens("return 1_000");

        convert_luau_number(&mut block, &model);

        assert_eq!(
            number_info(&mut block),
            vec![(Some("1000".to_owned()), 0)],
            "the lexeme must be rebuilt without underscores"
        );
    }

    #[test]
    fn converts_binary_without_the_source_and_keeps_the_token() {
        let (mut block, model) = crate::rules::test_support::analyze_with_tokens("return 0b1010");

        convert_luau_number(&mut block, &model);

        assert_eq!(
            number_info(&mut block),
            vec![(Some("0xa".to_owned()), 0)],
            "the binary literal must become hexadecimal"
        );
    }

    #[test]
    fn keeps_number_token_trivia_when_rebuilding_the_lexeme() {
        let (mut block, model) =
            crate::rules::test_support::analyze_with_tokens("return\n--[[keep]]\n1_000");
        let before = number_info(&mut block);
        assert!(
            before.iter().any(|(_, trivia)| *trivia > 0),
            "the comment must be attached to the number token: {before:?}"
        );

        convert_luau_number(&mut block, &model);

        let after = number_info(&mut block);
        assert_eq!(
            before.iter().map(|(_, trivia)| *trivia).collect::<Vec<_>>(),
            after.iter().map(|(_, trivia)| *trivia).collect::<Vec<_>>(),
            "the token's trivia must be preserved when the lexeme is rebuilt"
        );
        assert_eq!(after[0].0.as_deref(), Some("1000"));
    }
}
