//! Tests for symbol display formatting.

use r4::semantic::{MultiModuleModel, SymbolDisplayFormat, SymbolKind, SymbolRef};
use r4::Parser;

fn analyze(code: &str) -> (r4::nodes::Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn symbol_id(model: &r4::semantic::SemanticModel, name: &str) -> r4::semantic::SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol named `{name}`"))
        .id
}

#[test]
fn renders_the_name_by_default() {
    let (_, model) = analyze("local value = 1\nreturn value");
    let id = symbol_id(&model, "value");
    assert_eq!(model.display_symbol(id).to_string(), "value");
}

#[test]
fn includes_the_declaration_kind() {
    let (_, model) = analyze("local value = 1\nreturn value");
    let id = symbol_id(&model, "value");
    let format = SymbolDisplayFormat::new().with_kind(true);
    assert_eq!(
        model.display_symbol_with(id, format).to_string(),
        "local value"
    );

    let (_, model) = analyze("local function f(parameter) return parameter end\nreturn f");
    let id = symbol_id(&model, "parameter");
    assert_eq!(
        model.display_symbol_with(id, format).to_string(),
        "parameter parameter"
    );
}

#[test]
fn includes_the_declaring_scope_kind() {
    let (_, model) = analyze("local function f(parameter) return parameter end\nreturn f");
    let id = symbol_id(&model, "parameter");
    let format = SymbolDisplayFormat::new()
        .with_kind(true)
        .with_scope_kind(true);
    let display = model.display_symbol_with(id, format);
    assert_eq!(display.to_string(), "parameter parameter (function scope)");
    assert_eq!(display.kind(), SymbolKind::Parameter);
    assert_eq!(display.name(), "parameter");
    assert_eq!(
        display.scope_kind(),
        Some(r4::semantic::ScopeKind::Function)
    );

    let (_, model) = analyze("local top = 1\nreturn top");
    let id = symbol_id(&model, "top");
    assert_eq!(
        model.display_symbol_with(id, format).to_string(),
        "local top (file scope)"
    );
}

#[test]
fn renders_globals_from_the_global_scope() {
    let (_, model) = analyze("return print");
    let id = symbol_id(&model, "print");
    let format = SymbolDisplayFormat::new().with_kind(true);
    let display = model.display_symbol_with(id, format);
    assert_eq!(display.to_string(), "global print");
    assert_eq!(display.kind(), SymbolKind::Global);
    assert_eq!(display.scope_kind(), Some(r4::semantic::ScopeKind::Global));
}

#[test]
fn display_format_accessors() {
    let format = SymbolDisplayFormat::new();
    assert!(!format.kind());
    assert!(!format.scope_kind());
    let format = SymbolDisplayFormat::new()
        .with_kind(true)
        .with_scope_kind(true);
    assert!(format.kind());
    assert!(format.scope_kind());
}

#[test]
fn symbol_implements_display_as_its_name() {
    let (_, model) = analyze("local value = 1\nreturn value");
    let id = symbol_id(&model, "value");
    assert_eq!(model.symbol(id).to_string(), "value");
}

#[test]
fn shadowed_symbols_display_with_their_own_declaration() {
    let (_, model) =
        analyze("local value = 1\ndo\n  local value = 2\n  print(value)\nend\nreturn value");
    let format = SymbolDisplayFormat::new()
        .with_kind(true)
        .with_scope_kind(true);
    let displays: Vec<String> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.name == "value" && symbol.kind == SymbolKind::Local)
        .map(|symbol| model.display_symbol_with(symbol.id, format).to_string())
        .collect();
    assert_eq!(
        displays,
        vec!["local value (file scope)", "local value (block scope)"]
    );
}

#[test]
fn multi_module_display_resolves_locals_and_globals() {
    let mut graph = MultiModuleModel::new();
    let (block, model) = analyze("local x = 1\nprint(x)\nreturn x");
    graph
        .add_module("main".to_owned(), block, model)
        .expect("the module is valid");

    let print = graph.global_symbol("print").expect("print is global");
    assert_eq!(
        graph.display_symbol(SymbolRef::Global(print)).to_string(),
        "print"
    );

    let format = SymbolDisplayFormat::new().with_kind(true);
    let index = graph.whole_program_index();
    let local = (0..index.local_count() as u32)
        .find_map(|rank| {
            index.local(rank).filter(|qualified| {
                let symbol = SymbolRef::Local {
                    module: qualified.module,
                    symbol: qualified.symbol,
                };
                graph.display_symbol_with(symbol, format).name() == "x"
            })
        })
        .expect("the module local `x` is indexed");
    let local = SymbolRef::Local {
        module: local.module,
        symbol: local.symbol,
    };
    assert_eq!(
        graph.display_symbol_with(local, format).to_string(),
        "local x"
    );
}
