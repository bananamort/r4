//! Tests for unreachable-code removal and its typed errors.

use r4::cfg::analysis::ConstantPropagationError;
use r4::cfg::{CfgError, CfgErrorKind, UnidentifiedBlock};
use r4::nodes::Block;
use r4::rules::canonicalize::{remove_unreachable_code, UnreachableCodeError};
use r4::semantic::SemanticModel;
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

#[test]
fn removes_unreachable_branches_and_loops() {
    for code in [
        "if true then a() else b() end\nwhile false do c() end\nreturn 0",
        "if false then a() end\nreturn 0",
        "local x = 1\nif x then a() else b() end\nreturn 0",
        "local x = true\nif x then a() else b() end\nreturn 0",
        "while false do work() end\nreturn 0",
        "repeat work() until true\nreturn 0",
        "return 0",
    ] {
        let (mut block, model) = analyze(code);
        let _ = remove_unreachable_code(&mut block, &model)
            .unwrap_or_else(|error| panic!("`{code}`: {error}"));
    }
}

#[test]
fn unreachable_code_errors_render() {
    for error in [
        UnreachableCodeError::Cfg(CfgError {
            kind: CfgErrorKind::BreakOutsideLoop,
            line: None,
        }),
        UnreachableCodeError::ConstantPropagation(ConstantPropagationError::UnresolvedIdentifier {
            name: "x".to_owned(),
            line: Some(1),
        }),
        UnreachableCodeError::Provenance(UnidentifiedBlock { line: Some(2) }),
    ] {
        assert!(!error.to_string().is_empty());
    }

    let _: UnreachableCodeError = CfgError {
        kind: CfgErrorKind::MissingBlockIdentity,
        line: None,
    }
    .into();
    let _: UnreachableCodeError = ConstantPropagationError::UnresolvedIdentifier {
        name: "x".to_owned(),
        line: None,
    }
    .into();
    let _: UnreachableCodeError = UnidentifiedBlock { line: None }.into();
}
