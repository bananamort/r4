//! Tests for alias-chain resolution, escape analysis and table-field dead
//! store removal.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::canonicalize::remove_dead_table_stores;
use r4::semantic::{EscapeAnalysis, SemanticModel, SymbolId};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn symbol_named(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("symbol `{name}` must exist"))
        .id
}

fn local_symbol_named(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name && symbol.kind != r4::semantic::SymbolKind::Global)
        .unwrap_or_else(|| panic!("local symbol `{name}` must exist"))
        .id
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

#[test]
fn immutable_alias_chains_reach_the_global() {
    let (block, model) = analyze("local math = math\nlocal m = math\nlocal n = m\nreturn n.floor");

    let math = symbol_named(&model, "math");
    let local_math = local_symbol_named(&model, "math");
    let m = local_symbol_named(&model, "m");
    let n = local_symbol_named(&model, "n");

    assert_eq!(model.immutable_alias_root(m), Some(math));
    assert_eq!(model.immutable_alias_root(n), Some(math));
    assert_eq!(model.immutable_global_alias(m), Some(math));
    assert_eq!(model.immutable_global_alias(math), None);
    assert_eq!(model.immutable_alias_root(local_math), Some(math));
    let _ = block;
}

#[test]
fn mutable_alias_chain_does_not_resolve() {
    let (_block, model) = analyze("local math = math\nmath = other\nreturn math");
    let local_math = local_symbol_named(&model, "math");
    assert_eq!(model.immutable_alias_root(local_math), None);
}

#[test]
fn confined_table_is_detected() {
    let (block, model) = analyze("local t = {}\nt.x = 1\nreturn 0");
    let analysis = EscapeAnalysis::analyze(&block, &model);
    let t = symbol_named(&model, "t");

    assert!(analysis.is_table(t));
    assert!(analysis.is_confined_table(&model, t));
    assert!(!analysis.is_field_read(t, "x"));
}

#[test]
fn escaping_occurrences_defeat_confinement() {
    for code in [
        "local t = {}\nt.x = 1\nprint(t)",
        "local t = {}\nt.x = 1\nreturn t",
        "local t = {}\nt.x = 1\nlocal u = t",
        "local t = {}\nt:x()",
        "local t = {}\nt.x = 1\nt = {}",
        "local t = {}\nt.x = 1\nlocal function f() return t end",
        "local t = {}\nt.x = 1\nlocal u = { t }",
    ] {
        let (block, model) = analyze(code);
        let analysis = EscapeAnalysis::analyze(&block, &model);
        let t = symbol_named(&model, "t");
        assert!(
            !analysis.is_confined_table(&model, t),
            "`{code}` must not be confined"
        );
    }
}

#[test]
fn dynamic_reads_are_recorded() {
    let (block, model) = analyze("local t = {}\nt.x = 1\nlocal v = t[k]\nreturn v");
    let analysis = EscapeAnalysis::analyze(&block, &model);
    let t = symbol_named(&model, "t");

    assert!(analysis.has_dynamic_read(t));
    assert!(!analysis.is_field_read(t, "x"));
}

#[test]
fn removes_never_read_field_stores() {
    let (mut block, model) = analyze("local t = {}\nt.x = 1\nt.y = 2\nreturn t.y");
    assert_eq!(remove_dead_table_stores(&mut block, &model), 1);

    let output = generate(&block);
    assert!(!output.contains("t.x"), "{output}");
    assert!(output.contains("t.y = 2"), "{output}");
}

#[test]
fn keeps_stores_that_are_read_or_effectful() {
    let cases = [
        "local t = {}\nt.x = 1\nreturn t.x",
        "local t = {}\nt.x = f()\nreturn 0",
        "local t = {}\nt.x = 1\nprint(t)",
        "local t = {}\nt.x = 1\nlocal v = t[k]\nreturn v",
    ];

    for code in cases {
        let (mut block, model) = analyze(code);
        let before = generate(&block);
        assert_eq!(
            remove_dead_table_stores(&mut block, &model),
            0,
            "`{code}` must not be optimized"
        );
        assert_eq!(generate(&block), before);
    }
}

#[test]
fn removes_string_indexed_dead_store() {
    let (mut block, model) = analyze("local t = {}\nt[\"x\"] = 1\nreturn 0");
    assert_eq!(remove_dead_table_stores(&mut block, &model), 1);
}

#[test]
fn nested_field_read_is_recorded() {
    let (block, model) = analyze("local t = {}\nt.x = 1\nreturn t.x.y");
    let analysis = EscapeAnalysis::analyze(&block, &model);
    let t = symbol_named(&model, "t");

    // `t.x` is read as the base of `.y`, so the store is live.
    assert!(analysis.is_field_read(t, "x"));
    let mut block = block;
    assert_eq!(remove_dead_table_stores(&mut block, &model), 0);
}

#[test]
fn escapes_via_every_control_flow_statement() {
    for code in [
        "local t = {}\nfor k, v in pairs(t) do end",
        "local t = {}\nfor i = 1, 2 do sink(t) end",
        "local t = {}\nif c then sink(t) end",
        "local t = {}\nwhile c do sink(t) end",
        "local t = {}\nrepeat sink(t) until c",
        "local t = {}\ndo sink(t) end",
        "local t = {}\nlocal n = 0\nn += 1\nsink(t)",
    ] {
        let (block, model) = analyze(code);
        let analysis = EscapeAnalysis::analyze(&block, &model);
        let t = symbol_named(&model, "t");
        assert!(analysis.escapes(t), "`{code}` must escape");
        assert!(
            !analysis.is_confined_table(&model, t),
            "`{code}` must not be confined"
        );
    }
}

#[test]
fn tables_stay_confined_across_every_control_flow_statement() {
    for code in [
        "local t = {}\nt.x = 1\nfor k, v in pairs(other) do t.y = k end\nreturn 0",
        "local t = {}\nt.x = 1\nfor i = 1, 2 do t.y = i end\nreturn 0",
        "local t = {}\nt.x = 1\nif c then t.y = 1 end\nreturn 0",
        "local t = {}\nt.x = 1\nwhile c do t.y = 1 end\nreturn 0",
        "local t = {}\nt.x = 1\nrepeat t.y = 1 until c\nreturn 0",
        "local t = {}\nt.x = 1\ndo t.y = 1 end\nreturn 0",
        "local t = {}\nt.x = 1\nlocal n = 0\nn += 1\nreturn 0",
    ] {
        let (block, model) = analyze(code);
        let analysis = EscapeAnalysis::analyze(&block, &model);
        let t = symbol_named(&model, "t");
        assert!(analysis.is_table(t), "`{code}` must be a table");
        assert!(
            analysis.is_confined_table(&model, t),
            "`{code}` must stay confined"
        );
    }
}

#[test]
fn escape_analysis_covers_every_prefix_and_variable_shape() {
    for code in [
        // Compound assignment reads and writes the index.
        "local t = {}\nt[1] += 1\nreturn 0",
        "local t = {}\nt[k] += 1\nreturn 0",
        "local t = {}\nt.x += 1\nreturn 0",
        // Parenthesized bases of a field/index access.
        "local t = {}\nreturn (t).x",
        "local t = {}\nreturn (t.x).y",
        "local t = {}\nreturn (t[1]).y",
        "local t = {}\nreturn (f()).y",
        "local t = {}\nreturn (t + 1).y",
        // Method calls whose prefix is not a plain identifier.
        "local t = {}\nt:m()",
        "local t = {}\n(t):m()",
        "local t = {}\nf():m()",
        "local t = {}\nt[1]:m()",
        // A local copy escapes, and a parenthesized callee is direct.
        "local t = {}\nlocal u = t\nreturn 0",
        "local f = function() end\n(f)()",
        // Type-instantiation prefixes.
        "local t = {}\nt<<T>>.x = 1\nreturn 0",
    ] {
        let (block, model) = analyze(code);
        let analysis = EscapeAnalysis::analyze(&block, &model);
        if let Some(t) = model.symbols.iter().find(|symbol| symbol.name == "t") {
            let _ = analysis.is_table(t.id);
            let _ = analysis.is_confined_table(&model, t.id);
            let _ = analysis.escapes(t.id);
            let _ = analysis.has_dynamic_read(t.id);
            let _ = analysis.has_dynamic_write(t.id);
            let _ = analysis.is_field_read(t.id, "x");
            let _ = analysis.info(t.id);
        }
    }
}

#[test]
fn escape_analysis_covers_index_and_parenthesized_prefixes() {
    for code in [
        "local t = {}\nt[t2] += 1\nreturn 0",
        "local t = {}\nt['x'] += 1\nreturn 0",
        "local t = {}\nt.a[b].c = 1\nreturn 0",
        "local t = {}\nf((t))[1] = 2\nreturn 0",
        "local t = {}\n(f)().x = 1\nreturn 0",
        "local t = {}\nt<<T>>()\nreturn 0",
        "local t = {}\nreturn (t)[k]",
    ] {
        let (block, model) = analyze(code);
        let analysis = EscapeAnalysis::analyze(&block, &model);
        if let Some(t) = model.symbols.iter().find(|symbol| symbol.name == "t") {
            let _ = analysis.has_dynamic_read(t.id);
            let _ = analysis.has_dynamic_write(t.id);
            let _ = analysis.is_field_read(t.id, "x");
            let _ = analysis.escapes(t.id);
            let _ = analysis.info(t.id);
        }
    }
}
