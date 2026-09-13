//! Tests for the multi-tree semantic layer, the module resolver and
//! identity-based cross-module resolution.

use r4::modules::{MemoryModuleProvider, ModuleResolver, ResolveError};
use r4::nodes::Block;
use r4::rules::require::{bundle_modules, BundleError};
use r4::semantic::{
    extract_interface, ModuleError, ModuleInterface, MultiModuleModel, SemanticAnalyzer,
    SemanticModel, SymbolId,
};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn interface_of(code: &str) -> ModuleInterface {
    let (block, model) = analyze(code);
    extract_interface(&block, &model).expect("interface extraction must succeed")
}

fn symbol_named(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("symbol `{name}` must exist"))
        .id
}

#[test]
fn interface_extracts_inline_returned_table() {
    let interface = interface_of(
        "local function add(a, b) return a + b end\nreturn { add = add, value = 1, [\"key\"] = 2 }",
    );

    assert!(interface.has_value("add"));
    assert!(interface.has_value("value"));
    assert!(interface.has_value("key"));
    assert!(interface.value_symbol("add").is_some());
    assert_eq!(interface.value_symbol("value"), None);
}

#[test]
fn interface_extracts_mutated_returned_table() {
    let interface =
        interface_of("local t = {}\nfunction t.f() end\nt.count = 1\nt[\"x\"] = 2\nreturn t");

    assert!(interface.has_value("f"));
    assert!(interface.has_value("count"));
    assert!(interface.has_value("x"));
}

#[test]
fn interface_extracts_single_value_export() {
    let interface = interface_of("local function f() end\nreturn f");

    assert!(interface.values.is_empty());
    assert!(interface.value.is_some());
}

#[test]
fn interface_exposes_exported_types_only() {
    let interface = interface_of("export type Point = number\ntype Hidden = string\nreturn 1");

    assert!(interface.type_alias("Point").is_some());
    assert_eq!(interface.type_alias("Hidden"), None);
}

#[test]
fn imported_type_resolves_by_identity() {
    let (a_block, a_model) = analyze("export type Point = number\nreturn 1");
    let (b_block, b_model) = analyze("local a = require('./a')\nreturn a");

    let mut graph = MultiModuleModel::new();
    let a = graph.add_module("a", a_block, a_model).expect("module a");
    let b = graph.add_module("b", b_block, b_model).expect("module b");

    let alias = symbol_named(&graph.module(b).model, "a");
    graph.bind_import(b, alias, a);

    let (target, id) = graph
        .resolve_imported_type(b, "Point")
        .expect("Point must resolve through the import");
    assert_eq!(target, a);
    assert_eq!(graph.module(a).interface.type_alias("Point"), Some(id));
    assert_eq!(graph.modules_exporting_type("Point"), vec![a]);
}

#[test]
fn unknown_and_ambiguous_imported_types_are_typed_errors() {
    let (a_block, a_model) = analyze("export type Point = number\nreturn 1");
    let (c_block, c_model) = analyze("export type Point = string\nreturn 1");
    let (b_block, b_model) =
        analyze("local a = require('./a')\nlocal c = require('./c')\nreturn a, c");

    let mut graph = MultiModuleModel::new();
    let a = graph.add_module("a", a_block, a_model).expect("module a");
    let c = graph.add_module("c", c_block, c_model).expect("module c");
    let b = graph.add_module("b", b_block, b_model).expect("module b");

    let alias_a = symbol_named(&graph.module(b).model, "a");
    let alias_c = symbol_named(&graph.module(b).model, "c");

    // No import yet: the type is unknown to b.
    assert!(matches!(
        graph.resolve_imported_type(b, "Point"),
        Err(ModuleError::UnknownImportedType { .. })
    ));

    graph.bind_import(b, alias_a, a);
    assert_eq!(
        graph.resolve_imported_type(b, "Point"),
        Ok((a, graph.module(a).interface.type_alias("Point").unwrap()))
    );

    graph.bind_import(b, alias_c, c);
    assert!(matches!(
        graph.resolve_imported_type(b, "Point"),
        Err(ModuleError::AmbiguousImportedType { .. })
    ));
}

#[test]
fn imported_value_resolves_by_identity() {
    let (a_block, a_model) =
        analyze("local function add(a, b) return a + b end\nreturn { add = add }");
    let (b_block, b_model) = analyze("local a = require('./a')\nreturn a.add(1, 2)");

    let mut graph = MultiModuleModel::new();
    let a = graph.add_module("a", a_block, a_model).expect("module a");
    let b = graph.add_module("b", b_block, b_model).expect("module b");

    let alias = symbol_named(&graph.module(b).model, "a");
    let add = symbol_named(&graph.module(a).model, "add");
    graph.bind_import(b, alias, a);

    assert_eq!(graph.resolve_imported_value(b, alias, "add"), Ok(Some(add)));
    assert!(matches!(
        graph.resolve_imported_value(b, alias, "missing"),
        Err(ModuleError::UnknownExport { .. })
    ));
    assert!(matches!(
        graph.resolve_imported_value(b, add, "add"),
        Err(ModuleError::UnknownImportAlias { .. })
    ));
}

#[test]
fn resolver_binds_imports_in_topological_order() {
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "local b = require('./b')\nreturn b");
    provider.insert("b", "return 1");

    let (root, root_model) = analyze("return require('./a')");
    let resolved = ModuleResolver::new(&provider)
        .resolve(root, root_model, "<root>")
        .expect("resolution must succeed");

    let a = resolved.graph.module_by_key("a").expect("module a");
    let b = resolved.graph.module_by_key("b").expect("module b");

    // Dependencies come before their dependents; the root is last.
    let order: Vec<_> = resolved
        .order
        .iter()
        .map(|id| resolved.graph.module(*id).key.clone())
        .collect();
    assert_eq!(
        order,
        vec!["b".to_owned(), "a".to_owned(), "<root>".to_owned()]
    );

    let alias = symbol_named(&resolved.graph.module(a).model, "b");
    assert_eq!(resolved.graph.import_target(a, alias), Some(b));
}

#[test]
fn resolver_reports_missing_and_cyclic_modules() {
    let (root, root_model) = analyze("return require('./missing')");
    let provider = MemoryModuleProvider::new();
    let error = ModuleResolver::new(&provider)
        .resolve(root, root_model, "<root>")
        .expect_err("a missing module must be reported");
    assert!(matches!(error, ResolveError::MissingModule { ref path } if path == "missing"));

    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "local b = require('./b')\nreturn b");
    provider.insert("b", "local a = require('./a')\nreturn a");
    let (root, root_model) = analyze("return require('./a')");
    let error = ModuleResolver::new(&provider)
        .resolve(root, root_model, "<root>")
        .expect_err("a cycle must be reported");
    assert!(matches!(error, ResolveError::CyclicRequire { .. }));
}

#[test]
fn bundling_respects_private_type_shadowing() {
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "export type Public = number\nreturn 1");
    provider.insert(
        "b",
        "local a = require('./a')\ntype Public = string\nlocal x: Public = 'x'\nreturn a",
    );

    let (mut block, model) = analyze("return require('./b')");
    bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");

    let output = generate(&block);
    assert!(
        output.contains("type Public = string"),
        "the private local type must not be retargeted: {output}"
    );
    assert!(
        output.contains("type __R4_TYPE_Public = number"),
        "{output}"
    );

    let reparsed = Parser::default().parse(&output).expect("output reparses");
    SemanticAnalyzer::new()
        .strict_types()
        .analyze(&reparsed)
        .expect("shadowed types must resolve strictly");
}

#[test]
fn bundling_rejects_unimported_type_references() {
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "export type Public = number\nreturn 1");
    provider.insert("b", "type Local = Public\nreturn 1");

    let (mut block, model) = analyze("return require('./a') + require('./b')");
    let error = bundle_modules(&mut block, &model, &provider)
        .expect_err("a type from an unimported module must be rejected");

    assert!(matches!(
        error,
        BundleError::UnimportedType { ref name, .. } if name == "Public"
    ));
}

#[test]
fn bundling_rejects_ambiguous_imported_types() {
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "export type Point = number\nreturn 1");
    provider.insert("c", "export type Point = string\nreturn 1");
    provider.insert(
        "b",
        "local a = require('./a')\nlocal c = require('./c')\ntype Local = Point\nreturn a, c",
    );

    let (mut block, model) = analyze("return require('./b')");
    let error = bundle_modules(&mut block, &model, &provider)
        .expect_err("an ambiguous imported type must be rejected");

    assert!(matches!(
        error,
        BundleError::Module(ModuleError::AmbiguousImportedType { ref name, .. }) if name == "Point"
    ));
}

fn generate(block: &Block) -> String {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}
