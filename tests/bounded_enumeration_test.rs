//! Bounded-exhaustive program enumeration (small-scope).
//!
//! Random fuzzing and a hand corpus sample the space of programs; they do not
//! cover it. This test enumerates *every* program up to a small statement bound
//! over a fixed statement pool (plus curated Lua 5.2 goto and typed programs)
//! and asserts the full semantic/CFG invariant set on each:
//!
//! * resolution: every identifier reference binds to the nearest visible
//!   declaration, recomputed here by a brute-force positional scan that uses
//!   the opposite method to the analyzer — it collects every declaration on the
//!   scope chain and orders it by `(scope distance, visibility offset)` instead
//!   of resolving incrementally as it walks;
//! * scoping laws: a binding is always accessible from the scope that uses it,
//!   a bounded lookup never returns a symbol that is not visible from the query
//!   scope, and a `goto` always targets a label in its own function;
//! * model arenas: symbol/scope ids match their index, scopes nest symmetrically,
//!   every symbol is declared in the scope that lists it, and the recorded
//!   read/write counts agree with the def-use list;
//! * provenance: every block carries a structural identity;
//! * CFG: `verify_invariants` (edges, terminators, dominator/liveness structure);
//! * liveness and constant propagation both succeed.

use std::collections::HashMap;

use r4::cfg::analysis::{ConstantPropagation, Liveness};
use r4::cfg::{build_chunk, validate_block_identities};
use r4::nodes::{Block, Identifier, VariableAssignment};
use r4::process::{
    DefaultVisitor, ImmutableProcessor, ImmutableVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::semantic::{AccessKind, ScopeId, ScopeKind, SemanticAnalyzer, SemanticModel, SymbolId};
use r4::Parser;

/// Statements that compose freely: no `break`/`continue`/`goto`, so any sequence
/// is a well-formed program.
const STATEMENTS: &[&str] = &[
    "local a = 1",
    "a = a + 1",
    "do local b = a end",
    "if a then local c = 1 else local c = 2 end",
    "while a do a = 0 end",
    "for i = 1, 3 do local d = i end",
    "for k, v in pairs(t) do print(k, v) end",
    "local function f(x) return x + a end",
    "local function g() return f(1) end",
    "repeat a = a - 1 until a <= 0",
    "print(a, b, c)",
    "local e = { 1, k = 2, [a] = 3 }",
];

/// Valid Lua 5.2 programs with labels and gotos (curated so labels never
/// collide when combined).
const LUA52_PROGRAMS: &[&str] = &[
    "::top::\nlocal a = 1\ngoto top",
    "do goto done end\n::done::\nreturn 1",
    "for i = 1, 3 do if i > 1 then goto continue end\n::continue:: end",
    "local x = 1\nif x then goto skip end\n::skip::\nreturn x",
    "::a::\n::b::\ngoto a",
    "do local y = 1 goto out end\n::out::\nreturn 0",
];

/// Typed programs exercising declarations, annotations, generics and strict
/// resolution.
const TYPED_PROGRAMS: &[&str] = &[
    "type T = number\nlocal x: T = 1\nreturn x",
    "type Pair<A, B> = { first: A, second: B }\nlocal p: Pair<number, string> = nil\nreturn p",
    "export type Public = { value: number }\nreturn { value = 1 }",
    "type Fn = (number, string) -> boolean\nlocal f: Fn = g\nreturn f",
    "type U = number | string\ntype O = number?\nlocal u: U = 1\nlocal o: O = nil\nreturn u, o",
    "local function id<T>(x: T): T return x end\nreturn id(1)",
];

/// Programs whose resolution hinges on ordering, shadowing or synthesized
/// declarations, checked by the brute-force differential below.
const RESOLUTION_CASES: &[&str] = &[
    "local x = x\nreturn x",
    "local x = 1\nlocal x = x + 1\nreturn x",
    "do print(x) local x = 1 end",
    "local x = 1\ndo local x = x end\nreturn x",
    "local function f() return f end\nreturn f",
    "local a = 1\nlocal function f(a) return a end\nreturn f",
    "for i = 1, 3 do local i = i end",
    "local t = {}\nfunction t:m() return self end\nreturn t",
    "local a, b = 1, 2\ndo local b = a return b end",
    "local a = 1\nlocal b = a\ndo local a = 2 return b + a end",
];

fn luau_programs() -> Vec<String> {
    let mut programs = Vec::new();
    for single in STATEMENTS {
        programs.push(format!("{single}\nreturn 0"));
    }
    for first in STATEMENTS {
        for second in STATEMENTS {
            programs.push(format!("{first}\n{second}\nreturn 0"));
        }
    }
    for first in STATEMENTS {
        for second in STATEMENTS {
            for third in STATEMENTS {
                programs.push(format!("{first}\n{second}\n{third}\nreturn 0"));
            }
        }
    }
    programs
}

fn assert_model_invariants(model: &SemanticModel) {
    for (index, symbol) in model.symbols.iter().enumerate() {
        assert_eq!(symbol.id.index(), index, "symbol id matches its index");
        assert!(
            symbol.declaring_scope.index() < model.scopes.len(),
            "declaring scope of `{}` exists",
            symbol.name
        );
        for scope in &symbol.capturing_scopes {
            assert!(
                scope.index() < model.scopes.len(),
                "capturing scope of `{}` exists",
                symbol.name
            );
        }

        let uses = model.uses_of(symbol.id);
        let reads = uses
            .iter()
            .filter(|usage| usage.kind == AccessKind::Read)
            .count();
        let writes = uses
            .iter()
            .filter(|usage| usage.kind == AccessKind::Write)
            .count();
        let declarations = uses
            .iter()
            .filter(|usage| usage.kind == AccessKind::Declaration)
            .count();
        assert_eq!(reads, symbol.read_count, "read count of `{}`", symbol.name);
        assert_eq!(
            declarations + writes,
            symbol.write_count,
            "write count (declaration + writes) of `{}`",
            symbol.name
        );
        for usage in uses {
            assert_eq!(usage.symbol, symbol.id, "use belongs to its symbol");
            assert!(
                usage.scope.index() < model.scopes.len(),
                "use scope of `{}` exists",
                symbol.name
            );
        }
    }

    for (index, scope) in model.scopes.iter().enumerate() {
        assert_eq!(scope.id.index(), index, "scope id matches its index");
        for &child in &scope.children {
            assert_eq!(
                model.scope(child).parent,
                Some(scope.id),
                "parent/child scopes are symmetric"
            );
        }
        for &symbol in &scope.symbols {
            assert_eq!(
                model.symbol(symbol).declaring_scope,
                scope.id,
                "a symbol is declared in the scope listing it"
            );
        }
    }

    for binding in &model.goto_bindings {
        assert!(
            binding.label.index() < model.labels.len(),
            "a goto binding targets an existing label"
        );
    }
}

/// The source offset at which each declared symbol becomes visible.
///
/// A `local` becomes visible only after its whole declaration statement, so
/// `local x = x` binds the initializer to the outer `x`; every other declaration
/// (parameters, iteration variables, `local function` names) is visible from its
/// own identifier. Globals are visible everywhere, and a synthesized declaration
/// without a token (for example the implicit `self`) is visible from the start
/// of its scope.
struct Visibilities {
    starts: HashMap<SymbolId, usize>,
}

impl Visibilities {
    fn start_of(&self, symbol: SymbolId) -> usize {
        self.starts.get(&symbol).copied().unwrap_or(0)
    }
}

fn collect_visibilities(code: &str, block: &Block, model: &SemanticModel) -> Visibilities {
    struct Collector<'a> {
        code: &'a str,
        model: &'a SemanticModel,
        starts: HashMap<SymbolId, usize>,
    }

    impl NodeProcessor for Collector<'_> {
        fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
            // A `local` becomes visible after its whole statement, so the last
            // token of its values is the boundary (`local x = x` therefore
            // binds the initializer to the outer `x`). The program is parsed
            // with tokens preserved, so a missing position is a defect, not a
            // case to paper over.
            let end = assignment.mutate_last_token().get_end().unwrap_or_else(|| {
                panic!(
                    "a local declaration has no positioned last token in `{}`",
                    self.code
                )
            });
            for variable in assignment.iter_variables() {
                if let Ok(symbol) = self.model.try_declared_symbol(variable.get_identifier()) {
                    self.starts.insert(symbol, end);
                }
            }
        }
    }

    // The statement boundary is read through `mutate_last_token`, so collect it
    // from a scratch copy to leave the analyzed tree untouched.
    let mut scratch = block.clone();
    let mut collector = Collector {
        code,
        model,
        starts: HashMap::new(),
    };
    DefaultVisitor::visit_block(&mut scratch, &mut collector);

    for symbol in &model.symbols {
        if collector.starts.contains_key(&symbol.id) {
            continue;
        }
        let start = model
            .uses_of(symbol.id)
            .iter()
            .find(|usage| usage.kind == AccessKind::Declaration)
            .and_then(|usage| usage.token)
            .map(|token| token.start)
            .unwrap_or(0);
        collector.starts.insert(symbol.id, start);
    }

    Visibilities {
        starts: collector.starts,
    }
}

/// Returns whether `ancestor` is `scope` itself or one of its enclosing scopes.
fn is_ancestor_or_self(model: &SemanticModel, ancestor: ScopeId, scope: ScopeId) -> bool {
    let mut current = Some(scope);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = model.scope(id).parent;
    }
    false
}

/// Resolves `name` at `position` from `scope` by brute force: the nearest scope
/// on the chain that declares `name` before the position wins, and within that
/// scope the latest such declaration wins.
fn brute_force_symbol(
    model: &SemanticModel,
    visibilities: &Visibilities,
    scope: ScopeId,
    name: &str,
    position: usize,
) -> Option<SymbolId> {
    let mut current = Some(scope);
    while let Some(scope_id) = current {
        let mut best: Option<SymbolId> = None;
        for &symbol in &model.scope(scope_id).symbols {
            if model.symbol(symbol).name != name || visibilities.start_of(symbol) > position {
                continue;
            }
            best = match best {
                Some(previous)
                    if (visibilities.start_of(previous), previous.0)
                        >= (visibilities.start_of(symbol), symbol.0) =>
                {
                    Some(previous)
                }
                _ => Some(symbol),
            };
        }
        if best.is_some() {
            return best;
        }
        current = model.scope(scope_id).parent;
    }
    None
}

/// Asserts the resolution differential and the scoping laws for one analyzed
/// program, and returns how many reference resolutions were compared. The AST
/// must have been parsed with tokens preserved.
fn assert_resolution_is_sound(code: &str, block: &Block, model: &SemanticModel) -> usize {
    let visibilities = collect_visibilities(code, block, model);
    let mut checked = 0;

    // The source name of every identifier, keyed by its offset, so an
    // occurrence's name is read from the program rather than from the model.
    struct Names {
        map: HashMap<usize, String>,
    }
    impl ImmutableProcessor for Names {
        fn process_identifier(&mut self, identifier: &Identifier) {
            if let Some(token) = identifier.get_token() {
                if let Some(start) = token.get_start() {
                    self.map.insert(start, identifier.get_name().clone());
                }
            }
        }
    }
    let mut names = Names {
        map: HashMap::new(),
    };
    Walk::visit_block(block, &mut names);

    for symbol in &model.symbols {
        for usage in model.uses_of(symbol.id) {
            // Law: a binding is always accessible from the scope that uses it.
            assert!(
                model
                    .try_can_be_accessed_in(symbol.id, usage.scope)
                    .expect("the use scope belongs to the model"),
                "`{}` is not accessible from its own use site in `{code}`",
                symbol.name
            );

            let Some(token) = usage.token else {
                // Only synthesized declarations (for example the implicit
                // `self`) lack a token. A *reference* without a position could
                // not be checked, so it must not be silently skipped.
                assert_eq!(
                    usage.kind,
                    AccessKind::Declaration,
                    "a reference of `{}` has no source position in `{code}`",
                    symbol.name
                );
                continue;
            };

            let name = names
                .map
                .get(&token.start)
                .unwrap_or_else(|| panic!("no identifier at offset {} in `{code}`", token.start));
            assert_eq!(
                name, &symbol.name,
                "the symbol does not match the occurrence at offset {} in `{code}`",
                token.start
            );

            // Law: a bounded lookup only returns a symbol visible from the query
            // scope, and it finds the declaration this occurrence binds to.
            let found = model
                .try_find_variable(usage.scope, name, ScopeKind::Global)
                .expect("the use scope belongs to the model")
                .unwrap_or_else(|| panic!("`{name}` is used but not declared in `{code}`"));
            assert!(
                model
                    .try_can_be_accessed_in(found, usage.scope)
                    .expect("both scopes belong to the model"),
                "the lookup of `{name}` returned an invisible symbol in `{code}`"
            );

            if usage.kind == AccessKind::Declaration {
                continue;
            }

            // Differential: the reference must bind to the symbol the
            // brute-force scan finds.
            let expected = brute_force_symbol(model, &visibilities, usage.scope, name, token.start)
                .unwrap_or_else(|| panic!("no visible `{name}` for its use in `{code}`"));
            assert_eq!(
                expected, symbol.id,
                "the reference to `{name}` at offset {} resolves to the wrong symbol in `{code}`",
                token.start
            );
            checked += 1;
        }
    }

    // Law: a `goto` targets a label in its own function, visible from the goto.
    for binding in &model.goto_bindings {
        let label = model.label(binding.label);
        assert_eq!(
            label.function_scope, binding.site.function_scope,
            "a goto crosses a function boundary in `{code}`"
        );
        assert!(
            is_ancestor_or_self(model, label.scope, binding.site.scope),
            "a goto targets a label that is not visible from it in `{code}`"
        );
    }

    checked
}

fn assert_program_invariants(code: &str, lua52: bool) {
    let parser = if lua52 {
        Parser::default().preserve_tokens().lua52()
    } else {
        Parser::default().preserve_tokens()
    };

    let block = parser
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    assert_model_invariants(&model);
    assert_resolution_is_sound(code, &block, &model);

    let mut identified = block.clone();
    validate_block_identities(&mut identified)
        .unwrap_or_else(|error| panic!("provenance of `{code}` is invalid: {error}"));

    let cfg = build_chunk(&block, &model)
        .unwrap_or_else(|error| panic!("failed to build the CFG of `{code}`: {error}"));
    cfg.verify_invariants()
        .unwrap_or_else(|error| panic!("CFG invariants of `{code}` are invalid: {error:?}"));
    Liveness::analyze(&cfg, &model)
        .unwrap_or_else(|error| panic!("liveness of `{code}` failed: {error}"));
    ConstantPropagation::analyze(&cfg, &model)
        .unwrap_or_else(|error| panic!("constant propagation of `{code}` failed: {error}"));
}

#[test]
fn every_bounded_luau_program_holds_the_invariants() {
    for code in luau_programs() {
        assert_program_invariants(&code, false);
    }
}

#[test]
fn every_bounded_lua52_program_holds_the_invariants() {
    for code in LUA52_PROGRAMS {
        assert_program_invariants(code, true);
    }
}

#[test]
fn every_typed_program_holds_the_invariants() {
    for code in TYPED_PROGRAMS {
        assert_program_invariants(code, false);
    }
}

#[test]
fn resolution_cases_agree_with_the_brute_force_resolver() {
    for code in RESOLUTION_CASES {
        assert_program_invariants(code, false);
    }
}

#[test]
fn resolution_differential_checks_real_references() {
    // Guards against the differential silently comparing nothing.
    let mut checked = 0;
    for code in RESOLUTION_CASES {
        let block = Parser::default()
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
        checked += assert_resolution_is_sound(code, &block, &model);
    }
    assert!(
        checked >= 20,
        "the differential must compare real references, compared {checked}"
    );
}
