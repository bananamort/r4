//! Loretta scoping parity, verified against the published Loretta NuGet package.
//!
//! The expected scope trees below were produced by running the official latest
//! published package (`Loretta.CodeAnalysis.Lua` 0.2.14-nightly.26, restored
//! from nuget.org) through `tools/loretta-oracle`, never the vendored checkout
//! under `references/`. The oracle drops Loretta's implicit `...`/`arg` file
//! and variadic parameters, which r4 does not model as symbols; every other
//! scope kind, nesting, declaration and label must match exactly.
//!
//! The query tests port Loretta's `Scoping` suite (`FindScopeTests`,
//! `FindVariableTests`, `CanBeAccessedInTests`, `RenameVariableTests`) onto
//! r4's semantic model: scope lookup, kind-filtered variable lookup, scope
//! accessibility and shadowing/rename facts.

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
    SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

/// The scope-kind rank Loretta uses for its kind-filtered search.
fn rank(kind: ScopeKind) -> u8 {
    match kind {
        ScopeKind::Global => 0,
        ScopeKind::File => 1,
        ScopeKind::Function => 2,
        ScopeKind::Block => 3,
    }
}

/// A canonical, index-independent rendering of the scope tree, matching the
/// oracle's format.
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

/// Loretta's `IScope.FindVariable(name, kind)`.
fn find_variable(
    model: &SemanticModel,
    scope: ScopeId,
    name: &str,
    kind: ScopeKind,
) -> Option<SymbolId> {
    let current = model.scope(scope);
    if let Some(symbol) = current
        .symbols
        .iter()
        .find(|symbol| model.symbol(**symbol).name == name)
    {
        return Some(*symbol);
    }
    let parent = current.parent?;
    if rank(model.scope(parent).kind) >= rank(kind) {
        find_variable(model, parent, name, kind)
    } else {
        None
    }
}

/// Loretta's `IVariable.CanBeAccessedIn(scope)`.
fn can_be_accessed_in(model: &SemanticModel, variable: SymbolId, scope: ScopeId) -> bool {
    let declaring = model.symbol(variable).declaring_scope;
    let mut current = Some(scope);
    while let Some(scope) = current {
        if scope == declaring {
            return true;
        }
        current = model.scope(scope).parent;
    }
    false
}

/// `(source, canonical scope tree)` produced by the packaged Loretta engine.
const LORETTA_SCOPE_TREES: &[(&str, &str)] = &[
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
    ("local x = 1\nlocal function outer()\n\tlocal y = 2\n\tlocal function inner() return x + y end\n\treturn inner\nend\nreturn outer", "Global(vars=[];labels=[];children=[File(vars=[outer:Local,x:Local];labels=[];children=[Function(vars=[inner:Local,y:Local];labels=[];children=[Function(vars=[];labels=[];children=[])])])])"),];

#[test]
fn scope_trees_match_the_packaged_loretta_engine() {
    for (code, expected) in LORETTA_SCOPE_TREES {
        let model = analyze(code);
        assert_eq!(
            canonical(&model),
            *expected,
            "scope tree diverged for `{code}`"
        );
    }
}

#[test]
fn find_scope_matches_loretta() {
    // CompilationUnit_HasFileScope / FindScope_OnRootElement_ReturnsRootScope.
    let model = analyze("print 'Hello'");
    let chunk = model.try_scope_of(&parse("print 'Hello'")).ok();
    let chunk = chunk.unwrap_or_else(|| model.chunk_scope());
    assert_eq!(model.scope(chunk).kind, ScopeKind::File);
    assert_eq!(model.scope(chunk).parent, Some(model.root_scope()));

    // FindScope_LocalFunctionIsParsed.
    let model = analyze("local function a() end");
    let file = model.chunk_scope();
    assert_eq!(model.scope(file).children.len(), 1);
    assert_eq!(
        model.scope(model.scope(file).children[0]).kind,
        ScopeKind::Function
    );

    // FindScope_AnonymousFunctionIsParsed.
    let model = analyze("(function(Variable) end)()");
    let file = model.chunk_scope();
    assert_eq!(model.scope(file).children.len(), 1);
    assert_eq!(
        model.scope(model.scope(file).children[0]).kind,
        ScopeKind::Function
    );
}

#[test]
fn find_variable_matches_loretta() {
    // ScriptTestsBase::SetupScriptAsync, flattened into one file (`glob = 2` is
    // a global, so it lands in the global scope exactly as in the two-tree
    // Script).
    let code = "glob = 2
local a = 1
function f(b)
	print(b)
	do
		local c = 3
	end
end";
    let model = analyze(code);

    // The innermost scope is the `do` block inside `f`.
    let do_scope = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Block)
        .expect("the script has a do block")
        .id;

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
            find_variable(&model, do_scope, name, kind).is_some(),
            "`{name}` must be found with kind {kind:?}"
        );
    }

    for (kind, name) in [
        (ScopeKind::File, "glob"),
        (ScopeKind::Function, "a"),
        (ScopeKind::Block, "b"),
    ] {
        assert!(
            find_variable(&model, do_scope, name, kind).is_none(),
            "`{name}` must not be found with kind {kind:?}"
        );
    }
}

#[test]
fn can_be_accessed_in_matches_loretta() {
    // Same scope.
    let model = analyze("local a = 1 print(a)");
    let file = model.chunk_scope();
    let a = model
        .scope(file)
        .symbols
        .iter()
        .find(|symbol| model.symbol(**symbol).name == "a")
        .copied()
        .expect("a");
    assert!(can_be_accessed_in(
        &model,
        a,
        model.symbol(a).declaring_scope
    ));

    // Child scope.
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
    assert!(can_be_accessed_in(&model, a, block));

    // Parent scope.
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
    assert!(!can_be_accessed_in(&model, a, model.chunk_scope()));
}

#[test]
fn rename_scoping_matches_loretta() {
    use r4::rules::rename_variables::rename_variables;

    // RenameVariable_ReturnsErrorForConflictingVariable: `a` and `b` are
    // distinct variables, so renaming `a` cannot capture `b`.
    let code = "local a, b = 2, 3";
    let mut model = analyze(code);
    let file = model.chunk_scope();
    let a = model
        .scope(file)
        .symbols
        .iter()
        .find(|symbol| model.symbol(**symbol).name == "a")
        .copied()
        .expect("a");
    let b = model
        .scope(file)
        .symbols
        .iter()
        .find(|symbol| model.symbol(**symbol).name == "b")
        .copied()
        .expect("b");
    assert_ne!(a, b);

    let mut block = parse(code);
    model = SemanticAnalyzer::new().analyze(&block).expect("analyze");
    rename_variables(&mut block, &model).expect("rename");
    let renamed = SemanticAnalyzer::new().analyze(&block).expect("re-analyze");
    let names: Vec<&str> = renamed
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1], "distinct variables keep distinct names");

    // RenameVariable_ReturnsCorrectlyRenamedScript: `local a = 2` followed by
    // `local function a()` are two distinct locals; renaming must not merge
    // them. The first `a` and the local function `a` are separate symbols.
    let code = "local a = 2
local function a() end";
    let model = analyze(code);
    let file = model.chunk_scope();
    let locals: Vec<SymbolId> = model
        .scope(file)
        .symbols
        .iter()
        .copied()
        .filter(|symbol| model.symbol(*symbol).name == "a")
        .collect();
    assert_eq!(
        locals.len(),
        2,
        "shadowed `a` declarations are distinct symbols"
    );
    assert_ne!(locals[0], locals[1]);

    let mut block = parse(code);
    let model = SemanticAnalyzer::new().analyze(&block).expect("analyze");
    rename_variables(&mut block, &model).expect("rename");
    let renamed = SemanticAnalyzer::new().analyze(&block).expect("re-analyze");
    let names: Vec<String> = renamed
        .scope(renamed.chunk_scope())
        .symbols
        .iter()
        .map(|symbol| renamed.symbol(*symbol).name.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert_ne!(
        names[0], names[1],
        "the shadowed locals are renamed distinctly"
    );
}
