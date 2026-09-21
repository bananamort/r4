use r4::nodes::{Arguments, Block, Expression, Identifier, Statement, Type};
use r4::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
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

fn local_symbols_named<'a>(model: &'a SemanticModel, name: &str) -> Vec<&'a Symbol> {
    symbols_named(model, name)
        .into_iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local)
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
fn goto_binding_is_available_structurally() {
    let block = parse_lua52("goto target\n::target::");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("token-less analysis must succeed");

    assert_eq!(model.goto_bindings.len(), 1);
    let binding = &model.goto_bindings[0];
    assert_eq!(binding.site.name, "target");
    assert_eq!(model.label(binding.label).name, "target");

    let Statement::Label(label) = block
        .iter_statements()
        .nth(1)
        .expect("the block must contain a label")
    else {
        panic!("unexpected second statement");
    };
    assert_eq!(model.resolve_label(label.get_label()), Some(binding.label));
}

#[test]
fn resolve_in_scope_returns_the_innermost_declaration() {
    let model = analyze(
        r#"
        local value = 1
        do
            local value = 2
            print(value)
        end
        print(value)
        "#,
    );

    let locals = local_symbols_named(&model, "value");
    assert_eq!(locals.len(), 2);
    let outer = locals[0];
    let inner = locals[1];

    assert_eq!(
        model.resolve_in_scope(inner.declaring_scope, "value"),
        Some(inner.id),
        "the inner scope must resolve to its own declaration"
    );
    assert_eq!(
        model.resolve_in_scope(outer.declaring_scope, "value"),
        Some(outer.id),
        "the outer scope must resolve to the outer declaration"
    );
    assert_eq!(
        model.resolve_in_scope(model.chunk_scope(), "value"),
        Some(outer.id)
    );
}

#[test]
fn resolve_in_scope_never_returns_a_shadowed_global() {
    let model = analyze("local print = 1\nprint(print)");

    let local = local_symbols_named(&model, "print")[0];
    assert_eq!(
        model.resolve_in_scope(local.declaring_scope, "print"),
        Some(local.id)
    );
    assert_eq!(
        model.global_symbol("print"),
        None,
        "a fully shadowed builtin must not be interned as a global"
    );
}

#[test]
fn resolve_in_scope_resolves_undeclared_names_to_globals() {
    let model = analyze("return missing");

    let global = model
        .global_symbol("missing")
        .expect("an undeclared name creates a global symbol");
    assert_eq!(model.symbol(global).kind, SymbolKind::Global);
    assert_eq!(
        model.resolve_in_scope(model.chunk_scope(), "missing"),
        Some(global)
    );

    assert_eq!(
        model.resolve_in_scope(model.chunk_scope(), "never_mentioned"),
        None,
        "a name that was never analyzed does not resolve"
    );
}

#[test]
fn captured_symbols_contains_exactly_the_captured_locals() {
    let model = analyze(
        r#"
        local captured = 1
        local unrelated = 2
        local function closure()
            return captured
        end
        return closure, unrelated
        "#,
    );

    let names: Vec<&str> = model
        .captured_symbols()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names, vec!["captured"]);

    let unrelated = symbols_named(&model, "unrelated")[0];
    assert!(!unrelated.is_captured());
    assert!(unrelated.capturing_scopes.is_empty());
}

#[test]
fn captured_symbols_marks_transitive_captures_once_per_function() {
    let model = analyze(
        r#"
        local outermost = 1
        local function level_one()
            local middle = outermost
            local function level_two()
                local function level_three()
                    return outermost + middle
                end
                return level_three
            end
            return level_two
        end
        return level_one
        "#,
    );

    let names: Vec<&str> = model
        .captured_symbols()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names, vec!["outermost", "middle"]);

    let outermost = symbols_named(&model, "outermost")[0];
    assert_eq!(
        outermost.capturing_scopes.len(),
        3,
        "all three nested functions must carry the upvalue"
    );
    let middle = symbols_named(&model, "middle")[0];
    assert_eq!(middle.capturing_scopes.len(), 2);
}

#[test]
fn captured_symbols_distinguishes_shadowed_names() {
    let model = analyze(
        r#"
        local value = 1
        local function closure()
            local value = 2
            return value
        end
        return closure, value
        "#,
    );

    let locals = local_symbols_named(&model, "value");
    assert_eq!(locals.len(), 2);
    assert!(!locals[0].is_captured(), "the outer value stays outside");
    assert!(
        !locals[1].is_captured(),
        "the inner value is local to the closure"
    );
    assert_eq!(
        model.captured_symbols().count(),
        0,
        "no variable crosses a function boundary"
    );
}

#[test]
fn declaration_initializers_record_global_aliases() {
    let model = analyze(
        r#"
        local math = math
        return math.sqrt(4)
        "#,
    );

    let local = local_symbols_named(&model, "math")[0];
    let global = model
        .global_symbol("math")
        .expect("the initializer read must create a global symbol");

    assert_eq!(local.initializer, Some(global));
    assert!(model.is_immutable(local.id));
    assert_eq!(model.immutable_global_alias(local.id), Some(global));
    assert!(!model.is_unused(local.id));
}

#[test]
fn reassigned_aliases_are_not_immutable() {
    let model = analyze(
        r#"
        local math = math
        math = {}
        return math
        "#,
    );

    let local = local_symbols_named(&model, "math")[0];
    assert_eq!(
        local.initializer,
        Some(model.global_symbol("math").unwrap())
    );
    assert!(!model.is_immutable(local.id));
    assert_eq!(model.immutable_global_alias(local.id), None);
}

#[test]
fn initializers_bind_before_declaring_the_local() {
    let model = analyze(
        r#"
        local x = 1
        local x = x
        "#,
    );

    let locals = symbols_named(&model, "x");
    assert_eq!(locals.len(), 2);
    assert_eq!(locals[0].initializer, None);
    assert_eq!(locals[1].initializer, Some(locals[0].id));
    assert!(model.is_immutable(locals[1].id));
    assert_eq!(
        model.immutable_global_alias(locals[1].id),
        None,
        "an alias of a local is not a global alias"
    );
}

#[test]
fn non_identifier_initializers_are_not_aliases() {
    let model = analyze("local x = 1");

    let symbol = symbols_named(&model, "x")[0];
    assert_eq!(symbol.initializer, None);
    assert_eq!(model.immutable_global_alias(symbol.id), None);
}

#[test]
fn unused_symbols_are_reported() {
    let model = analyze(
        r#"
        local unused = compute()
        return 1
        "#,
    );

    let unused = symbols_named(&model, "unused")[0];
    assert!(model.is_unused(unused.id));
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
fn identifier_bindings_are_structural_with_token_metadata() {
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
        .expect("the identifier must resolve structurally");

    assert_eq!(model.symbol(symbol).name, "value");

    // Token data stays available as passive diagnostics.
    assert!(
        model
            .uses_of(symbol)
            .iter()
            .all(|usage| usage.token.is_some()),
        "token-preserved analysis must record token metadata"
    );
}

#[test]
fn labels_are_bound_structurally_with_token_metadata() {
    let (block, model) = analyze_lua52_with_tokens("::target::");

    let Statement::Label(label) = block
        .first_statement()
        .expect("the block must start with a label")
    else {
        panic!("unexpected first statement");
    };

    let label_id = model
        .resolve_label(label.get_label())
        .expect("the label must resolve structurally");

    assert_eq!(model.label(label_id).name, "target");

    // Token data stays available as passive diagnostics.
    let token = label
        .get_label()
        .get_token()
        .expect("token data must be preserved");
    assert_eq!(
        model.label(label_id).token,
        r4::semantic::TokenKey::of(token)
    );
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

fn local_declaration_identifier(statement: &Statement) -> &Identifier {
    let Statement::LocalAssign(assignment) = statement else {
        panic!("expected a local assignment, got {statement:?}");
    };
    assignment
        .iter_variables()
        .next()
        .expect("the local assignment has a variable")
        .get_identifier()
}

fn call_argument_identifier(statement: &Statement) -> &Identifier {
    let Statement::Call(call) = statement else {
        panic!("expected a call, got {statement:?}");
    };
    let Arguments::Tuple(arguments) = call.get_arguments() else {
        panic!("expected tuple arguments");
    };
    let expression = arguments
        .iter_values()
        .next()
        .expect("the call has an argument");
    let Expression::Identifier(identifier) = expression else {
        panic!("expected an identifier argument, got {expression:?}");
    };
    identifier
}

#[test]
fn same_scope_shadowing_binds_each_occurrence_exactly() {
    // Both declarations share a name and a scope, so only occurrence-level
    // bindings can tell them apart. This must also hold without token data.
    for preserve_tokens in [true, false] {
        let mut parser = Parser::default();
        if preserve_tokens {
            parser = parser.preserve_tokens();
        }
        let block = parser
            .parse("local x = 1\nprint(x)\nlocal x = 2\nprint(x)")
            .expect("parse");
        let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

        let statements: Vec<&Statement> = block.iter_statements().collect();
        let first_decl = local_declaration_identifier(statements[0]);
        let first_use = call_argument_identifier(statements[1]);
        let second_decl = local_declaration_identifier(statements[2]);
        let second_use = call_argument_identifier(statements[3]);

        let first = model
            .resolve_identifier(first_decl)
            .expect("the first declaration must resolve");
        let first_read = model
            .resolve_identifier(first_use)
            .expect("the first read must resolve");
        let second = model
            .resolve_identifier(second_decl)
            .expect("the second declaration must resolve");
        let second_read = model
            .resolve_identifier(second_use)
            .expect("the second read must resolve");

        assert_eq!(first, first_read);
        assert_eq!(second, second_read);
        assert_ne!(first, second, "the declarations are distinct symbols");
        assert_eq!(model.symbol(first).name, "x");
        assert_eq!(model.symbol(second).name, "x");
        assert_eq!(
            model.symbol(first).declaring_scope,
            model.symbol(second).declaring_scope
        );
        assert_eq!(model.symbol(first).read_count, 1);
        assert_eq!(model.symbol(second).read_count, 1);

        // The name-level query returns the most recent declaration: this is
        // exactly why occurrence bindings exist. The positional query answers
        // for a specific declaration position.
        let chunk = model.chunk_scope();
        assert_eq!(model.resolve_in_scope(chunk, "x"), Some(second));
        assert_eq!(
            model.resolve_in_scope_at(chunk, "x", &[(chunk, 1)]),
            Some(first)
        );
        assert_eq!(
            model.resolve_in_scope_at(chunk, "x", &[(chunk, 2)]),
            Some(second)
        );
    }
}

#[test]
fn stale_bindings_are_rejected() {
    let (block, model) = analyze_with_tokens("local value = 1\nreturn value");
    let Statement::LocalAssign(assignment) = block
        .first_statement()
        .expect("the block must start with a local assignment")
    else {
        panic!("unexpected first statement");
    };
    let identifier = assignment
        .iter_variables()
        .next()
        .expect("the assignment has a variable")
        .get_identifier();
    assert!(model.resolve_identifier(identifier).is_some());

    let mut renamed = identifier.clone();
    renamed.set_name("renamed");

    assert_eq!(
        model.resolve_identifier(&renamed),
        None,
        "a binding whose name no longer matches the model must be rejected"
    );
}

#[test]
fn labels_are_bound_without_token_data() {
    let block = parse_lua52("::target::\ngoto target");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("token-less analysis must succeed");

    let statements: Vec<&Statement> = block.iter_statements().collect();
    let Statement::Label(label) = statements[0] else {
        panic!("expected a label statement");
    };

    let label_id = model
        .resolve_label(label.get_label())
        .expect("the label must resolve structurally");
    assert_eq!(model.label(label_id).name, "target");
    assert_eq!(model.goto_bindings.len(), 1);
    assert_eq!(model.goto_bindings[0].label, label_id);

    // A label binding is not a symbol binding.
    assert_eq!(model.resolve_identifier(label.get_label()), None);
}

#[test]
fn analyzed_blocks_have_unique_identities() {
    let block = parse("do local x = 1 end\nif condition then do end end");
    let _model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    struct Collector {
        identities: Vec<r4::nodes::BlockIdentity>,
    }

    impl NodeProcessor for Collector {
        fn process_block(&mut self, block: &mut Block) {
            self.identities.push(
                block
                    .get_identity()
                    .expect("every analyzed block must carry an identity"),
            );
        }
    }

    let mut collector = Collector {
        identities: Vec::new(),
    };
    let mut block = block;
    DefaultVisitor::visit_block(&mut block, &mut collector);

    let unique: std::collections::HashSet<_> = collector.identities.iter().copied().collect();
    assert_eq!(
        unique.len(),
        collector.identities.len(),
        "block identities must be unique"
    );
    assert!(collector.identities.len() >= 4);
}

fn local_declaration_type(statement: &Statement) -> &Type {
    let Statement::LocalAssign(assignment) = statement else {
        panic!("expected a local assignment, got {statement:?}");
    };
    assignment
        .iter_variables()
        .next()
        .expect("the local assignment has a variable")
        .get_type()
        .expect("the local declaration has a type annotation")
}

fn type_name_identifier(r#type: &Type) -> &Identifier {
    let Type::Name(type_name) = r#type else {
        panic!("expected a type name, got {type:?}");
    };
    type_name.get_type_name()
}

#[test]
fn type_declarations_are_bound_structurally() {
    let block = parse("type Point = { x: number, y: number }\ntype Shape = Point");
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("token-less type analysis must succeed");

    let statements: Vec<&Statement> = block.iter_statements().collect();
    let Statement::TypeDeclaration(point) = statements[0] else {
        panic!("expected a type declaration");
    };
    let Statement::TypeDeclaration(shape) = statements[1] else {
        panic!("expected a type declaration");
    };

    let point_id = model
        .resolve_type(point.get_name())
        .expect("the declared type must resolve");
    assert_eq!(model.type_symbol(point_id).name, "Point");
    assert!(!model.type_symbol(point_id).is_exported);
    assert_eq!(
        model.resolve_type_in_scope(model.chunk_scope(), "Point"),
        Some(point_id)
    );

    let shape_id = model
        .resolve_type(shape.get_name())
        .expect("the declared type must resolve");
    assert_ne!(point_id, shape_id);
    assert_eq!(
        model.resolve_type(type_name_identifier(shape.get_type())),
        Some(point_id),
        "the alias use must bind to the declared type"
    );
}

#[test]
fn exported_type_declarations_are_flagged() {
    let block = parse("export type Public = number");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let Statement::TypeDeclaration(declaration) = block
        .first_statement()
        .expect("the block must start with a type declaration")
    else {
        panic!("expected a type declaration");
    };

    let id = model
        .resolve_type(declaration.get_name())
        .expect("the exported type must resolve");
    assert!(model.type_symbol(id).is_exported);
}

#[test]
fn same_scope_type_shadowing_binds_each_occurrence() {
    let block = parse(
        r#"
        type T = number
        local a: T = 1
        type T = string
        local b: T = 'x'
        "#,
    );
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let statements: Vec<&Statement> = block.iter_statements().collect();
    let Statement::TypeDeclaration(first) = statements[0] else {
        panic!("expected a type declaration");
    };
    let Statement::TypeDeclaration(second) = statements[2] else {
        panic!("expected a type declaration");
    };

    let first_id = model.resolve_type(first.get_name()).expect("first T");
    let second_id = model.resolve_type(second.get_name()).expect("second T");
    assert_ne!(first_id, second_id);
    assert_eq!(model.type_symbol(first_id).name, "T");
    assert_eq!(model.type_symbol(second_id).name, "T");

    assert_eq!(
        model.resolve_type(type_name_identifier(local_declaration_type(statements[1]))),
        Some(first_id),
        "the first annotation sees the first declaration"
    );
    assert_eq!(
        model.resolve_type(type_name_identifier(local_declaration_type(statements[3]))),
        Some(second_id),
        "the second annotation sees the redeclaration"
    );
}

#[test]
fn type_shadowing_respects_nested_scopes() {
    let block = parse(
        r#"
        type T = number
        local outer: T = 1
        do
            type T = string
            local inner: T = 'x'
        end
        local after: T = 2
        "#,
    );
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let statements: Vec<&Statement> = block.iter_statements().collect();
    let Statement::TypeDeclaration(outer_declaration) = statements[0] else {
        panic!("expected a type declaration");
    };
    let Statement::Do(do_statement) = statements[2] else {
        panic!("expected a do statement");
    };

    let outer = model
        .resolve_type(outer_declaration.get_name())
        .expect("outer T");
    let inner = model
        .resolve_type(first_type_declaration(do_statement.get_block()).get_name())
        .expect("inner T");
    assert_ne!(outer, inner);

    assert_eq!(
        model.resolve_type(type_name_identifier(local_declaration_type(statements[1]))),
        Some(outer)
    );
    assert_eq!(
        model.resolve_type(type_name_identifier(local_declaration_type(statements[3]))),
        Some(outer),
        "the shadow must not leak out of the do block"
    );
}

fn first_type_declaration(block: &Block) -> &r4::nodes::TypeDeclarationStatement {
    let first = block
        .first_statement()
        .expect("the block must contain a statement");
    let Statement::TypeDeclaration(declaration) = first else {
        panic!("expected a type declaration, got {first:?}");
    };
    declaration
}

#[test]
fn strict_typing_rejects_undeclared_types() {
    let block = parse("type Shape = MissingThing");
    let error = SemanticAnalyzer::new()
        .strict_types()
        .analyze(&block)
        .expect_err("an undeclared type name must be rejected");

    assert!(matches!(
        error.kind,
        SemanticErrorKind::UnresolvedType { ref name } if name == "MissingThing"
    ));

    // Default analysis leaves the occurrence unbound without guessing `any`.
    let block = parse("type Shape = MissingThing");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let Statement::TypeDeclaration(declaration) = block.first_statement().unwrap() else {
        panic!("expected a type declaration");
    };
    assert_eq!(
        model.resolve_type(type_name_identifier(declaration.get_type())),
        None
    );
}

#[test]
fn strict_typing_accepts_builtins_and_generic_parameters() {
    let block = parse(
        r#"
        type Pair<T> = { first: T, second: T }
        type Maybe = Pair<number>?
        type Alias = string | nil | boolean | thread | buffer | vector | unknown | never | any
        "#,
    );

    SemanticAnalyzer::new()
        .strict_types()
        .analyze(&block)
        .expect("built-in and generic type names must resolve");
}

#[test]
fn strict_typing_resolves_aliases_in_type_declarations() {
    let block = parse("type Base = { value: number }\ntype Derived = Base");
    let model = SemanticAnalyzer::new()
        .strict_types()
        .analyze(&block)
        .expect("declared aliases must resolve");

    let Statement::TypeDeclaration(base) = block.first_statement().unwrap() else {
        panic!("expected a type declaration");
    };
    let base_id = model.resolve_type(base.get_name()).expect("Base resolves");
    assert_eq!(model.type_symbol(base_id).name, "Base");
}

#[test]
fn semantic_walker_handles_labels_and_generic_pack_types() {
    // Labels and gotos are Lua 5.2 syntax.
    let (block, model) = Parser::default()
        .lua52()
        .parse_analyzed("::top::\ngoto top\nreturn 1")
        .expect("labels analyze");
    assert!(!model.goto_bindings.is_empty());
    assert!(!block.is_empty());

    for code in [
        "local f: (A...) -> () = g\nreturn f",
        "local f: () -> A... = g\nreturn f",
        "local f: <A...>(A...) -> A... = g\nreturn f",
        "type T = A<B...>\nreturn 1",
        "local x: A<(number)> = y\nreturn x",
        "local function v1(...: number) return ... end\nreturn v1",
        "local function v2(a: number, ...: string): number return a end\nreturn v2",
        "function v3(...: A...) end\nreturn v3",
        "local v4 = function(...: number): () return end\nreturn v4",
        "type function v5(...: number) end\nreturn 1",
    ] {
        let _ = Parser::default()
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    }
}

#[test]
fn shadowed_and_captured_locals_bind_correctly() {
    use r4::semantic::SymbolKind;

    let model = analyze("local x = 1\ndo local x = 2\nprint(x) end\nprint(x)");
    let locals: Vec<_> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.name == "x" && symbol.kind == SymbolKind::Local)
        .collect();
    assert_eq!(locals.len(), 2, "two distinct `x` locals");
    assert_ne!(locals[0].declaring_scope, locals[1].declaring_scope);

    let model = analyze("local x = 1\nlocal function f() return x end\nreturn f");
    let x = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "x" && symbol.kind == SymbolKind::Local)
        .expect("x");
    assert!(x.is_captured(), "x is captured by the nested function");
}

#[test]
fn goto_resolves_to_the_visible_label() {
    let block = Parser::default()
        .lua52()
        .parse("do goto skip end\n::skip::\nreturn 1")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analyze");
    assert_eq!(model.goto_bindings.len(), 1);
    assert_eq!(model.goto_bindings[0].site.name, "skip");
}

#[test]
fn undefined_label_is_a_typed_error() {
    let block = Parser::default()
        .lua52()
        .parse("goto nowhere\nreturn 1")
        .expect("parse");
    let error = SemanticAnalyzer::new()
        .analyze(&block)
        .expect_err("an undefined label is rejected");
    assert_eq!(error.code(), "semantic/undefined-label");
}
