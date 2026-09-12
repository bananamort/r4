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

pub mod canonicalize;
pub mod desugar;
pub mod rename_variables;
pub mod require;
mod session;

pub use name_generator::NameGenerator;
pub use session::{Session, SessionError};

use crate::nodes::Block;
use crate::semantic::{SemanticAnalyzer, SemanticError, SemanticModel};

/// Applies all desugaring passes to transpile modern Luau down to vanilla Lua 5.1.
///
/// Every pass requires a `SemanticModel`; the pipeline re-analyzes the block
/// before each pass so each one sees a model that matches the current AST.
pub fn desugar_to_lua51(block: &mut Block) -> Result<(), SemanticError> {
    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_types(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_compound_assign(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_if_expression(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_interpolated_string(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_floor_division(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_continue(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::convert_luau_number(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_method_call(block, &model);

    Ok(())
}

/// Applies clean syntax canonicalization passes.
///
/// Every pass requires a `SemanticModel`. The purely syntactic passes ignore
/// it, but requiring it keeps the rule API uniform: a pass never runs on an
/// AST that was not analyzed.
pub fn canonicalize(block: &mut Block, model: &SemanticModel) {
    canonicalize::convert_index_to_field(block, model);
    canonicalize::remove_nil_declarations(block, model);
    canonicalize::remove_empty_do(block, model);
    canonicalize::remove_call_parens(block, model);
}
