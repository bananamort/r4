//! AST transformation passes.
//!
//! This module hosts the mutating counterpart of the read-only analyses in
//! [`crate::semantic`] and [`crate::cfg`]. The passes are split into:
//!
//! * [`desugar`]: lower modern Luau syntax to vanilla Lua constructs.
//! * [`canonicalize`]: normalize equivalents syntax forms without changing
//!   behavior.
//! * [`rename_variables`]: host environment knowledge used by renaming passes.

mod tracker;

pub mod canonicalize;
pub mod desugar;
pub mod rename_variables;

use crate::nodes::Block;

/// Applies all desugaring passes to transpile modern Luau down to vanilla Lua 5.1.
pub fn desugar_to_lua51(block: &mut Block) {
    desugar::remove_types(block);
    desugar::remove_compound_assign(block);
    desugar::remove_if_expression(block);
    desugar::remove_interpolated_string(block);
    desugar::remove_floor_division(block);
    desugar::remove_continue(block);
    desugar::convert_luau_number(block);
    desugar::remove_method_call(block);
}

/// Applies clean syntax canonicalization passes.
pub fn canonicalize(block: &mut Block) {
    canonicalize::convert_index_to_field(block);
    canonicalize::remove_nil_declarations(block);
    canonicalize::remove_empty_do(block);
    canonicalize::remove_call_parens(block);
}
