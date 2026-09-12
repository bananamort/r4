use r4::nodes::{Block, Statement};
use r4::semantic::{
    AccessKind, SemanticAnalyzer, SemanticErrorKind, SemanticModel, Symbol, SymbolKind,
};
use r4::Parser;

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn parse_lua52(code: &str) -> Block {
    Parser::default()
        .lua52()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze(code: &str) -> SemanticModel {
    let block = parse(code);
    SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"))
}

fn analyze_lua52(code: &str) -> SemanticModel {
    let block = parse_lua52(code);
    SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"))
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

fn symbols_named<'a>(model: &'a SemanticModel, name: &str) -> Vec<&'a Symbol> {
    model
        .symbols
        .iter()
        .filter(|symbol| symbol.name == name)
        .collect()
}

#[test]
fn shadowing_resolves_to_nearest_declaration() {
    let model = analyze(
        r#"
        local x = 1
        do
            local x = 2
            print(x)
        end
        print(x)
        "#,
    );

    let symbols = symbols_named(&model, "x");
    assert_eq!(symbols.len(), 2, "two distinct local symbols must exist");

    let outer = symbols[0];
    let inner = symbols[1];

    assert_eq!(outer.declaring_scope, model.chunk_scope());
    assert_ne!(inner.declaring_scope, outer.declaring_scope);
    assert_eq!(
        model.scope(inner.declaring_scope).kind,
        r4::semantic::ScopeKind::Block
    );

    assert_eq!(outer.read_count, 1);
    assert_eq!(inner.read_count, 1);
    assert_eq!(outer.write_count, 1);
    assert_eq!(inner.write_count, 1);
}

#[test]
fn unresolved_names_become_globals() {
    let model = analyze(
        r#"
        print(missing)
        missing = 1
        "#,
    );

    let missing = model
        .global_symbol("missing")
        .expect("a global symbol must be created for `missing`");
    let symbol = model.symbol(missing);

    assert_eq!(symbol.kind, SymbolKind::Global);
    assert_eq!(symbol.read_count, 1);
    assert_eq!(symbol.write_count, 1);
    assert_eq!(symbol.declaring_scope, model.root_scope());

    let print = model
        .global_symbol("print")
        .expect("a global symbol must be created for `print`");
    assert_eq!(model.symbol(print).read_count, 1);
}

#[test]
fn strict_mode_rejects_undeclared_symbols() {
    let block = parse("return missing");

    let error = SemanticAnalyzer::new()
        .strict_globals()
        .analyze(&block)
        .expect_err("strict mode must reject undeclared names");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::UndeclaredSymbol { ref name } if name == "missing"
    ));
}

#[test]
fn strict_mode_accepts_declared_locals() {
    let block = parse(
        r#"
        local x = 1
        return x
        "#,
    );

    SemanticAnalyzer::new()
        .strict_globals()
        .analyze(&block)
        .expect("declared locals must be accepted in strict mode");
}

#[test]
fn direct_and_indirect_upvalues_are_captured() {
    let model = analyze(
        r#"
        local x = 1
        local function outer()
            local y = x
            return function()
                return x + y
            end
        end
        "#,
    );

    let x = symbols_named(&model, "x");
    assert_eq!(x.len(), 1);
    let x = x[0];
    assert!(
        x.is_captured(),
        "x must be captured by the nested functions"
    );
    assert_eq!(
        x.capturing_scopes.len(),
        2,
        "both `outer` and the innermost closure capture x"
    );

    let y = symbols_named(&model, "y");
    assert_eq!(y.len(), 1);
    assert_eq!(y[0].capturing_scopes.len(), 1);
}

#[test]
fn locals_referenced_only_at_home_scope_are_not_captured() {
    let model = analyze(
        r#"
        local x = 1
        local function helper()
            local y = 2
            return y
        end
        return x
        "#,
    );

    assert!(!symbols_named(&model, "x")[0].is_captured());
    assert!(!symbols_named(&model, "y")[0].is_captured());
}

#[test]
fn read_and_write_counts_match_occurrences() {
    let model = analyze(
        r#"
        local x = 1
        x = x + 1
        print(x)
        "#,
    );

    let x = symbols_named(&model, "x")[0];
    assert_eq!(x.read_count, 2, "one read in `x + 1` and one in `print(x)`");
    assert_eq!(x.write_count, 2, "the declaration and the assignment");
}

#[test]
fn symbol_uses_are_recorded_in_order() {
    let model = analyze(
        r#"
        local x = 1
        x = x + 1
        "#,
    );

    let x = symbols_named(&model, "x")[0];
    let kinds: Vec<AccessKind> = model.uses_of(x.id).iter().map(|usage| usage.kind).collect();

    assert_eq!(
        kinds,
        vec![AccessKind::Declaration, AccessKind::Read, AccessKind::Write]
    );
}

#[test]
fn parameters_and_iteration_variables_have_their_kind() {
    let model = analyze(
        r#"
        local function f(a, b)
            for i = 1, 10 do end
            for k, v in pairs(t) do end
            return a
        end
        "#,
    );

    assert_eq!(symbols_named(&model, "f")[0].kind, SymbolKind::Local);
    assert_eq!(symbols_named(&model, "a")[0].kind, SymbolKind::Parameter);
    assert_eq!(symbols_named(&model, "b")[0].kind, SymbolKind::Parameter);
    assert_eq!(symbols_named(&model, "i")[0].kind, SymbolKind::Iteration);
    assert_eq!(symbols_named(&model, "k")[0].kind, SymbolKind::Iteration);
    assert_eq!(symbols_named(&model, "v")[0].kind, SymbolKind::Iteration);
}

#[test]
fn method_functions_declare_implicit_self() {
    let model = analyze(
        r#"
        local t = {}
        function t:m()
            return self
        end
        "#,
    );

    let self_symbol = symbols_named(&model, "self");
    assert_eq!(self_symbol.len(), 1);
    assert_eq!(self_symbol[0].kind, SymbolKind::Parameter);
    assert_eq!(self_symbol[0].read_count, 1);
}

#[test]
fn typeof_expressions_inside_types_are_analyzed() {
    let model = analyze(
        r#"
        local x = 1
        type Alias = typeof(x)
        return x
        "#,
    );

    let x = symbols_named(&model, "x")[0];
    assert_eq!(x.read_count, 2, "one read in `typeof`, one in `return`");
}

#[test]
fn goto_inside_a_nested_function_cannot_see_outer_labels() {
    let block = parse_lua52(
        r#"
        ::outer::
        local function f()
            goto outer
        end
        "#,
    );

    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("labels are not visible inside nested functions");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::UndefinedLabel { ref name } if name == "outer"
    ));
}

#[test]
fn goto_binding_is_available_through_token_data() {
    let (block, model) = analyze_lua52_with_tokens("goto target\n::target::");

    let Statement::Goto(goto_statement) = block
        .first_statement()
        .expect("the block must start with a goto")
    else {
        panic!("unexpected first statement");
    };

    let token = goto_statement
        .get_label()
        .get_token()
        .expect("token must exist");
    let binding = model
        .goto_binding_at_token(token)
        .expect("the goto token must resolve to a binding");

    assert_eq!(model.label(binding.label).name, "target");
}

#[test]
fn repeat_condition_sees_body_locals() {
    // In Lua the `until` condition is evaluated inside the scope of the repeat
    // body.
    let model = analyze(
        r#"
        repeat
            local done = check()
        until done
        "#,
    );

    let done = symbols_named(&model, "done")[0];
    assert_eq!(done.read_count, 1);
}

#[test]
fn forward_and_backward_gotos_resolve() {
    let model = analyze_lua52(
        r#"
        ::start::
        local x = 1
        goto exit
        ::exit::
        goto start
        "#,
    );

    assert_eq!(model.goto_bindings.len(), 2);

    let names: Vec<&str> = model
        .goto_bindings
        .iter()
        .map(|binding| model.label(binding.label).name.as_str())
        .collect();
    assert!(names.contains(&"exit"));
    assert!(names.contains(&"start"));
}

#[test]
fn goto_may_exit_a_nested_block_forward() {
    let model = analyze_lua52(
        r#"
        do
            goto done
        end
        ::done::
        "#,
    );

    assert_eq!(model.goto_bindings.len(), 1);
    assert_eq!(model.labels.len(), 1);
}

#[test]
fn goto_into_local_scope_is_rejected() {
    let block = parse_lua52(
        r#"
        goto done
        local x = 1
        ::done::
        print(x)
        "#,
    );

    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("jumping into the scope of `x` must fail");

    match error.kind {
        SemanticErrorKind::GotoIntoLocalScope { label, symbol } => {
            assert_eq!(label, "done");
            assert_eq!(symbol, "x");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn trailing_label_is_outside_the_scope_of_block_locals() {
    // Lua treats a label at the very end of a block as outside the scope of the
    // locals declared in that block, so this forward jump is legal.
    analyze_lua52(
        r#"
        goto done
        local x = 1
        ::done::
        "#,
    );
}

#[test]
fn block_end_label_allows_continue_idiom() {
    // The label is the last statement of the loop body, so the locals declared
    // in the body are considered out of scope at the label.
    analyze_lua52(
        r#"
        for i = 1, 10 do
            if i > 5 then
                goto continue
            end
            local x = i
            print(x)
            ::continue::
        end
        "#,
    );
}

#[test]
fn non_trailing_label_still_rejects_jumping_over_a_local() {
    let block = parse_lua52(
        r#"
        for i = 1, 10 do
            goto skip
            local x = i
            ::skip::
            print(x)
        end
        "#,
    );

    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("the label is followed by `print`, so it is inside `x`'s scope");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::GotoIntoLocalScope { ref symbol, .. } if symbol == "x"
    ));
}

#[test]
fn undefined_label_is_rejected() {
    let block = parse_lua52("goto nowhere");

    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("an undefined label must fail");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::UndefinedLabel { ref name } if name == "nowhere"
    ));
}

#[test]
fn duplicate_label_is_rejected() {
    let block = parse_lua52(
        r#"
        ::again::
        ::again::
        "#,
    );

    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("a duplicate label must fail");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::DuplicateLabel { ref name, .. } if name == "again"
    ));
}

#[test]
fn labels_in_separate_functions_may_share_a_name() {
    analyze_lua52(
        r#"
        ::again::
        local function f()
            ::again::
        end
        "#,
    );
}

#[test]
fn token_data_maps_identifiers_to_symbols() {
    let (block, model) = analyze_with_tokens(
        r#"
        local value = 1
        return value
        "#,
    );

    let Statement::LocalAssign(assign) = block
        .first_statement()
        .expect("the block must start with a local assignment")
    else {
        panic!("unexpected first statement");
    };

    let identifier = assign.get_variables()[0].get_identifier();
    let symbol = model
        .resolve_identifier(identifier)
        .expect("the identifier token must resolve to its symbol");

    assert_eq!(model.symbol(symbol).name, "value");
}

#[test]
fn token_data_maps_labels_to_ids() {
    let (block, model) = analyze_lua52_with_tokens("::target::");

    let Statement::Label(label) = block
        .first_statement()
        .expect("the block must start with a label")
    else {
        panic!("unexpected first statement");
    };

    let label_id = model
        .label_at_token(label.get_label().get_token().expect("token must exist"))
        .expect("the label token must resolve");

    assert_eq!(model.label(label_id).name, "target");
}

#[test]
fn scope_tree_is_consistent() {
    let model = analyze(
        r#"
        local function f()
            do
                local inner = 1
            end
        end
        "#,
    );

    let chunk = model.scope(model.chunk_scope());
    assert_eq!(chunk.kind, r4::semantic::ScopeKind::File);
    assert!(!chunk.children.is_empty());

    for scope in &model.scopes {
        for &child in &scope.children {
            assert_eq!(
                model.scope(child).parent,
                Some(scope.id),
                "parent/child links must be symmetric"
            );
        }
    }
}
