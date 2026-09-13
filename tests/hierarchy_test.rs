//! Tests for the unified global scope and scope hierarchy across modules.

use r4::nodes::Block;
use r4::semantic::{MultiModuleModel, ScopeRef, SemanticModel, SymbolId, SymbolKind, SymbolRef};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn global_named(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name && symbol.kind == SymbolKind::Global)
        .unwrap_or_else(|| panic!("global `{name}` must exist"))
        .id
}

#[test]
fn globals_share_identity_across_modules() {
    let (first_block, first_model) = analyze("print(1)\nreturn 0");
    let (second_block, second_model) = analyze("print(2)\nreturn 0");

    let mut graph = MultiModuleModel::new();
    let first = graph
        .add_module("first", first_block, first_model)
        .expect("module first");
    let second = graph
        .add_module("second", second_block, second_model)
        .expect("module second");

    let first_print = global_named(&graph.module(first).model, "print");
    let second_print = global_named(&graph.module(second).model, "print");

    // Raw ids are module-local; both modules happen to number `print` first.
    assert_eq!(first_print, SymbolId(0));
    assert_eq!(second_print, SymbolId(0));
    assert_eq!(
        graph.global_of(first, first_print),
        graph.global_of(second, second_print)
    );
    assert!(graph.same_global((first, first_print), (second, second_print)));

    let print = graph.global_symbol("print").expect("print is global");
    assert_eq!(graph.global(print).name, "print");
    assert_eq!(graph.globals().count(), 1);
}

#[test]
fn unified_hierarchy_reaches_the_shared_root() {
    let (block, model) = analyze("local x = 1\nprint(x)\nreturn x");
    let mut graph = MultiModuleModel::new();
    let module = graph.add_module("module", block, model).expect("module");

    let file = graph.file_scope(module);
    assert!(matches!(file, ScopeRef::Module { .. }));
    assert_eq!(graph.parent_scope(file), Some(ScopeRef::Global));
    assert_eq!(graph.parent_scope(ScopeRef::Global), None);

    let print = graph.global_symbol("print").expect("print is global");
    assert_eq!(
        graph.resolve_in_scope(file, "print"),
        Some(SymbolRef::Global(print))
    );

    let local = graph
        .resolve_in_scope(file, "x")
        .expect("x resolves in the file scope");
    assert!(matches!(local, SymbolRef::Local { .. }));
}

#[test]
fn file_scopes_of_distinct_modules_share_the_root() {
    let (first_block, first_model) = analyze("local a = 1\nreturn a");
    let (second_block, second_model) = analyze("local b = 2\nreturn b");

    let mut graph = MultiModuleModel::new();
    let first = graph
        .add_module("first", first_block, first_model)
        .expect("module first");
    let second = graph
        .add_module("second", second_block, second_model)
        .expect("module second");

    for module in [first, second] {
        let file = graph.file_scope(module);
        let parent = graph.parent_scope(file).expect("file scope has a parent");
        assert_eq!(parent, ScopeRef::Global);
        assert_eq!(graph.parent_scope(parent), None);
    }
}
