//! AST transformation passes.
//!
//! This module hosts the mutating counterpart of the read-only analyses in
//! [`crate::semantic`] and [`crate::cfg`]. The passes are split into:
//!
//! * [`desugar`]: lower modern Luau syntax to vanilla Lua constructs.
//! * [`canonicalize`]: normalize equivalents syntax forms without changing
//!   behavior.
//! * [`rename_variables`]: host environment knowledge used by renaming passes.

mod name_generator;
mod tracker;

pub mod canonicalize;
pub mod desugar;
pub mod rename_variables;
pub mod require;
mod session;

pub use name_generator::NameGenerator;
pub use session::{Session, SessionError};

use crate::nodes::Block;
use crate::semantic::{SemanticAnalyzer, SemanticError};

/// Applies all desugaring passes to transpile modern Luau down to vanilla Lua 5.1.
///
/// The model-dependent passes (`remove_interpolated_string` and
/// `remove_floor_division`) analyze the block internally at the point they run,
/// so every pass sees a model that matches the current AST.
pub fn desugar_to_lua51(block: &mut Block) -> Result<(), SemanticError> {
    desugar::remove_types(block);
    desugar::remove_compound_assign(block);
    desugar::remove_if_expression(block);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_interpolated_string(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_floor_division(block, &model);

    desugar::remove_continue(block);
    desugar::convert_luau_number(block);
    desugar::remove_method_call(block);

    Ok(())
}

/// Applies clean syntax canonicalization passes.
///
/// Semantic passes that need symbol or control-flow information, such as
/// [`canonicalize::remove_unused_variables`] and
/// [`canonicalize::remove_unreachable_code`], are not part of this pipeline;
/// call them with a [`crate::semantic::SemanticModel`]. They resolve through
/// structural bindings, so they work on token-less ASTs.
pub fn canonicalize(block: &mut Block) {
    canonicalize::convert_index_to_field(block);
    canonicalize::remove_nil_declarations(block);
    canonicalize::remove_empty_do(block);
    canonicalize::remove_call_parens(block);
}
