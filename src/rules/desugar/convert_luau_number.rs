use crate::{
    nodes::{Block, HexNumber, NumberExpression, Position, Token},
    process::{DefaultVisitor, NodeProcessor, NodeVisitor},
};

#[derive(Default)]
struct Processor<'a> {
    code: Option<&'a str>,
}

impl Processor<'_> {
    fn trim_underscores(&self, token: &mut Token) {
        let content = match self.code {
            Some(code) => token.read(code).to_owned(),
            None => match token.get_position() {
                Position::LineNumber { content, .. } | Position::Any { content } => {
                    content.to_string()
                }
                // Tokens parsed from source reference the original code by
                // position; without it the content cannot be read. Use
                // `convert_luau_number_with_code` to trim those tokens.
                Position::LineNumberReference { .. } => return,
            },
        };

        if content.contains('_') {
            token.replace_with_content(content.chars().filter(|c| *c != '_').collect::<String>());
        }
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_number_expression(&mut self, number: &mut NumberExpression) {
        match number {
            NumberExpression::Binary(binary) => {
                let value = binary.get_raw_value();
                *number = HexNumber::new(value, false).into();
            }
            NumberExpression::Hex(hex_number) => {
                if let Some(token) = hex_number.mutate_token() {
                    self.trim_underscores(token);
                }
            }
            NumberExpression::Decimal(decimal_number) => {
                if let Some(token) = decimal_number.mutate_token() {
                    self.trim_underscores(token);
                }
            }
        }
    }
}

/// Converts Luau number literals (binary numbers and underscore separators) to
/// regular Lua numbers.
///
/// Underscore removal requires reading the token content. Tokens that only
/// carry a source position (as produced by the parser) are left untouched by
/// this function; use [`convert_luau_number_with_code`] to also trim them.
pub fn convert_luau_number(block: &mut Block) {
    let mut processor = Processor { code: None };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Converts Luau number literals using the original source `code` to read
/// position-based tokens.
pub fn convert_luau_number_with_code(block: &mut Block, code: &str) {
    let mut processor = Processor { code: Some(code) };
    DefaultVisitor::visit_block(block, &mut processor);
}
