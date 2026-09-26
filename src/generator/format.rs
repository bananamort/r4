//! Formatting built on the non-destructive generator pipeline.
//!
//! Whitespace normalization is a code-generation concern, not an AST mutation:
//! it is implemented here, on top of the [`TokenBasedLuaGenerator`], which is
//! the single authoritative traversal that emits every token in source order
//! and tracks block nesting. Keeping it in the generator layer keeps it
//! separate from the AST-mutating trivia passes in
//! [`crate::rules::formatting`].

use crate::nodes::Block;

use super::{GeneratorError, LuaGenerator, TokenBasedLuaGenerator};

/// Normalizes whitespace and indentation while preserving every comment.
///
/// Every token and comment is emitted exactly once by the token-based generator
/// (there is no per-node union or second traversal), and indentation is derived
/// from the actual block nesting rather than guessed.
///
/// Whitespace runs collapse to a single space (or the single space required to
/// keep tokens from merging), lines are re-indented to the nesting depth with
/// four spaces, and newlines outside comments are normalized to `\n`.
pub fn normalize_whitespace(block: &Block, code: &str) -> Result<String, GeneratorError> {
    let mut generator = TokenBasedLuaGenerator::normalizing(code);
    generator.write_block(block);
    generator.try_into_string()
}

/// Normalizes whitespace with an explicit indentation unit and line separator.
pub fn normalize_whitespace_with(
    block: &Block,
    code: &str,
    indent: impl Into<String>,
    newline: impl Into<String>,
) -> Result<String, GeneratorError> {
    let mut generator = TokenBasedLuaGenerator::normalizing_with(code, indent, newline);
    generator.write_block(block);
    generator.try_into_string()
}
