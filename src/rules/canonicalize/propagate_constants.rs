//! Propagation of proven local constants into the expressions that read them.
//!
//! [`ConstantPropagation`] proves, for every control flow position, which local
//! symbols hold a constant.
//! [`fold_constant_branches_with_propagation`](crate::cfg::analysis::fold_constant_branches_with_propagation)
//! uses that only for branch conditions; this pass applies it as a general
//! rewrite, replacing a read of a proven-constant local with the literal itself
//! so the value propagates out of its declaration and into every expression
//! that used it.
//!
//! Only values that can be written back as a literal are propagated: `nil`,
//! booleans, finite numbers and strings. Function and table values are never
//! propagated, because evaluating the literal again would create a *new* object
//! and change identity. Identifiers are resolved through the structural
//! bindings recorded by semantic analysis and located through the control flow
//! graph's statement origins, so the pass works on token-less trees.
//!
//! The condition of an `if`/`while`/`repeat` statement is a terminator in the
//! graph rather than a statement, and
//! [`remove_unreachable_code`](super::remove_unreachable_code) already folds it
//! into control flow, so this pass does not rewrite conditions.

use std::collections::HashMap;
use std::fmt;

use crate::cfg::analysis::{ConstantPropagation, ConstantPropagationError};
use crate::cfg::{
    build_chunk, validate_block_identities, CfgError, StatementOrigin, Terminator,
    UnidentifiedBlock,
};
use crate::nodes::{Block, BlockIdentity, Expression, LastStatement, Statement};
use crate::process::{
    DefaultPostVisitor, Evaluator, LuaValue, NodePostProcessor, NodePostVisitor, NodeProcessor,
};
use crate::rules::canonicalize::compute_expression::is_safe_literal;
use crate::semantic::{SemanticModel, SymbolId};

/// An error produced while propagating constants.
#[derive(Clone, Debug, PartialEq)]
pub enum PropagateConstantsError {
    /// The AST and the semantic model do not match.
    Provenance(UnidentifiedBlock),
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Constant propagation could not resolve an identifier.
    ConstantPropagation(ConstantPropagationError),
}

impl From<UnidentifiedBlock> for PropagateConstantsError {
    fn from(error: UnidentifiedBlock) -> Self {
        Self::Provenance(error)
    }
}

impl From<CfgError> for PropagateConstantsError {
    fn from(error: CfgError) -> Self {
        Self::Cfg(error)
    }
}

impl From<ConstantPropagationError> for PropagateConstantsError {
    fn from(error: ConstantPropagationError) -> Self {
        Self::ConstantPropagation(error)
    }
}

impl fmt::Display for PropagateConstantsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provenance(error) => write!(f, "{error}"),
            Self::Cfg(error) => write!(f, "{error}"),
            Self::ConstantPropagation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PropagateConstantsError {}

/// Replaces reads of provably constant locals with their literal value.
///
/// Returns the number of reads replaced. A read is only replaced when the
/// symbol is a tracked local (not captured, redefined or an iteration variable)
/// and the constant can be written as a literal. See the module documentation
/// for what is deliberately not rewritten.
pub fn propagate_constants(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, PropagateConstantsError> {
    validate_block_identities(block)?;
    let cfg = build_chunk(block, model)?;
    let analysis = ConstantPropagation::analyze(&cfg, model)?;

    // The constants in effect before every lowered statement, and at the
    // `return` that ends a block.
    let mut statements: HashMap<StatementOrigin, HashMap<SymbolId, LuaValue>> = HashMap::new();
    let mut returns: HashMap<BlockIdentity, HashMap<SymbolId, LuaValue>> = HashMap::new();

    for block_id in cfg.iter_block_ids() {
        let basic = &cfg.blocks[block_id.index()];
        for (index, origin) in basic.statement_origins.iter().enumerate() {
            if let Some(constants) = analysis.constants_before_statement(block_id, index) {
                statements
                    .entry(*origin)
                    .or_insert_with(|| constants.clone());
            }
        }
        if matches!(basic.terminator, Terminator::Return(_)) {
            // A `return` is the last statement of its block, so the constants
            // that hold at the terminator are the ones in effect for it.
            if let Some(origin) = basic.statement_origins.last() {
                returns.insert(
                    origin.block,
                    analysis.constants_before_terminator(block_id).clone(),
                );
            }
        }
    }

    let mut replacer = ConstantReplacer {
        model,
        evaluator: Evaluator::for_dialect(model.dialect()),
        statements,
        returns,
        frames: Vec::new(),
        current: None,
        replaced: 0,
    };
    DefaultPostVisitor::visit_block(block, &mut replacer);

    Ok(replacer.replaced)
}

/// Returns the literal for a read of a proven constant, if it can be written.
fn literal_for(
    model: &SemanticModel,
    evaluator: &Evaluator,
    constants: &HashMap<SymbolId, LuaValue>,
    expression: &Expression,
) -> Option<Expression> {
    let Expression::Identifier(identifier) = expression else {
        return None;
    };
    let symbol = model.resolve_identifier(identifier)?;
    let value = constants.get(&symbol)?;
    if !is_safe_literal(value) {
        return None;
    }
    evaluator.value_to_expression(value.clone())
}

struct ConstantReplacer<'m> {
    model: &'m SemanticModel,
    evaluator: Evaluator,
    statements: HashMap<StatementOrigin, HashMap<SymbolId, LuaValue>>,
    returns: HashMap<BlockIdentity, HashMap<SymbolId, LuaValue>>,
    /// The blocks currently being visited, each with the index of the next
    /// statement to process.
    frames: Vec<(BlockIdentity, u32)>,
    /// The constants in effect for the statement being visited.
    current: Option<HashMap<SymbolId, LuaValue>>,
    replaced: usize,
}

impl NodeProcessor for ConstantReplacer<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let identity = block
            .get_identity()
            .expect("block identities were validated before the rewrite");
        self.frames.push((identity, 0));
    }

    fn process_statement(&mut self, _statement: &mut Statement) {
        let Some((identity, index)) = self.frames.last_mut() else {
            return;
        };
        let origin = StatementOrigin {
            block: *identity,
            index: *index,
        };
        *index += 1;
        self.current = self.statements.get(&origin).cloned();
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        let literal = self
            .current
            .as_ref()
            .and_then(|constants| literal_for(self.model, &self.evaluator, constants, expression));
        if let Some(literal) = literal {
            *expression = literal;
            self.replaced += 1;
        }
    }

    fn process_last_statement(&mut self, statement: &mut LastStatement) {
        // The statements before the `return` set `current` for their own
        // expressions; the return must not reuse them.
        self.current = None;

        let LastStatement::Return(return_statement) = statement else {
            return;
        };
        let Some((identity, _)) = self.frames.last() else {
            return;
        };
        let Some(constants) = self.returns.get(identity) else {
            return;
        };
        for expression in return_statement.iter_mut_expressions() {
            if let Some(literal) = literal_for(self.model, &self.evaluator, constants, expression) {
                *expression = literal;
                self.replaced += 1;
            }
        }
    }
}

impl NodePostProcessor for ConstantReplacer<'_> {
    fn process_after_block(&mut self, _block: &mut Block) {
        self.frames.pop();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, analyze_lua53, generate};

    #[test]
    fn propagates_a_local_constant_into_later_reads() {
        let (mut block, model) = analyze("local x = 1\nlocal y = x + 2\nreturn y");

        let replaced = propagate_constants(&mut block, &model).expect("propagation must succeed");

        assert_eq!(replaced, 2);
        let output = generate(&block);
        assert!(output.contains("local y = 1 + 2"), "{output}");
        assert!(output.contains("return 3"), "{output}");
    }

    #[test]
    fn propagates_integer_constants_with_the_integer_subtype() {
        let (mut block, model) = analyze_lua53("local a = 1\nlocal b = 2\nprint(a + b)\nreturn 0");

        propagate_constants(&mut block, &model).expect("propagation must succeed");

        let output = generate(&block);
        assert!(output.contains("print(1 + 2)"), "{output}");
        assert!(!output.contains("1.0"), "{output}");
    }

    #[test]
    fn propagates_the_value_that_reaches_a_read() {
        // `x` is redefined before the read, so the later definition's value is
        // the one that propagates.
        let (mut block, model) = analyze("local x = 1\nx = 2\nreturn x");

        propagate_constants(&mut block, &model).expect("propagation must succeed");

        let output = generate(&block);
        assert!(output.contains("return 2"), "{output}");
        assert!(!output.contains("return 1"), "{output}");
    }

    #[test]
    fn leaves_a_symbol_that_is_not_provably_constant() {
        for code in [
            "local x = 1\nif condition then\n    x = 2\nend\nreturn x",
            "local x = 0\nwhile condition do\n    x = x + 1\nend\nreturn x",
        ] {
            let (mut block, model) = analyze(code);
            propagate_constants(&mut block, &model).expect("propagation must succeed");
            let output = generate(&block);
            assert!(
                output.contains("return x"),
                "`{code}` must keep the read: {output}"
            );
        }
    }

    #[test]
    fn does_not_propagate_function_or_table_values() {
        // A fresh evaluation of the literal would create a new object, so the
        // value must stay behind the identifier.
        let (mut block, model) = analyze("local f = function() end\nreturn f == f");

        let replaced = propagate_constants(&mut block, &model).expect("propagation must succeed");

        assert_eq!(replaced, 0);
        assert!(generate(&block).contains("f == f"));
    }

    #[test]
    fn does_not_propagate_captured_locals() {
        // A nested function can run at any time, so a captured local is not a
        // constant across the closure.
        let (mut block, model) =
            analyze("local x = 1\nlocal f = function() return x end\nreturn f()");

        propagate_constants(&mut block, &model).expect("propagation must succeed");

        assert!(generate(&block).contains("return x"));
    }
}
