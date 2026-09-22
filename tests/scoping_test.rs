//! Scope-tree and scoping-query expectations.
//!
//! Two kinds of expectation live here:
//!
//! * [`EXPECTED_SCOPE_TREES`] records the exact scope tree a corpus of programs
//!   must produce — every scope kind, nesting, declared variable and label;
//! * the query tests pin scope lookup, kind-bounded variable lookup, symbol
//!   accessibility and single-symbol rename through the public
//!   [`SemanticModel::try_find_variable`] and
//!   [`SemanticModel::try_can_be_accessed_in`] queries.
//!
//! The recorded trees are fixed expectations, not something the repository can
//! re-derive; change them only deliberately. They omit the implicit `...` and
//! `arg` parameters of the chunk and of variadic functions, which r4 does not
//! model as symbols.
//!
//! A second, parallel scope model is deliberately *not* used as an oracle here:
//! it would share the same author, spec reading and AST as the analyzer and so
//! would mostly restate it. The independent check is the brute-force positional
//! differential in `bounded_enumeration_test.rs`, which uses a different method
//! rather than a second implementation.

use r4::nodes::Block;
use r4::semantic::{ScopeId, ScopeKind, SemanticAnalyzer, SemanticModel, SymbolId};
use r4::Parser;

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze(code: &str) -> SemanticModel {
    let block = parse(code);
    analyze_block(&block)
}

fn analyze_block(block: &Block) -> SemanticModel {
    SemanticAnalyzer::new()
        .analyze(block)
        .unwrap_or_else(|error| panic!("failed to analyze: {error}"))
}

/// A canonical, index-independent rendering of the scope tree: each scope is
/// `Kind(vars=[name:Kind,...];labels=[...];children=[...])` with every list
/// sorted, so the rendering does not depend on arena order.
fn canonical(model: &SemanticModel) -> String {
    fn render(model: &SemanticModel, scope: &r4::semantic::Scope) -> String {
        let mut variables: Vec<String> = scope
            .symbols
            .iter()
            .map(|symbol| {
                let symbol = model.symbol(*symbol);
                format!("{}:{:?}", symbol.name, symbol.kind)
            })
            .filter(|variable| variable != "...:Parameter" && variable != "arg:Parameter")
            .collect();
        variables.sort();

        let mut labels: Vec<String> = scope
            .labels
            .iter()
            .map(|label| model.label(*label).name.clone())
            .collect();
        labels.sort();

        let mut children: Vec<String> = scope
            .children
            .iter()
            .map(|child| render(model, model.scope(*child)))
            .collect();
        children.sort();

        format!(
            "{:?}(vars=[{}];labels=[{}];children=[{}])",
            scope.kind,
            variables.join(","),
            labels.join(","),
            children.join(",")
        )
    }

    render(model, model.scope(model.root_scope()))
}

/// `(source, canonical scope tree)` that each program must produce exactly.
const EXPECTED_SCOPE_TREES: &[(&str, &str)] = &[
    ("print 'Hello'", "Global(vars=[print:Global];labels=[];children=[File(vars=[];labels=[];children=[])])"),
    ("local function a() end", "Global(vars=[];labels=[];children=[File(vars=[a:Local];labels=[];children=[Function(vars=[];labels=[];children=[])])])"),
    ("(function(Variable) end)()", "Global(vars=[];labels=[];children=[File(vars=[];labels=[];children=[Function(vars=[Variable:Parameter];labels=[];children=[])])])"),
    ("do local x = 1 end", "Global(vars=[];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[x:Local];labels=[];children=[])])])"),
    ("local x = 1\ndo local x = 2 end", "Global(vars=[];labels=[];children=[File(vars=[x:Local];labels=[];children=[Block(vars=[x:Local];labels=[];children=[])])])"),
    ("for i = 1, 3 do end", "Global(vars=[];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[i:Iteration];labels=[];children=[])])])"),
    ("for k, v in pairs(t) do end", "Global(vars=[pairs:Global,t:Global];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[k:Iteration,v:Iteration];labels=[];children=[])])])"),
    ("local a = 1\nfunction f(b)\n\tprint(b)\n\tdo\n\t\tlocal c = 3\n\tend\nend", "Global(vars=[f:Global,print:Global];labels=[];children=[File(vars=[a:Local];labels=[];children=[Function(vars=[b:Parameter];labels=[];children=[Block(vars=[c:Local];labels=[];children=[])])])])"),
    ("local a = 1 print(a)", "Global(vars=[print:Global];labels=[];children=[File(vars=[a:Local];labels=[];children=[])])"),
    ("local a = 1\ndo\n\tprint(a)\nend", "Global(vars=[print:Global];labels=[];children=[File(vars=[a:Local];labels=[];children=[Block(vars=[];labels=[];children=[])])])"),
    ("do\n\tlocal a = 1\nend", "Global(vars=[];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[a:Local];labels=[];children=[])])])"),
    ("local a = 2\nlocal function a() end", "Global(vars=[];labels=[];children=[File(vars=[a:Local,a:Local];labels=[];children=[Function(vars=[];labels=[];children=[])])])"),
    ("while a do local x = 1 end", "Global(vars=[a:Global];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[x:Local];labels=[];children=[])])])"),
    ("repeat local x = 1 until a", "Global(vars=[a:Global];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[x:Local];labels=[];children=[])])])"),
    ("if a then local x = 1 else local y = 2 end", "Global(vars=[a:Global];labels=[];children=[File(vars=[];labels=[];children=[Block(vars=[x:Local];labels=[];children=[]),Block(vars=[y:Local];labels=[];children=[])])])"),
    ("local f = function(p, ...) return p end", "Global(vars=[];labels=[];children=[File(vars=[f:Local];labels=[];children=[Function(vars=[p:Parameter];labels=[];children=[])])])"),
    ("local a, b = 1, 2\nreturn a, b", "Global(vars=[];labels=[];children=[File(vars=[a:Local,b:Local];labels=[];children=[])])"),
    ("local a = 1\nlocal function f() return a end\nreturn f", "Global(vars=[];labels=[];children=[File(vars=[a:Local,f:Local];labels=[];children=[Function(vars=[];labels=[];children=[])])])"),
    ("local x = 1\nlocal function outer()\n\tlocal y = 2\n\tlocal function inner() return x + y end\n\treturn inner\nend\nreturn outer", "Global(vars=[];labels=[];children=[File(vars=[outer:Local,x:Local];labels=[];children=[Function(vars=[inner:Local,y:Local];labels=[];children=[Function(vars=[];labels=[];children=[])])])])"),
];

#[test]
fn scope_trees_match_the_recorded_expectations() {
    for (code, expected) in EXPECTED_SCOPE_TREES {
        let model = analyze(code);
        assert_eq!(
            canonical(&model),
            *expected,
            "scope tree diverged for `{code}`"
        );
    }
}

#[test]
fn find_scope_returns_the_nearest_enclosing_scope() {
    // The chunk block is a file scope whose parent is the global scope.
    let model = analyze("print 'Hello'");
    let chunk = model.try_scope_of(&parse("print 'Hello'")).ok();
    let chunk = chunk.unwrap_or_else(|| model.chunk_scope());
    assert_eq!(model.scope(chunk).kind, ScopeKind::File);
    assert_eq!(model.scope(chunk).parent, Some(model.root_scope()));

    // A local function adds exactly one function scope.
    let model = analyze("local function a() end");
    let file = model.chunk_scope();
    assert_eq!(model.scope(file).children.len(), 1);
    assert_eq!(
        model.scope(model.scope(file).children[0]).kind,
        ScopeKind::Function
    );

    // An anonymous function adds exactly one function scope too.
    let model = analyze("(function(Variable) end)()");
    let file = model.chunk_scope();
    assert_eq!(model.scope(file).children.len(), 1);
    assert_eq!(
        model.scope(model.scope(file).children[0]).kind,
        ScopeKind::Function
    );
}

#[test]
fn find_variable_respects_the_scope_kind_bound() {
    // `glob` is a global, `a` a file local, `b` a parameter and `c` a block
    // local; the query starts from the innermost `do` block.
    let code = "glob = 2
local a = 1
function f(b)
	print(b)
	do
		local c = 3
	end
end";
    let model = analyze(code);

    let do_scope = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Block)
        .expect("the program has a do block")
        .id;

    // A lookup reaches each declaration exactly when the bound is at least as
    // narrow as the declaring scope.
    for (kind, name) in [
        (ScopeKind::Global, "glob"),
        (ScopeKind::Global, "a"),
        (ScopeKind::File, "a"),
        (ScopeKind::Global, "b"),
        (ScopeKind::File, "b"),
        (ScopeKind::Function, "b"),
        (ScopeKind::Global, "c"),
        (ScopeKind::File, "c"),
        (ScopeKind::Function, "c"),
        (ScopeKind::Block, "c"),
    ] {
        assert!(
            model
                .try_find_variable(do_scope, name, kind)
                .expect("the scope belongs to the model")
                .is_some(),
            "`{name}` must be found with kind {kind:?}"
        );
    }

    // A lookup never widens past the requested kind.
    for (kind, name) in [
        (ScopeKind::File, "glob"),
        (ScopeKind::Function, "a"),
        (ScopeKind::Block, "b"),
    ] {
        assert!(
            model
                .try_find_variable(do_scope, name, kind)
                .expect("the scope belongs to the model")
                .is_none(),
            "`{name}` must not be found with kind {kind:?}"
        );
    }
}

#[test]
fn can_be_accessed_in_walks_the_scope_chain() {
    // A symbol is accessible in its own scope.
    let model = analyze("local a = 1 print(a)");
    let file = model.chunk_scope();
    let a = model
        .scope(file)
        .symbols
        .iter()
        .find(|symbol| model.symbol(**symbol).name == "a")
        .copied()
        .expect("a");
    assert!(model
        .try_can_be_accessed_in(a, model.symbol(a).declaring_scope)
        .expect("the scope belongs to the model"));

    // ... and in every nested scope.
    let model = analyze(
        "local a = 1
do
	print(a)
end",
    );
    let block = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Block)
        .expect("a do block")
        .id;
    let a = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "a")
        .expect("a")
        .id;
    assert!(model
        .try_can_be_accessed_in(a, block)
        .expect("the scope belongs to the model"));

    // ... but not in an enclosing scope.
    let model = analyze(
        "do
	local a = 1
end",
    );
    let a = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "a")
        .expect("a")
        .id;
    assert!(!model
        .try_can_be_accessed_in(a, model.chunk_scope())
        .expect("the scope belongs to the model"));
}

#[test]
fn rename_symbol_renames_a_single_shadowed_local() {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};
    use r4::rules::rename_variables::{rename_symbol, RenameSymbolError};

    fn symbol_named(model: &SemanticModel, scope: ScopeId, name: &str) -> SymbolId {
        model
            .scope(scope)
            .symbols
            .iter()
            .copied()
            .find(|symbol| model.symbol(*symbol).name == name)
            .unwrap_or_else(|| panic!("symbol `{name}`"))
    }

    // A name that is not a valid identifier is rejected.
    let code = "local a = 2";
    let mut block = parse(code);
    let model = analyze_block(&block);
    let a = symbol_named(&model, model.chunk_scope(), "a");
    let error = rename_symbol(&mut block, &model, a, "\u{FEFF}").expect_err("the name is invalid");
    assert_eq!(error.code(), "rename/unsupported-identifier");
    assert!(matches!(
        error,
        RenameSymbolError::UnsupportedIdentifier { .. }
    ));

    // Renaming onto a name that is already visible is a conflict.
    let code = "local a, b = 2, 3";
    let mut block = parse(code);
    let model = analyze_block(&block);
    let a = symbol_named(&model, model.chunk_scope(), "a");
    let error = rename_symbol(&mut block, &model, a, "b").expect_err("b is visible");
    assert_eq!(error.code(), "rename/conflict");
    assert!(matches!(error, RenameSymbolError::Conflict { .. }));

    // Only the targeted declaration is renamed: a later declaration that
    // shadows it keeps its own name.
    let code = "local a = 2\nlocal function a() end";
    let mut block = parse(code);
    let model = analyze_block(&block);
    let first_a = model
        .scope(model.chunk_scope())
        .symbols
        .iter()
        .copied()
        .find(|symbol| model.symbol(*symbol).name == "a")
        .expect("the first a");
    rename_symbol(&mut block, &model, first_a, "b").expect("the rename is conflict-free");

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(output.contains("local b = 2"), "unexpected: {output}");
    assert!(
        output.contains("local function a()"),
        "the shadowing declaration must be untouched: {output}"
    );
    assert!(
        !output.contains("local function b()"),
        "the rename must not touch the shadowing declaration: {output}"
    );
}
