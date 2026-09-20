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

#[test]
fn filesystem_provider_paths_and_accessors() {
    use r4::modules::FileSystemModuleProvider;
    use std::path::{Path, PathBuf};

    let provider = FileSystemModuleProvider::with_extension("/tmp/r4-root", "lua");
    assert_eq!(provider.root(), Path::new("/tmp/r4-root"));
    assert_eq!(provider.extension(), "lua");
    assert_eq!(provider.extensions(), ["lua".to_owned()]);

    let provider = provider
        .with_extensions(["lua", "luau"])
        .with_module_folders(["init"]);
    assert_eq!(provider.extensions().len(), 2);

    // A key without a file extension is only tried with suffixes.
    let candidates = provider.candidate_paths("pkg/util");
    assert!(candidates.iter().all(|path| path.extension().is_some()));

    // A key that already has an extension also tries the literal path first.
    let literal = provider.candidate_paths("pkg/util.lua");
    assert_eq!(
        literal.first(),
        Some(&PathBuf::from("/tmp/r4-root/pkg/util.lua"))
    );

    // Current-directory components are ignored; `..` escapes are rejected.
    let _ = provider.candidate_paths("./pkg/./util");
    assert!(provider.candidate_paths("../etc/passwd").is_empty());
    assert!(provider.path_for("does/not/exist").is_none());
}

#[test]
fn memory_provider_and_analysis_cache() {
    use r4::modules::{AnalysisCache, AnalyzedModule, MemoryModuleProvider, ModuleProvider};

    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "return 1");
    assert_eq!(
        provider.load("a").expect("load succeeds"),
        Some("return 1".to_owned())
    );
    assert_eq!(provider.load("missing").expect("load succeeds"), None);

    let (block, model) = analyze("return 1");
    let mut cache = AnalysisCache::new();
    assert!(cache.is_empty());
    let stored = cache.insert(AnalyzedModule {
        source: "return 1".to_owned(),
        block,
        model,
    });
    assert_eq!(cache.len(), 1);
    assert!(cache.get("return 1").is_some());
    assert!(cache.get("other").is_none());
    assert_eq!(stored.source, "return 1");
    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn classifies_require_call_shapes() {
    use r4::modules::{classify_require_call, resolve_instance_path, InstanceSegment, RequireCall};
    use r4::nodes::{Expression, FunctionCall, Statement};

    fn call_and_model(code: &str) -> (FunctionCall, SemanticModel) {
        let (block, model) = analyze(code);
        for statement in block.iter_statements() {
            if let Statement::LocalAssign(assign) = statement {
                for value in assign.iter_values() {
                    if let Expression::Call(call) = value {
                        return ((**call).clone(), model);
                    }
                }
            }
        }
        panic!("no call in `{code}`");
    }

    let (call, model) = call_and_model("local x = require(\"a.b\")");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::Path {
            path: "a.b".to_owned()
        }
    );

    let (call, model) = call_and_model("local x = require(script.Parent.Util)");
    let RequireCall::Instance { segments } = classify_require_call(&call, &model) else {
        panic!("expected an instance path");
    };
    assert!(!segments.is_empty());

    let (call, model) = call_and_model("local x = require(game:GetService(\"Players\"))");
    let RequireCall::Instance { segments } = classify_require_call(&call, &model) else {
        panic!("expected a service path");
    };
    assert!(segments.contains(&InstanceSegment::Service("Players".to_owned())));

    let (call, model) = call_and_model("local x = require(name)");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::NonLiteral
    );

    let (call, model) = call_and_model("local x = require({})");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::NonLiteral
    );

    let (call, model) = call_and_model("local x = require(\"a\", \"b\")");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::NonLiteral
    );

    let (call, model) = call_and_model("local x = notrequire(\"a\")");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::NotRequire
    );

    // String-call syntax and index/parenthesized instance paths.
    let (call, model) = call_and_model("local x = require \"a.b\"");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::Path {
            path: "a.b".to_owned()
        }
    );

    let (call, model) = call_and_model("local x = require(script[\"Child\"])");
    let RequireCall::Instance { segments } = classify_require_call(&call, &model) else {
        panic!("expected an instance path");
    };
    assert!(segments.contains(&InstanceSegment::Child("Child".to_owned())));

    // A statically-known string index extends the path; a dynamic one does not.
    let (call, model) = call_and_model("local x = require(script[\"Child\"].Grandchild)");
    let RequireCall::Instance { segments } = classify_require_call(&call, &model) else {
        panic!("expected an instance path");
    };
    assert_eq!(
        segments,
        vec![
            InstanceSegment::Child("Child".to_owned()),
            InstanceSegment::Child("Grandchild".to_owned()),
        ]
    );
    let (call, model) = call_and_model("local x = require(script[child].Grandchild)");
    let _ = classify_require_call(&call, &model);

    let (call, model) = call_and_model("local x = require((script).Child)");
    let RequireCall::Instance { segments } = classify_require_call(&call, &model) else {
        panic!("expected an instance path");
    };
    assert!(!segments.is_empty());

    for code in [
        "local x = require(script:FindFirstChild(\"Child\"))",
        "local x = require(script:WaitForChild(\"Child\"))",
        "local x = require(script.Parent.Parent)",
    ] {
        let (call, model) = call_and_model(code);
        let _ = classify_require_call(&call, &model);
    }

    let path = resolve_instance_path(
        "script",
        &[
            InstanceSegment::Parent,
            InstanceSegment::Child("Util".to_owned()),
        ],
    );
    assert!(path.is_some());
    let _ = resolve_instance_path("script", &[InstanceSegment::Service("Players".to_owned())]);
    let _ = resolve_instance_path("a/b", &[InstanceSegment::Parent]);
}

#[test]
fn module_and_bundle_errors_render() {
    use r4::modules::{ModuleLoadError, ResolveError};
    use r4::rules::require::BundleError;

    let load = ModuleLoadError::Read {
        key: "a".to_owned(),
        message: "io".to_owned(),
    };
    assert_eq!(load.code(), "module/read");
    assert!(!load.to_string().is_empty());

    let resolve_errors = [
        ResolveError::MissingModule {
            path: "a".to_owned(),
        },
        ResolveError::CyclicRequire {
            path: "a".to_owned(),
        },
        ResolveError::NonLiteralRequire {
            module: "a".to_owned(),
            line: Some(3),
        },
        ResolveError::NonLiteralRequire {
            module: "a".to_owned(),
            line: None,
        },
        ResolveError::ModuleLoad {
            error: load.clone(),
        },
    ];
    for error in resolve_errors {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
        let _ = error.to_diagnostic();
    }

    let bundle_errors = [
        BundleError::MissingModule {
            path: "a".to_owned(),
        },
        BundleError::CyclicRequire {
            path: "a".to_owned(),
        },
        BundleError::NonLiteralRequire {
            module: Some("a".to_owned()),
            line: Some(2),
        },
        BundleError::NonLiteralRequire {
            module: None,
            line: None,
        },
        BundleError::DuplicateExportedType {
            name: "T".to_owned(),
        },
        BundleError::UnimportedType {
            module: "a".to_owned(),
            name: "T".to_owned(),
        },
        BundleError::ModuleLoad { error: load },
    ];
    for error in bundle_errors {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn multi_module_model_accessors() {
    use r4::semantic::{MultiModuleModel, ScopeRef, SymbolRef};

    let mut graph = MultiModuleModel::new();
    assert!(graph.is_empty());
    assert_eq!(graph.len(), 0);
    assert_eq!(graph.iter_ids().count(), 0);

    let (block_a, model_a) = analyze("print(1)\nlocal x = 1\nreturn x");
    let (block_b, model_b) = analyze("print(2)\ndo local y = 1 end\nreturn 2");
    let a = graph.add_module("a", block_a, model_a).expect("module a");
    let b = graph.add_module("b", block_b, model_b).expect("module b");

    assert_eq!(graph.len(), 2);
    assert!(!graph.is_empty());
    assert_eq!(graph.iter_ids().count(), 2);
    assert_eq!(graph.module(a).key, "a");
    assert_eq!(graph.module_by_key("a"), Some(a));
    assert_eq!(graph.module_by_key("missing"), None);

    // The same global unifies across modules.
    let print = graph.global_symbol("print").expect("print is a global");
    assert_eq!(graph.global(print).name, "print");
    assert_eq!(graph.globals().count(), 1);
    let print_a = symbol_named(&graph.module(a).model, "print");
    assert_eq!(graph.global_of(a, print_a), Some(print));
    assert!(graph.same_global((a, print_a), (a, print_a)));

    // Scope navigation reaches the shared global root.
    let root = graph.root_scope();
    assert_eq!(root, ScopeRef::Global);
    assert_eq!(graph.parent_scope(ScopeRef::Global), None);
    assert!(graph.scope(ScopeRef::Global).is_none());
    let file_a = graph.file_scope(a);
    assert!(graph.parent_scope(file_a).is_some());
    assert!(graph.scope(file_a).is_some());
    assert_eq!(
        graph.resolve_in_scope(file_a, "print"),
        Some(SymbolRef::Global(print))
    );
    assert_eq!(
        graph.resolve_in_scope(root, "print"),
        Some(SymbolRef::Global(print))
    );
    let _ = graph.parent_scope(graph.file_scope(b));
    let _ = graph.display_symbol(SymbolRef::Global(print));

    // A nested scope has a module-scoped parent.
    let (block_c, model_c) = analyze("do local z = 1 end\nreturn 3");
    let c = graph.add_module("c", block_c, model_c).expect("module c");
    let mut nested = None;
    for (scope_id, scope) in graph.module(c).model.scopes.iter().enumerate() {
        if scope.kind == r4::semantic::ScopeKind::Block {
            nested = Some(r4::semantic::ScopeId(scope_id as u32));
        }
    }
    if let Some(nested) = nested {
        let scope_ref = ScopeRef::Module {
            module: c,
            scope: nested,
        };
        assert!(graph.scope(scope_ref).is_some());
        assert!(graph.parent_scope(scope_ref).is_some());
    }
    let _ = graph.whole_program_index();
}

#[test]
fn classifies_additional_require_shapes() {
    use r4::modules::{classify_require_call, resolve_instance_path, InstanceSegment, RequireCall};
    use r4::nodes::{Expression, FunctionCall, Statement};

    fn call_and_model(code: &str) -> (FunctionCall, SemanticModel) {
        let (block, model) = analyze(code);
        for statement in block.iter_statements() {
            if let Statement::LocalAssign(assign) = statement {
                for value in assign.iter_values() {
                    if let Expression::Call(call) = value {
                        return ((**call).clone(), model);
                    }
                }
            }
        }
        panic!("no call in `{code}`");
    }

    for (code, expected) in [
        ("local x = require(1)", RequireCall::NonLiteral),
        ("local x = require({})", RequireCall::NonLiteral),
        ("local x = require(script[child])", RequireCall::NonLiteral),
        (
            "local x = require(script:Unknown('X'))",
            RequireCall::NonLiteral,
        ),
        (
            "local x = require(script:FindFirstChild())",
            RequireCall::NonLiteral,
        ),
        ("local x = require(function() end)", RequireCall::NonLiteral),
    ] {
        let (call, model) = call_and_model(code);
        assert_eq!(classify_require_call(&call, &model), expected, "`{code}`");
    }

    // A parenthesized instance path is still recognized.
    let (call, model) = call_and_model("local x = require((script.Child))");
    assert!(matches!(
        classify_require_call(&call, &model),
        RequireCall::Instance { .. }
    ));

    // `require "..."` with an unreadable string falls back to non-literal.
    let (call, model) = call_and_model("local x = require '\\255'");
    assert_eq!(
        classify_require_call(&call, &model),
        RequireCall::NonLiteral
    );

    for (key, segments) in [
        ("a", vec![InstanceSegment::Parent]),
        ("a", Vec::new()),
        (
            "a/b/c",
            vec![InstanceSegment::Parent, InstanceSegment::Parent],
        ),
        (
            "a",
            vec![
                InstanceSegment::Child("b".to_owned()),
                InstanceSegment::Child("c".to_owned()),
            ],
        ),
        ("a", vec![InstanceSegment::Service("X".to_owned())]),
    ] {
        let _ = resolve_instance_path(key, &segments);
    }
}

#[test]
fn multi_module_resolves_identifier_occurrences() {
    use r4::nodes::{Expression, LastStatement};
    use r4::semantic::{MultiModuleModel, SymbolRef};

    let mut graph = MultiModuleModel::new();
    let (block, model) = analyze("local x = 1\nreturn x");
    let a = graph.add_module("a", block, model).expect("module a");

    let Some(LastStatement::Return(statement)) = graph.module(a).block.get_last_statement() else {
        panic!("expected a return statement");
    };
    let Some(Expression::Identifier(identifier)) = statement.iter_expressions().next() else {
        panic!("expected an identifier");
    };
    assert!(matches!(
        graph.resolve_identifier(a, identifier),
        Some(SymbolRef::Local { .. })
    ));

    // An unresolved occurrence returns `None`.
    let fresh = r4::nodes::Identifier::new("never");
    assert_eq!(graph.resolve_identifier(a, &fresh), None);
}
