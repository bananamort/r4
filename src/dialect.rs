//! Dialect detection for the default parser.
//!
//! When no dialect is chosen explicitly, the parser lexes the source once and
//! eliminates every dialect a construct rules out. Unless a `<<`/`>>` call could
//! be read two ways, the permissive grammar reads the source exactly as every
//! dialect that accepts it would, so that one parse decides it; a genuinely
//! divergent `<<` call is reported instead of guessed.
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
/// Lua 5.1 and Luau both treat `goto` as an ordinary identifier; every dialect
/// between them reserves it as a statement keyword.
const LUA51_OR_LUAU: &[Dialect] = &[Dialect::Lua51, Dialect::Luau];
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

/// The result of scanning a source with no explicit dialect.
pub(crate) struct Scan {
    /// The dialects that remain possible after the markers.
    pub candidates: Vec<Dialect>,
    /// Whether the source contains a `<<` call (`<<` followed by `>>(`), the one
    /// construct whose readings can differ between dialects: Luau type
    /// instantiation versus a Lua 5.3/5.4 shift chain. Both readings are valid
    /// programs, so the caller is asked to choose.
    pub may_diverge: bool,
    /// Whether the source contains a `<<`/`>>` that only Luau can read, such as
    /// `f<<T>>`, `f<<T>>:m()` or `f<<T>>.x`. Lua 5.3/5.4 reject these, so they
    /// name Luau without ambiguity.
    pub luau_instantiation: bool,
}

/// Scans `code` with a single lexer pass, returning the surviving dialects and
/// how the source reads any `<<`/`>>`.
pub(crate) fn scan(code: &str) -> Scan {
    // `Lexer::new` handles a leading `#!` shebang, and `collect` drains every
    // token; a lexer error still yields the tokens it recovered.
    let (tokens, clean) = match Lexer::new(code, full_moon::LuaVersion::default()).collect() {
        LexerResult::Ok(tokens) => (tokens, true),
        LexerResult::Recovered(tokens, _) => (tokens, false),
        LexerResult::Fatal(_) => (Vec::new(), false),
    };

    let significant: Vec<&Token> = tokens
        .iter()
        .filter(|token| !is_trivia(token.token_type()))
        .collect();

    let mut allowed = ALL.to_vec();
    for (index, token) in significant.iter().enumerate() {
        if let Some(marker) = marker(token.token_type(), &significant, index) {
            allowed.retain(|dialect| marker.contains(dialect));
        }
    }

    // A source the lexer could not fully tokenize is left to the parser.
    let (may_diverge, luau_instantiation) = if clean {
        classify_instantiation(&significant)
    } else {
        (false, false)
    };

    Scan {
        candidates: allowed,
        may_diverge,
        luau_instantiation,
    }
}

/// Classifies the `<<`/`>>` in a source, returning whether it contains a call
/// that reads two ways (`a << b >> (c)`) and whether it contains one only Luau
/// can read (`f<<T>>`, `f<<T>>.x`, `f<<T>>:m()`).
///
/// A `<<` with no `>>` after it, or with a shift operand after the `>>`, is an
/// ordinary shift chain that every dialect with the operator reads the same way.
fn classify_instantiation(tokens: &[&Token]) -> (bool, bool) {
    let mut may_diverge = false;
    let mut luau_instantiation = false;
    for index in 0..tokens.len() {
        if !is_symbol(tokens[index], Symbol::DoubleLessThan) {
            continue;
        }
        let Some(close) = matching_greater(tokens, index) else {
            continue;
        };
        let after = tokens.get(close + 1).copied();
        if after.is_some_and(|token| is_symbol(token, Symbol::LeftParen)) {
            // `>> (` continues a shift and also calls a Luau instantiation.
            may_diverge = true;
        } else if after.is_none_or(follows_instantiation) {
            // Lua 5.3/5.4 cannot continue a shift chain here, so only Luau reads
            // the `<<`/`>>` as type instantiation.
            luau_instantiation = true;
        }
    }
    (may_diverge, luau_instantiation)
}

/// Finds the `>>` that closes a `<<` without leaving the brackets the `<<` sits
/// in. A `)` between them, as in `(a << b) >> (c)`, means the `<<` was an
/// ordinary shift rather than the start of a type instantiation.
fn matching_greater(tokens: &[&Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start + 1) {
        if is_symbol(token, Symbol::DoubleGreaterThan) && depth == 0 {
            return Some(index);
        }
        if opens_bracket(token) {
            depth += 1;
        } else if closes_bracket(token) {
            if depth == 0 {
                return None;
            }
            depth -= 1;
        }
    }
    None
}

/// Returns whether a token opens a bracketed group.
fn opens_bracket(token: &Token) -> bool {
    is_symbol(token, Symbol::LeftParen)
        || is_symbol(token, Symbol::LeftBracket)
        || is_symbol(token, Symbol::LeftBrace)
}

/// Returns whether a token closes a bracketed group.
fn closes_bracket(token: &Token) -> bool {
    is_symbol(token, Symbol::RightParen)
        || is_symbol(token, Symbol::RightBracket)
        || is_symbol(token, Symbol::RightBrace)
}

/// Returns whether the token after a `>>` cannot continue a Lua 5.3/5.4 shift
/// chain, so the preceding `<<`/`>>` must be a Luau type instantiation.
fn follows_instantiation(token: &Token) -> bool {
    match token.token_type() {
        TokenType::Eof => true,
        TokenType::Symbol { symbol } => matches!(
            symbol,
            Symbol::RightParen
                | Symbol::RightBracket
                | Symbol::RightBrace
                | Symbol::Comma
                | Symbol::Semicolon
                | Symbol::Equal
                | Symbol::Dot
                | Symbol::LeftBracket
                | Symbol::Colon
                | Symbol::TwoColons
        ),
        _ => false,
    }
}

/// Returns whether a token is the given symbol.
fn is_symbol(token: &Token, symbol: Symbol) -> bool {
    matches!(token.token_type(), TokenType::Symbol { symbol: found } if *found == symbol)
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
            // it is an ordinary identifier, which Lua 5.1 and Luau both accept.
            Symbol::Goto if next_is_identifier(tokens, index) => Some(LUA52_PLUS),
            Symbol::Goto => Some(LUA51_OR_LUAU),
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
