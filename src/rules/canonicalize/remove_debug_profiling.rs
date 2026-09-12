use std::collections::HashMap;

use crate::nodes::{Block, Expression, FunctionCall, Prefix};
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
/// immutable alias of it), so a shadowing local named `debug` is never
/// touched. Argument side effects are preserved.
pub fn remove_debug_profiling(block: &mut Block, model: &SemanticModel) -> usize {
    remove_call_matches(block, model, &DebugProfilingMatcher)
}
