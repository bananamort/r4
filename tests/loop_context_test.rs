//! Loop-context validation for every function body.
//!
//! `build_chunk` validates the chunk's own `break`/`continue`; these tests cover
//! the nested-function case, where a `break`/`continue` is only valid if *that
//! function* has an enclosing loop.

use r4::cfg::{validate_function_loop_context, CfgErrorKind};
use r4::rules::{desugar_to_lua51, DesugarError};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn validate(code: &str) -> Result<(), CfgErrorKind> {
    let block = Parser::default()
        .luau()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    validate_function_loop_context(&block, &model).map_err(|error| error.kind)
}

#[test]
fn break_and_continue_outside_a_loop_in_a_function_are_rejected() {
    assert_eq!(
        validate("function f() continue end"),
        Err(CfgErrorKind::ContinueOutsideLoop)
    );
    assert_eq!(
        validate("function f() break end"),
        Err(CfgErrorKind::BreakOutsideLoop)
    );
    assert_eq!(
        validate("local g = function() break end"),
        Err(CfgErrorKind::BreakOutsideLoop)
    );
    assert_eq!(
        validate("function outer() local inner = function() continue end end"),
        Err(CfgErrorKind::ContinueOutsideLoop)
    );
}

#[test]
fn loops_inside_a_function_are_valid() {
    assert_eq!(
        validate("function f() while true do continue end end"),
        Ok(())
    );
    assert_eq!(
        validate("function f() for i = 1, 3 do break end end"),
        Ok(())
    );
    assert_eq!(
        validate("function f() while true do local g = function() return 1 end break end end"),
        Ok(())
    );
    assert_eq!(validate("return function() return 1 end"), Ok(()));
}

#[test]
fn an_outer_loop_does_not_extend_into_a_nested_function() {
    assert_eq!(
        validate("while true do local g = function() continue end end"),
        Err(CfgErrorKind::ContinueOutsideLoop)
    );
}

#[test]
fn the_chunk_itself_is_left_to_build_chunk() {
    // A top-level loop is not a function body, so this validator says nothing.
    assert_eq!(validate("while true do continue end"), Ok(()));
}

#[test]
fn desugar_to_lua51_reports_invalid_loop_context() {
    let mut block = Parser::default()
        .luau()
        .parse("function f() break end")
        .expect("parse");
    let error = desugar_to_lua51(&mut block).expect_err("`break` outside a loop");
    match error {
        DesugarError::Cfg(error) => assert_eq!(error.kind, CfgErrorKind::BreakOutsideLoop),
        other => panic!("unexpected error: {other:?}"),
    }

    // A valid loop still lowers.
    let mut block = Parser::default()
        .luau()
        .parse("function f() while true do break end end")
        .expect("parse");
    desugar_to_lua51(&mut block).expect("a loop with a break lowers");
}
