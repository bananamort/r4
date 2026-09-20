//! Tests for the standard-library purity model and its effect on passes.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, Expression, LastStatement};
use r4::rules::canonicalize::remove_unused_variables;
use r4::semantic::{call_is_pure, SemanticModel};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn returned_call(block: &Block) -> &r4::nodes::FunctionCall {
    let Some(LastStatement::Return(statement)) = block.get_last_statement() else {
        panic!("expected a return statement");
    };
    match statement.iter_expressions().next() {
        Some(Expression::Call(call)) => call,
        other => panic!("expected a call, got {other:?}"),
    }
}

#[test]
fn pure_stdlib_calls_are_recognized() {
    for code in [
        "return math.floor(x)",
        "return string.len(s)",
        "return table.concat(t)",
        "return utf8.len(s)",
        "return bit32.band(a, b)",
        "return type(x)",
        "return tostring(x)",
    ] {
        let (block, model) = analyze(code);
        assert!(
            call_is_pure(&model, returned_call(&block)),
            "{code} must be pure"
        );
    }
}

#[test]
fn impure_or_unknown_calls_are_not_pure() {
    for code in [
        "return math.random()",
        "return math.randomseed(1)",
        "return print(x)",
        "return require('./x')",
        "return os.execute('ls')",
        "return table.insert(t, 1)",
        "return string.gsub(s, 'a', f)",
        "return unknown.helper(x)",
        "return x:method()",
        "return setmetatable(t, m)",
    ] {
        let (block, model) = analyze(code);
        assert!(
            !call_is_pure(&model, returned_call(&block)),
            "{code} must not be pure"
        );
    }
}

#[test]
fn aliases_of_stdlib_globals_are_pure() {
    let (block, model) = analyze("local m = math\nreturn m.floor(x)");
    assert!(call_is_pure(&model, returned_call(&block)));
}

#[test]
fn shadowed_stdlib_globals_are_not_pure() {
    let (block, model) = analyze("local math = {}\nreturn math.floor(x)");
    assert!(!call_is_pure(&model, returned_call(&block)));

    let (block, model) = analyze("local math = math\nmath = {}\nreturn math.floor(x)");
    assert!(!call_is_pure(&model, returned_call(&block)));
}

fn remove_unused(code: &str) -> String {
    let (mut block, model) = analyze(code);
    remove_unused_variables(&mut block, &model).expect("the pass succeeds");
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    generator.into_string()
}

#[test]
fn dead_pure_stdlib_results_are_removed() {
    let output = remove_unused("local x = math.floor(1.5)\nreturn 0");
    assert!(!output.contains("math.floor"), "{output}");
}

#[test]
fn dead_impure_stdlib_results_are_kept() {
    let output = remove_unused("local x = math.random()\nreturn 0");
    assert!(output.contains("math.random"), "{output}");

    let output = remove_unused("local x = print(1)\nreturn 0");
    assert!(output.contains("print"), "{output}");
}

#[test]
fn dead_shadowed_stdlib_results_are_kept() {
    let output = remove_unused("local math = {}\nlocal x = math.floor(1.5)\nreturn 0");
    assert!(output.contains("math.floor"), "{output}");
}

#[test]
fn dead_aliased_stdlib_results_are_removed() {
    let output = remove_unused("local m = math\nlocal x = m.floor(1.5)\nreturn 0");
    assert!(!output.contains("m.floor"), "{output}");
}

#[test]
fn classifies_pure_and_impure_calls() {
    use r4::process::Evaluator;
    use r4::semantic::has_side_effects;

    for (code, pure) in [
        ("return math.sqrt(16)", true),
        ("return string.format('%d', 1)", true),
        ("return table.concat({})", true),
        ("return bit32.band(1, 2)", true),
        ("return print(1)", false),
        ("return obj:method()", false),
        ("return unknown(1)", false),
        ("return (math).sqrt(1)", false),
    ] {
        let (block, model) = analyze(code);
        assert_eq!(
            call_is_pure(&model, returned_call(&block)),
            pure,
            "`{code}`"
        );
    }

    let evaluator = Evaluator::default();
    let (block, model) = analyze("return math.sqrt(16)");
    let pure = Expression::from(returned_call(&block).clone());
    assert!(!has_side_effects(&model, &evaluator, &pure));

    let (block, model) = analyze("return print(1)");
    let impure = Expression::from(returned_call(&block).clone());
    assert!(has_side_effects(&model, &evaluator, &impure));
}
