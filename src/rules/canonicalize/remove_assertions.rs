use std::collections::HashMap;
use std::iter;

use crate::nodes::{Block, Expression, FunctionCall, Prefix, TupleArguments};
use crate::semantic::SemanticModel;

use super::remove_call_match::{identifier_resolves_to_global, remove_call_matches, CallMatcher};

const ASSERT_FUNCTION_NAME: &str = "assert";
const SELECT_FUNCTION_NAME: &str = "select";

struct AssertMatcher;

impl CallMatcher for AssertMatcher {
    fn matches(&self, call: &FunctionCall, model: &SemanticModel) -> bool {
        match call.get_prefix() {
            Prefix::Identifier(identifier) => {
                identifier_resolves_to_global(model, identifier, ASSERT_FUNCTION_NAME)
            }
            _ => false,
        }
    }

    fn compute_result(
        &self,
        call: &FunctionCall,
        reserved: &HashMap<&'static str, String>,
    ) -> Option<Expression> {
        let expressions = call.get_arguments().clone().to_expressions();

        Some(match expressions.len() {
            0 => Expression::nil(),
            1 => expressions
                .into_iter()
                .next()
                .expect("the single argument must exist"),
            _ => FunctionCall::from_name(
                reserved
                    .get(SELECT_FUNCTION_NAME)
                    .expect("select must be reserved for multi-argument assertions")
                    .clone(),
            )
            .with_arguments(TupleArguments::from_iter(
                iter::once(Expression::from(1)).chain(expressions),
            ))
            .into(),
        })
    }

    fn reserve_globals(&self) -> &'static [&'static str] {
        &[SELECT_FUNCTION_NAME]
    }

    fn needs_reserved_globals(&self, call: &FunctionCall) -> bool {
        call.get_arguments().len() > 1
    }
}

/// Removes `assert(...)` calls.
///
/// A call is removed only when `assert` resolves to the global function or to a
/// read-only alias of it; a shadowing local or mutable alias is left alone. The
/// replacement is `nil` for zero arguments, the single argument for one, and
/// `select(1, ...)` for several. When `select` is shadowed, a collision-free
/// alias is defined at the top of the block.
///
/// Nothing is removed when the program can reach globals from code the model
/// cannot see (dynamic loading, an environment change or reflection), because
/// `assert` may have been replaced by then.
pub fn remove_assertions(block: &mut Block, model: &SemanticModel) -> usize {
    if model.global_environment_is_open() {
        // A loaded chunk or an environment change can replace `assert`, so the
        // call may no longer be the assertion it was written as.
        return 0;
    }
    remove_call_matches(block, model, &AssertMatcher)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze_with_tokens, generate, parse};

    #[test]
    fn removes_assert_statements() {
        let (mut block, model) = analyze_with_tokens("assert(true)\nreturn 1");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("assert"), "unexpected: {output}");
    }

    #[test]
    fn rewrites_multi_argument_assert_with_select() {
        let (mut block, model) = analyze_with_tokens("local x = assert(1, \"message\")\nreturn x");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("select(1, 1"), "unexpected: {output}");
    }

    #[test]
    fn keeps_shadowed_assert_calls() {
        let (mut block, model) = analyze_with_tokens(
            "local assert = function(value) return value end\nlocal x = assert(true)\nreturn x",
        );

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 0, "a non-alias shadowing local must not match");
        assert!(generate(&block).contains("assert(true)"));
    }

    #[test]
    fn keeps_assert_calls_when_the_global_is_reassigned() {
        let (mut block, model) = analyze_with_tokens(
            "assert = function(value) return value end\nlocal x = assert(true)\nreturn x",
        );

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(
            removed, 0,
            "a reassigned `assert` global no longer denotes the builtin"
        );
        assert!(generate(&block).contains("assert(true)"));
    }

    #[test]
    fn removes_assert_calls_through_a_read_only_alias() {
        let (mut block, model) =
            analyze_with_tokens("local assert = assert\nlocal x = assert(true)\nreturn x");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1, "a read-only alias must match");
        let output = generate(&block);
        assert!(output.contains("local x = true"), "unexpected: {output}");
    }

    #[test]
    fn allocates_a_collision_free_select_alias() {
        let (mut block, model) = analyze_with_tokens(
            "local __R4_SELECT = 1\nlocal select = function() end\nlocal x = assert(1, 2)\nreturn x + __R4_SELECT",
        );

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(
            output.contains("__R4_SELECT1 = select"),
            "the alias must avoid the existing name: {output}"
        );
        assert!(
            output.contains("__R4_SELECT1(1, 1, 2)"),
            "the replacement must use the fresh alias: {output}"
        );
        parse(&output);
    }

    #[test]
    fn single_argument_assert_does_not_reserve_select() {
        let (mut block, model) = analyze_with_tokens(
            "local select = function() end\nlocal x = assert(1)\nreturn x, select",
        );

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(
            !output.contains("__R4_"),
            "no alias is needed for a single argument: {output}"
        );
    }
}

#[cfg(test)]
mod argument_side_effects {
    use super::*;
    use crate::rules::test_support::{analyze_with_tokens, generate, parse};

    #[test]
    fn preserves_multiple_argument_side_effects() {
        let (mut block, model) = analyze_with_tokens("assert(a.b, compute())\nreturn 1");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("a.b"), "{output}");
        assert!(output.contains("compute()"), "{output}");
        parse(&output);
    }

    #[test]
    fn preserves_table_argument_side_effects() {
        let (mut block, model) = analyze_with_tokens("assert{ key = compute() }\nreturn 1");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(output.contains("compute()"), "{output}");
        parse(&output);
    }

    #[test]
    fn drops_side_effect_free_string_arguments() {
        let (mut block, model) = analyze_with_tokens("assert 'literal'\nreturn 1");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("assert"), "{output}");
        assert!(!output.contains("literal"), "{output}");
    }

    #[test]
    fn keeps_assertions_when_the_global_environment_is_open() {
        // Loaded source can replace `assert`, so the call must survive.
        let (mut block, model) = analyze_with_tokens("load('return 1')\nassert(value)");

        let removed = remove_assertions(&mut block, &model);

        assert_eq!(removed, 0);
        assert!(generate(&block).contains("assert"));
    }
}
