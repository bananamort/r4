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
//!
//! Note the upstream grammar boundary documented on [`crate::parser`]: Luau
//! generic call instantiation is only recognised in the double-angle form
//! `f<<T>>(x)`. The single-angle Luau spelling `f<T>(x)` is read as a comparison
//! in every dialect, so it is never treated as a divergent overlap here.

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
    /// Whether the permissive lexer merged a hex integer and its `..` into one
    /// number token (`0xf3ad..x`). The dialects with a non-greedy number lexer
    /// read it identically as a concatenation.
    pub conflated_number: bool,
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
    // The permissive lexer greedily reads `0x<hex>.` as the start of a hex float
    // and absorbs a following `..`; that token text is the signal.
    let conflated_number = !clean
        && significant.iter().any(|token| {
            matches!(
                token.token_type(),
                TokenType::Number { text } if text.contains("..")
            )
        });

    Scan {
        candidates: allowed,
        may_diverge,
        luau_instantiation,
        conflated_number,
    }
}

/// Classifies the `<<`/`>>` in a source, returning whether it contains a call
/// that reads two ways (`a << b >> (c)`) and whether it contains one only Luau
/// can read (`f<<T>>`, `f<<T>>.x`, `f<<T>>:m()`, or the statement call
/// `f<<T>>()`).
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
        match tokens.get(close + 1).copied() {
            // `PREFIX << T >> (args)` calls a Luau instantiation and shifts in
            // Lua 5.3/5.4. Both are programs only when the shift can be an
            // expression: a statement call, or arguments a shift cannot take,
            // leave Luau as the only reading.
            Some(token) if is_symbol(token, Symbol::LeftParen) => {
                if !shift_is_expression(tokens, index, close) {
                    // Only Luau can read the call.
                    luau_instantiation = true;
                } else if is_type_like(&tokens[index + 1..close]) {
                    // Both readings are valid programs.
                    may_diverge = true;
                }
            }
            // `f<<T>>`, `f<<T>>.x`, `f<<T>>:m()`: a shift cannot continue here.
            Some(token) if cannot_continue_shift(token) => luau_instantiation = true,
            _ => {}
        }
    }
    (may_diverge, luau_instantiation)
}

/// Returns whether `PREFIX << T >> (args)` is a valid Lua 5.3/5.4 expression, so
/// the source reads both as a shift and as a Luau instantiation call.
fn shift_is_expression(tokens: &[&Token], angle: usize, close: usize) -> bool {
    // `obj:m<<T>>(...)` — a method reference without a call is not a Lua prefix,
    // so the shift reading cannot exist.
    if angle >= 2 && is_symbol(tokens[angle - 2], Symbol::Colon) {
        return false;
    }
    if statement_position(tokens, angle) {
        return false;
    }
    if !is_lua_expression(&tokens[angle + 1..close], false) {
        return false;
    }
    let open = close + 1;
    let Some(paren) = matching_close(tokens, open) else {
        return false;
    };
    is_lua_expression(&tokens[open + 1..paren], true)
}

/// Returns whether `tokens` is a single Lua expression: non-empty, no top-level
/// comma, and made only of tokens an expression can contain. Method calls are
/// only allowed where Lua permits them (call arguments, not a type argument
/// list, where `:` introduces a table field).
fn is_lua_expression(tokens: &[&Token], method_calls: bool) -> bool {
    !tokens.is_empty()
        && !has_top_level_comma(tokens)
        && tokens
            .iter()
            .all(|token| is_expression_token(token, method_calls))
}

/// Returns whether a token can appear inside a Lua expression.
fn is_expression_token(token: &Token, method_calls: bool) -> bool {
    if is_symbol(token, Symbol::Colon) {
        return method_calls;
    }
    match token.token_type() {
        TokenType::Identifier { .. }
        | TokenType::Number { .. }
        | TokenType::StringLiteral { .. }
        | TokenType::InterpolatedString { .. } => true,
        TokenType::Symbol { symbol } => matches!(
            symbol,
            Symbol::Dot
                | Symbol::LeftParen
                | Symbol::RightParen
                | Symbol::LeftBrace
                | Symbol::RightBrace
                | Symbol::LeftBracket
                | Symbol::RightBracket
                | Symbol::True
                | Symbol::False
                | Symbol::Nil
                | Symbol::Ellipsis
                | Symbol::Not
                | Symbol::Hash
                | Symbol::Minus
                | Symbol::Plus
                | Symbol::Star
                | Symbol::Slash
                | Symbol::DoubleSlash
                | Symbol::Percent
                | Symbol::Caret
                | Symbol::TwoDots
                | Symbol::Ampersand
                | Symbol::Pipe
                | Symbol::Tilde
                | Symbol::LessThan
                | Symbol::GreaterThan
                | Symbol::LessThanEqual
                | Symbol::GreaterThanEqual
                | Symbol::TwoEqual
                | Symbol::TildeEqual
                | Symbol::And
                | Symbol::Or
        ),
        _ => false,
    }
}

/// Returns whether a slice could be an instantiation's type argument list.
fn is_type_like(tokens: &[&Token]) -> bool {
    tokens.iter().all(|token| is_type_token(token))
}

/// Returns whether a token can appear inside a Luau type.
fn is_type_token(token: &Token) -> bool {
    match token.token_type() {
        TokenType::Identifier { .. } | TokenType::StringLiteral { .. } => true,
        TokenType::Symbol { symbol } => matches!(
            symbol,
            Symbol::Dot
                | Symbol::Comma
                | Symbol::Colon
                | Symbol::LeftParen
                | Symbol::RightParen
                | Symbol::LeftBrace
                | Symbol::RightBrace
                | Symbol::LeftBracket
                | Symbol::RightBracket
                | Symbol::LessThan
                | Symbol::GreaterThan
                | Symbol::QuestionMark
                | Symbol::ThinArrow
                | Symbol::Ampersand
                | Symbol::Pipe
                | Symbol::Ellipsis
                | Symbol::True
                | Symbol::False
                | Symbol::Nil
        ),
        _ => false,
    }
}

/// Returns whether a slice contains a comma outside any brackets.
fn has_top_level_comma(tokens: &[&Token]) -> bool {
    let mut depth = 0usize;
    for token in tokens {
        if opens_bracket(token) {
            depth += 1;
        } else if closes_bracket(token) {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && is_symbol(token, Symbol::Comma) {
            return true;
        }
    }
    false
}

/// Returns whether the expression ending at `angle` (the `<<`) starts a
/// statement. A statement's expression must be a call, so a `<<` call there can
/// only be Luau.
fn statement_position(tokens: &[&Token], angle: usize) -> bool {
    let start = prefix_start(tokens, angle);
    start == 0 || is_statement_boundary(tokens[start - 1])
}

/// Walks back over the prefix (`a.b`, `f()`, `t[1]`, `(a).b`, `f"s"`) ending at
/// `end`, returning the index of its first token.
fn prefix_start(tokens: &[&Token], end: usize) -> usize {
    let mut index = end;
    while index > 0 {
        let previous = tokens[index - 1];
        if matches!(previous.token_type(), TokenType::Identifier { .. }) {
            if index >= 2
                && (is_symbol(tokens[index - 2], Symbol::Dot)
                    || is_symbol(tokens[index - 2], Symbol::Colon))
            {
                index -= 2;
                continue;
            }
            return index - 1;
        }
        if opens_bracket(previous) {
            break;
        }
        if closes_bracket(previous) {
            match matching_open(tokens, index - 1) {
                Some(open) => index = open,
                None => break,
            }
            continue;
        }
        if matches!(
            previous.token_type(),
            TokenType::StringLiteral { .. } | TokenType::InterpolatedString { .. }
        ) {
            index -= 1;
            continue;
        }
        break;
    }
    index
}

/// Returns whether a token ends one statement and lets another begin.
fn is_statement_boundary(token: &Token) -> bool {
    match token.token_type() {
        TokenType::Number { .. }
        | TokenType::StringLiteral { .. }
        | TokenType::InterpolatedString { .. }
        | TokenType::Identifier { .. } => true,
        TokenType::Symbol { symbol } => matches!(
            symbol,
            Symbol::Semicolon
                | Symbol::Do
                | Symbol::Then
                | Symbol::Else
                | Symbol::ElseIf
                | Symbol::Repeat
                | Symbol::End
                | Symbol::True
                | Symbol::False
                | Symbol::Nil
                | Symbol::Ellipsis
                | Symbol::RightParen
                | Symbol::RightBracket
                | Symbol::RightBrace
        ),
        _ => false,
    }
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

/// Finds the `)`/`]`/`}` that matches the bracket at `open`.
fn matching_close(tokens: &[&Token], open: usize) -> Option<usize> {
    let (open_symbol, close_symbol) = bracket_pair(tokens[open])?;
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        if is_symbol(token, close_symbol) {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        } else if is_symbol(token, open_symbol) {
            depth += 1;
        }
    }
    None
}

/// Finds the opening bracket matching the `)`/`]`/`}` at `close`.
fn matching_open(tokens: &[&Token], close: usize) -> Option<usize> {
    let (open_symbol, close_symbol) = bracket_pair(tokens[close])?;
    let mut depth = 0usize;
    for index in (0..=close).rev() {
        let token = tokens[index];
        if is_symbol(token, close_symbol) {
            depth += 1;
        } else if is_symbol(token, open_symbol) {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// Returns the matching open/close symbols for a bracket token.
fn bracket_pair(token: &Token) -> Option<(Symbol, Symbol)> {
    if is_symbol(token, Symbol::LeftParen) || is_symbol(token, Symbol::RightParen) {
        Some((Symbol::LeftParen, Symbol::RightParen))
    } else if is_symbol(token, Symbol::LeftBracket) || is_symbol(token, Symbol::RightBracket) {
        Some((Symbol::LeftBracket, Symbol::RightBracket))
    } else if is_symbol(token, Symbol::LeftBrace) || is_symbol(token, Symbol::RightBrace) {
        Some((Symbol::LeftBrace, Symbol::RightBrace))
    } else {
        None
    }
}

/// Returns whether the token after a `>>` cannot continue a Lua 5.3/5.4 shift
/// chain, so the preceding `<<`/`>>` must be a Luau type instantiation.
fn cannot_continue_shift(token: &Token) -> bool {
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
                | Symbol::GreaterThan
        ),
        _ => false,
    }
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
