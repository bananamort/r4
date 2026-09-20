//! Tests for the programmatic (Loretta/Roslyn-style) API surface:
//! semantic constant injection, path aliases, exported-type hoisting and
//! generator-level minification.

use std::collections::HashMap;

use r4::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::process::LuaValue;
use r4::rules::require::{apply_path_aliases, bundle_modules, AliasError, MemoryModuleProvider};
use r4::rules::{inject_global_values, InjectGlobalValueError, Session};
use r4::semantic::{SemanticAnalyzer, SemanticModel};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn values(entries: &[(&str, LuaValue)]) -> HashMap<String, LuaValue> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn session_generates_readable_and_minified_code() {
    let session = Session::parse("local a = 1\nlocal b = 2\nreturn a + b").expect("session");

    let readable = session.generate();
    assert!(readable.contains('\n'), "readable output: {readable}");

    let minified = session.generate_minified();
    assert!(!minified.contains('\n'), "minified output: {minified}");
    assert!(!minified.contains("--"), "minified output: {minified}");
    assert!(minified.len() <= readable.len());

    let dense = session.generate_with(DenseLuaGenerator::default());
    assert_eq!(minified, dense);

    Parser::default()
        .parse(&readable)
        .expect("readable reparses");
    Parser::default()
        .parse(&minified)
        .expect("minified reparses");
}

#[test]
fn injects_global_constants_through_the_model() {
    let (mut block, model) = analyze("local x = MATH_PI\nreturn MATH_PI + offset");

    let replaced = inject_global_values(
        &mut block,
        &model,
        &values(&[("MATH_PI", LuaValue::Number(3.5))]),
    )
    .expect("injection must succeed");

    assert_eq!(replaced, 2);
    let output = generate(&block);
    assert_eq!(output.trim(), "local x = 3.5\n\nreturn 3.5 + offset");
}

#[test]
fn injection_leaves_shadowed_names_and_writes_untouched() {
    for code in [
        "local MATH_PI = 1\nreturn MATH_PI",
        "local function f(MATH_PI) return MATH_PI end",
        "MATH_PI = 1\nreturn MATH_PI",
    ] {
        let (mut block, model) = analyze(code);
        let replaced = inject_global_values(
            &mut block,
            &model,
            &values(&[("MATH_PI", LuaValue::Number(3.5))]),
        )
        .expect("injection must succeed");
        assert_eq!(replaced, 0, "must not inject for `{code}`");
    }
}

#[test]
fn injection_uses_immutable_aliases() {
    let (mut block, model) = analyze("local MATH_PI = MATH_PI\nreturn MATH_PI");

    let replaced = inject_global_values(
        &mut block,
        &model,
        &values(&[("MATH_PI", LuaValue::Number(3.5))]),
    )
    .expect("injection must succeed");

    assert_eq!(replaced, 2);
    assert!(generate(&block).contains("3.5"));
}

#[test]
fn injection_rejects_unrepresentable_values() {
    let (mut block, model) = analyze("return GLOBAL_VALUE");

    for value in [
        LuaValue::Table,
        LuaValue::Function,
        LuaValue::Unknown,
        LuaValue::Number(f64::INFINITY),
    ] {
        let error = inject_global_values(&mut block, &model, &values(&[("GLOBAL_VALUE", value)]))
            .expect_err("an unrepresentable value must be rejected");
        assert!(matches!(
            error,
            InjectGlobalValueError::UnrepresentableValue { .. }
        ));
    }
}

#[test]
fn aliases_rewrite_require_paths() {
    let (mut block, model) = analyze("return require('@pkg/foo')");

    let replaced = apply_path_aliases(&mut block, &model, [("@pkg", "./vendor/pkg")])
        .expect("alias conversion must succeed");

    assert_eq!(replaced, 1);
    assert_eq!(
        generate(&block).trim(),
        "return require('./vendor/pkg/foo')"
    );
}

#[test]
fn aliases_use_the_longest_prefix() {
    let converter = r4::rules::require::AliasRequireConverter::new([
        ("@pkg", "./vendor/pkg"),
        ("@pkg/extra", "./special"),
    ])
    .expect("aliases build");

    assert_eq!(
        converter.resolve("@pkg/foo"),
        Some("./vendor/pkg/foo".to_owned())
    );
    assert_eq!(
        converter.resolve("@pkg/extra/bar"),
        Some("./special/bar".to_owned())
    );
}

#[test]
fn aliases_reject_malformed_and_recursive_mappings() {
    assert!(matches!(
        r4::rules::require::AliasRequireConverter::new([("pkg", "./x")]),
        Err(AliasError::MalformedAlias { .. })
    ));
    assert!(matches!(
        r4::rules::require::AliasRequireConverter::new([("@pkg", "")]),
        Err(AliasError::MalformedAlias { .. })
    ));
    assert!(matches!(
        r4::rules::require::AliasRequireConverter::new([("@a", "@b")]),
        Err(AliasError::NestedAlias { .. })
    ));
    assert!(matches!(
        r4::rules::require::AliasRequireConverter::new([("@a", "./x"), ("@a", "./y")]),
        Err(AliasError::DuplicateAlias { .. })
    ));
}

#[test]
fn aliases_report_unresolved_aliased_requires() {
    let (mut block, model) = analyze("return require('@nope/x')");

    let error = apply_path_aliases(&mut block, &model, [("@pkg", "./vendor/pkg")])
        .expect_err("an unresolved alias must be reported");

    assert!(matches!(error, AliasError::UnresolvedAlias { .. }));
}

#[test]
fn aliases_ignore_shadowed_require() {
    let code = "local require = function(path) return path end\nreturn require('@pkg/x')";
    let (mut block, model) = analyze(code);

    let replaced = apply_path_aliases(&mut block, &model, [("@pkg", "./vendor/pkg")])
        .expect("a shadowed require is not an alias target");

    assert_eq!(replaced, 0);
    assert!(generate(&block).contains("@pkg/x"));
}

#[test]
fn bundles_hoist_exported_types() {
    let (mut block, model) = analyze("return require('./a')");
    let mut provider = MemoryModuleProvider::new();
    provider.insert(
        "a",
        "export type Point = { x: number, y: number }\nreturn 1",
    );

    let bundled = bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
    assert_eq!(bundled, 1);

    let output = generate(&block);
    assert!(output.contains("type __R4_TYPE_Point"), "{output}");
    assert!(!output.contains("export type"), "{output}");

    let reparsed = Parser::default().parse(&output).expect("output reparses");
    SemanticAnalyzer::new()
        .strict_types()
        .analyze(&reparsed)
        .expect("hoisted types must resolve strictly");
}

#[test]
fn bundles_hoist_type_dependencies_and_retarget_cross_module_references() {
    let (mut block, model) = analyze("return require('./b')");
    let mut provider = MemoryModuleProvider::new();
    provider.insert(
        "a",
        "type Helper = number\nexport type Public = Helper\nreturn 1",
    );
    provider.insert(
        "b",
        "local a = require('./a')\ntype Local = Public\nreturn a",
    );

    let bundled = bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
    assert_eq!(bundled, 2);

    let output = generate(&block);
    assert!(output.contains("type __R4_TYPE_Helper"), "{output}");
    assert!(
        output.contains("type __R4_TYPE_Public = __R4_TYPE_Helper"),
        "{output}"
    );
    assert!(
        output.contains("type Local = __R4_TYPE_Public"),
        "the requiring module must reference the hoisted type: {output}"
    );

    let reparsed = Parser::default().parse(&output).expect("output reparses");
    SemanticAnalyzer::new()
        .strict_types()
        .analyze(&reparsed)
        .expect("hoisted cross-module types must resolve strictly");
}

#[test]
fn bundles_reject_duplicate_exported_type_names() {
    let (mut block, model) = analyze("return require('./a') + require('./b')");
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "export type Foo = number\nreturn 1");
    provider.insert("b", "export type Foo = string\nreturn 1");

    let error = bundle_modules(&mut block, &model, &provider)
        .expect_err("duplicate exported types must be rejected");

    assert!(matches!(
        error,
        r4::rules::require::BundleError::DuplicateExportedType { ref name } if name == "Foo"
    ));
}

#[test]
fn round_trips_source_through_the_readable_generator() {
    let source = r#"
        local x = 10 + 20
        local function add(a, b)
            return a + b
        end
        if x > 25 then
            print("x is large:", x)
        else
            print("x is small")
        end
    "#;

    let block = Parser::default()
        .parse(source)
        .expect("the source must parse");
    assert!(block.iter_statements().count() >= 3);

    let output = generate(&block);
    assert!(output.contains("local x = 10 + 20"), "{output}");
    assert!(output.contains("local function add(a, b)"), "{output}");
    assert!(output.contains("if x > 25 then"), "{output}");
}
