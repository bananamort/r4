//! Tests for unused-variable removal and its typed errors.

use r4::nodes::{Block, Expression, Statement, TypedIdentifier, VariableAssignment};
use r4::rules::canonicalize::{remove_unused_variables, UnusedVariableError};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

#[test]
fn unused_variable_errors_render() {
    for error in [
        UnusedVariableError::UnresolvedDeclaration {
            name: "x".to_owned(),
            line: Some(1),
        },
        UnusedVariableError::UnresolvedDeclaration {
            name: "x".to_owned(),
            line: None,
        },
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_declaration_missing_from_the_model_is_rejected() {
    let block = Parser::default().parse("return 1").expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analyze");

    let mut mutated: Block = block;
    mutated.push_statement(Statement::LocalAssign(VariableAssignment::new(
        vec![TypedIdentifier::new("y")],
        vec![Expression::from(1.0)],
    )));

    let error = remove_unused_variables(&mut mutated, &model)
        .expect_err("an unresolved declaration is reported");
    assert!(matches!(
        error,
        UnusedVariableError::UnresolvedDeclaration { .. }
    ));
}

#[test]
fn removes_unused_locals() {
    let mut block = Parser::default()
        .parse("local unused = 1\nlocal used = 2\nreturn used")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analyze");
    let removed = remove_unused_variables(&mut block, &model).expect("removes");
    assert!(removed >= 1);
}
