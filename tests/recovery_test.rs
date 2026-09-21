//! Tests for recovered parsing and multi-error collection.

use r4::semantic::{SemanticAnalyzer, SemanticOptions};
use r4::{Compilation, Parser};

#[test]
fn recovers_multiple_syntax_errors() {
    let (block, diagnostics) = Parser::default()
        .parse_recovering("local x = \nlocal y = \nreturn x + y")
        .expect("recovery always yields a block");

    assert!(
        diagnostics.len() >= 2,
        "expected several syntax diagnostics, got {diagnostics:?}"
    );
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code == Some("parse/syntax")));
    // The recovered tree is still analyzable.
    SemanticAnalyzer::new()
        .analyze(&block)
        .expect("the recovered tree analyzes");
}

#[test]
fn strict_parse_error_exposes_all_diagnostics() {
    let error = Parser::default()
        .parse("local x = \nlocal y = \nreturn x + y")
        .expect_err("invalid code must fail");
    let diagnostics = error.to_diagnostics();
    assert!(diagnostics.len() >= 2, "{diagnostics:?}");
    assert_eq!(error.to_diagnostic(), diagnostics[0]);
}

#[test]
fn collects_every_semantic_error() {
    let block = Parser::default()
        .parse("return a + b")
        .expect("valid syntax");
    let analysis = SemanticAnalyzer::new()
        .strict_globals()
        .analyze_collecting(&block);

    assert!(!analysis.is_ok());
    assert_eq!(analysis.errors.len(), 2, "{:?}", analysis.errors);
    assert_eq!(analysis.diagnostics().len(), 2);
}

#[test]
fn collects_type_errors_without_stopping() {
    let block = Parser::default()
        .parse("local a: Missing = 1\nlocal b: AlsoMissing = 2\nreturn a, b")
        .expect("valid syntax");
    let analysis = SemanticAnalyzer::new()
        .strict_types()
        .analyze_collecting(&block);

    assert_eq!(analysis.errors.len(), 2, "{:?}", analysis.errors);
}

#[test]
fn collects_goto_errors_in_lua52() {
    let block = Parser::default()
        .lua52()
        .parse("goto first\ngoto second\nreturn 0")
        .expect("valid Lua 5.2 syntax");
    let analysis = SemanticAnalyzer::new().analyze_collecting(&block);

    assert_eq!(analysis.errors.len(), 2, "{:?}", analysis.errors);
}

#[test]
fn compilation_records_recovered_diagnostics() {
    let mut compilation = Compilation::with_options(SemanticOptions {
        strict_globals: true,
        strict_types: false,
    });
    let tree = compilation
        .add_tree_recovering("return a + b")
        .expect("recovery stores the tree");

    assert!(compilation.block(tree).is_some());
    assert!(compilation.model(tree).is_ok());
    let codes: Vec<_> = compilation
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert_eq!(
        codes,
        vec![
            Some("semantic/undeclared-symbol"),
            Some("semantic/undeclared-symbol")
        ]
    );
}

#[test]
fn compilation_recovering_surfaces_syntax_diagnostics() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree_recovering("local x = \nlocal y = \nreturn x + y")
        .expect("recovery stores the tree");

    assert!(compilation.block(tree).is_some());
    assert!(compilation
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.code == Some("parse/syntax")));
    assert!(compilation.diagnostics().len() >= 2);
}

#[test]
fn strict_compilation_still_fails_fast() {
    let mut compilation = Compilation::new();
    assert!(compilation.add_tree("local x = ").is_err());
    assert_eq!(compilation.len(), 0);
}
