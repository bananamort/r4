use std::collections::HashMap;

use crate::nodes::{Block, Expression, FunctionCall, Prefix};
use crate::parser::Dialect;
use crate::semantic::SemanticModel;

use super::remove_call_match::{identifier_resolves_to_global, remove_call_matches, CallMatcher};

const DEBUG_LIBRARY_NAME: &str = "debug";
const PROFILE_BEGIN_FUNCTION: &str = "profilebegin";
const PROFILE_END_FUNCTION: &str = "profileend";

struct DebugProfilingMatcher;

impl CallMatcher for DebugProfilingMatcher {
    fn matches(&self, call: &FunctionCall, model: &SemanticModel) -> bool {
        let Prefix::Field(field) = call.get_prefix() else {
            return false;
        };

        let name = field.get_field().get_name();
        if name != PROFILE_BEGIN_FUNCTION && name != PROFILE_END_FUNCTION {
            return false;
        }

        let Prefix::Identifier(identifier) = field.get_prefix() else {
            return false;
        };

        identifier_resolves_to_global(model, identifier, DEBUG_LIBRARY_NAME)
    }

    fn compute_result(
        &self,
        _call: &FunctionCall,
        _reserved: &HashMap<&'static str, String>,
    ) -> Option<Expression> {
        None
    }
}

/// Removes `debug.profilebegin` and `debug.profileend` calls.
///
/// The call only matches when `debug` resolves to the global library (or an
/// immutable alias of it), so a shadowing local named `debug` is never touched.
/// Argument side effects are preserved.
///
/// `debug.profilebegin`/`profileend` only exist in Luau, so the pass refuses to
/// run for any other dialect: standard Lua has no such functions, and a program
/// may legitimately define or call them. [`Dialect::Default`] means the dialect
/// was not determined, which is not enough to prove the call is Luau's.
///
/// Nothing is removed when the program can reach globals from code the model
/// cannot see (dynamic loading, an environment change or reflection), because
/// `debug` may have been replaced by then.
pub fn remove_debug_profiling(block: &mut Block, model: &SemanticModel) -> usize {
    if model.dialect() != Dialect::Luau {
        return 0;
    }
    if model.global_environment_is_open() {
        // A loaded chunk or an environment change can replace `debug`, so the
        // call may no longer be the profiling function it was written as.
        return 0;
    }
    remove_call_matches(block, model, &DebugProfilingMatcher)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{
        analyze_lua52_with_tokens, analyze_luau_with_tokens, generate,
    };

    #[test]
    fn removes_profiling_calls_to_the_debug_global() {
        let (mut block, model) = analyze_luau_with_tokens("debug.profilebegin('x')");

        let removed = remove_debug_profiling(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("profilebegin"), "unexpected: {output}");
    }

    #[test]
    fn keeps_profiling_calls_on_a_shadowed_debug() {
        let (mut block, model) =
            analyze_luau_with_tokens("local debug = {}\ndebug.profilebegin('x')");

        let removed = remove_debug_profiling(&mut block, &model);

        assert_eq!(removed, 0, "a shadowed debug table must not match");
        assert!(generate(&block).contains("profilebegin"));
    }

    #[test]
    fn keeps_profiling_calls_outside_luau() {
        // Standard Lua's `debug` library has no `profilebegin`, so the call must
        // not be removed when the source is not Luau.
        let (mut block, model) = analyze_lua52_with_tokens("debug.profilebegin('x')");

        let removed = remove_debug_profiling(&mut block, &model);

        assert_eq!(removed, 0, "a non-Luau source must not be rewritten");
        assert!(generate(&block).contains("profilebegin"));
    }

    #[test]
    fn preserves_arguments_that_can_raise() {
        // `x + 1` raises when `x` is not a number; dropping it would remove the
        // error, so the removed call's argument must survive.
        let (mut block, model) = analyze_luau_with_tokens("debug.profilebegin(x + 1)");

        let removed = remove_debug_profiling(&mut block, &model);

        assert_eq!(removed, 1);
        let output = generate(&block);
        assert!(!output.contains("profilebegin"), "unexpected: {output}");
        assert!(
            output.contains("x + 1"),
            "the argument must be preserved: {output}"
        );
    }

    #[test]
    fn keeps_profiling_when_the_global_environment_is_open() {
        // A loaded chunk can replace `debug`, so the call must survive.
        let (mut block, model) =
            analyze_luau_with_tokens("require('./module')\ndebug.profilebegin('x')");

        let removed = remove_debug_profiling(&mut block, &model);

        assert_eq!(removed, 0);
        assert!(generate(&block).contains("profilebegin"));
    }
}
