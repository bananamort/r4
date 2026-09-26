//! Loop-context validation for the chunk and every function body.
//!
//! A `break`/`continue` is only valid if the innermost loop-bearing region (the
//! chunk or a function) has an enclosing loop; these tests cover both regions.

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
fn the_chunk_loop_context_is_validated() {
    assert_eq!(validate("while true do continue end"), Ok(()));
    assert_eq!(validate("for i = 1, 3 do break end"), Ok(()));
    assert_eq!(
        validate("break"),
        Err(CfgErrorKind::BreakOutsideLoop),
        "a top-level break outside any loop must be reported"
    );
    assert_eq!(
        validate("continue"),
        Err(CfgErrorKind::ContinueOutsideLoop),
        "a top-level continue outside any loop must be reported"
    );
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

    // A top-level `break` is equally invalid and must fail before lowering.
    let mut block = Parser::default().lua51().parse("break").expect("parse");
    let error = desugar_to_lua51(&mut block).expect_err("top-level `break`");
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

#[test]
fn desugar_reports_an_unlowerable_repeat_continue() {
    let mut block = Parser::default()
        .luau()
        .parse(
            "local i = 0\nrepeat\n  local done = i >= 3\n  i = i + 1\n  if not done then continue end\nuntil done",
        )
        .expect("parse");
    let error = desugar_to_lua51(&mut block).expect_err("the repeat condition reads a body local");
    assert!(
        matches!(error, DesugarError::Continue(_)),
        "unexpected error: {error:?}"
    );
}
