//! Shared helpers for the colocated rule unit tests.
//!
//! Each rule module keeps its own `#[cfg(test)] mod test`; these helpers are the
//! common parse/analyze/generate plumbing so a rule test states only what it is
//! checking.

use crate::generator::{LuaGenerator, ReadableLuaGenerator};
use crate::nodes::Block;
use crate::semantic::{SemanticAnalyzer, SemanticModel};
use crate::Parser;

/// Parses `code` with the default dialect and no token data.
pub(crate) fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

/// Parses `code` as Lua 5.2 with the default no-token configuration.
pub(crate) fn parse_lua52(code: &str) -> Block {
    Parser::default()
        .lua52()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

/// Parses and analyzes `code`, returning both the tree and its model.
pub(crate) fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to parse and analyze `{code}`: {error}"))
}

/// Parses and analyzes `code` as Lua 5.3, so the model records the dialect that
/// enables integer number semantics.
pub(crate) fn analyze_lua53(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .lua53()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to parse and analyze `{code}`: {error}"))
}

/// Parses `code` with token data and analyzes it.
pub(crate) fn analyze_with_tokens(code: &str) -> (Block, SemanticModel) {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    (block, model)
}

/// Parses `code` as Lua 5.2 with token data and analyzes it.
pub(crate) fn analyze_lua52_with_tokens(code: &str) -> (Block, SemanticModel) {
    let block = Parser::default()
        .lua52()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    (block, model)
}

/// Parses `code` as Luau with token data and records the Luau dialect on the
/// model, so dialect-gated passes see it.
pub(crate) fn analyze_luau_with_tokens(code: &str) -> (Block, SemanticModel) {
    let block = Parser::default()
        .luau()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .with_dialect(crate::Dialect::Luau)
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    (block, model)
}

/// Renders `block` with the readable generator.
pub(crate) fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}
