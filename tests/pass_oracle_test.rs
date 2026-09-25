//! A metamorphic oracle for every transformation pass.
//!
//! No pass is "covered" by a snapshot alone. Each pass is classified and checked
//! against a property independent of its implementation:
//!
//! * trivia-only passes must leave the tree equivalent to the input
//!   (`is_equivalent_to` ignores trivia, positions and token presence);
//! * every structural pass must produce output that re-parses, re-analyzes and
//!   rebuilds a valid control-flow graph (a lowering pass is not expected to be
//!   structurally equivalent, so re-analysis and CFG validity is the oracle).

use std::collections::HashMap;

use r4::cfg::build_chunk;
use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::{canonicalize, desugar, formatting, inject, rename_variables};
use r4::semantic::{SemanticAnalyzer, SemanticModel};
use r4::{Equivalent, Parser};

/// Valid programs that exercise scopes, captures, loops, control flow, tables,
/// types and interpolation.
const PROGRAMS: &[&str] = &[
    "local a, b = 1, 2\nreturn a + b",
    "local x = 1\ndo local x = 2\nprint(x) end\nreturn x",
    "local x = 1\nlocal function f() return x end\nreturn f()",
    "if a then local x = 1 else local x = 2 end\nreturn x",
    "while a do a = a - 1 end\nreturn a",
    "for i = 1, 10, 2 do if i % 2 == 0 then continue end end\nreturn 1",
    "for k, v in pairs(t) do print(k, v) end\nreturn t",
    "repeat a = a - 1 until a <= 0\nreturn a",
    "local t = { 1, 2, key = 3, [a] = 4 }\nt.key = 5\nreturn t",
    "return `a {b} c {d}`",
    "local value = if a then 1 elseif b then 2 else 3\nreturn value",
    "local x = 1\nx += 2\nx -= 1\nx *= 3\nreturn x",
    "print(a, b, c)",
    "local f = function(x) return x + 1 end\nreturn f(1)",
    "return #t, -a, not b",
    "type T = number\nlocal x: T = 1\nreturn x",
];

/// A pass whose output must be structurally equivalent to the input.
macro_rules! check_equivalent_pass {
    ($name:expr, $code:expr, $pass:expr) => {{
        let name = $name;
        let code = $code;
        let original = Parser::default()
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let mut block = original.clone();
        ($pass)(&mut block);
        assert!(
            original.is_equivalent_to(&block),
            "`{name}` changed the tree of `{code}`"
        );

        let output = render(&block);
        Parser::default()
            .parse(&output)
            .unwrap_or_else(|error| panic!("`{name}` output of `{code}` does not parse: {error}"));
    }};
}

/// A structural pass: it must succeed on valid input and its output must
/// re-parse, re-analyze and rebuild a valid CFG.
macro_rules! check_structural_pass {
    ($name:expr, $code:expr, $pass:expr) => {{
        let name = $name;
        let code = $code;
        let mut block = parse(code);
        let model = analyze(&block);
        ensure_ok(($pass)(&mut block, &model), name, code);
        assert_output_valid(name, code, &block);
    }};
}

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze(block: &Block) -> SemanticModel {
    SemanticAnalyzer::new()
        .analyze(block)
        .unwrap_or_else(|error| panic!("failed to analyze a program: {error}"))
}

fn render(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn assert_output_valid(pass: &str, code: &str, block: &Block) {
    let output = render(block);
    let reparsed = Parser::default().parse(&output).unwrap_or_else(|error| {
        panic!("`{pass}` output of `{code}` does not parse: {error}\n{output}")
    });
    let model = SemanticAnalyzer::new()
        .analyze(&reparsed)
        .unwrap_or_else(|error| {
            panic!("`{pass}` output of `{code}` does not analyze: {error}\n{output}")
        });
    let cfg = build_chunk(&reparsed, &model).unwrap_or_else(|error| {
        panic!("`{pass}` output of `{code}` does not lower: {error}\n{output}")
    });
    cfg.verify_invariants().unwrap_or_else(|error| {
        panic!("`{pass}` output of `{code}` has an invalid CFG: {error:?}\n{output}")
    });
}

fn ensure_ok<T: Outcome>(result: T, pass: &str, code: &str) {
    result.ensure_ok(pass, code);
}

trait Outcome {
    fn ensure_ok(self, pass: &str, code: &str);
}

impl Outcome for () {
    fn ensure_ok(self, _pass: &str, _code: &str) {}
}

impl Outcome for usize {
    fn ensure_ok(self, _pass: &str, _code: &str) {}
}

impl<T, E: std::fmt::Display> Outcome for Result<T, E> {
    fn ensure_ok(self, pass: &str, code: &str) {
        if let Err(error) = self {
            panic!("`{pass}` failed on valid input `{code}`: {error}");
        }
    }
}

#[test]
fn trivia_only_passes_preserve_the_tree() {
    for code in PROGRAMS {
        check_equivalent_pass!("remove_comments", code, |block: &mut Block| {
            formatting::remove_comments(block)
        });
        check_equivalent_pass!("remove_spaces", code, |block: &mut Block| {
            formatting::remove_spaces(block)
        });
        check_equivalent_pass!("shift_token_line", code, |block: &mut Block| {
            formatting::shift_token_line(block, 1)
        });
        check_equivalent_pass!("append_text_comment", code, |block: &mut Block| {
            formatting::append_text_comment(block, "note", formatting::AppendLocation::End)
        });
    }
}

#[test]
fn canonicalize_passes_produce_valid_output() {
    for code in PROGRAMS {
        check_structural_pass!(
            "convert_index_to_field",
            code,
            |block: &mut Block, model| { canonicalize::convert_index_to_field(block, model) }
        );
        check_structural_pass!(
            "remove_nil_declarations",
            code,
            |block: &mut Block, model| { canonicalize::remove_nil_declarations(block, model) }
        );
        check_structural_pass!("remove_empty_do", code, |block: &mut Block, model| {
            canonicalize::remove_empty_do(block, model)
        });
        check_structural_pass!("remove_call_parens", code, |block: &mut Block, model| {
            canonicalize::remove_call_parens(block, model)
        });
        check_structural_pass!("compute_expression", code, |block: &mut Block, model| {
            canonicalize::compute_expression(block, model)
        });
        check_structural_pass!("remove_assertions", code, |block: &mut Block, model| {
            canonicalize::remove_assertions(block, model)
        });
        check_structural_pass!(
            "remove_debug_profiling",
            code,
            |block: &mut Block, model| { canonicalize::remove_debug_profiling(block, model) }
        );
        check_structural_pass!(
            "remove_unused_variables",
            code,
            |block: &mut Block, model| { canonicalize::remove_unused_variables(block, model) }
        );
        check_structural_pass!("remove_unused_labels", code, |block: &mut Block, model| {
            canonicalize::remove_unused_labels(block, model)
        });
        check_structural_pass!("remove_dead_stores", code, |block: &mut Block, model| {
            canonicalize::remove_dead_stores(block, model)
        });
        check_structural_pass!(
            "remove_unreachable_code",
            code,
            |block: &mut Block, model| { canonicalize::remove_unreachable_code(block, model) }
        );
        check_structural_pass!("group_local", code, |block: &mut Block, model| {
            canonicalize::group_local(block, model)
        });
        check_structural_pass!("hoist_loop_invariants", code, |block: &mut Block, model| {
            canonicalize::hoist_loop_invariants(block, model)
        });
    }
}

#[test]
fn desugar_passes_produce_valid_output() {
    for code in PROGRAMS {
        check_structural_pass!("remove_types", code, |block: &mut Block, model| {
            desugar::remove_types(block, model)
        });
        check_structural_pass!(
            "remove_compound_assign",
            code,
            |block: &mut Block, model| { desugar::remove_compound_assign(block, model) }
        );
        check_structural_pass!("remove_if_expression", code, |block: &mut Block, model| {
            desugar::remove_if_expression(block, model)
        });
        check_structural_pass!(
            "remove_interpolated_string",
            code,
            |block: &mut Block, model| { desugar::remove_interpolated_string(block, model) }
        );
        check_structural_pass!("remove_floor_division", code, |block: &mut Block, model| {
            desugar::remove_floor_division(block, model)
        });
        check_structural_pass!("remove_continue", code, |block: &mut Block, model| {
            desugar::remove_continue(block, model)
        });
        check_structural_pass!("convert_luau_number", code, |block: &mut Block, model| {
            desugar::convert_luau_number(block, model)
        });
        check_structural_pass!("remove_method_call", code, |block: &mut Block, model| {
            desugar::remove_method_call(block, model)
        });
        check_structural_pass!("make_assignment_local", code, |block: &mut Block, model| {
            desugar::make_assignment_local(block, model)
        });
        check_structural_pass!("remove_attribute", code, |block: &mut Block, model| {
            desugar::remove_attribute(block, model)
        });
        check_structural_pass!(
            "remove_method_definition",
            code,
            |block: &mut Block, model| { desugar::remove_method_definition(block, model) }
        );
        check_structural_pass!(
            "convert_function_to_assignment",
            code,
            |block: &mut Block, model| { desugar::convert_function_to_assignment(block, model) }
        );
        check_structural_pass!(
            "convert_local_function_to_assignment",
            code,
            |block: &mut Block, model| desugar::convert_local_function_to_assignment(block, model)
        );
        check_structural_pass!("rename_variables", code, |block: &mut Block, model| {
            rename_variables::rename_variables(block, model)
        });
        check_structural_pass!("inject_global_values", code, |block: &mut Block, model| {
            inject::inject_global_values(block, model, &HashMap::new())
        });
    }
}

#[test]
fn require_passes_produce_valid_output_without_requires() {
    use r4::modules::MemoryModuleProvider;
    use r4::nodes::{Expression, StringExpression};
    use r4::rules::require::{apply_path_aliases, bundle_modules, RequirePathConverter};

    struct Echo;

    impl RequirePathConverter for Echo {
        fn convert(&self, path: &str) -> Option<Expression> {
            Some(Expression::String(StringExpression::from_value(path)))
        }
    }

    for code in PROGRAMS {
        check_structural_pass!("convert_require", code, |block: &mut Block, model| {
            r4::rules::require::convert_require(block, model, &Echo)
        });
        check_structural_pass!("apply_path_aliases", code, |block: &mut Block, model| {
            apply_path_aliases(block, model, [("@pkg", "src/pkg")])
        });
        check_structural_pass!("bundle_modules", code, |block: &mut Block, model| {
            bundle_modules(block, model, &MemoryModuleProvider::new())
        });
    }
}

/// A rename must preserve the shape of the symbol graph: the same symbols with
/// the same kinds, implicitness and access counts. Only names may change, so a
/// bug that renames a syntactically fixed symbol (a colon receiver has no
/// declaration to rename) shows up as a changed shape or a spurious global.
fn symbol_shape(model: &SemanticModel) -> Vec<(&'static str, bool, usize, usize)> {
    let mut shape: Vec<_> = model
        .symbols
        .iter()
        .map(|symbol| {
            (
                symbol.kind.as_str(),
                symbol.is_implicit(),
                symbol.read_count,
                symbol.write_count,
            )
        })
        .collect();
    shape.sort_unstable();
    shape
}

/// Programs that only the rename oracle uses: colon methods, whose implicit
/// receiver must survive renaming.
const RENAME_PROGRAMS: &[&str] = &[
    "local t = {}\nfunction t:m(x) return self, x end\nreturn t",
    "local t = {}\nfunction t:m() local y = self return y end\nreturn t",
];

#[test]
fn rename_preserves_the_symbol_graph() {
    for code in PROGRAMS.iter().chain(RENAME_PROGRAMS).copied() {
        let block = parse(code);
        let model = analyze(&block);
        let before = symbol_shape(&model);

        let mut renamed = block.clone();
        ensure_ok(
            rename_variables::rename_variables(&mut renamed, &model),
            "rename_variables",
            code,
        );

        let output = render(&renamed);
        let after = symbol_shape(&analyze(&parse(&output)));

        assert_eq!(
            before, after,
            "renaming changed the symbol graph of `{code}`:\n{output}"
        );
    }
}
