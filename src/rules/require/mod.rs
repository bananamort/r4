//! Model-first `require` resolution, conversion and module bundling.
//!
//! Every entry point in this module resolves `require` through the semantic
//! model: a call is only treated as a `require` call when its prefix
//! identifier resolves to the global `require` symbol or to an immutable alias
//! of it. A local or parameter named `require` is never converted, which
//! eliminates darklua's Claim D bug by construction.

mod bundler;
mod convert;

pub use bundler::{bundle_modules, BundleError, MemoryModuleProvider, ModuleProvider};
pub use convert::{convert_require, ConvertRequireError, RequirePathConverter};

use crate::nodes::{Arguments, Expression, FunctionCall, Prefix, StringExpression};
use crate::semantic::SemanticModel;

/// The name of the global `require` function.
pub const REQUIRE_NAME: &str = "require";

/// Classifies a call as a statically resolvable `require` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequireCall {
    /// The call is not a `require` call, or `require` is shadowed.
    NotRequire,
    /// The call is a `require` call with a single string-literal argument.
    Path {
        /// The literal path.
        path: String,
    },
    /// The call is a `require` call whose argument is not a single string
    /// literal (or is not valid UTF-8).
    NonLiteral,
}

/// Classifies `call`, resolving the `require` prefix through `model`.
///
/// Returns [`RequireCall::NotRequire`] when the prefix is not an identifier,
/// when it does not resolve to the global `require` (or an immutable alias),
/// or when the call has a method.
pub fn classify_require_call(call: &FunctionCall, model: &SemanticModel) -> RequireCall {
    if call.get_method().is_some() {
        return RequireCall::NotRequire;
    }

    let Prefix::Identifier(identifier) = call.get_prefix() else {
        return RequireCall::NotRequire;
    };

    if !crate::rules::canonicalize::identifier_resolves_to_global(model, identifier, REQUIRE_NAME) {
        return RequireCall::NotRequire;
    }

    match call.get_arguments() {
        Arguments::String(string) => match path_of_string(string) {
            Some(path) => RequireCall::Path { path },
            None => RequireCall::NonLiteral,
        },
        Arguments::Tuple(tuple) => {
            let mut values = tuple.iter_values();
            match (values.next(), values.next()) {
                (Some(Expression::String(string)), None) => match path_of_string(string) {
                    Some(path) => RequireCall::Path { path },
                    None => RequireCall::NonLiteral,
                },
                _ => RequireCall::NonLiteral,
            }
        }
        Arguments::Table(_) => RequireCall::NonLiteral,
    }
}

fn path_of_string(string: &StringExpression) -> Option<String> {
    std::str::from_utf8(string.get_value())
        .ok()
        .map(str::to_owned)
}

/// Returns the source line of a `require` call's prefix, when token data is
/// available.
pub(crate) fn require_call_line(call: &FunctionCall) -> Option<usize> {
    match call.get_prefix() {
        Prefix::Identifier(identifier) => identifier
            .get_token()
            .and_then(|token| token.get_line_number()),
        _ => None,
    }
}

/// Returns the directory prefix of a normalized module path.
pub(crate) fn parent_directory(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_owned(),
        None => String::new(),
    }
}

/// Normalizes a relative `require` path against the directory of the module
/// that contains it.
///
/// Paths starting with `./` or `../` are resolved segment by segment; any
/// other path is treated as an absolute module key and returned unchanged.
pub(crate) fn normalize_require_path(path: &str, base_directory: &str) -> String {
    if !path.starts_with("./") && !path.starts_with("../") {
        return path.to_owned();
    }

    let mut segments: Vec<&str> = if base_directory.is_empty() {
        Vec::new()
    } else {
        base_directory
            .split('/')
            .filter(|s| !s.is_empty())
            .collect()
    };

    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }

    segments.join("/")
}
