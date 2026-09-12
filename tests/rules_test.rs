use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, Expression, Statement};
use r4::process::{Evaluator, LuaValue};
use r4::rules::{self, canonicalize, desugar, rename_variables, require, NameGenerator};
use r4::semantic::{SemanticAnalyzer, SemanticModel, SymbolKind};
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

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to parse and analyze `{code}`: {error}"))
}

fn analyze_lua52_with_tokens(code: &str) -> (Block, SemanticModel) {
    let block = Parser::default()
        .lua52()
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
    let (mut block, model) = analyze("local x: number = 10");

    desugar::remove_types(&mut block, &model);

    assert_eq!(generate(&block).trim(), "local x = 10");
}

#[test]
fn strips_function_types() {
    let (mut block, model) =
        analyze("local function add(a: number, b: number): number return a + b end");

    desugar::remove_types(&mut block, &model);

    let output = generate(&block);
    assert!(!output.contains(':'), "types must be removed: {output}");
    assert!(
        output.contains("local function add(a, b)"),
        "unexpected: {output}"
    );
}

#[test]
fn removes_compound_assignments() {
    let (mut block, model) = analyze("x += 1");

    desugar::remove_compound_assign(&mut block, &model);

    assert_eq!(generate(&block).trim(), "x = x + 1");
}

#[test]
fn removes_compound_assignments_with_index_side_effects_once() {
    let (mut block, model) = analyze("t[index()] += 1");

    desugar::remove_compound_assign(&mut block, &model);

    let output = generate(&block);
    assert!(output.contains("index()"), "unexpected: {output}");
    assert!(
        output.matches("index()").count() == 1,
        "the index expression must be evaluated only once: {output}"
    );
}

#[test]
fn lowers_if_expressions() {
    let (mut block, model) = analyze("local x = if true then 1 else 2");

    desugar::remove_if_expression(&mut block, &model);

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
    let (mut block, model) = analyze(r#"return t["foo"]"#);

    canonicalize::convert_index_to_field(&mut block, &model);

    assert_eq!(generate(&block).trim(), "return t.foo");
}

#[test]
fn converts_index_table_entries_to_fields() {
    let (mut block, model) = analyze(r#"return { ["key"] = 1, value = 2 }"#);

    canonicalize::convert_index_to_field(&mut block, &model);

    let output = generate(&block);
    assert!(output.contains("key = 1"), "unexpected: {output}");
}

#[test]
fn desugars_continue_statements() {
    let (mut block, model) = analyze(
        r#"
        for i = 1, 10 do
            if i > 5 then
                continue
            end
            print(i)
        end
        "#,
    );

    desugar::remove_continue(&mut block, &model);

    let output = generate(&block);
    assert!(!output.contains("continue"), "unexpected: {output}");
    parse(&output);
}

#[test]
fn continue_lowering_avoids_existing_identifiers() {
    let (mut block, model) = analyze(
        r#"
        for i = 1, 10 do
            if i > 5 then
                continue
            end
            local __R4_CONTINUE1 = 1
            print(__R4_CONTINUE1)
        end
        "#,
    );

    desugar::remove_continue(&mut block, &model);

    let output = generate(&block);
    assert!(!output.contains("continue"), "unexpected: {output}");
    assert!(
        output.contains("local __R4_CONTINUE1 = 1"),
        "the user local must keep its name: {output}"
    );
    assert!(
        output.contains("local __R4_CONTINUE ="),
        "the generated flag must avoid the user identifier: {output}"
    );
    parse(&output);
}

#[test]
fn converts_floor_division() {
    let (mut block, model) = analyze("return value // 2");

    desugar::remove_floor_division(&mut block, &model);

    assert_eq!(generate(&block).trim(), "return math.floor(value / 2)");
}

#[test]
fn converts_compound_floor_division() {
    let (mut block, model) = analyze("value //= 2");

    desugar::remove_floor_division(&mut block, &model);

    assert_eq!(generate(&block).trim(), "value = math.floor(value / 2)");
}

#[test]
fn removes_method_calls() {
    let (mut block, model) = analyze("obj:method(1)");

    desugar::remove_method_call(&mut block, &model);

    assert_eq!(generate(&block).trim(), "obj.method(obj, 1)");
}

#[test]
fn removes_trailing_nil_declarations() {
    let (mut block, model) = analyze("local x, y = 1, nil");

    canonicalize::remove_nil_declarations(&mut block, &model);

    assert_eq!(generate(&block).trim(), "local x, y = 1");
}

#[test]
fn removes_empty_do_statements() {
    let (mut block, model) = analyze("do do end end");

    canonicalize::remove_empty_do(&mut block, &model);

    assert!(generate(&block).trim().is_empty());
}

#[test]
fn removes_call_parentheses() {
    let (mut block, model) = analyze(r#"f("hello")"#);

    canonicalize::remove_call_parens(&mut block, &model);

    assert_eq!(generate(&block).trim(), "f'hello'");
}

#[test]
fn groups_local_assignments() {
    let (mut block, model) = analyze(
        r#"
        local a = 1
        local b = 2
        "#,
    );

    canonicalize::group_local(&mut block, &model);

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

    rules::desugar_to_lua51(&mut block).expect("the desugar pipeline must analyze successfully");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("the desugared block must analyze");
    rules::canonicalize(&mut block, &model);

    let output = generate(&block);
    assert!(!output.contains("continue"), "unexpected: {output}");
    assert!(!output.contains("//"), "unexpected: {output}");
    assert!(output.contains("math.floor"), "unexpected: {output}");

    parse(&output);
}

#[test]
fn name_generator_avoids_model_symbols_and_host_globals() {
    let (_, model) =
        analyze_with_tokens("local a = 1\nlocal __R4_SELECT = 2\nreturn a + __R4_SELECT");

    let mut generator = NameGenerator::new(&model);
    let names: Vec<String> = (0..5).map(|_| generator.generate()).collect();

    let unique: std::collections::HashSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "generated names must be unique");

    for name in &names {
        assert!(
            !model.symbols.iter().any(|symbol| symbol.name == *name),
            "generated `{name}` collides with a model symbol"
        );
        assert!(!rename_variables::DEFAULT_GLOBALS.contains(&name.as_str()));
        assert!(!rename_variables::ROBLOX_GLOBALS.contains(&name.as_str()));
    }

    let prefixed = generator.generate_with_prefix("__R4_SELECT");
    assert_ne!(
        prefixed, "__R4_SELECT",
        "the prefix collides with a model symbol"
    );
}

#[test]
fn removes_assert_statement() {
    let (mut block, model) = analyze_with_tokens("assert(true)\nreturn 1");

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("assert"), "unexpected: {output}");
}

#[test]
fn rewrites_multi_argument_assert_with_select() {
    let (mut block, model) = analyze_with_tokens("local x = assert(1, \"message\")\nreturn x");

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(output.contains("select(1, 1"), "unexpected: {output}");
}

#[test]
fn keeps_shadowed_assert_calls() {
    let (mut block, model) = analyze_with_tokens(
        "local assert = function(value) return value end\nlocal x = assert(true)\nreturn x",
    );

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 0, "a non-alias shadowing local must not match");
    assert!(generate(&block).contains("assert(true)"));
}

#[test]
fn removes_assert_calls_through_read_only_alias() {
    let (mut block, model) =
        analyze_with_tokens("local assert = assert\nlocal x = assert(true)\nreturn x");

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 1, "a read-only alias must match");
    let output = generate(&block);
    assert!(output.contains("local x = true"), "unexpected: {output}");
}

#[test]
fn reserved_select_alias_is_collision_free() {
    let (mut block, model) = analyze_with_tokens(
        "local __R4_SELECT = 1\nlocal select = function() end\nlocal x = assert(1, 2)\nreturn x + __R4_SELECT",
    );

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(
        output.contains("__R4_SELECT1 = select"),
        "the alias must avoid the existing name: {output}"
    );
    assert!(
        output.contains("__R4_SELECT1(1, 1, 2)"),
        "the replacement must use the fresh alias: {output}"
    );
    parse(&output);
}

#[test]
fn single_argument_assert_does_not_reserve_select() {
    let (mut block, model) =
        analyze_with_tokens("local select = function() end\nlocal x = assert(1)\nreturn x, select");

    let removed = canonicalize::remove_assertions(&mut block, &model);

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(
        !output.contains("__R4_"),
        "no alias is needed for a single argument: {output}"
    );
}

#[test]
fn removes_unused_labels() {
    let (mut block, model) = analyze_lua52_with_tokens("::unused::\nreturn 1");

    let removed = canonicalize::remove_unused_labels(&mut block, &model)
        .expect("unused label removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("::unused::"), "unexpected: {output}");
}

#[test]
fn keeps_targeted_labels() {
    let (mut block, model) = analyze_lua52_with_tokens("::again::\ngoto again");

    let removed = canonicalize::remove_unused_labels(&mut block, &model)
        .expect("unused label removal must succeed with tokens");

    assert_eq!(removed, 0);
    assert!(generate(&block).contains("::again::"));
}

#[test]
fn removes_unused_labels_in_nested_functions() {
    let (mut block, model) = analyze_lua52_with_tokens(
        r#"
        local function f()
            ::inner::
            return 1
        end
        return f
        "#,
    );

    let removed = canonicalize::remove_unused_labels(&mut block, &model)
        .expect("unused label removal must succeed with tokens");
    assert_eq!(removed, 1);
}

#[test]
fn removes_unused_labels_without_token_data() {
    let mut block = Parser::default()
        .lua52()
        .parse("::unused::\nreturn 1")
        .expect("parse");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let removed = canonicalize::remove_unused_labels(&mut block, &model)
        .expect("token-less labels must be removable");

    assert_eq!(removed, 1);
    assert!(!generate(&block).contains("::unused::"));
}

#[test]
fn removes_dead_stores() {
    let (mut block, model) = analyze_with_tokens("local x\nx = 1\nx = 2\nreturn x");

    let removed = canonicalize::remove_dead_stores(&mut block, &model)
        .expect("dead store removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("x = 1"), "unexpected: {output}");
    assert!(output.contains("x = 2"), "unexpected: {output}");
    parse(&output);
}

#[test]
fn keeps_conditional_stores() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local x
        if condition then
            x = 1
        else
            x = 2
        end
        return x
        "#,
    );

    let removed = canonicalize::remove_dead_stores(&mut block, &model)
        .expect("dead store removal must succeed with tokens");
    assert_eq!(removed, 0, "both stores reach the return");
}

#[test]
fn keeps_stores_to_captured_locals() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local x
        local function observe()
            return x
        end
        x = 1
        return observe
        "#,
    );

    let removed = canonicalize::remove_dead_stores(&mut block, &model)
        .expect("dead store removal must succeed with tokens");
    assert_eq!(removed, 0, "a nested function can observe the store");
}

#[test]
fn keeps_side_effecting_stores() {
    let (mut block, model) = analyze_with_tokens("local x\nx = compute()\nx = 2\nreturn x");

    let removed = canonicalize::remove_dead_stores(&mut block, &model)
        .expect("dead store removal must succeed with tokens");
    assert_eq!(removed, 0, "the call must keep running");
}

#[test]
fn dead_store_pass_works_without_token_data() {
    let mut block = parse("local x\nx = 1\nx = 2\nreturn x");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let removed = canonicalize::remove_dead_stores(&mut block, &model)
        .expect("token-less stores must be removable");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("x = 1"), "unexpected: {output}");
}

#[test]
fn renames_locals_consistently() {
    let (mut block, model) = analyze_with_tokens("local long_name = 1\nreturn long_name");

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    assert_eq!(renamed, 1);
    let output = generate(&block);
    assert!(!output.contains("long_name"), "unexpected: {output}");

    let reparsed = parse(&output);
    let new_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("renamed code must analyze");
    let local = new_model
        .symbols
        .iter()
        .find(|symbol| symbol.kind == SymbolKind::Local)
        .expect("the renamed local must exist");
    assert_eq!(local.read_count, 1);
    assert_eq!(local.write_count, 1);
}

#[test]
fn preserves_shadowing_bindings() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local value = 1
        do
            local value = 2
            print(value)
        end
        print(value)
        "#,
    );

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 2);

    let output = generate(&block);
    let reparsed = parse(&output);
    let new_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("renamed code must analyze");

    let mut locals: Vec<_> = new_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local)
        .collect();
    locals.sort_by_key(|symbol| symbol.id);
    assert_eq!(locals.len(), 2);
    assert_ne!(
        locals[0].name, locals[1].name,
        "shadowed locals must receive distinct names"
    );
    assert_eq!(locals[0].read_count, 1);
    assert_eq!(locals[1].read_count, 1);
}

#[test]
fn keeps_global_references() {
    let (mut block, model) = analyze_with_tokens("local print = print\nprint(\"x\")");

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    assert_eq!(
        output.matches("print").count(),
        1,
        "only the global reference keeps its name: {output}"
    );
}

#[test]
fn renames_captured_upvalues_consistently() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local captured = 1
        local function f()
            return captured
        end
        return f
        "#,
    );

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    assert!(!output.contains("captured"), "unexpected: {output}");
    assert!(!output.contains("function f"), "unexpected: {output}");

    let reparsed = parse(&output);
    let new_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("renamed code must analyze");
    let captured = new_model
        .symbols
        .iter()
        .find(|symbol| symbol.capturing_scopes.len() == 1)
        .expect("the captured symbol must still be captured");
    assert_eq!(captured.read_count, 1);
}

#[test]
fn renames_parameters_and_iteration_variables() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local function f(long_parameter)
            for index = 1, 10 do
                return long_parameter + index
            end
        end
        return f
        "#,
    );

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 3, "function, parameter and iteration variable");

    let output = generate(&block);
    assert!(!output.contains("long_parameter"), "unexpected: {output}");
    assert!(!output.contains("index"), "unexpected: {output}");
    parse(&output);
}

#[test]
fn rename_pass_works_without_token_data() {
    let mut block = parse("local long_name = 1\nreturn long_name");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("token-less symbols must be renamed");

    assert!(renamed >= 1);
    assert!(
        !generate(&block).contains("long_name"),
        "the local must be renamed: {}",
        generate(&block)
    );
}

#[test]
fn removes_code_after_infinite_loop() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        while true do
            work()
        end
        after()
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(output.contains("work()"), "unexpected: {output}");
    assert!(!output.contains("after()"), "unexpected: {output}");
}

#[test]
fn folds_constant_if_statements() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local enabled = false
        if enabled then
            dead()
        else
            live()
        end
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("dead()"), "unexpected: {output}");
    assert!(output.contains("live()"), "unexpected: {output}");
    assert!(!output.contains("if enabled"), "unexpected: {output}");
}

#[test]
fn folds_constant_true_if_statements() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        if true then
            keep()
        else
            drop()
        end
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(output.contains("keep()"), "unexpected: {output}");
    assert!(!output.contains("drop()"), "unexpected: {output}");
}

#[test]
fn removes_false_while_loops() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local running = false
        while running do
            work()
        end
        after()
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("while"), "unexpected: {output}");
    assert!(!output.contains("work()"), "unexpected: {output}");
    assert!(output.contains("after()"), "unexpected: {output}");
}

#[test]
fn removes_code_after_do_return() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        do
            return 1
        end
        after()
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(output.contains("return 1"), "unexpected: {output}");
    assert!(!output.contains("after()"), "unexpected: {output}");
}

#[test]
fn keeps_unknown_control_flow() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        if condition then
            a()
        else
            b()
        end
        c()
        "#,
    );

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed with tokens");

    assert_eq!(removed, 0);
    let output = generate(&block);
    assert!(output.contains("a()"), "unexpected: {output}");
    assert!(output.contains("b()"), "unexpected: {output}");
    assert!(output.contains("c()"), "unexpected: {output}");
}

#[test]
fn unreachable_code_pass_works_without_token_data() {
    let mut block = parse("local running = false\nwhile running do work() end\nafter()");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let removed = canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("token-less declarations must be resolved");

    assert!(removed >= 1);
    let output = generate(&block);
    assert!(!output.contains("while"), "unexpected: {output}");
    assert!(output.contains("after()"), "unexpected: {output}");
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
fn unused_variable_pass_works_without_token_data() {
    let mut block = parse("local unused = 1\nreturn 2");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("semantic analysis must succeed");

    let removed = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("token-less declarations must be removable");

    assert_eq!(removed, 1);
    let output = generate(&block);
    assert!(!output.contains("unused"), "unexpected: {output}");
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

#[test]
fn hoists_pure_loop_invariants() {
    let (mut block, model) = analyze(
        r#"
        local base = 1
        while condition do
            local offset = base + 2
            use(offset)
        end
        "#,
    );

    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(hoisted, 1);
    let output = generate(&block);
    assert!(
        !output.contains("offset"),
        "the hoisted local must be renamed: {output}"
    );
    let declaration = output
        .find("= base + 2")
        .unwrap_or_else(|| panic!("the invariant must be declared: {output}"));
    let loop_start = output
        .find("while")
        .unwrap_or_else(|| panic!("the loop must remain: {output}"));
    assert!(
        declaration < loop_start,
        "the invariant must be hoisted before the loop: {output}"
    );
    assert_eq!(
        output.matches("base + 2").count(),
        1,
        "the invariant must evaluate exactly once: {output}"
    );

    // Semantic oracle: the rewritten program analyzes and reads the hoisted
    // local exactly once.
    let reparsed = parse(&output);
    let new_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("the hoisted program must analyze");
    let locals: Vec<_> = new_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local)
        .collect();
    assert_eq!(locals.len(), 2, "base and the hoisted local: {output}");
    assert!(
        locals.iter().any(|symbol| symbol.read_count == 1),
        "the hoisted local must be read: {output}"
    );
}

#[test]
fn hoisting_leaves_impure_expressions_in_place() {
    for code in [
        "while condition do local x = compute() use(x) end",
        "while condition do local t = {} use(t) end",
        "while condition do local v = object.field use(v) end",
        "while condition do local s = `value {value}` use(s) end",
    ] {
        let (mut block, model) = analyze(code);
        let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");
        assert_eq!(hoisted, 0, "impure expression hoisted for `{code}`");
    }
}

#[test]
fn hoisting_respects_loop_written_variables() {
    let code = r#"
        local total = 0
        while condition do
            total = total + 1
            local x = total + 1
            use(x)
        end
        "#;

    let (mut block, model) = analyze(code);
    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(
        hoisted, 0,
        "a variable written in the loop is not invariant"
    );
}

#[test]
fn hoisting_respects_for_iteration_variables() {
    let (mut block, model) = analyze("for i = 1, 10 do local x = i + 1\nuse(x) end");

    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(hoisted, 0, "the iteration variable changes every iteration");
}

#[test]
fn hoisting_respects_closure_captures() {
    let code = r#"
        local base = 1
        while condition do
            local x = base + 1
            local f = function() return x end
            use(f)
        end
        "#;

    let (mut block, model) = analyze(code);
    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(hoisted, 0, "a captured local keeps per-iteration bindings");
}

#[test]
fn hoisting_skips_conditional_blocks() {
    let code = r#"
        local base = 1
        while condition do
            if other then
                local x = base + 1
                use(x)
            end
        end
        "#;

    let (mut block, model) = analyze(code);
    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(
        hoisted, 0,
        "a block that does not dominate every latch must not be hoisted"
    );
}

#[test]
fn hoisting_works_from_for_loop_bodies() {
    let code = r#"
        local step = 2
        for i = 1, 10 do
            local scaled = step * 2
            use(scaled)
        end
        "#;

    let (mut block, model) = analyze(code);
    let hoisted = canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting must succeed with a matching model");

    assert_eq!(hoisted, 1);
    let output = generate(&block);
    let declaration = output
        .find("= step * 2")
        .unwrap_or_else(|| panic!("the invariant must be declared: {output}"));
    let loop_start = output
        .find("for")
        .unwrap_or_else(|| panic!("the loop must remain: {output}"));
    assert!(
        declaration < loop_start,
        "the invariant must be hoisted before the loop: {output}"
    );
    assert!(
        !output.contains("local scaled"),
        "the original declaration must be removed: {output}"
    );
}

struct IdentifierConverter;

impl require::RequirePathConverter for IdentifierConverter {
    fn convert(&self, path: &str) -> Option<Expression> {
        let name = path.trim_start_matches("./").replace(['/', '.'], "_");
        Some(Expression::identifier(format!("module_{name}")))
    }
}

#[test]
fn convert_require_rejects_shadowed_require() {
    for code in [
        "local require = function(path) return path end\nreturn require('./x')",
        "local function f(require) return require('./x') end",
        "local require = require\nrequire = other\nreturn require('./x')",
    ] {
        let (mut block, model) = analyze(code);
        let converted = require::convert_require(&mut block, &model, &IdentifierConverter)
            .expect("shadowed require calls are never converted");

        assert_eq!(converted, 0, "converted a shadowed require for `{code}`");
        assert!(
            generate(&block).contains("require("),
            "the shadowed call must be left in place for `{code}`"
        );
    }
}

#[test]
fn convert_require_converts_global_and_immutable_alias() {
    for code in [
        "return require('./x')",
        "local require = require\nreturn require('./x')",
    ] {
        let (mut block, model) = analyze(code);
        let converted = require::convert_require(&mut block, &model, &IdentifierConverter)
            .expect("global require must convert");

        assert_eq!(converted, 1, "failed to convert `{code}`");
        let output = generate(&block);
        assert!(output.contains("module_x"), "unexpected: {output}");
        assert!(!output.contains("require("), "unexpected: {output}");
    }
}

#[test]
fn convert_require_handles_statement_calls() {
    let (mut block, model) = analyze("require('./x')");
    let converted = require::convert_require(&mut block, &model, &IdentifierConverter)
        .expect("global require must convert");

    assert_eq!(converted, 1);
    let output = generate(&block);
    assert!(output.contains("module_x"), "unexpected: {output}");
}

#[test]
fn convert_require_rejects_non_literal_arguments() {
    let (mut block, model) = analyze("return require(path)");
    let error = require::convert_require(&mut block, &model, &IdentifierConverter)
        .expect_err("a dynamic require argument must be reported");

    assert!(matches!(
        error,
        require::ConvertRequireError::NonLiteralRequireArgument { .. }
    ));
}

#[test]
fn bundles_required_modules() {
    let (mut block, model) = analyze("local math = require('./math')\nreturn math.add(1, 2)");
    let mut provider = require::MemoryModuleProvider::new();
    provider.insert(
        "math",
        "local function add(a, b) return a + b end\nreturn { add = add }",
    );

    let bundled =
        require::bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
    assert_eq!(bundled, 1);

    let output = generate(&block);
    assert!(output.contains("__R4_MODULES"), "unexpected: {output}");
    assert!(!output.contains("require("), "unexpected: {output}");
    assert!(output.contains("['math']"), "unexpected: {output}");

    let reparsed = parse(&output);
    let new_model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .expect("bundled code must analyze");
    assert!(
        new_model
            .symbols
            .iter()
            .any(|symbol| symbol.name == "math" && symbol.read_count >= 1),
        "the bundled code must keep the module binding: {output}"
    );
}

#[test]
fn bundles_nested_modules() {
    let (mut block, model) = analyze("return require('./a')");
    let mut provider = require::MemoryModuleProvider::new();
    provider.insert("a", "return require('./b')");
    provider.insert("b", "return 1");

    let bundled =
        require::bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
    assert_eq!(bundled, 2);

    let output = generate(&block);
    assert!(output.contains("['a']"), "unexpected: {output}");
    assert!(output.contains("['b']"), "unexpected: {output}");
    assert!(!output.contains("require("), "unexpected: {output}");
}

#[test]
fn bundle_reports_missing_modules() {
    let (mut block, model) = analyze("return require('./missing')");
    let provider = require::MemoryModuleProvider::new();
    let error = require::bundle_modules(&mut block, &model, &provider)
        .expect_err("a missing module must be reported");

    assert!(matches!(
        error,
        require::BundleError::MissingModule { ref path } if path == "missing"
    ));
}

#[test]
fn bundle_reports_cyclic_modules() {
    let (mut block, model) = analyze("return require('./a')");
    let mut provider = require::MemoryModuleProvider::new();
    provider.insert("a", "return require('./b')");
    provider.insert("b", "return require('./a')");

    let error = require::bundle_modules(&mut block, &model, &provider)
        .expect_err("a cycle must be reported");

    assert!(matches!(error, require::BundleError::CyclicRequire { .. }));
}

#[test]
fn bundle_reports_non_literal_requires() {
    let (mut block, model) = analyze("return require(path)");
    let provider = require::MemoryModuleProvider::new();
    let error = require::bundle_modules(&mut block, &model, &provider)
        .expect_err("a dynamic require must be reported");

    assert!(matches!(
        error,
        require::BundleError::NonLiteralRequire { module: None, .. }
    ));
}

#[test]
fn bundle_ignores_shadowed_require() {
    let code = "local require = function(path) return path end\nreturn require('./x')";
    let (mut block, model) = analyze(code);
    let provider = require::MemoryModuleProvider::new();

    let bundled =
        require::bundle_modules(&mut block, &model, &provider).expect("no require to bundle");
    assert_eq!(bundled, 0);
    assert!(generate(&block).contains("require("));
}

#[test]
fn convert_local_function_reports_a_foreign_model() {
    let block = parse("local function f() end");
    let other = parse("local function g() end");
    let model = SemanticAnalyzer::new()
        .analyze(&other)
        .expect("the other block must analyze");

    let mut block = block;
    let error = desugar::convert_local_function_to_assignment(&mut block, &model)
        .expect_err("a function that is not bound in the model must be reported");

    assert!(matches!(
        error,
        desugar::ConvertLocalFunctionError::UnresolvedFunction { ref name } if name == "f"
    ));
}
