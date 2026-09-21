//! Bounded-exhaustive program enumeration (small-scope).
//!
//! Random fuzzing and a hand corpus sample the space of programs; they do not
//! cover it. This test enumerates *every* program up to a small statement bound
//! over a fixed statement pool (plus curated Lua 5.2 goto and typed programs)
//! and asserts the full semantic/CFG invariant set on each:
//!
//! * model arenas: symbol/scope ids match their index, scopes nest symmetrically,
//!   every symbol is declared in the scope that lists it, and the recorded
//!   read/write counts agree with the def-use list;
//! * provenance: every block carries a structural identity;
//! * CFG: `verify_invariants` (edges, terminators, dominator/liveness structure);
//! * liveness and constant propagation both succeed.

use r4::cfg::analysis::{ConstantPropagation, Liveness};
use r4::cfg::{build_chunk, validate_block_identities};
use r4::semantic::{AccessKind, SemanticAnalyzer, SemanticModel};
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

fn assert_program_invariants(code: &str, lua52: bool) {
    let parser = if lua52 {
        Parser::default().lua52()
    } else {
        Parser::default()
    };

    let block = parser
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    assert_model_invariants(&model);

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
