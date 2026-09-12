//! Token-key provenance linking control flow graph contents back to AST nodes.
//!
//! Basic blocks store cloned statements and branch conditions. When the AST was
//! parsed with token preservation, the clones share the source positions of
//! their originals, so the first-token key of a CFG element can be matched
//! against the first-token key of an AST statement or expression. This is the
//! bridge that lets reachability-driven passes delete the right source nodes
//! without maintaining a separate statement-id arena.

use crate::nodes::{Expression, Statement};
use crate::semantic::TokenKey;
use crate::utils::lines;

/// Returns the source key of a statement's first token, when token data is
/// available.
pub fn statement_token_key(statement: &Statement) -> Option<TokenKey> {
    lines::first_statement_token(statement).and_then(TokenKey::of)
}

/// Returns the source key of an expression's first token, when token data is
/// available.
pub fn expression_token_key(expression: &Expression) -> Option<TokenKey> {
    lines::first_expression_token(expression).and_then(TokenKey::of)
}
