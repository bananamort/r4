//! Output-parity tests for the semantic upgrade of the standard-library passes.
//!
//! For programs without shadowing, the model-aware variants must produce
//! exactly the same output as the legacy name-based variants (parity). For
//! programs with read-only aliases (`local math = math`), the model-aware
//! variants may produce strictly better output; those cases are checked for
//! semantic equivalence by re-evaluating the transformed code.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, LastStatement};
use r4::process::{Evaluator, LuaValue};
use r4::rules::{canonicalize, desugar, rename_variables, Session};
use r4::semantic::{SemanticAnalyzer, SemanticModel, SymbolKind};
use r4::{AnalyzedParseError, Parser};

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn parse_tokens(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .preserve_tokens()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to parse and analyze `{code}`: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn evaluate_first_return(output: &str) -> LuaValue {
    let block = parse(output);
    let Some(LastStatement::Return(return_statement)) = block.get_last_statement() else {
        panic!("expected a return statement in:\n{output}");
    };
    let expression = return_statement
        .iter_expressions()
        .next()
        .expect("the return statement has a value");
    Evaluator::default().evaluate(expression)
}

type LegacyPass = fn(&mut Block);
type ModelPass = fn(&mut Block, &SemanticModel);

fn assert_parity(code: &str, legacy: LegacyPass, modern: ModelPass) {
    let (block, model) = parse_tokens(code);

    let mut legacy_block = block.clone();
    legacy(&mut legacy_block);

    let mut modern_block = block;
    modern(&mut modern_block, &model);

    assert_eq!(
        generate(&legacy_block),
        generate(&modern_block),
        "legacy and model-aware outputs must match for:\n{code}"
    );
}

#[test]
fn sqrt_pass_is_parity_without_shadowing() {
    for code in [
        "return math.sqrt(4)",
        "local x = math.sqrt(value)\nreturn x",
        "math.sqrt(compute())",
        "return math.sqrt(a) + math.sqrt(b)",
    ] {
        assert_parity(
            code,
            canonicalize::convert_square_root_call,
            canonicalize::convert_square_root_call_with_model,
        );
    }
}

#[test]
fn floor_pass_is_parity_without_shadowing() {
    for code in [
        "return value // 2",
        "local x = 1\nx //= 2\nreturn x",
        "local a = 1\nlocal b = a // 2\nreturn b",
        "return (a + b) // (c - d)",
    ] {
        assert_parity(
            code,
            desugar::remove_floor_division,
            desugar::remove_floor_division_with_model,
        );
    }
}

#[test]
fn interpolated_string_pass_is_parity_without_shadowing() {
    for code in [
        "return `hello`",
        "return `value = {value}`",
        "local name = 'x'\nreturn `hello {name}!`",
        "return `a {1} b {2} c`",
    ] {
        assert_parity(
            code,
            desugar::remove_interpolated_string,
            desugar::remove_interpolated_string_with_model,
        );
    }
}

#[test]
fn mutable_shadows_stay_conservative_in_both_variants() {
    for code in [
        "local math = {}\nreturn math.sqrt(4)",
        "local math = make_math()\nreturn math.sqrt(4)",
        "local math = math\nmath = {}\nreturn math.sqrt(4)",
        "local math = {}\nreturn value // 2",
        "local tostring = function(x) return x end\nreturn `{value}`",
    ] {
        assert_parity(
            code,
            canonicalize::convert_square_root_call,
            canonicalize::convert_square_root_call_with_model,
        );
    }
}

#[test]
fn sqrt_optimizes_through_immutable_alias() {
    let code = "local math = math\nreturn math.sqrt(16)";
    let (block, model) = parse_tokens(code);

    let mut legacy_block = block.clone();
    canonicalize::convert_square_root_call(&mut legacy_block);
    let legacy_output = generate(&legacy_block);
    assert!(
        legacy_output.contains("math.sqrt"),
        "legacy variant must refuse the optimization: {legacy_output}"
    );

    let mut modern_block = block;
    canonicalize::convert_square_root_call_with_model(&mut modern_block, &model);
    let modern_output = generate(&modern_block);
    assert!(
        modern_output.contains("16 ^ 0.5"),
        "model variant must optimize through the alias: {modern_output}"
    );
    assert_eq!(
        evaluate_first_return(&modern_output),
        LuaValue::Number(4.0),
        "the optimized call must keep its value"
    );
}

#[test]
fn floor_optimizes_through_immutable_alias() {
    let code = "local math = math\nreturn value // 2";
    let (block, model) = parse_tokens(code);

    let mut legacy_block = block.clone();
    desugar::remove_floor_division(&mut legacy_block);
    let legacy_output = generate(&legacy_block);
    assert!(
        legacy_output.contains("__R4_MATH_FLOOR"),
        "legacy variant must introduce the alias: {legacy_output}"
    );

    let mut modern_block = block;
    desugar::remove_floor_division_with_model(&mut modern_block, &model);
    let modern_output = generate(&modern_block);
    assert!(
        modern_output.contains("math.floor(value / 2)"),
        "model variant must keep the direct call: {modern_output}"
    );
    assert!(
        !modern_output.contains("__R4_MATH_FLOOR"),
        "no synthetic alias is needed: {modern_output}"
    );
}

#[test]
fn interpolated_string_optimizes_through_immutable_alias() {
    let code = "local tostring = tostring\nreturn `value = {value}`";
    let (block, model) = parse_tokens(code);

    let mut legacy_block = block.clone();
    desugar::remove_interpolated_string(&mut legacy_block);
    let legacy_output = generate(&legacy_block);
    assert!(
        legacy_output.contains("__R4_TO_STR"),
        "legacy variant must introduce the alias: {legacy_output}"
    );

    let mut modern_block = block;
    desugar::remove_interpolated_string_with_model(&mut modern_block, &model);
    let modern_output = generate(&modern_block);
    assert!(
        modern_output.contains("tostring(value)"),
        "model variant must call through the alias directly: {modern_output}"
    );
    assert!(
        !modern_output.contains("__R4_TO_STR"),
        "no synthetic alias is needed: {modern_output}"
    );
}

#[test]
fn group_local_is_parity_without_shadowing() {
    for code in [
        "local a = 1\nlocal b = 2\nreturn a + b",
        "local a = 1\nlocal b = a\nreturn b",
        "local a = 1\nlocal b = 2\nlocal c = a",
    ] {
        assert_parity(
            code,
            canonicalize::group_local,
            canonicalize::group_local_with_model,
        );
    }
}

#[test]
fn model_group_local_merges_past_unrelated_shadowed_reads() {
    let code = r#"
        local value = 1
        local a = 1
        local b = (function()
            local value = 2
            return value
        end)()
        return value + a + b
        "#;

    let (block, model) = parse_tokens(code);

    let mut legacy_block = block.clone();
    canonicalize::group_local(&mut legacy_block);
    let legacy_output = generate(&legacy_block);
    assert!(
        !legacy_output.contains("local value, a, b"),
        "the name-based variant must stay conservative: {legacy_output}"
    );

    let mut modern_block = block;
    canonicalize::group_local_with_model(&mut modern_block, &model);
    let modern_output = generate(&modern_block);
    assert!(
        modern_output.contains("local value, a, b"),
        "the model variant must merge past the unrelated shadow: {modern_output}"
    );

    let reparsed = parse(&modern_output);
    let after_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("merged code must analyze");
    let values: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.name == "value" && symbol.kind == SymbolKind::Local)
        .collect();
    assert_eq!(
        values.len(),
        2,
        "the closure keeps its own local value: {modern_output}"
    );
}

#[test]
fn session_refreshes_model_after_model_less_pass() {
    let mut session = Session::parse(
        r#"
        for i = 1, 10 do
            if i > 5 then
                continue
            end
        end
        "#,
    )
    .expect("session must parse and analyze");

    session.apply(desugar::remove_continue);

    let model = session.model().expect("the model must refresh");
    assert!(
        model
            .symbols
            .iter()
            .any(|symbol| symbol.name.starts_with("__R4_CONTINUE")),
        "the refreshed model must see the generated flag"
    );
}

#[test]
fn session_composes_semantic_and_model_less_passes() {
    let mut session = Session::parse(
        r#"
        local function compute() return 1 end
        local function live() end
        local function dead() end
        local unused = compute()
        local enabled = false
        if enabled then
            dead()
        else
            live()
        end
        return 1
        "#,
    )
    .expect("session must parse and analyze");

    let removed_unused = session
        .apply_semantic_result(canonicalize::remove_unused_variables)
        .expect("unused variable removal must succeed");
    assert!(removed_unused >= 1);

    let removed_unreachable = session
        .apply_semantic_result(canonicalize::remove_unreachable_code)
        .expect("unreachable code removal must succeed");
    assert!(removed_unreachable >= 1);

    fn has_call(output: &str, name: &str) -> bool {
        let call = format!("{}()", name);
        output.lines().any(|line| line.trim() == call)
    }

    let intermediate = generate(session.block());
    assert!(
        !has_call(&intermediate, "dead"),
        "unexpected: {intermediate}"
    );
    assert!(
        !intermediate.contains("local unused"),
        "unexpected: {intermediate}"
    );
    assert!(
        has_call(&intermediate, "compute"),
        "unexpected: {intermediate}"
    );
    assert!(
        has_call(&intermediate, "live"),
        "unexpected: {intermediate}"
    );

    let renamed = session
        .apply_semantic_result(rename_variables::rename_variables)
        .expect("renaming must succeed");
    assert!(renamed >= 1);

    let output = generate(session.block());
    assert!(
        !output.contains("dead") && !output.contains("unused") && !output.contains("live"),
        "renaming must remove the original names: {output}"
    );

    let reparsed = parse(&output);
    SemanticAnalyzer::new()
        .strict_globals()
        .analyze(&reparsed)
        .unwrap_or_else(|error| panic!("composed output must analyze strictly: {error}\n{output}"));
}

#[test]
fn parse_analyzed_returns_a_bound_model() {
    let (block, model) = Parser::default()
        .preserve_tokens()
        .parse_analyzed("local x = 1\nreturn x")
        .expect("parse and analyze must succeed");

    assert!(block.first_statement().is_some());
    let local = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "x")
        .expect("the local symbol must exist");
    assert_eq!(local.read_count, 1);
}

#[test]
fn parse_analyzed_reports_semantic_errors() {
    let error = Parser::default()
        .lua52()
        .preserve_tokens()
        .parse_analyzed("goto missing")
        .expect_err("an undefined label must be reported");

    assert!(matches!(error, AnalyzedParseError::Semantic(_)));
}
