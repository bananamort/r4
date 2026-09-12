use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, Statement};
use r4::process::{Evaluator, LuaValue};
use r4::rules::{self, canonicalize, desugar, rename_variables};
use r4::semantic::{SemanticAnalyzer, SemanticModel};
use r4::Parser;

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze_with_tokens(code: &str) -> (Block, SemanticModel) {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    (block, model)
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

#[test]
fn strips_luau_types() {
    let mut block = parse("local x: number = 10");

    desugar::remove_types(&mut block);

    assert_eq!(generate(&block).trim(), "local x = 10");
}

#[test]
fn strips_function_types() {
    let mut block = parse("local function add(a: number, b: number): number return a + b end");

    desugar::remove_types(&mut block);

    let output = generate(&block);
    assert!(!output.contains(':'), "types must be removed: {output}");
    assert!(
        output.contains("local function add(a, b)"),
        "unexpected: {output}"
    );
}

#[test]
fn removes_compound_assignments() {
    let mut block = parse("x += 1");

    desugar::remove_compound_assign(&mut block);

    assert_eq!(generate(&block).trim(), "x = x + 1");
}

#[test]
fn removes_compound_assignments_with_index_side_effects_once() {
    let mut block = parse("t[index()] += 1");

    desugar::remove_compound_assign(&mut block);

    let output = generate(&block);
    assert!(output.contains("index()"), "unexpected: {output}");
    assert!(
        output.matches("index()").count() == 1,
        "the index expression must be evaluated only once: {output}"
    );
}

#[test]
fn lowers_if_expressions() {
    let mut block = parse("local x = if true then 1 else 2");

    desugar::remove_if_expression(&mut block);

    let output = generate(&block);
    let generated = parse(&output);

    let Statement::LocalAssign(assign) = generated
        .first_statement()
        .expect("the generated block starts with an assignment")
    else {
        panic!("expected a local assignment, got: {output}");
    };

    let value = assign.last_value().expect("the assignment has a value");
    assert_eq!(Evaluator::default().evaluate(value), LuaValue::Number(1.0));
}

#[test]
fn converts_string_index_to_field() {
    let mut block = parse(r#"return t["foo"]"#);

    canonicalize::convert_index_to_field(&mut block);

    assert_eq!(generate(&block).trim(), "return t.foo");
}

#[test]
fn converts_index_table_entries_to_fields() {
    let mut block = parse(r#"return { ["key"] = 1, value = 2 }"#);

    canonicalize::convert_index_to_field(&mut block);

    let output = generate(&block);
    assert!(output.contains("key = 1"), "unexpected: {output}");
}

#[test]
fn desugars_continue_statements() {
    let mut block = parse(
        r#"
        for i = 1, 10 do
            if i > 5 then
                continue
            end
            print(i)
        end
        "#,
    );

    desugar::remove_continue(&mut block);

    let output = generate(&block);
    assert!(!output.contains("continue"), "unexpected: {output}");
    parse(&output);
}

#[test]
fn converts_floor_division() {
    let mut block = parse("return value // 2");

    desugar::remove_floor_division(&mut block);

    assert_eq!(generate(&block).trim(), "return math.floor(value / 2)");
}

#[test]
fn converts_compound_floor_division() {
    let mut block = parse("value //= 2");

    desugar::remove_floor_division(&mut block);

    assert_eq!(generate(&block).trim(), "value = math.floor(value / 2)");
}

#[test]
fn removes_method_calls() {
    let mut block = parse("obj:method(1)");

    desugar::remove_method_call(&mut block);

    assert_eq!(generate(&block).trim(), "obj.method(obj, 1)");
}

#[test]
fn removes_trailing_nil_declarations() {
    let mut block = parse("local x, y = 1, nil");

    canonicalize::remove_nil_declarations(&mut block);

    assert_eq!(generate(&block).trim(), "local x, y = 1");
}

#[test]
fn removes_empty_do_statements() {
    let mut block = parse("do do end end");

    canonicalize::remove_empty_do(&mut block);

    assert!(generate(&block).trim().is_empty());
}

#[test]
fn removes_call_parentheses() {
    let mut block = parse(r#"f("hello")"#);

    canonicalize::remove_call_parens(&mut block);

    assert_eq!(generate(&block).trim(), "f'hello'");
}

#[test]
fn groups_local_assignments() {
    let mut block = parse(
        r#"
        local a = 1
        local b = 2
        "#,
    );

    canonicalize::group_local(&mut block);

    assert_eq!(generate(&block).trim(), "local a, b = 1, 2");
}

#[test]
fn desugar_pipeline_lowers_a_luau_function() {
    let mut block = parse(
        r#"
        local function clamp(value: number, minimum: number, maximum: number): number
            if value < minimum then
                return minimum
            end
            return if value > maximum then maximum else value
        end
        local total: number = 0
        for i = 1, 10 do
            if i % 2 == 0 then
                continue
            end
            total += i
        end
        return clamp(total // 2, 0, 100)
        "#,
    );

    rules::desugar_to_lua51(&mut block);
    rules::canonicalize(&mut block);

    let output = generate(&block);
    assert!(!output.contains("continue"), "unexpected: {output}");
    assert!(!output.contains("//"), "unexpected: {output}");
    assert!(output.contains("math.floor"), "unexpected: {output}");

    parse(&output);
}

#[test]
fn removes_unused_local_assignments() {
    let (mut block, model) = analyze_with_tokens("local unused = 1\nreturn 2");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");

    assert_eq!(removed, 1);
    assert_eq!(generate(&block).trim(), "return 2");
}

#[test]
fn preserves_side_effects_of_unused_initializers() {
    let (mut block, model) = analyze_with_tokens("local unused = compute()\nreturn 2");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");

    assert_eq!(removed, 1);
    assert_eq!(generate(&block).trim(), "compute()\n\nreturn 2");
}

#[test]
fn keeps_used_locals() {
    let (mut block, model) = analyze_with_tokens("local used = 1\nreturn used");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");

    assert_eq!(removed, 0);
    assert_eq!(generate(&block).trim(), "local used = 1\n\nreturn used");
}

#[test]
fn removes_unused_local_functions() {
    let (mut block, model) = analyze_with_tokens("local function unused() return 1 end\nreturn 2");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");

    assert_eq!(removed, 1);
    assert_eq!(generate(&block).trim(), "return 2");
}

#[test]
fn partially_used_multi_assignments_are_kept() {
    let (mut block, model) = analyze_with_tokens("local a, b = 1, 2\nreturn b");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");

    assert_eq!(removed, 0);
}

#[test]
fn unused_variable_pass_requires_token_data() {
    let mut block = parse("local unused = 1\nreturn 2");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let error = canonicalize::remove_unused_variables(&mut block, &model)
        .expect_err("token-less declarations must be rejected explicitly");
    assert!(matches!(
        error,
        canonicalize::UnusedVariableError::MissingTokenData { .. }
    ));

    // The block must be left untouched when validation fails.
    assert_eq!(generate(&block).trim(), "local unused = 1\n\nreturn 2");
}

#[test]
fn host_globals_are_exposed() {
    use rename_variables::{DEFAULT_GLOBALS, ROBLOX_GLOBALS};

    assert_eq!(DEFAULT_GLOBALS.len(), 40);
    assert_eq!(ROBLOX_GLOBALS.len(), 58);

    assert!(DEFAULT_GLOBALS.contains(&"print"));
    assert!(DEFAULT_GLOBALS.contains(&"_VERSION"));
    assert!(ROBLOX_GLOBALS.contains(&"game"));
    assert!(ROBLOX_GLOBALS.contains(&"workspace"));
}
