//! Tests for the parser's typed error surfaces.

use r4::{AnalyzedParseError, Parser};

#[test]
fn parser_error_kinds_and_diagnostics() {
    // A conversion error (a type-pack type-instantiation argument).
    let error = Parser::default()
        .parse("return f<<A...>>")
        .expect_err("the conversion is rejected");
    assert_eq!(error.code(), "parse/conversion");
    assert!(!error.to_diagnostic().message.is_empty());
    assert!(!error.to_diagnostics().is_empty());
    assert!(!error.to_string().is_empty());

    // An upstream panic is wrapped into a typed error.
    let error = Parser::default()
        .parse("return 3 & 1")
        .expect_err("the upstream panic becomes an error");
    assert_eq!(error.code(), "parse/panic");
    assert!(!error.to_string().is_empty());

    // A syntax error.
    let error = Parser::default()
        .parse("local x = ")
        .expect_err("a syntax error is reported");
    assert_eq!(error.code(), "parse/syntax");

    // A semantic error through `parse_analyzed`.
    let error = Parser::default()
        .lua52()
        .parse_analyzed("goto missing")
        .expect_err("an undefined label is reported");
    assert!(matches!(error, AnalyzedParseError::Semantic(_)));
    assert!(!error.to_string().is_empty());
    assert!(!format!("{error:?}").is_empty());

    // The `From` conversion from a parser error.
    let parse_error = Parser::default()
        .parse("local x = ")
        .expect_err("a syntax error is reported");
    let converted: AnalyzedParseError = parse_error.into();
    assert!(matches!(converted, AnalyzedParseError::Parse(_)));
    assert!(!converted.to_string().is_empty());
}
