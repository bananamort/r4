//! Tests for loop-invariant hoisting error surfaces.

use r4::cfg::analysis::{LicmError, LivenessError};
use r4::cfg::{CfgError, CfgErrorKind, UnidentifiedBlock};
use r4::rules::canonicalize::HoistLoopInvariantsError;

#[test]
fn hoist_loop_invariants_errors_render() {
    for error in [
        HoistLoopInvariantsError::Provenance(UnidentifiedBlock { line: Some(1) }),
        HoistLoopInvariantsError::Cfg(CfgError {
            kind: CfgErrorKind::MissingBlockIdentity,
            line: None,
        }),
        HoistLoopInvariantsError::Liveness(LicmError::Liveness(
            LivenessError::UnresolvedIdentifier {
                name: "x".to_owned(),
                line: None,
            },
        )),
    ] {
        assert!(!error.to_string().is_empty());
    }

    let _: HoistLoopInvariantsError = UnidentifiedBlock { line: None }.into();
    let _: HoistLoopInvariantsError = CfgError {
        kind: CfgErrorKind::MissingBlockIdentity,
        line: None,
    }
    .into();
    let _: HoistLoopInvariantsError = LicmError::Liveness(LivenessError::UnresolvedIdentifier {
        name: "x".to_owned(),
        line: None,
    })
    .into();
}

#[test]
fn licm_error_renders_and_converts() {
    let error = LicmError::Liveness(LivenessError::UnresolvedIdentifier {
        name: "x".to_owned(),
        line: Some(1),
    });
    assert!(!error.to_string().is_empty());
    let converted: LicmError = LivenessError::UnresolvedIdentifier {
        name: "x".to_owned(),
        line: None,
    }
    .into();
    assert!(!converted.to_string().is_empty());
}
