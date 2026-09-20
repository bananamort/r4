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

#[cfg(test)]
pub(crate) mod test_support;

pub mod canonicalize;
pub mod desugar;
pub mod formatting;
pub mod inject;
pub mod rename_variables;
pub mod require;
mod session;

pub use formatting::{
    append_text_comment, append_text_comment_from_file, remove_comments, remove_comments_where,
    remove_spaces, replace_referenced_tokens, shift_token_line, AppendLocation,
};
pub use inject::{inject_global_values, InjectGlobalValueError};
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{generate, parse};
    use crate::semantic::SemanticAnalyzer;

    #[test]
    fn desugar_pipeline_lowers_a_luau_function() {
        let mut block = parse(
            r#"
            local function clamp(value: number, minimum: number, maximum: number): number
                if value < minimum then
                    return minimum
                end
                return if value > maximum then maximum else value
            end
            local total: number = 0
            for i = 1, 10 do
                if i % 2 == 0 then
                    continue
                end
                total += i
            end
            return clamp(total // 2, 0, 100)
            "#,
        );

        desugar_to_lua51(&mut block).expect("the desugar pipeline must analyze successfully");
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .expect("the desugared block must analyze");
        canonicalize(&mut block, &model);

        let output = generate(&block);
        assert!(!output.contains("continue"), "unexpected: {output}");
        assert!(!output.contains("//"), "unexpected: {output}");
        assert!(output.contains("math.floor"), "unexpected: {output}");

        parse(&output);
    }
}
