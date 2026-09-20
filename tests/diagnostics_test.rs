//! Tests for the diagnostics catalog, formatter and per-node queries.

use std::collections::HashSet;

use r4::diagnostic::{descriptor_for, DiagnosticFormatError, DiagnosticQueryError};
use r4::nodes::Token;
use r4::semantic::SemanticOptions;
use r4::{
    Compilation, Diagnostic, DiagnosticFormatter, LinePosition, LinePositionSpan, Parser, Severity,
    TextSpan,
};

fn strict_compilation() -> Compilation {
    let mut compilation = Compilation::with_parser(Parser::default().preserve_tokens());
    compilation.set_options(SemanticOptions {
        strict_globals: true,
        strict_types: false,
    });
    compilation
}

#[test]
fn catalog_codes_are_unique_and_described() {
    let mut seen = HashSet::new();
    for descriptor in r4::DIAGNOSTIC_CATALOG {
        assert!(
            seen.insert(descriptor.code),
            "duplicate code {}",
            descriptor.code
        );
        assert_eq!(descriptor.severity, Severity::Error);
    }
    assert!(descriptor_for("semantic/undeclared-symbol").is_some());
    assert!(descriptor_for("not/a-code").is_none());
}

#[test]
fn formatter_renders_a_caret_annotation() {
    let mut compilation = strict_compilation();
    let error = compilation
        .add_tree("return missing")
        .expect_err("strict globals rejects the global");

    let rendered = DiagnosticFormatter::new("return missing")
        .render(&error)
        .expect("the diagnostic renders");
    assert!(
        rendered.contains("error[semantic/undeclared-symbol]"),
        "{rendered}"
    );
    assert!(rendered.contains("--> 1:8"), "{rendered}");
    assert!(rendered.contains("return missing"), "{rendered}");
    assert!(rendered.contains("^^^^^^^"), "{rendered}");
}

#[test]
fn formatter_adds_context_lines() {
    let diagnostic = Diagnostic::error("something")
        .with_code("semantic/undeclared-symbol")
        .with_location(LinePositionSpan::new(
            LinePosition::new(2, 1),
            LinePosition::new(2, 3),
        ));
    let rendered = DiagnosticFormatter::new("first\nsecond\nthird\n")
        .with_context_lines(1)
        .render(&diagnostic)
        .expect("the diagnostic renders");
    assert!(rendered.contains("first"), "{rendered}");
    assert!(rendered.contains("second"), "{rendered}");
    assert!(rendered.contains("third"), "{rendered}");
}

#[test]
fn formatter_rejects_uncataloged_codes_and_bad_spans() {
    let unknown = Diagnostic::error("x").with_code("bogus/code");
    assert!(matches!(
        DiagnosticFormatter::new("return 1").render(&unknown),
        Err(DiagnosticFormatError::UnknownCode { .. })
    ));

    let bad_span = Diagnostic::error("x")
        .with_code("semantic/undeclared-symbol")
        .with_span(TextSpan::new(1000, 1001));
    assert!(matches!(
        DiagnosticFormatter::new("return 1").render(&bad_span),
        Err(DiagnosticFormatError::InvalidSpan { .. })
    ));

    let bad_location = Diagnostic::error("x")
        .with_code("semantic/undeclared-symbol")
        .with_location(LinePositionSpan::new(
            LinePosition::new(99, 1),
            LinePosition::new(99, 2),
        ));
    assert!(matches!(
        DiagnosticFormatter::new("return 1").render(&bad_location),
        Err(DiagnosticFormatError::InvalidLocation { .. })
    ));
}

#[test]
fn per_node_diagnostics_use_token_spans() {
    let mut compilation = strict_compilation();
    let tree = compilation
        .add_tree("return missing")
        .expect_err("strict globals rejects the global");

    let diagnostics = compilation.diagnostics_in(TextSpan::new(0, 14));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(tree.code, Some("semantic/undeclared-symbol"));

    // A token that covers the diagnostic span finds it.
    let token = Token::new_with_line(7, 14, 1);
    let matches = compilation
        .diagnostics_for_token(&token)
        .expect("the token has a span");
    assert_eq!(matches.len(), 1);

    // A token without a span is a typed error.
    let spanless = Token::from_content("missing");
    assert!(matches!(
        compilation.diagnostics_for_token(&spanless),
        Err(DiagnosticQueryError::MissingSpan)
    ));
    assert_eq!(
        DiagnosticQueryError::MissingSpan.code(),
        "query/missing-span"
    );
}

#[test]
fn diagnostics_can_be_rendered_through_the_compilation() {
    let mut compilation = strict_compilation();
    let error = compilation
        .add_tree("return missing")
        .expect_err("strict globals rejects the global");
    let rendered = compilation
        .format_diagnostic(&error, "return missing")
        .expect("renders");
    assert!(
        rendered.contains("undeclared symbol `missing`"),
        "{rendered}"
    );
}

#[test]
fn diagnostic_builders_and_positions_are_covered() {
    use r4::diagnostic::{LinePosition, LinePositionSpan, Severity, TextSpan};
    use r4::Diagnostic;

    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
        assert!(!severity.as_str().is_empty());
        assert!(!severity.to_string().is_empty());
    }

    let span = TextSpan::new(2, 5);
    assert_eq!(span.start, 2);
    assert_eq!(span.end, 5);
    assert_eq!(span.len(), 3);
    assert!(span.contains(2));
    assert!(!span.contains(5));
    assert!(TextSpan::new(5, 5).is_empty());

    let start = LinePosition::new(1, 1);
    let end = LinePosition::new(1, 4);
    assert_eq!(start.line, 1);
    assert_eq!(start.column, 1);
    assert!(!start.to_string().is_empty());
    let location = LinePositionSpan::new(start, end);
    assert!(!location.to_string().is_empty());

    let message = Diagnostic::warning("careful")
        .with_code("custom/warning")
        .with_optional_span(Some(span))
        .with_optional_location(Some(location))
        .with_message("changed");
    assert_eq!(message.severity, Severity::Warning);
    assert_eq!(message.message, "changed");
    assert!(!Diagnostic::info("note").to_string().is_empty());

    // Resolving a location from a source span.
    let resolved = Diagnostic::error("boom")
        .with_span(span)
        .resolve_location("abcdefg");
    assert!(resolved.location.is_some());

    // The descriptor lookup and formatting of an unknown code.
    let descriptor = r4::diagnostic::descriptor_for("custom/warning");
    let _ = descriptor;
}

#[test]
fn diagnostic_descriptors_severities_and_format_errors() {
    use r4::diagnostic::{
        DiagnosticDescriptor, DiagnosticFormatError, LinePosition, LinePositionSpan, Severity,
        TextSpan,
    };
    use r4::Diagnostic;

    let descriptor = DiagnosticDescriptor::new("test/code", Severity::Warning, "A title");
    assert_eq!(descriptor.code, "test/code");
    assert_eq!(descriptor.severity, Severity::Warning);
    assert_eq!(descriptor.title, "A title");

    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
        assert!(!severity.to_string().is_empty());
    }

    let with_code = Diagnostic::error("boom").with_code("custom/code");
    assert!(with_code.to_string().contains("[custom/code]"));

    let with_location = Diagnostic::error("boom")
        .with_span(TextSpan::new(0, 3))
        .resolve_location("abcd");
    assert!(with_location.to_string().contains("at "));

    let with_bytes = Diagnostic::error("boom").with_span(TextSpan::new(1, 4));
    assert!(with_bytes.to_string().contains("at bytes 1..4"));

    for error in [
        DiagnosticFormatError::UnknownCode {
            code: "x".to_owned(),
        },
        DiagnosticFormatError::InvalidSpan {
            span: TextSpan::new(0, 99),
            source_len: 3,
        },
        DiagnosticFormatError::InvalidLocation {
            location: LinePositionSpan::new(LinePosition::new(9, 9), LinePosition::new(9, 10)),
        },
    ] {
        assert!(!error.to_string().is_empty());
    }
}
