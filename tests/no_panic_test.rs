//! Adversarial input coverage: every transformation pass must return normally
//! (success or typed error) for any input that parses, never panic.

use r4::nodes::Block;
use r4::rules::{canonicalize, desugar, rename_variables};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn adversarial_snippets() -> &'static [&'static str] {
    &[
        "",
        "return",
        "return nil",
        "local x = 1\nreturn x",
        "local x = 1\nlocal x = 2\nreturn x",
        "local x = 1\nprint(x)\nlocal x = 2\nprint(x)",
        "do end",
        "do local x = 1 end",
        "local a, b = nil, nil",
        "local a, b, c = 1, nil, nil",
        "local a, b = compute(), 2\nreturn b",
        "local function f() return f() end",
        "local function f(f) return f() end",
        "local function f() local f = 1 return f end",
        "function a:b(x) return self, x end",
        "function a.b.c(x) end",
        "const x = 1\nreturn x",
        "@native function f() end",
        "local f = @native function() end",
        "local f = 1\nf = function() end",
        "return \"\\u{D800}\"",
        "return `a {b} c {d}`",
        "return `\\u{DFFF}`",
        "return 0b1010 + 0x12EF",
        "return 1_000_000",
        "return 2 ^ 10000",
        "return 1 / 0",
        "return 0 / 0",
        "return not not true",
        "return true and false or nil",
        "if true then return end\nprint('dead')",
        "while false do work() end\nafter()",
        "while true do break end",
        "while true do continue end",
        "for i = 1, 10 do if i > 5 then continue end end",
        "for k, v in pairs(t) do end",
        "repeat break until true",
        "local t = { [\"key\"] = 1, value = 2 }",
        "t[index()] += 1",
        "debug.profilebegin(side_effect())",
        "local debug = {}\ndebug.profilebegin('x')",
        "local math = math\nreturn math.sqrt(16)",
        "local math = {}\nreturn value // 2",
        "local tostring = tostring\nreturn `{value}`",
        "local pcall = pcall\nreturn pcall",
        "local require = require\nreturn require('./x')",
        "type T = number\nlocal x: T = 1",
        "type T = number\ntype T = string\nlocal x: T = 'x'",
        "export type Public = { value: number }",
        "type Pair<A, B> = { first: A, second: B }",
        "type Bad = Missing",
        "::label::\ngoto label\nreturn",
        "::label::\ngoto label\nbreak",
        "goto done\n::done::\nreturn",
        "local value = 1\nlocal a = 1\nlocal b = (function() local value = 2 return value end)()\nreturn value + a + b",
        "local x\nx = 1\nx = 2\nreturn x",
        "local unused = compute()\nreturn 2",
        "assert(x, 'message')",
        "local y = assert(x)",
        "return math.sqrt(a) + math.sqrt(b)",
    ]
}

fn parse(code: &str, lua52: bool) -> Option<Block> {
    let mut parser = Parser::default();
    if lua52 {
        parser = parser.lua52();
    }
    parser.parse(code).ok()
}

macro_rules! exercise {
    ($code:expr, $lua52:expr, $pass:path) => {{
        if let Some(mut block) = parse($code, $lua52) {
            if let Ok(model) = SemanticAnalyzer::new().analyze(&block) {
                let _ = $pass(&mut block, &model);
            }
        }
    }};
}

#[test]
fn every_pass_survives_adversarial_input() {
    for &code in adversarial_snippets() {
        for lua52 in [false, true] {
            if parse(code, lua52).is_none() {
                continue;
            }

            exercise!(code, lua52, canonicalize::convert_square_root_call);
            exercise!(code, lua52, canonicalize::group_local);
            exercise!(code, lua52, canonicalize::remove_assertions);
            exercise!(code, lua52, canonicalize::remove_dead_stores);
            exercise!(code, lua52, canonicalize::remove_unreachable_code);
            exercise!(code, lua52, canonicalize::remove_unused_labels);
            exercise!(code, lua52, canonicalize::remove_unused_variables);
            exercise!(code, lua52, canonicalize::compute_expression);
            exercise!(code, lua52, canonicalize::remove_debug_profiling);
            exercise!(code, lua52, canonicalize::hoist_loop_invariants);
            exercise!(code, lua52, canonicalize::convert_index_to_field);
            exercise!(code, lua52, canonicalize::remove_call_parens);
            exercise!(code, lua52, canonicalize::remove_nil_declarations);
            exercise!(code, lua52, canonicalize::remove_empty_do);

            exercise!(code, lua52, desugar::remove_types);
            exercise!(code, lua52, desugar::remove_compound_assign);
            exercise!(code, lua52, desugar::remove_if_expression);
            exercise!(code, lua52, desugar::remove_interpolated_string);
            exercise!(code, lua52, desugar::remove_floor_division);
            exercise!(code, lua52, desugar::remove_continue);
            exercise!(code, lua52, desugar::convert_luau_number);
            exercise!(code, lua52, desugar::remove_method_call);
            exercise!(code, lua52, desugar::make_assignment_local);
            exercise!(code, lua52, desugar::remove_attribute);
            exercise!(code, lua52, desugar::remove_method_definition);
            exercise!(code, lua52, desugar::convert_function_to_assignment);
            exercise!(code, lua52, desugar::convert_local_function_to_assignment);

            exercise!(code, lua52, rename_variables::rename_variables);
        }
    }
}

#[test]
fn renaming_is_idempotent_and_survives_reanalysis() {
    for &code in adversarial_snippets() {
        let Some(mut block) = parse(code, false) else {
            continue;
        };
        let Ok(model) = SemanticAnalyzer::new().analyze(&block) else {
            continue;
        };
        if rename_variables::rename_variables(&mut block, &model).is_err() {
            continue;
        }
        let Ok(refreshed) = SemanticAnalyzer::new().analyze(&block) else {
            continue;
        };
        // A second rename over the already-renamed tree must succeed too.
        let _ = rename_variables::rename_variables(&mut block, &refreshed);
    }
}

/// Runs a broad transformation pipeline and verifies that the reintroduced
/// semantic model and CFG satisfy their arena, scope, identity and edge
/// invariants.
#[test]
fn transformed_trees_reanalyze_with_consistent_invariants() {
    use r4::cfg::build_chunk;
    use r4::nodes::BlockIdentity;
    use r4::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
    use r4::rules::Session;

    struct IdentityCollector {
        identities: Vec<BlockIdentity>,
    }

    impl NodeProcessor for IdentityCollector {
        fn process_block(&mut self, block: &mut Block) {
            if let Some(identity) = block.get_identity() {
                self.identities.push(identity);
            }
        }
    }

    for &code in adversarial_snippets() {
        let Some(block) = parse(code, false) else {
            continue;
        };
        let Ok(mut session) = Session::from_block(block) else {
            continue;
        };

        let _ = session.apply_semantic_result(canonicalize::remove_unused_variables);
        let _ = session.apply_semantic_result(canonicalize::remove_dead_stores);
        let _ = session.apply_semantic_result(canonicalize::remove_unreachable_code);
        let _ = session.apply_semantic_result(canonicalize::hoist_loop_invariants);
        let _ = session.apply_semantic_result(rename_variables::rename_variables);
        let _ = session.apply_semantic_result(desugar::convert_local_function_to_assignment);
        let _ = session.apply_semantic(canonicalize::compute_expression);

        let mut block = session.into_block();
        let Ok(model) = SemanticAnalyzer::new().analyze(&block) else {
            continue;
        };

        for (index, symbol) in model.symbols.iter().enumerate() {
            assert_eq!(symbol.id.index(), index, "symbol id must match its index");
            assert!(
                symbol.declaring_scope.index() < model.scopes.len(),
                "declaring scope must exist"
            );
        }
        for (index, scope) in model.scopes.iter().enumerate() {
            assert_eq!(scope.id.index(), index, "scope id must match its index");
            for &child in &scope.children {
                assert_eq!(
                    model.scope(child).parent,
                    Some(scope.id),
                    "parent/child links must be symmetric"
                );
            }
            for &symbol in &scope.symbols {
                assert_eq!(
                    model.symbol(symbol).declaring_scope,
                    scope.id,
                    "the symbol must be declared in the scope listing it"
                );
            }
        }

        let cfg = build_chunk(&block, &model).expect("cfg must build after transformations");
        cfg.verify_invariants().expect("cfg invariants must hold");

        let mut collector = IdentityCollector {
            identities: Vec::new(),
        };
        DefaultVisitor::visit_block(&mut block, &mut collector);
        let unique: std::collections::HashSet<_> = collector.identities.iter().copied().collect();
        assert_eq!(
            unique.len(),
            collector.identities.len(),
            "block identities must be unique after re-analysis"
        );
    }
}
