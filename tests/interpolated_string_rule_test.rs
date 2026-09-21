//! Tests for removing interpolated strings with both replacement strategies.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::desugar::{remove_interpolated_string, remove_interpolated_string_with_tostring};
use r4::Parser;

fn analyze(code: &str) -> (Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

#[test]
fn removes_interpolated_strings_with_both_strategies() {
    for code in [
        "local x = 1\nreturn `value {x}`",
        "return `{1}`",
        "return `{a}{b}`",
        "return `plain`",
        "return `{nil}`",
        "return `{true}`",
        "return `{ {} }`",
        "local t = {}\nreturn `before {t} after`",
        "return `{x} {y} {z}`",
        "return `prefix {x}`",
        "return ``",
        "local string = {}\nreturn `a {x} b`",
        "local tostring = {}\nreturn `{x}`",
        "local string = {}\nlocal tostring = {}\nreturn `a {x} b {y}`",
    ] {
        let (mut block, model) = analyze(code);
        remove_interpolated_string(&mut block, &model);
        let output = generate(&block);
        assert!(!output.is_empty(), "`{code}`");

        let (mut block, model) = analyze(code);
        remove_interpolated_string_with_tostring(&mut block, &model);
        let output = generate(&block);
        assert!(!output.is_empty(), "`{code}` (tostring)");
    }
}
