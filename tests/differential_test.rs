//! Differential verification against the published `darklua` crate.
//!
//! Each test feeds identical source to `r4` and to the published
//! `darklua_core` crate in-process and compares the `ReadableLuaGenerator`
//! output. Both engines parse into their own ASTs, run their own rules, and
//! generate their own readable output, so the comparison is end-to-end.
//!
//! The suite covers the subset of r4 rules with a darklua_core equivalent:
//!
//! | r4 pass | darklua_core rule |
//! |---|---|
//! | `convert_square_root_call` | `ConvertSquareRootCall` |
//! | `group_local` | `GroupLocalAssignment` |
//! | `remove_assertions` | `RemoveAssertions` |
//! | `remove_call_parens` | `RemoveFunctionCallParens` |
//! | `remove_compound_assign` | `RemoveCompoundAssignment` |
//! | `remove_empty_do` | `RemoveEmptyDo` |
//! | `remove_if_expression` | `RemoveIfExpression` |
//! | `remove_interpolated_string` | `RemoveInterpolatedString` |
//! | `remove_method_call` | `RemoveMethodCall` |
//! | `remove_nil_declarations` | `RemoveNilDeclaration` |
//! | `remove_types` | `RemoveTypes` |
//! | `remove_unused_variables` | `RemoveUnusedVariable` |
//! | `remove_floor_division` | `RemoveFloorDivision` |
//! | `rename_variables` | `RenameVariables` |
//! | `convert_index_to_field` | `ConvertIndexToField` |
//! | `convert_luau_number` | `ConvertLuauNumber` |
//! | `remove_unreachable_code` (`while false`) | `RemoveUnusedWhile` |
//!
//! Rules with no darklua equivalent (`remove_dead_stores`,
//! `remove_unreachable_code` at large, `remove_unused_labels`,
//! `hoist_loop_invariants`) and the require/bundle surface (r4 exposes a
//! different, model-first API that is not a structural port of darklua's
//! configuration-driven `ConvertRequire`/`Bundle`) are intentionally excluded.

use darklua_core::generator::{LuaGenerator as DarkluaGenerator, ReadableLuaGenerator};
use darklua_core::rules::{ContextBuilder, Rule};
use darklua_core::Resources;

use r4::generator::{LuaGenerator as R4Generator, ReadableLuaGenerator as R4ReadableLuaGenerator};
use r4::semantic::SemanticModel;
use r4::Parser as R4Parser;

/// Runs a darklua rule on `code` and returns the readable-generated output.
fn darklua_apply(rule: &dyn Rule, code: &str) -> String {
    let resources = Resources::from_memory();
    let mut block = darklua_core::Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("darklua failed to parse `{code}`: {error}"));
    let context = ContextBuilder::new("input.lua", &resources, code).build();
    rule.process(&mut block, &context)
        .unwrap_or_else(|error| panic!("darklua rule failed on `{code}`: {error}"));

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    generator.into_string()
}

/// Runs an r4 pass on `code` and returns the readable-generated output.
///
/// Fallible passes are wrapped by the caller so an error fails the test
/// instead of being ignored.
fn r4_apply(pass: impl FnOnce(&mut r4::nodes::Block, &SemanticModel), code: &str) -> String {
    let (mut block, model) = R4Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("r4 failed to parse `{code}`: {error}"));
    pass(&mut block, &model);

    let mut generator = R4ReadableLuaGenerator::default();
    generator.write_block(&block);
    generator.into_string()
}

/// Asserts byte-identical output between an r4 pass and a darklua rule.
fn assert_parity(
    code: &str,
    rule: &dyn Rule,
    pass: impl FnOnce(&mut r4::nodes::Block, &SemanticModel),
) {
    let darklua = darklua_apply(rule, code);
    let r4 = r4_apply(pass, code);
    assert_eq!(
        darklua, r4,
        "parity mismatch for:\n{code}\n--- darklua ---\n{darklua}\n--- r4 ---\n{r4}"
    );
}

#[test]
fn parity_remove_call_parens() {
    use darklua_core::rules::RemoveFunctionCallParens;
    for code in [
        "f(\"hello\")",
        "f({1})",
        "obj:method(\"x\")",
        "f(\"a\", 1)",
        "f(x)",
    ] {
        assert_parity(
            code,
            &RemoveFunctionCallParens::default(),
            r4::rules::canonicalize::remove_call_parens,
        );
    }
}

#[test]
fn parity_convert_index_to_field() {
    use darklua_core::rules::ConvertIndexToField;
    for code in [
        "return t[\"foo\"]",
        "return { [\"key\"] = 1, value = 2 }",
        "return t[1]",
        "return t[\"not valid\"]",
    ] {
        assert_parity(
            code,
            &ConvertIndexToField::default(),
            r4::rules::canonicalize::convert_index_to_field,
        );
    }
}

#[test]
fn parity_remove_empty_do() {
    use darklua_core::rules::RemoveEmptyDo;
    for code in ["do end\nprint('x')", "do do end end", "do print(1) end"] {
        assert_parity(
            code,
            &RemoveEmptyDo::default(),
            r4::rules::canonicalize::remove_empty_do,
        );
    }
}

#[test]
fn parity_remove_nil_declarations() {
    use darklua_core::rules::RemoveNilDeclaration;
    for code in [
        "local x, y = 1, nil",
        "local x = nil",
        "local a, b, c = 1, nil, nil",
    ] {
        assert_parity(
            code,
            &RemoveNilDeclaration::default(),
            r4::rules::canonicalize::remove_nil_declarations,
        );
    }
}

#[test]
fn parity_remove_types() {
    use darklua_core::rules::RemoveTypes;
    for code in [
        "local x: number = 10",
        "local function add(a: number, b: number): number return a + b end",
        "type T = number\nlocal x: T = 1",
        "local x = value :: number",
    ] {
        assert_parity(
            code,
            &RemoveTypes::default(),
            r4::rules::desugar::remove_types,
        );
    }
}

#[test]
fn parity_remove_compound_assign() {
    use darklua_core::rules::RemoveCompoundAssignment;
    for code in ["x += 1", "x ..= \"a\"", "x -= y * 2", "x += 1\nx += 2"] {
        assert_parity(
            code,
            &RemoveCompoundAssignment::default(),
            r4::rules::desugar::remove_compound_assign,
        );
    }
}

#[test]
fn parity_remove_if_expression() {
    use darklua_core::rules::RemoveIfExpression;
    for code in [
        "local x = if true then 1 else 2",
        "return if a then b else c",
        "local x = if a then 1 elseif b then 2 else 3",
    ] {
        assert_parity(
            code,
            &RemoveIfExpression::default(),
            r4::rules::desugar::remove_if_expression,
        );
    }
}

#[test]
fn parity_remove_method_call() {
    use darklua_core::rules::RemoveMethodCall;
    for code in ["obj:method(1)", "a.b:method(1, 2)"] {
        assert_parity(
            code,
            &RemoveMethodCall::default(),
            r4::rules::desugar::remove_method_call,
        );
    }
}

#[test]
fn parity_remove_floor_division() {
    use darklua_core::rules::RemoveFloorDivision;
    for code in [
        "return value // 2",
        "local x = 1\nx //= 2\nreturn x",
        "return (a + b) // (c - d)",
    ] {
        assert_parity(
            code,
            &RemoveFloorDivision::default(),
            r4::rules::desugar::remove_floor_division,
        );
    }
}

#[test]
fn parity_remove_interpolated_string() {
    use darklua_core::rules::RemoveInterpolatedString;
    for code in [
        "return `hello`",
        "return `value = {value}`",
        "return `a {1} b {2} c`",
    ] {
        assert_parity(
            code,
            &RemoveInterpolatedString::default(),
            r4::rules::desugar::remove_interpolated_string,
        );
    }
}

#[test]
fn parity_convert_square_root_call() {
    use darklua_core::rules::ConvertSquareRootCall;
    for code in [
        "return math.sqrt(4)",
        "local x = math.sqrt(value)\nreturn x",
        "return math.sqrt(a) + math.sqrt(b)",
    ] {
        assert_parity(
            code,
            &ConvertSquareRootCall::default(),
            r4::rules::canonicalize::convert_square_root_call,
        );
    }
}

#[test]
fn parity_group_local() {
    use darklua_core::rules::GroupLocalAssignment;
    for code in [
        "local a = 1\nlocal b = 2\nreturn a + b",
        "local a = 1\nlocal b = a\nreturn b",
        "local a = 1\nlocal b = 2\nlocal c = a",
    ] {
        assert_parity(
            code,
            &GroupLocalAssignment::default(),
            r4::rules::canonicalize::group_local,
        );
    }
}

#[test]
fn parity_remove_assertions() {
    use darklua_core::rules::RemoveAssertions;
    for code in [
        "assert(x)",
        "assert(x, \"msg\")",
        "local y = assert(x)",
        "return assert(x)",
    ] {
        assert_parity(code, &RemoveAssertions::default(), |block, model| {
            r4::rules::canonicalize::remove_assertions(block, model);
        });
    }
}

#[test]
fn parity_remove_unused_variables() {
    use darklua_core::rules::RemoveUnusedVariable;
    for code in [
        "local unused = 1\nreturn 2",
        "local unused = compute()\nreturn 2",
        "local a, b = compute(), 2\nreturn b",
        "local function f() end\nreturn 1",
    ] {
        assert_parity(code, &RemoveUnusedVariable::default(), |block, model| {
            r4::rules::canonicalize::remove_unused_variables(block, model)
                .expect("r4 unused-variable removal must succeed");
        });
    }
}

#[test]
fn parity_convert_luau_number() {
    use darklua_core::rules::ConvertLuauNumber;
    for code in ["return 0b1010", "return 0x12EF"] {
        assert_parity(
            code,
            &ConvertLuauNumber::default(),
            r4::rules::desugar::convert_luau_number,
        );
    }
}

#[test]
fn parity_rename_variables() {
    use darklua_core::rules::RenameVariables;
    for code in [
        "local long_name = 1\nreturn long_name",
        "local alpha = 1\nlocal beta = alpha\nreturn alpha + beta",
        "local pcall = pcall\nreturn pcall",
    ] {
        assert_parity(code, &RenameVariables::default(), |block, model| {
            r4::rules::rename_variables::rename_variables(block, model)
                .expect("r4 renaming must succeed");
        });
    }
}

#[test]
fn parity_remove_unused_while() {
    use darklua_core::rules::RemoveUnusedWhile;
    // darklua's constant-condition rule and r4's CFG reachability agree here.
    for code in [
        "while false do work() end\nafter()",
        "while false do\n    work()\nend\nafter()",
    ] {
        assert_parity(code, &RemoveUnusedWhile::default(), |block, model| {
            r4::rules::canonicalize::remove_unreachable_code(block, model)
                .expect("r4 unreachable-code removal must succeed");
        });
    }
}

/// The two engines allocate different prefixes for synthesized identifiers.
/// After normalizing exactly those generated tokens, the outputs must be
/// byte-identical, which proves the transformation is otherwise identical.
#[test]
fn synthesized_identifiers_match_after_prefix_normalization() {
    use darklua_core::rules::{RemoveCompoundAssignment, RemoveContinue};

    let normalize = |output: &str| {
        output
            .replace("__DARKLUA_VAR", "__GENERATED")
            .replace("__R4_VAR", "__GENERATED")
            .replace("__DARKLUA_CONTINUE_1", "__GENERATED")
            .replace("__R4_CONTINUE", "__GENERATED")
    };

    let compound = "t[index()] += 1";
    let darklua = darklua_apply(&RemoveCompoundAssignment::default(), compound);
    let r4 = r4_apply(r4::rules::desugar::remove_compound_assign, compound);
    assert_ne!(darklua, r4, "the generated prefixes must actually differ");
    assert_eq!(normalize(&darklua), normalize(&r4));

    let continue_code = "for i = 1, 10 do\n  if i > 5 then continue end\n  print(i)\nend";
    let darklua = darklua_apply(&RemoveContinue::default(), continue_code);
    let r4 = r4_apply(r4::rules::desugar::remove_continue, continue_code);
    assert_ne!(darklua, r4, "the generated prefixes must actually differ");
    assert_eq!(normalize(&darklua), normalize(&r4));
}

/// Audit claim B: darklua cannot prove `local math = math` is a read-only
/// alias, so it refuses to rewrite `math.sqrt`; r4 proves it and converts.
#[test]
fn square_root_through_immutable_alias_darklua_bails() {
    use darklua_core::rules::ConvertSquareRootCall;
    let code = "local math = math\nreturn math.sqrt(16)";

    let darklua = darklua_apply(&ConvertSquareRootCall::default(), code);
    assert_eq!(darklua, "local math = math\n\nreturn math.sqrt(16)\n");

    let r4 = r4_apply(r4::rules::canonicalize::convert_square_root_call, code);
    assert_eq!(r4, "local math = math\n\nreturn 16 ^ 0.5\n");
    assert_eq!(evaluate_first_return(&r4), 4.0);
}

/// Audit claim B: darklua introduces a synthetic alias for floor division when
/// `math` is shadowed by a read-only alias; r4 keeps the direct call.
#[test]
fn floor_division_through_immutable_alias_darklua_aliases() {
    use darklua_core::rules::RemoveFloorDivision;
    let code = "local math = math\nreturn value // 2";

    let darklua = darklua_apply(&RemoveFloorDivision::default(), code);
    assert_eq!(
        darklua,
        "local __DARKLUA_MATH_FLOOR = math.floor\nlocal math = math\n\nreturn __DARKLUA_MATH_FLOOR(value / 2)\n"
    );

    let r4 = r4_apply(r4::rules::desugar::remove_floor_division, code);
    assert_eq!(r4, "local math = math\n\nreturn math.floor(value / 2)\n");
}

/// Audit claim B: same for `tostring` inside an interpolated string.
#[test]
fn interpolated_string_through_immutable_alias_darklua_aliases() {
    use darklua_core::rules::RemoveInterpolatedString;
    let code = "local tostring = tostring\nreturn `value = {value}`";

    let darklua = darklua_apply(&RemoveInterpolatedString::default(), code);
    assert_eq!(
        darklua,
        "local __DARKLUA_TO_STR = tostring\nlocal tostring = tostring\n\nreturn string.format('value = %s', __DARKLUA_TO_STR(value))\n"
    );

    let r4 = r4_apply(r4::rules::desugar::remove_interpolated_string, code);
    assert_eq!(
        r4,
        "local tostring = tostring\n\nreturn string.format('value = %s', tostring(value))\n"
    );
}

/// Audit claim D-adjacent grouping: darklua's name-based tracker refuses to
/// merge `local b = ...` because a nested scope declares a different `value`;
/// r4 resolves the read to its own symbol and merges.
#[test]
fn group_local_merges_past_unrelated_shadow_darklua_conservative() {
    use darklua_core::rules::GroupLocalAssignment;
    let code = "local value = 1\nlocal a = 1\nlocal b = (function()\n  local value = 2\n  return value\nend)()\nreturn value + a + b";

    let darklua = darklua_apply(&GroupLocalAssignment::default(), code);
    assert!(
        darklua.contains("local value, a = 1, 1"),
        "darklua must stay conservative: {darklua}"
    );
    assert!(
        !darklua.contains("local value, a, b"),
        "darklua must not merge the third local: {darklua}"
    );

    let r4 = r4_apply(r4::rules::canonicalize::group_local, code);
    assert!(
        r4.contains("local value, a, b = 1, 1, (function()"),
        "r4 must merge past the unrelated shadow: {r4}"
    );
}

/// Audit claim C: darklua's pattern-based dead-code rules fold
/// `if true then return end` but cannot delete the statements after it; r4's
/// CFG reachability removes them.
#[test]
fn unreachable_code_after_constant_if_darklua_cannot_remove() {
    use darklua_core::rules::RemoveUnusedIfBranch;
    let code = "if true then return end\nprint('dead')";

    let darklua = darklua_apply(&RemoveUnusedIfBranch::default(), code);
    assert_eq!(
        darklua, "do\n    return\nend\n\nprint('dead')\n",
        "darklua is expected to leave the unreachable statement"
    );

    let r4 = r4_apply(
        |block, model| {
            r4::rules::canonicalize::remove_unreachable_code(block, model)
                .expect("r4 unreachable-code removal must succeed");
        },
        code,
    );
    assert_eq!(
        r4, "do\n    return\nend\n",
        "r4 must delete the unreachable statement"
    );
}

fn evaluate_first_return(output: &str) -> f64 {
    use r4::nodes::LastStatement;
    use r4::process::{Evaluator, LuaValue};

    let block = R4Parser::default()
        .parse(output)
        .unwrap_or_else(|error| panic!("r4 failed to re-parse `{output}`: {error}"));
    let Some(LastStatement::Return(return_statement)) = block.get_last_statement() else {
        panic!("expected a return statement in `{output}`");
    };
    let expression = return_statement
        .iter_expressions()
        .next()
        .expect("the return statement has a value");
    match Evaluator::default().evaluate(expression) {
        LuaValue::Number(value) => value,
        other => panic!("expected a number, got {other:?}"),
    }
}
