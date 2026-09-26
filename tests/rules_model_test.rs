//! Model-mandatory tests for the semantic transformation passes.
//!
//! `convert_square_root_call`, `group_local`, `remove_floor_division` and
//! `remove_interpolated_string` no longer have name-based entry points: every
//! call takes a `&SemanticModel`, resolves identifiers structurally, and
//! allocates any synthesized identifier through `NameGenerator`.

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

/// Parses and analyzes without token data: the passes must not need tokens.
fn parse_tokenless(code: &str) -> (Block, SemanticModel) {
    Parser::default()
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

#[test]
fn sqrt_pass_converts_without_shadowing() {
    for code in [
        "return math.sqrt(4)",
        "local x = math.sqrt(value)\nreturn x",
        "return math.sqrt(a) + math.sqrt(b)",
    ] {
        let (mut block, model) = parse_tokenless(code);
        canonicalize::convert_square_root_call(&mut block, &model);
        let output = generate(&block);
        assert!(
            !output.contains("math.sqrt"),
            "the call must be converted for `{code}`: {output}"
        );
        assert!(
            output.contains("^ 0.5"),
            "the conversion must use exponentiation for `{code}`: {output}"
        );
        parse(&output);
    }
}

#[test]
fn sqrt_statement_preserves_argument_side_effects() {
    let (mut block, model) = parse_tokenless("math.sqrt(compute())");
    canonicalize::convert_square_root_call(&mut block, &model);

    let output = generate(&block);
    assert!(!output.contains("math.sqrt"), "unexpected: {output}");
    assert!(output.contains("compute()"), "unexpected: {output}");
}

#[test]
fn mutable_shadowing_prevents_conversion() {
    for code in [
        "local math = {}\nreturn math.sqrt(4)",
        "local math = make_math()\nreturn math.sqrt(4)",
        "local math = math\nmath = {}\nreturn math.sqrt(4)",
    ] {
        let (mut block, model) = parse_tokenless(code);
        canonicalize::convert_square_root_call(&mut block, &model);
        let output = generate(&block);
        assert!(
            output.contains("math.sqrt"),
            "a mutable or non-alias shadow must prevent conversion for `{code}`: {output}"
        );
    }
}

#[test]
fn sqrt_optimizes_through_immutable_alias() {
    let (mut block, model) = parse_tokens("local math = math\nreturn math.sqrt(16)");
    canonicalize::convert_square_root_call(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("16 ^ 0.5"),
        "the alias is read-only, so the optimization is proven safe: {output}"
    );
    assert_eq!(
        evaluate_first_return(&output),
        LuaValue::Number(4.0),
        "the optimized call must keep its value"
    );
}

#[test]
fn floor_pass_rewrites_and_keeps_direct_math_access() {
    let (mut block, model) = parse_tokenless("local math = math\nreturn value // 2");
    desugar::remove_floor_division(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("math.floor(value / 2)"),
        "the read-only alias must be transparent: {output}"
    );
    assert!(
        !output.contains("__R4_MATH_FLOOR"),
        "no synthetic alias is needed: {output}"
    );
}

#[test]
fn floor_pass_aliases_mutable_shadow_with_fresh_name() {
    let (mut block, model) = parse_tokenless("local math = {}\nreturn value // 2");
    desugar::remove_floor_division(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("local __R4_MATH_FLOOR = math.floor"),
        "a mutable shadow needs a fresh alias: {output}"
    );
    assert!(
        output.contains("__R4_MATH_FLOOR(value / 2)"),
        "uses must be routed through the alias: {output}"
    );
}

#[test]
fn floor_pass_avoids_taken_alias_names() {
    let code = "local math = {}\nlocal __R4_MATH_FLOOR = 1\nreturn value // 2";
    let (mut block, model) = parse_tokenless(code);
    desugar::remove_floor_division(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("local __R4_MATH_FLOOR1 = math.floor"),
        "the alias must avoid the user identifier: {output}"
    );
    assert!(
        output.contains("__R4_MATH_FLOOR1(value / 2)"),
        "uses must be routed through the fresh alias: {output}"
    );
}

#[test]
fn interpolated_string_optimizes_through_immutable_alias() {
    let (mut block, model) = parse_tokens("local tostring = tostring\nreturn `value = {value}`");
    desugar::remove_interpolated_string(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("tostring(value)"),
        "the read-only alias must be transparent: {output}"
    );
    assert!(
        !output.contains("__R4_TO_STR"),
        "no synthetic alias is needed: {output}"
    );
}

#[test]
fn interpolated_string_aliases_mutable_shadow_with_fresh_name() {
    let code = "local tostring = function(x) return x end\nreturn `{value}`";
    let (mut block, model) = parse_tokenless(code);
    desugar::remove_interpolated_string(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("local __R4_TO_STR = tostring"),
        "a mutable shadow needs a fresh alias: {output}"
    );
    assert!(
        output.contains("__R4_TO_STR(value)"),
        "uses must be routed through the alias: {output}"
    );
}

#[test]
fn interpolated_string_avoids_taken_alias_names() {
    let code = "local tostring = function(x) return x end\nlocal __R4_TO_STR = 1\nreturn `{value}`";
    let (mut block, model) = parse_tokenless(code);
    desugar::remove_interpolated_string(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("local __R4_TO_STR1 = tostring"),
        "the alias must avoid the user identifier: {output}"
    );
    assert!(
        output.contains("__R4_TO_STR1(value)"),
        "uses must be routed through the fresh alias: {output}"
    );
}

#[test]
fn group_local_merges_plain_assignments() {
    for (code, expected) in [
        (
            "local a = 1\nlocal b = 2\nreturn a + b",
            "local a, b = 1, 2",
        ),
        ("local a = 1\nlocal b = 2\nlocal c = a", "local a, b = 1, 2"),
    ] {
        let (mut block, model) = parse_tokenless(code);
        canonicalize::group_local(&mut block, &model);
        let output = generate(&block);
        assert!(
            output.contains(expected),
            "the locals must be merged for `{code}`: {output}"
        );
    }
}

#[test]
fn group_local_merges_past_unrelated_shadowed_reads() {
    let code = r#"
        local value = 1
        local a = 1
        local b = (function()
            local value = 2
            return value
        end)()
        return value + a + b
        "#;

    let (mut block, model) = parse_tokenless(code);
    canonicalize::group_local(&mut block, &model);

    let output = generate(&block);
    assert!(
        output.contains("local value, a, b"),
        "the unrelated shadow must not block the merge: {output}"
    );

    let reparsed = parse(&output);
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
        "the closure keeps its own local value: {output}"
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

    session
        .apply_semantic_result(desugar::remove_continue)
        .expect("continue lowering must succeed");

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

#[test]
fn session_refreshes_model_after_semantic_pass() {
    let mut session = Session::parse("local unused = 1\nlocal value = 2\nreturn value")
        .expect("session must parse and analyze");

    // Renaming changes identifier names, so the cached model must not be
    // reused by the next semantic pass.
    session
        .apply_semantic_result(rename_variables::rename_variables)
        .expect("renaming must succeed");

    let removed = session
        .apply_semantic_result(canonicalize::remove_unused_variables)
        .expect("unused variable removal must succeed after renaming");
    assert_eq!(removed, 1);

    let output = generate(session.block());
    assert!(!output.contains("unused"), "unexpected: {output}");
}
