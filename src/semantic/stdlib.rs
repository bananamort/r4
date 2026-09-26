//! A curated standard-library purity model.
//!
//! [`call_is_pure`] decides whether a call invokes a known side-effect-free
//! standard-library function. It resolves the callee through the semantic model
//! by identity: the callee must be a global (or an immutable alias of one)
//! whose name is not shadowed, and the exact library path must be in the
//! curated table below. Anything not in the table is treated as effectful — the
//! model never guesses.
//!
//! [`has_side_effects`] combines this with [`Evaluator::has_side_effects_with`]
//! so optimization passes can remove or hoist standard-library calls whose
//! results are unused or loop invariant.

use crate::nodes::{FunctionCall, Identifier, Prefix};
use crate::process::Evaluator;

use super::{SemanticModel, SymbolKind};

/// Returns whether `call` invokes a known pure standard-library function.
pub fn call_is_pure(model: &SemanticModel, call: &FunctionCall) -> bool {
    if call.get_method().is_some() {
        // Method calls pass a receiver; the model does not describe them.
        return false;
    }

    let Some((root, fields)) = callee_path(call.get_prefix()) else {
        return false;
    };

    let Some(symbol) = model.resolve_identifier(root) else {
        return false;
    };
    let Some(global) = model.immutable_alias_root(symbol) else {
        return false;
    };
    let global_symbol = model.symbol(global);
    if global_symbol.kind != SymbolKind::Global {
        return false;
    }
    if global_symbol.write_count != 0 {
        // The program reassigns this global, so it no longer denotes the
        // standard-library function hidden behind its name.
        return false;
    }
    if !model.name_is_transparent_global(&global_symbol.name) {
        return false;
    }

    is_pure_path(&global_symbol.name, &fields)
}

/// Returns whether evaluating `expression` can have side effects, treating pure
/// standard-library calls as effect free.
pub fn has_side_effects(
    model: &SemanticModel,
    evaluator: &Evaluator,
    expression: &crate::nodes::Expression,
) -> bool {
    evaluator.has_side_effects_with(expression, &|call| call_is_pure(model, call))
}

fn callee_path(prefix: &Prefix) -> Option<(&Identifier, Vec<&str>)> {
    match prefix {
        Prefix::Identifier(identifier) => Some((identifier, Vec::new())),
        Prefix::Field(field) => {
            let (root, mut fields) = callee_path(field.get_prefix())?;
            fields.push(field.get_field().get_name().as_str());
            Some((root, fields))
        }
        _ => None,
    }
}

fn is_pure_path(root: &str, fields: &[&str]) -> bool {
    match fields {
        [] => matches!(
            root,
            "type"
                | "tostring"
                | "tonumber"
                | "rawequal"
                | "rawlen"
                | "rawget"
                | "getmetatable"
                | "select"
        ),
        [field] => match root {
            "math" => matches!(
                *field,
                "abs"
                    | "acos"
                    | "asin"
                    | "atan"
                    | "atan2"
                    | "ceil"
                    | "clamp"
                    | "cos"
                    | "cosh"
                    | "deg"
                    | "exp"
                    | "floor"
                    | "fmod"
                    | "frexp"
                    | "ldexp"
                    | "log"
                    | "log10"
                    | "max"
                    | "min"
                    | "modf"
                    | "noise"
                    | "pow"
                    | "rad"
                    | "round"
                    | "sign"
                    | "sin"
                    | "sinh"
                    | "sqrt"
                    | "tan"
                    | "tanh"
            ),
            "string" => matches!(
                *field,
                "byte"
                    | "char"
                    | "find"
                    | "format"
                    | "len"
                    | "lower"
                    | "match"
                    | "rep"
                    | "reverse"
                    | "split"
                    | "sub"
                    | "upper"
                    | "pack"
                    | "unpack"
                    | "packsize"
            ),
            "table" => matches!(*field, "concat" | "pack" | "unpack"),
            "utf8" => matches!(
                *field,
                "char"
                    | "codepoint"
                    | "len"
                    | "offset"
                    | "codes"
                    | "nfcnormalize"
                    | "nfdnormalize"
                    | "graphemes"
            ),
            "bit32" => matches!(
                *field,
                "arshift"
                    | "band"
                    | "bnot"
                    | "bor"
                    | "btest"
                    | "bxor"
                    | "extract"
                    | "lrotate"
                    | "lshift"
                    | "replace"
                    | "rrotate"
                    | "rshift"
            ),
            "os" => matches!(*field, "clock" | "date" | "difftime" | "getenv" | "time"),
            _ => false,
        },
        _ => false,
    }
}
