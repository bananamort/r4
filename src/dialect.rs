//! Dialect detection for the default parser.
//!
//! When no dialect is chosen explicitly, the parser lexes the source once and
//! eliminates every dialect a construct rules out. The surviving dialects are
//! then parsed; if they disagree the caller is asked to choose instead of the
//! parser guessing.
//!
//! The marker sets below are conservative: a construct is only used as a marker
//! when it is valid in a known set of dialects. Constructs shared between
//! dialects with different meanings (for example `<<`, which is Luau type
//! instantiation and a Lua 5.3 shift) name every dialect they are valid in, so
//! they never eliminate one side of an overlap.

use full_moon::tokenizer::{Lexer, LexerResult, Symbol, Token, TokenType};

use crate::parser::Dialect;

/// Every dialect the detector considers, in report order.
pub(crate) const ALL: [Dialect; 6] = [
    Dialect::Lua51,
    Dialect::Lua52,
    Dialect::Lua53,
    Dialect::Lua54,
    Dialect::LuaJit,
    Dialect::Luau,
];

/// Symbols only Luau accepts.
const LUAU_ONLY: &[Dialect] = &[Dialect::Luau];
/// Symbols accepted by Luau and by Lua 5.3/5.4, with a different meaning.
const SHARED_LUAU_53: &[Dialect] = &[Dialect::Luau, Dialect::Lua53, Dialect::Lua54];
/// Symbols only Lua 5.3 and 5.4 accept.
const LUA53_PLUS: &[Dialect] = &[Dialect::Lua53, Dialect::Lua54];
/// Symbols only Lua 5.4 accepts (`<const>`/`<close>`).
const LUA54_ONLY: &[Dialect] = &[Dialect::Lua54];
/// Symbols Lua 5.2 and its supersets accept (`goto`).
const LUA52_PLUS: &[Dialect] = &[
    Dialect::Lua52,
    Dialect::Lua53,
    Dialect::Lua54,
    Dialect::LuaJit,
];
/// Symbols every dialect but Lua 5.1 accepts (`::`).
const NOT_LUA51: &[Dialect] = &[
    Dialect::Lua52,
    Dialect::Lua53,
    Dialect::Lua54,
    Dialect::LuaJit,
    Dialect::Luau,
];

/// Returns the dialects that remain possible for `code` after one lexer pass.
///
/// An empty vector means the markers are mutually inconsistent; the caller then
/// falls back to the permissive union.
pub(crate) fn candidates(code: &str) -> Vec<Dialect> {
    let mut allowed = ALL.to_vec();
    let mut lexer = Lexer::new(code, full_moon::LuaVersion::default());
    let mut tokens: Vec<Token> = Vec::new();
    loop {
        match lexer.process_next() {
            None => break,
            Some(LexerResult::Ok(token)) => tokens.push(token),
            // A source the lexer cannot tokenize is left to the parser.
            Some(LexerResult::Recovered(_, _)) | Some(LexerResult::Fatal(_)) => {
                return ALL.to_vec()
            }
        }
    }

    let significant: Vec<&Token> = tokens
        .iter()
        .filter(|token| !is_trivia(token.token_type()))
        .collect();

    for (index, token) in significant.iter().enumerate() {
        let Some(marker) = marker(token.token_type(), &significant, index) else {
            continue;
        };
        allowed.retain(|dialect| marker.contains(dialect));
    }

    allowed
}

/// Returns whether a token is trivia (whitespace or a comment).
fn is_trivia(token_type: &TokenType) -> bool {
    matches!(
        token_type,
        TokenType::Whitespace { .. }
            | TokenType::SingleLineComment { .. }
            | TokenType::MultiLineComment { .. }
            | TokenType::Shebang { .. }
    )
}

/// Returns the dialects a token is valid in, or `None` when it is not a marker.
fn marker(token_type: &TokenType, tokens: &[&Token], index: usize) -> Option<&'static [Dialect]> {
    match token_type {
        TokenType::InterpolatedString { .. } => Some(LUAU_ONLY),
        TokenType::Symbol { symbol } => match symbol {
            Symbol::PlusEqual
            | Symbol::MinusEqual
            | Symbol::StarEqual
            | Symbol::SlashEqual
            | Symbol::CaretEqual
            | Symbol::PercentEqual
            | Symbol::TwoDotsEqual
            | Symbol::DoubleSlashEqual
            // Luau type syntax: `->`, attributes (`@name`) and optional types.
            | Symbol::ThinArrow
            | Symbol::AtSign
            | Symbol::QuestionMark => Some(LUAU_ONLY),
            Symbol::Tilde => Some(LUA53_PLUS),
            // `&`, `|`, `//`, `<<` and `>>` are valid in Luau too (type
            // intersection, type union, floor division, type instantiation), so
            // they never eliminate Luau.
            Symbol::Ampersand
            | Symbol::Pipe
            | Symbol::DoubleSlash
            | Symbol::DoubleLessThan
            | Symbol::DoubleGreaterThan => Some(SHARED_LUAU_53),
            Symbol::TwoColons => Some(NOT_LUA51),
            Symbol::LessThan if is_lua54_attribute(tokens, index) => Some(LUA54_ONLY),
            // `goto` is a statement only when a label name follows it; otherwise
            // it is an ordinary identifier (legal before Lua 5.2).
            Symbol::Goto if next_is_identifier(tokens, index) => Some(LUA52_PLUS),
            _ => None,
        },
        _ => None,
    }
}

/// Returns whether the token after `index` is an identifier.
fn next_is_identifier(tokens: &[&Token], index: usize) -> bool {
    matches!(
        tokens.get(index + 1).map(|token| token.token_type()),
        Some(TokenType::Identifier { .. })
    )
}

/// Returns whether `< name >` at `index` is a Lua 5.4 local attribute
/// (`<const>` or `<close>`). Comparison uses like `a < const > b` parse to the
/// same tree under every dialect, so a false positive here only costs a parse.
fn is_lua54_attribute(tokens: &[&Token], index: usize) -> bool {
    let name = match tokens.get(index + 1).map(|token| token.token_type()) {
        Some(TokenType::Identifier { identifier }) => identifier,
        _ => return false,
    };
    matches!(name.as_str(), "const" | "close")
        && matches!(
            tokens.get(index + 2).map(|token| token.token_type()),
            Some(TokenType::Symbol {
                symbol: Symbol::GreaterThan
            })
        )
}
