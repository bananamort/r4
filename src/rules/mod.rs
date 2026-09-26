//! AST transformation passes.
//!
//! This module hosts the mutating counterpart of the read-only analyses in
//! [`crate::semantic`] and [`crate::cfg`]. The passes are split into:
//!
//! * [`desugar`]: lower modern Luau syntax to vanilla Lua constructs.
//! * [`canonicalize`](mod@crate::rules::canonicalize): normalize equivalent syntax forms without changing
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
pub use name_generator::{NameGenerator, NameGeneratorError};
pub use session::{Session, SessionError};

use crate::nodes::Block;
use crate::semantic::{SemanticAnalyzer, SemanticError, SemanticModel};
use crate::{validate_dialect, Dialect, DialectSupportError};

/// An error produced by [`desugar_to_lua51`].
#[derive(Clone, Debug)]
pub enum DesugarError {
    /// A pass could not analyze the block.
    Semantic(SemanticError),
    /// A function body has invalid control flow (a `break`/`continue` with no
    /// enclosing loop).
    Cfg(crate::cfg::CfgError),
    /// A `continue` could not be lowered without changing the meaning of its
    /// loop.
    Continue(desugar::RemoveContinueError),
    /// A Luau number literal could not be normalized because its source text
    /// was not available.
    Number(desugar::ConvertLuauNumberError),
    /// The lowered block still contains syntax Lua 5.1 cannot represent.
    UnsupportedDialect(DialectSupportError),
}

impl std::fmt::Display for DesugarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesugarError::Semantic(error) => write!(f, "{error}"),
            DesugarError::Cfg(error) => write!(f, "{error}"),
            DesugarError::Continue(error) => write!(f, "{error}"),
            DesugarError::Number(error) => write!(f, "{error}"),
            DesugarError::UnsupportedDialect(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DesugarError {}

impl From<SemanticError> for DesugarError {
    fn from(error: SemanticError) -> Self {
        DesugarError::Semantic(error)
    }
}

impl From<crate::cfg::CfgError> for DesugarError {
    fn from(error: crate::cfg::CfgError) -> Self {
        DesugarError::Cfg(error)
    }
}

impl From<desugar::RemoveContinueError> for DesugarError {
    fn from(error: desugar::RemoveContinueError) -> Self {
        DesugarError::Continue(error)
    }
}

impl From<desugar::ConvertLuauNumberError> for DesugarError {
    fn from(error: desugar::ConvertLuauNumberError) -> Self {
        DesugarError::Number(error)
    }
}

impl From<DialectSupportError> for DesugarError {
    fn from(error: DialectSupportError) -> Self {
        DesugarError::UnsupportedDialect(error)
    }
}

/// Applies all desugaring passes to transpile modern Luau down to vanilla Lua 5.1.
///
/// Every pass requires a `SemanticModel`; the pipeline re-analyzes the block
/// before each pass so each one sees a model that matches the current AST. The
/// lowered block is finally validated against Lua 5.1: if a construct the
/// pipeline does not lower survives (a Lua 5.2 `goto`, a Lua 5.3 bitwise
/// operator), it fails fast instead of producing unparseable output.
pub fn desugar_to_lua51(block: &mut Block) -> Result<(), DesugarError> {
    run(block, None)
}

/// Like [`desugar_to_lua51`], but reads number tokens from `code`.
///
/// Use this when the tree was parsed with
/// [`Parser::preserve_tokens`](crate::Parser::preserve_tokens): a number token
/// that only carries a source position can be normalized only with its source,
/// so [`desugar_to_lua51`] fails with
/// [`DesugarError::Number`] rather than guessing a lexeme.
pub fn desugar_to_lua51_with_code(block: &mut Block, code: &str) -> Result<(), DesugarError> {
    run(block, Some(code))
}

fn run(block: &mut Block, code: Option<&str>) -> Result<(), DesugarError> {
    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_types(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_attribute(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_local_attributes(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::make_assignment_local(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_compound_assign(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_if_expression(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_interpolated_string(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_floor_division(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_continue(block, &model)?;

    let model = SemanticAnalyzer::new().analyze(block)?;
    match code {
        Some(code) => desugar::convert_luau_number_with_code(block, code, &model)?,
        None => desugar::convert_luau_number(block, &model)?,
    }

    let model = SemanticAnalyzer::new().analyze(block)?;
    desugar::remove_method_call(block, &model);

    let model = SemanticAnalyzer::new().analyze(block)?;
    crate::cfg::validate_function_loop_context(block, &model)?;

    validate_dialect(block, Dialect::Lua51)?;

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
