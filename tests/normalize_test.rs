//! Tests for the comment-preserving whitespace normalizer.

use r4::generator::{normalize_whitespace, normalize_whitespace_with};
use r4::Parser;

fn normalize(code: &str) -> String {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    normalize_whitespace(&block, code).expect("the source matches the tokens")
}

#[test]
fn preserves_comments_and_reindents_blocks() {
    let code = "local x   =   1 -- keep me\nif x then\n        local y = 2\n              return y -- tail\nend\n";
    let output = normalize(code);

    assert!(output.contains("-- keep me"), "{output}");
    assert!(output.contains("-- tail"), "{output}");
    assert_eq!(
        output.trim_end(),
        "local x = 1 -- keep me\nif x then\n    local y = 2\n    return y -- tail\nend",
        "{output}"
    );
}

#[test]
fn honors_custom_indentation() {
    let code = "if a then\n  b()\nend\n";
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .expect("valid code");
    let output =
        normalize_whitespace_with(&block, code, "\t", "\n").expect("the source matches the tokens");
    assert!(output.contains("\n\tb()"), "{output}");
}

#[test]
fn normalizing_is_idempotent() {
    let code = "do\n      local x = 1 -- c\nend\n";
    let first = normalize(code);
    let second = normalize(&first);
    assert_eq!(first, second);
}

#[test]
fn reports_a_source_that_does_not_match_the_tokens() {
    use r4::generator::GeneratorError;

    let block = Parser::default()
        .preserve_tokens()
        .parse("return 1")
        .expect("valid code");

    // The token range 7..8 is not a slice of the empty source.
    let error = normalize_whitespace(&block, "").expect_err("the range is out of bounds");
    assert!(matches!(error, GeneratorError::Source(_)));
    assert!(!error.to_string().is_empty());
}

#[test]
fn preserves_multiline_comments() {
    let code = "--[[ a\nmultiline\ncomment ]]\nlocal x = 1\n";
    let output = normalize(code);
    assert!(output.contains("--[[ a\nmultiline\ncomment ]]"), "{output}");
}
