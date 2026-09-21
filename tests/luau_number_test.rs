//! Tests for rewriting Luau numeric separators.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
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

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

#[test]
fn trims_numeric_separators() {
    for code in [
        "return 1_000",
        "return 1_000_000",
        "return 0x_FF",
        "return 0b1_0",
        "return 1_000.5",
        "return 12_34.56_78",
    ] {
        let (mut block, model) = analyze_tokens(code);
        convert_luau_number(&mut block, &model);
        let output = generate(&block);
        assert!(!output.contains('_'), "`{code}` -> {output}");

        // The code-carrying variant reads the source through the token span.
        let (mut block, model) = analyze_tokens(code);
        convert_luau_number_with_code(&mut block, code, &model);
        let output = generate(&block);
        assert!(!output.contains('_'), "`{code}` (with code) -> {output}");
    }
}
