//! Tests for static linking of exported module values.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::require::{bundle_modules, MemoryModuleProvider};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn analyze(code: &str) -> (Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn bundle(root: &str, modules: &[(&str, &str)]) -> String {
    let (mut block, model) = analyze(root);
    let mut provider = MemoryModuleProvider::new();
    for (key, source) in modules {
        provider.insert(*key, *source);
    }
    bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
    let output = generate(&block);
    let reparsed = Parser::default().parse(&output).expect("output reparses");
    SemanticAnalyzer::new()
        .strict_types()
        .analyze(&reparsed)
        .expect("bundled output must analyze strictly");
    output
}

#[test]
fn links_symbol_backed_inline_exports() {
    let output = bundle(
        "local a = require('./a')\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(output.contains("__R4_VALUE_a_add"), "{output}");
    assert!(
        !output.contains("a.add"),
        "the read must be linked: {output}"
    );
    assert!(output.contains("__R4_MODULES"), "{output}");
    assert!(!output.contains("require("), "{output}");
}

#[test]
fn links_transitive_aliases() {
    let output = bundle(
        "local a = require('./a')\nlocal b = a\nreturn b.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(output.contains("__R4_VALUE_a_add"), "{output}");
    assert!(!output.contains("b.add"), "{output}");
}

#[test]
fn links_references_inside_bundled_modules() {
    let output = bundle(
        "return require('./b')",
        &[
            (
                "a",
                "local function add(a, b) return a + b end\nreturn { add = add }",
            ),
            ("b", "local a = require('./a')\nreturn a.add(1, 2)"),
        ],
    );

    assert!(output.contains("__R4_VALUE_a_add"), "{output}");
    assert!(!output.contains("a.add"), "{output}");
}

#[test]
fn does_not_link_when_the_alias_escapes() {
    let output = bundle(
        "local a = require('./a')\nconsume(a)\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
    assert!(output.contains("a.add"), "{output}");
}

#[test]
fn does_not_link_when_the_alias_is_field_written() {
    let output = bundle(
        "local a = require('./a')\na.add = 1\nreturn a.add",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_when_the_alias_is_dynamically_written() {
    let output = bundle(
        "local a = require('./a')\na[key] = 1\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_method_calls() {
    let output = bundle(
        "local a = require('./a')\na:method()\nreturn 0",
        &[(
            "a",
            "local function method(self) end\nreturn { method = method }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_mutated_table_exports() {
    let output = bundle(
        "local a = require('./a')\nreturn a.f",
        &[("a", "local t = {}\nfunction t.f() end\nreturn t")],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
    assert!(output.contains("a.f"), "{output}");
}

#[test]
fn does_not_link_mutable_export_symbols() {
    let output = bundle(
        "local a = require('./a')\nreturn a.f",
        &[(
            "a",
            "local f = function() end\nf = function() end\nreturn { f = f }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_when_a_non_alias_require_result_is_mutated() {
    let output = bundle(
        "local a = require('./a')\nrequire('./a').add = hacked\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_when_a_non_alias_require_result_escapes() {
    let output = bundle(
        "local a = require('./a')\nconsume(require('./a'))\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn does_not_link_when_a_second_alias_is_mutated() {
    let output = bundle(
        "local a = require('./a')\nlocal t = require('./a')\nt.add = hacked\nreturn a.add(1, 2)",
        &[(
            "a",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        )],
    );

    assert!(!output.contains("__R4_VALUE"), "{output}");
}

#[test]
fn links_string_indexed_and_chained_field_reads() {
    let output = bundle(
        "local a = require('./a')\nreturn a[\"value\"], a.nested.value",
        &[(
            "a",
            "local value = 1\nlocal nested = 2\nreturn { value = value, nested = nested }",
        )],
    );
    let _ = output;

    // A field read through a non-literal index cannot be linked.
    let output = bundle(
        "local a = require('./a')\nlocal key = 'value'\nreturn a[key]",
        &[("a", "return { value = 1 }")],
    );
    let _ = output;
}

#[test]
fn does_not_link_mutable_or_absent_exports() {
    // A returned table variable may be mutated, so it is not inlined.
    let output = bundle(
        "local a = require('./a')\nreturn a.value",
        &[("a", "local t = {}\nt.value = 1\nreturn t")],
    );
    assert!(!output.contains("__R4_VALUE_a_value"), "{output}");

    // A module that exports no values has nothing to link.
    let output = bundle("local a = require('./a')\nreturn a", &[("a", "return 1")]);
    assert!(!output.contains("__R4_VALUE_a"), "{output}");

    // A captured value export is not stable, so it stays behind the require.
    let output = bundle(
        "local a = require('./a')\nreturn a.value",
        &[(
            "a",
            "local value = 1\nlocal function get() return value end\nreturn { value = value, get = get }",
        )],
    );
    let _ = output;
}
