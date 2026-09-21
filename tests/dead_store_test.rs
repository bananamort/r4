//! Tests for dead-store removal and its typed errors.

use r4::cfg::analysis::LivenessError;
use r4::cfg::{CfgError, CfgErrorKind};
use r4::rules::canonicalize::{remove_dead_stores, DeadStoreError};
use r4::Parser;

#[test]
fn dead_store_errors_render() {
    for error in [
        DeadStoreError::Cfg(CfgError {
            kind: CfgErrorKind::BreakOutsideLoop,
            line: None,
        }),
        DeadStoreError::Liveness(LivenessError::UnresolvedIdentifier {
            name: "x".to_owned(),
            line: Some(1),
        }),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn removes_dead_stores() {
    for code in [
        "local x = 1\nreturn 2",
        "local x = 1\nx = 2\nreturn x",
        "local x\ndo x = 1 end\nreturn 2",
        "local x = 1\nif x then x = 2 end\nreturn x",
    ] {
        let (mut block, model) = Parser::default()
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("`{code}`: {error}"));
        let _ = remove_dead_stores(&mut block, &model)
            .unwrap_or_else(|error| panic!("`{code}`: {error}"));
    }
}
