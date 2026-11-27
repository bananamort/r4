//! Fail-fast API coverage for the local-function inliner.

use r4::rules::canonicalize::{inline_local_functions, InlineError, ReachingError};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

#[test]
fn a_foreign_semantic_model_is_a_typed_scope_error() {
    let block = Parser::default()
        .parse("local function f()\n    return 1\nend\nreturn f()")
        .expect("parse");
    let _model = SemanticAnalyzer::new().analyze(&block).expect("analyze");

    let other = Parser::default()
        .parse("return 0")
        .expect("parse the second program");
    let other_model = SemanticAnalyzer::new()
        .analyze(&other)
        .expect("analyze the second program");

    let mut malformed = block.clone();
    let error = inline_local_functions(&mut malformed, &other_model)
        .expect_err("a foreign model must be rejected");
    assert!(
        matches!(error, InlineError::Scope(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn a_break_outside_a_loop_is_a_typed_reaching_error() {
    let block = Parser::default().parse("break").expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analyze");

    let mut block = block;
    let error = inline_local_functions(&mut block, &model)
        .expect_err("a function region with an invalid break must fail");
    assert!(
        matches!(error, InlineError::Reaching(ReachingError::ControlFlow(_))),
        "unexpected error: {error:?}"
    );
}
