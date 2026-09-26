//! Luau / Lua 5.4 semantic validation: attributes, generics and export placement.

use r4::semantic::{SemanticAnalyzer, SemanticErrorKind};
use r4::Parser;

fn analyze(code: &str) -> r4::semantic::SemanticModel {
    let block = Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn error_code(code: &str) -> &'static str {
    let block = Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("the source must be rejected")
        .code()
}

#[test]
fn const_and_close_variables_reject_reassignment() {
    assert_eq!(
        error_code("local x <const> = 1\nx = 2"),
        "semantic/const-reassignment"
    );
    assert_eq!(
        error_code("local x <const> = 1\nx += 2"),
        "semantic/const-reassignment"
    );
    assert_eq!(
        error_code("local x <close> = f()\nx = 2"),
        "semantic/close-reassignment"
    );
}

#[test]
fn reading_a_const_variable_is_fine() {
    analyze("local x <const> = 1\nreturn x");
    analyze("local x <close> = f()\nreturn x");
}

#[test]
fn a_shadowing_local_is_not_const() {
    // The inner `x` is a fresh, unwritable-attribute-free symbol.
    analyze("local x <const> = 1\ndo local x = 2\nx = 3\nreturn x end\nreturn x");
}

#[test]
fn the_model_tracks_local_attributes() {
    let model = analyze("local number <const> = 1\nlocal resource <close> = f()\nlocal plain = 3");
    let find = |name: &str| {
        model
            .symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .unwrap_or_else(|| panic!("symbol `{name}`"))
    };

    assert!(find("number").is_const());
    assert!(find("number").is_read_only());
    assert_eq!(find("number").attribute(), Some("const"));

    assert!(find("resource").is_close());
    assert!(find("resource").is_read_only());
    assert_eq!(find("resource").attribute(), Some("close"));

    assert!(!find("plain").is_read_only());
    assert_eq!(find("plain").attribute(), None);
}

#[test]
fn duplicate_generic_parameters_are_rejected() {
    assert_eq!(
        error_code("local function f<T, T>() end"),
        "semantic/duplicate-type-parameter"
    );
    assert_eq!(
        error_code("type A<T, T> = T"),
        "semantic/duplicate-type-parameter"
    );
    assert_eq!(
        error_code("local function f<T, U, T>(a: T, b: U) end"),
        "semantic/duplicate-type-parameter"
    );
}

#[test]
fn distinct_generic_parameters_are_fine() {
    analyze("local function f<T, U>(a: T, b: U): T return a end");
    analyze("type A<T, U> = (T, U) -> T");
}

#[test]
fn export_type_is_only_allowed_at_the_top_level() {
    analyze("export type T = number\nreturn 1");
    assert_eq!(
        error_code("do export type T = number end"),
        "semantic/misplaced-export-type"
    );
    assert_eq!(
        error_code("local function f()\n    export type T = number\nend"),
        "semantic/misplaced-export-type"
    );
}

#[test]
fn unknown_local_attributes_are_rejected() {
    assert_eq!(
        error_code("local x <foo> = 1"),
        "semantic/unknown-local-attribute"
    );

    // The two known attributes still analyze.
    analyze("local x <const> = 1\nreturn x");
    analyze("local y <close> = f()\nreturn y");
}

#[test]
fn the_error_kinds_carry_their_operands() {
    let block = Parser::default()
        .parse("local x <const> = 1\nx = 2")
        .expect("parse");
    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("rejected");
    match &error.kind {
        SemanticErrorKind::ConstReassignment { name } => assert_eq!(name, "x"),
        other => panic!("unexpected error kind: {other:?}"),
    }
    assert!(!error.to_string().is_empty());
}
