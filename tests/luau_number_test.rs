//! Tests for rewriting Luau numeric separators.

use r4::generator::{LuaGenerator, TokenBasedLuaGenerator};
use r4::nodes::Block;
use r4::rules::desugar::{convert_luau_number, convert_luau_number_with_code};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn analyze_tokens(code: &str) -> (Block, r4::semantic::SemanticModel) {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    (block, model)
}

#[test]
fn trims_numeric_separators_with_the_source() {
    for code in [
        "return 1_000",
        "return 1_000_000",
        "return 0x_FF",
        "return 0b1_0",
        "return 1_000.5",
        "return 12_34.56_78",
    ] {
        let (mut block, model) = analyze_tokens(code);
        convert_luau_number_with_code(&mut block, code, &model)
            .unwrap_or_else(|error| panic!("`{code}` must normalize: {error}"));

        // Token-based generation emits the token text, so it proves the
        // separators were actually removed from the token.
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&block);
        let output = generator.into_string();
        assert!(!output.contains('_'), "`{code}` -> {output}");
    }
}

#[test]
fn refuses_to_normalize_a_position_token_without_the_source() {
    let (mut block, model) = analyze_tokens("return 1_000");

    let error = convert_luau_number(&mut block, &model)
        .expect_err("a position token cannot be read without the source");

    assert_eq!(error.code(), "desugar/unreadable-number-token");
}

#[test]
fn desugar_with_code_normalizes_number_tokens() {
    use r4::rules::desugar_to_lua51_with_code;

    let source = "return 1_000";
    let mut block = Parser::default()
        .preserve_tokens()
        .parse(source)
        .expect("valid code");

    desugar_to_lua51_with_code(&mut block, source).expect("the source is valid");

    let mut generator = TokenBasedLuaGenerator::new(source);
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(!output.contains('_'), "`{output}` must have no separator");
}

#[test]
fn desugar_without_the_source_reports_the_number_error() {
    use r4::rules::{desugar_to_lua51, DesugarError};

    let mut block = Parser::default()
        .preserve_tokens()
        .parse("return 1_000")
        .expect("valid code");

    let error = desugar_to_lua51(&mut block).expect_err("the source is missing");
    assert!(matches!(error, DesugarError::Number(_)), "{error:?}");
}
