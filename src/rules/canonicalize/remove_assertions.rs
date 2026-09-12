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
pub fn remove_assertions(block: &mut Block, model: &SemanticModel) -> usize {
    remove_call_matches(block, model, &AssertMatcher)
}
