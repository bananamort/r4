//! Tests for trivia kinds, trivia enumeration and `--!` directives.

use r4::nodes::{TriviaError, TriviaKind};
use r4::process::collect_trivia;
use r4::semantic::{Directive, DirectiveError, Directives, SemanticAnalyzer, Strictness};
use r4::{Parser, SyntaxFactory};

fn parse_with_tokens(code: &str) -> r4::nodes::Block {
    Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

#[test]
fn whitespace_is_classified_into_spaces_and_line_breaks() {
    let block = parse_with_tokens("local x = 1\nreturn x");
    let trivia = collect_trivia(&block);

    assert!(
        trivia
            .iter()
            .any(|piece| piece.kind() == TriviaKind::EndOfLine),
        "the newline must be end-of-line trivia"
    );
    assert!(
        trivia
            .iter()
            .any(|piece| piece.kind() == TriviaKind::Whitespace),
        "the spaces around `=` must be whitespace trivia"
    );
    assert!(trivia.iter().all(|piece| piece.get_start().is_some()));
}

#[test]
fn trivia_is_deduplicated_and_source_ordered() {
    let block = parse_with_tokens("-- first\nlocal x = 1 -- trailing\nreturn x");
    let trivia = collect_trivia(&block);

    let comments: Vec<_> = trivia
        .iter()
        .filter(|piece| piece.kind() == TriviaKind::Comment)
        .collect();
    assert_eq!(comments.len(), 2, "each comment appears exactly once");

    let starts: Vec<_> = trivia
        .iter()
        .filter_map(|piece| piece.get_start())
        .collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted, "trivia is returned in source order");
}

#[test]
fn trivia_read_fails_fast_on_an_invalid_range() {
    let block = parse_with_tokens("-- comment\nreturn 1");
    let trivia = collect_trivia(&block);
    let comment = trivia
        .iter()
        .find(|piece| piece.kind() == TriviaKind::Comment)
        .expect("a comment");
    // A source that is too short for the recorded range is a typed error.
    assert!(matches!(
        comment.try_read_with("--"),
        Err(TriviaError::InvalidRange { .. })
    ));
    assert_eq!(
        TriviaError::InvalidRange {
            start: 0,
            end: 1,
            length: 0
        }
        .code(),
        "trivia/invalid-range"
    );
}

#[test]
fn parses_directives_exactly() {
    assert_eq!(
        Directive::parse("--!strict").unwrap(),
        Some(Directive {
            name: "strict".to_owned(),
            value: None
        })
    );
    assert_eq!(
        Directive::parse("--!optimize 2").unwrap(),
        Some(Directive {
            name: "optimize".to_owned(),
            value: Some("2".to_owned())
        })
    );
    assert_eq!(
        Directive::parse("--!native\n").unwrap().unwrap().name,
        "native"
    );
    assert_eq!(Directive::parse("-- not a directive").unwrap(), None);
    assert_eq!(Directive::parse("-- strict").unwrap(), None);
    assert_eq!(
        Directive::parse("--!").unwrap_err(),
        DirectiveError::MissingName
    );
    assert_eq!(DirectiveError::MissingName.code(), "directive/missing-name");
}

#[test]
fn directive_names_select_strictness() {
    assert_eq!(
        Directive::parse("--!strict").unwrap().unwrap().strictness(),
        Some(Strictness::Strict)
    );
    assert_eq!(
        Directive::parse("--!nonstrict")
            .unwrap()
            .unwrap()
            .strictness(),
        Some(Strictness::NonStrict)
    );
    assert_eq!(
        Directive::parse("--!nocheck")
            .unwrap()
            .unwrap()
            .strictness(),
        Some(Strictness::NoCheck)
    );
    assert_eq!(
        Directive::parse("--!native").unwrap().unwrap().strictness(),
        None
    );
}

#[test]
fn collects_file_directives() {
    let source = "--!strict\n--!native\nlocal x = 1\nreturn x";
    let block = parse_with_tokens(source);

    let directives = Directives::from_block(&block, source).expect("directives parse");

    assert_eq!(directives.len(), 2);
    assert_eq!(directives.strictness(), Some(Strictness::Strict));
    assert!(directives.get("native").is_some());
}

#[test]
fn conflicting_strictness_is_an_error() {
    let source = "--!strict\n--!nonstrict\nreturn 1";
    let block = parse_with_tokens(source);

    let error =
        Directives::from_block(&block, source).expect_err("conflicting strictness must fail");

    assert!(matches!(
        error,
        DirectiveError::ConflictingStrictness {
            first: Strictness::Strict,
            second: Strictness::NonStrict
        }
    ));
    assert_eq!(error.code(), "directive/conflicting-strictness");
}

#[test]
fn strict_directive_enables_strict_checking() {
    let source = "--!strict\nreturn print";
    let block = parse_with_tokens(source);
    let directives = Directives::from_block(&block, source).expect("directives parse");

    let analysis = SemanticAnalyzer::new()
        .with_directives(&directives)
        .analyze_collecting(&block);

    assert_eq!(analysis.errors.len(), 1, "`print` is an undeclared global");
}

#[test]
fn nonstrict_directive_disables_strict_checking() {
    let source = "--!nonstrict\nreturn print";
    let block = parse_with_tokens(source);
    let directives = Directives::from_block(&block, source).expect("directives parse");

    let analysis = SemanticAnalyzer::new()
        .strict_globals()
        .with_directives(&directives)
        .analyze_collecting(&block);

    assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
}

#[test]
fn factory_builds_end_of_line_trivia() {
    let trivia = SyntaxFactory::end_of_line("\n");
    assert_eq!(trivia.kind(), TriviaKind::EndOfLine);
}

#[test]
fn trivia_traversal_reaches_every_token_holder() {
    // Comments attached to non-expression positions (declarations, function
    // names, attributes, table entries, types) must all be found.
    let source = "--!strict\nlocal x: number = 1 -- decl\nlocal function f(a) return a end -- fn\nlocal t = { key = 1, [expr] = 2 } -- table\n@native function g() end -- attribute\nreturn t";
    let block = parse_with_tokens(source);
    let trivia = collect_trivia(&block);
    let comments: Vec<String> = trivia
        .iter()
        .filter(|piece| piece.kind() == TriviaKind::Comment)
        .map(|piece| piece.try_read_with(source).unwrap_or("").to_owned())
        .collect();

    for expected in ["--!strict", "-- decl", "-- fn", "-- table", "-- attribute"] {
        assert!(
            comments.iter().any(|comment| comment == expected),
            "missing `{expected}` in {comments:?}"
        );
    }
    assert_eq!(comments.len(), 5, "{comments:?}");
}
