//! Adversarial semantic-oracle tests for the transformation pipeline.
//!
//! Instead of asserting on generated strings alone, each test re-parses and
//! re-analyzes the transformed program and checks that the semantic structure
//! (access counts, capture relationships, global identity) is preserved or
//! changed exactly as intended.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::canonicalize;
use r4::rules::rename_variables::{self, DEFAULT_GLOBALS, ROBLOX_GLOBALS};
use r4::semantic::{AccessKind, SemanticAnalyzer, SemanticModel, SymbolKind};
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

fn analyze_output(output: &str) -> SemanticModel {
    let block = parse(output);
    SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("transformed code must analyze: {error}\n{output}"))
}

/// Counts declaration, write and read accesses across every symbol.
fn access_counts(model: &SemanticModel) -> (usize, usize, usize) {
    let mut declarations = 0;
    let mut writes = 0;
    let mut reads = 0;

    for symbol in &model.symbols {
        for usage in model.uses_of(symbol.id) {
            match usage.kind {
                AccessKind::Declaration => declarations += 1,
                AccessKind::Write => writes += 1,
                AccessKind::Read => reads += 1,
            }
        }
    }

    (declarations, writes, reads)
}

#[test]
fn renaming_preserves_access_structure() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local print = print
        local function run(print)
            print("x")
            return print
        end
        return run(print)
        "#,
    );

    let before = access_counts(&model);
    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    let after_model = analyze_output(&output);
    let after = access_counts(&after_model);

    assert_eq!(
        before, after,
        "renaming must not change the def-use structure:\n{output}"
    );

    let globals: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Global && symbol.name == "print")
        .collect();
    assert_eq!(globals.len(), 1, "the global print must survive: {output}");
    assert_eq!(globals[0].read_count, 1);

    let locals: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Global)
        .collect();
    assert!(
        locals.iter().all(|symbol| symbol.name != "print"),
        "locals must not be named after the shadowed global: {output}"
    );
}

#[test]
fn renaming_preserves_capture_relationships() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local outer = 1
        local function a()
            local middle = outer
            return function()
                return middle + outer
            end
        end
        return a
        "#,
    );

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    let after_model = analyze_output(&output);

    let captured_twice: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local && symbol.capturing_scopes.len() == 2)
        .collect();
    assert_eq!(
        captured_twice.len(),
        1,
        "outer must stay captured: {output}"
    );
    assert_eq!(captured_twice[0].read_count, 2);

    let captured_once: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local && symbol.capturing_scopes.len() == 1)
        .collect();
    assert_eq!(
        captured_once.len(),
        1,
        "middle must stay captured: {output}"
    );
}

#[test]
fn generated_identifiers_avoid_host_globals() {
    let (mut block, model) = analyze_with_tokens("local value = 1\nreturn value");

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    let after_model = analyze_output(&output);

    for symbol in &after_model.symbols {
        if symbol.kind != SymbolKind::Global {
            assert!(
                !DEFAULT_GLOBALS.contains(&symbol.name.as_str()),
                "generated `{}` collides with a host global",
                symbol.name
            );
            assert!(
                !ROBLOX_GLOBALS.contains(&symbol.name.as_str()),
                "generated `{}` collides with a Roblox global",
                symbol.name
            );
        }
    }
}

#[test]
fn dead_code_and_unused_removal_compose() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local unused = compute()
        local enabled = false
        if enabled then
            dead()
        else
            live()
        end
        local kept = 1
        return kept
        "#,
    );

    let removed_unused = canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed with tokens");
    assert!(removed_unused >= 1);

    let refreshed = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("the reduced block must still analyze");
    let removed_unreachable = canonicalize::remove_unreachable_code(&mut block, &refreshed)
        .expect("unreachable code removal must succeed with tokens");
    assert!(removed_unreachable >= 1);

    let output = generate(&block);
    assert!(output.contains("compute()"), "unexpected: {output}");
    assert!(!output.contains("unused"), "unexpected: {output}");
    assert!(!output.contains("dead()"), "unexpected: {output}");
    assert!(output.contains("live()"), "unexpected: {output}");
    assert!(output.contains("return kept"), "unexpected: {output}");

    analyze_output(&output);
}

#[test]
fn renamed_program_has_no_undeclared_symbols_in_strict_mode() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local function add(a, b)
            return a + b
        end
        local total = add(1, 2)
        return total
        "#,
    );

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    let reparsed = parse(&output);

    SemanticAnalyzer::new()
        .strict_globals()
        .analyze(&reparsed)
        .unwrap_or_else(|error| {
            panic!("renamed code must not introduce undeclared symbols: {error}\n{output}")
        });
}

#[test]
fn pipeline_preserves_strict_semantics_of_a_shadowing_program() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local pcall = pcall
        local function safe(handler, ...)
            return pcall(handler, ...)
        end
        return safe
        "#,
    );

    rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");

    let output = generate(&block);
    let after_model = analyze_output(&output);

    // The alias is a local; the initializer must keep reading the real global.
    let globals: Vec<_> = after_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Global && symbol.name == "pcall")
        .collect();
    assert_eq!(globals.len(), 1, "pcall must stay a global read: {output}");
    assert_eq!(globals[0].read_count, 1);

    assert!(
        after_model
            .symbols
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Global)
            .all(|symbol| symbol.name != "pcall"),
        "the shadowing alias must be renamed: {output}"
    );
}

/// Attribute names, field names and type names are not variable bindings, so
/// they must never reach `process_variable_expression`: renaming must succeed
/// on attribute code and leave every name that is not a renamable binding
/// untouched — including the receiver a colon method introduces implicitly.
#[test]
fn renaming_survives_names_that_are_not_bindings() {
    for (code, preserved) in [
        ("@native function f() end", &["@native", "native", "f"][..]),
        ("@native local function g() end", &["@native", "native"][..]),
        (
            "local h = @native function() end",
            &["@native", "native"][..],
        ),
        (
            "local obj = { field_name = 1 }\nreturn obj",
            &["field_name"][..],
        ),
        (
            "local obj = {}\nfunction obj:method_name() return self end\nreturn obj",
            &["method_name", "self"][..],
        ),
        (
            "local obj = {}\nfunction obj.inner.deep:method_name(a) return self, a end\nreturn obj",
            &["inner", "deep", "method_name", "self"][..],
        ),
        (
            "local obj = {}\nlocal value = obj.field_name\nreturn value",
            &["field_name"][..],
        ),
        (
            "local obj = {}\nobj.field_name = 1\nreturn obj",
            &["field_name"][..],
        ),
    ] {
        let (mut block, model) = analyze_with_tokens(code);
        rename_variables::rename_variables(&mut block, &model)
            .unwrap_or_else(|error| panic!("renaming `{code}` must succeed: {error}"));

        let output = generate(&block);
        for token in preserved {
            assert!(
                output.contains(token),
                "`{token}` lost in `{code}`: {output}"
            );
        }
        analyze_output(&output);
    }
}

#[test]
fn colon_method_receiver_is_not_renamed() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local t = {}
        function t:method(a)
            self.value = a
            return self.value
        end
        return t
        "#,
    );

    let receiver = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "self")
        .expect("the colon method binds an implicit receiver");
    assert!(receiver.is_implicit());
    assert_eq!(receiver.kind, SymbolKind::Parameter);

    // Only `t` and the explicit parameter `a` are renamable.
    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 2);

    let output = generate(&block);
    let after = analyze_output(&output);

    let receivers: Vec<_> = after
        .symbols
        .iter()
        .filter(|symbol| symbol.name == "self")
        .collect();
    assert_eq!(
        receivers.len(),
        1,
        "the implicit receiver must keep `self`: {output}"
    );
    assert!(receivers[0].is_implicit());
    assert_eq!(
        receivers[0].read_count, 2,
        "both `self` reads must still bind to the receiver: {output}"
    );
    assert!(
        !after
            .symbols
            .iter()
            .any(|symbol| symbol.kind == SymbolKind::Parameter && symbol.name == "a"),
        "the explicit parameter must be renamed: {output}"
    );
}

#[test]
fn colon_method_explicit_self_parameter_is_renamed() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local t = {}
        function t:m(self)
            return self * 3
        end
        return t
        "#,
    );

    // The explicit parameter shadows the implicit receiver in the same scope.
    let self_symbols: Vec<_> = model.symbols.iter().filter(|s| s.name == "self").collect();
    assert_eq!(self_symbols.len(), 2);
    assert_eq!(
        self_symbols.iter().filter(|s| s.is_implicit()).count(),
        1,
        "exactly one of the two `self` symbols is the implicit receiver"
    );

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 2, "`t` and the explicit `self` are renamable");

    let output = generate(&block);
    let after = analyze_output(&output);

    let remaining: Vec<_> = after.symbols.iter().filter(|s| s.name == "self").collect();
    assert_eq!(
        remaining.len(),
        1,
        "only the implicit receiver keeps `self`: {output}"
    );
    assert!(remaining[0].is_implicit());
    assert_eq!(
        remaining[0].read_count, 0,
        "the body referred to the explicit parameter, not the receiver: {output}"
    );
    assert!(
        after
            .symbols
            .iter()
            .any(|symbol| symbol.kind == SymbolKind::Parameter
                && !symbol.is_implicit()
                && symbol.read_count == 1),
        "the explicit parameter and its use must rename together: {output}"
    );
}

#[test]
fn dot_method_explicit_self_parameter_is_renamed() {
    let (mut block, model) = analyze_with_tokens(
        r#"
        local t = {}
        function t.m(self)
            return self * 3
        end
        return t
        "#,
    );

    // Dot syntax declares `self` explicitly; there is no implicit receiver.
    let self_symbols: Vec<_> = model.symbols.iter().filter(|s| s.name == "self").collect();
    assert_eq!(self_symbols.len(), 1);
    assert!(!self_symbols[0].is_implicit());

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 2);

    let output = generate(&block);
    let after = analyze_output(&output);
    assert!(
        !after.symbols.iter().any(|symbol| symbol.name == "self"),
        "the explicit parameter must be renamed: {output}"
    );
}

#[test]
fn local_named_self_is_renamed() {
    let (mut block, model) = analyze_with_tokens("local self = 10\nreturn self");

    let local = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "self")
        .expect("the local declaration");
    assert_eq!(local.kind, SymbolKind::Local);
    assert!(!local.is_implicit());

    let renamed = rename_variables::rename_variables(&mut block, &model)
        .expect("renaming must succeed with tokens");
    assert_eq!(renamed, 1);

    let output = generate(&block);
    let after = analyze_output(&output);
    assert!(
        !after.symbols.iter().any(|symbol| symbol.name == "self"),
        "a local named `self` must rename normally: {output}"
    );
}

#[test]
fn targeted_rename_rejects_an_implicit_receiver() {
    use r4::rules::rename_variables::{rename_symbol, RenameSymbolError};

    let (mut block, model) =
        analyze_with_tokens("local t = {}\nfunction t:m() return self end\nreturn t");
    let receiver = model
        .symbols
        .iter()
        .find(|symbol| symbol.is_implicit())
        .expect("the implicit receiver")
        .id;

    let error = rename_symbol(&mut block, &model, receiver, "x").expect_err("implicit receiver");
    assert_eq!(error.code(), "rename/implicit-symbol");
    assert!(matches!(error, RenameSymbolError::ImplicitSymbol));
}
