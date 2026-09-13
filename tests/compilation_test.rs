//! Tests for the diagnostics and compilation layers.

use r4::diagnostic::Severity;
use r4::modules::MemoryModuleProvider;
use r4::nodes::{Block, Expression, LastStatement};
use r4::semantic::{Binding, SemanticOptions};
use r4::{Compilation, Parser};

fn returned_identifier(block: &Block) -> &r4::nodes::Identifier {
    let Some(LastStatement::Return(statement)) = block.get_last_statement() else {
        panic!("expected a return statement");
    };
    match statement.iter_expressions().next() {
        Some(Expression::Identifier(identifier)) => identifier,
        other => panic!("expected an identifier, got {other:?}"),
    }
}

#[test]
fn compilation_parses_and_analyzes_trees() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("local x = 1\nreturn x")
        .expect("valid code");

    assert_eq!(compilation.len(), 1);
    assert!(compilation.diagnostics().is_empty());
    assert!(compilation.tree(tree).is_some());
    assert!(compilation.block(tree).is_some());
    assert!(compilation.model(tree).is_some());
    assert!(compilation.source(tree).unwrap().contains("local x = 1"));

    let model = compilation.model(tree).unwrap();
    let identifier = returned_identifier(compilation.block(tree).unwrap());
    assert!(model.resolve_identifier(identifier).is_some());
}

#[test]
fn compilation_reports_parse_errors_with_locations() {
    let mut compilation = Compilation::new();
    let error = compilation
        .add_tree("local x = ")
        .expect_err("invalid code must fail");

    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.code, Some("parse/syntax"));
    assert!(error.location.is_some(), "syntax errors carry a location");
    assert!(compilation.tree(TreeIdForTest::FIRST).is_none());
    assert_eq!(compilation.diagnostics().len(), 1);
    assert_eq!(compilation.diagnostics()[0], error);
}

// A tiny helper so the test does not depend on internal count semantics.
struct TreeIdForTest;
impl TreeIdForTest {
    const FIRST: r4::TreeId = r4::TreeId(0);
}

#[test]
fn compilation_reports_analysis_errors_with_spans() {
    let mut compilation = Compilation::with_parser(Parser::default().preserve_tokens());
    compilation.set_options(SemanticOptions {
        strict_globals: true,
        strict_types: false,
    });

    let error = compilation
        .add_tree("return missing")
        .expect_err("strict globals must reject the global");

    assert_eq!(error.code, Some("semantic/undeclared-symbol"));
    assert!(
        error.span.is_some(),
        "a preserved-token error carries a span"
    );
    assert!(
        error.location.is_some(),
        "the location is resolved from the span"
    );
    let location = error.location.unwrap();
    assert_eq!(location.start.line, 1);
    assert_eq!(location.start.column, 8);
}

#[test]
fn compilation_returns_unified_bindings() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("type Point = number\nlocal x: Point = 1\nreturn x")
        .expect("valid code");

    let block = compilation.block(tree).unwrap();
    let identifier = returned_identifier(block);
    let binding = compilation
        .binding_of(tree, identifier)
        .expect("the identifier must bind");
    let Binding::Symbol(symbol) = binding else {
        panic!("expected a symbol binding, got {binding:?}");
    };
    assert_eq!(binding.as_symbol(), Some(symbol));
    assert_eq!(compilation.model(tree).unwrap().symbol(symbol).name, "x");
}

#[test]
fn compilation_resolves_module_graphs_and_records_failures() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("return require('./a')")
        .expect("valid code");

    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "return { value = 1 }");
    let resolved = compilation
        .resolve_modules(tree, &provider)
        .expect("resolution must succeed");
    assert!(resolved.graph.module_by_key("a").is_some());

    let error = compilation
        .resolve_modules(tree, &MemoryModuleProvider::new())
        .expect_err("a missing module must fail");
    assert_eq!(error.code, Some("module/missing"));
    assert!(compilation
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code == Some("module/missing")));
}

#[test]
fn semantic_errors_convert_to_diagnostics() {
    let error = r4::semantic::SemanticError {
        kind: r4::semantic::SemanticErrorKind::UnresolvedType {
            name: "Missing".to_owned(),
        },
        line: Some(3),
        span: None,
    };
    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.code, Some("semantic/unresolved-type"));
    assert!(diagnostic.message.contains("Missing"));
}
