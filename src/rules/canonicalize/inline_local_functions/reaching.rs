//! Reaching definitions for local values, used by the local-function inliner.
//!
//! The inliner must know which function a local holds at every call site.
//! Inspecting only the declaration is not enough: a local may be assigned in
//! conditional branches, loops and `goto` regions. This module runs a classic
//! forward *may* reaching-definition analysis over the control-flow graph of
//! every function region, records which definitions are in effect immediately
//! before each statement, loop header, branch condition and terminator, and
//! classifies each definition as a function literal, an alias of another
//! symbol, or an unknown value.
//!
//! The analysis reads cloned CFG statements and resolves identifiers through
//! the structural bindings of the semantic model, so it never guesses.

use std::collections::{HashMap, HashSet};

use bitvec::prelude::*;

use crate::cfg::analysis::Reachability;
use crate::cfg::{
    BlockId, CfgError, ConditionOrigin, ControlFlowGraph, StatementOrigin, Terminator,
};
use crate::nodes::{Expression, Identifier, Statement, Variable};
use crate::semantic::{SemanticModel, SymbolId, SymbolKind};

use super::FunctionKey;

/// Identifies a definition inside one function region.
pub(crate) type DefinitionId = usize;

/// The reaching sets immediately before each statement of each block.
type StatementDefinitions = Vec<Vec<Vec<DefinitionId>>>;

/// The value a definition stores, as far as the inliner can prove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DefinitionValue {
    /// A function literal with the given structural key.
    Function(FunctionKey),
    /// A direct assignment of another symbol's value.
    Alias(SymbolId),
    /// Anything else, including calls that may return any value.
    Other,
}

/// A single write to a local tracked by the reaching-definition analysis.
#[derive(Clone, Debug)]
pub(crate) struct Definition {
    /// The written symbol.
    pub symbol: SymbolId,
    /// The value classification.
    pub value: DefinitionValue,
    /// The statement that performs the write. Alias resolution uses the
    /// definitions in effect at this point, before the statement writes.
    pub origin: StatementOrigin,
}

/// An error produced while analyzing function values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReachingError {
    /// A declaration or read is not bound in the semantic model.
    UnresolvedIdentifier {
        /// The name of the unresolved identifier.
        name: String,
        /// The source line, when token data is available.
        line: Option<usize>,
    },
    /// A CFG statement has no structural origin, so its position cannot be
    /// used to resolve reaching definitions.
    MissingStatementOrigin {
        /// The CFG block.
        block: BlockId,
        /// The index of the statement inside the block.
        index: usize,
    },
    /// The control-flow graph of a function region could not be built.
    ControlFlow(CfgError),
}

impl std::fmt::Display for ReachingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedIdentifier { name, line } => match line {
                Some(line) => write!(
                    f,
                    "line {}: identifier `{}` is not bound in the semantic model",
                    line, name
                ),
                None => write!(
                    f,
                    "identifier `{}` is not bound in the semantic model",
                    name
                ),
            },
            Self::MissingStatementOrigin { block, index } => write!(
                f,
                "CFG block {} has no origin for statement {}",
                block.0, index
            ),
            Self::ControlFlow(error) => {
                write!(f, "cannot build a function's control flow: {error}")
            }
        }
    }
}

impl std::error::Error for ReachingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnresolvedIdentifier { .. } | Self::MissingStatementOrigin { .. } => None,
            Self::ControlFlow(error) => Some(error),
        }
    }
}

impl From<CfgError> for ReachingError {
    fn from(error: CfgError) -> Self {
        Self::ControlFlow(error)
    }
}

/// The reaching-definition information of one function region (the chunk or a
/// function body).
#[derive(Clone, Debug)]
pub(crate) struct FunctionRegion {
    /// The control-flow graph this region describes.
    pub cfg: ControlFlowGraph,
    /// All definitions, addressed by [`DefinitionId`].
    pub definitions: Vec<Definition>,
    /// The reaching set immediately before each CFG statement, indexed by
    /// block and then by statement position inside that block.
    statement_definitions: StatementDefinitions,
    /// The reaching set after the last statement of each block, before the
    /// terminator.
    terminator_definitions: Vec<Vec<DefinitionId>>,
    /// The reaching set at a loop initializer.
    loop_initial_definitions: HashMap<StatementOrigin, Vec<DefinitionId>>,
    /// The reaching set at the entry of each branch condition.
    condition_definitions: HashMap<ConditionOrigin, Vec<DefinitionId>>,
    /// Maps the structural origin of a lowered statement to its CFG location.
    statement_blocks: HashMap<StatementOrigin, (BlockId, usize)>,
    /// Maps the structural origin of a lowered terminator to its block.
    terminator_blocks: HashMap<StatementOrigin, BlockId>,
}

impl FunctionRegion {
    /// Builds the region for `cfg`.
    pub(crate) fn analyze(
        cfg: ControlFlowGraph,
        model: &SemanticModel,
    ) -> Result<Self, ReachingError> {
        let reachability = Reachability::analyze(&cfg);
        let (definitions, statement_definitions_per_block) = collect_definitions(&cfg, model)?;
        let (entry, _exit) = reaching_definitions(
            &cfg,
            &definitions,
            &statement_definitions_per_block,
            &reachability,
        );

        let mut statement_definitions = Vec::with_capacity(cfg.blocks.len());
        let mut terminator_definitions = Vec::with_capacity(cfg.blocks.len());
        let mut statement_blocks = HashMap::new();
        let mut terminator_blocks = HashMap::new();

        for block in &cfg.blocks {
            let mut current = entry[block.id.index()].clone();
            let mut per_statement = Vec::with_capacity(block.statements.len());
            for (index, ids) in statement_definitions_per_block[block.id.index()]
                .iter()
                .enumerate()
            {
                per_statement.push(ids_from_bits(&current));
                if let Some(origin) = block.statement_origins.get(index) {
                    statement_blocks.insert(*origin, (block.id, index));
                }
                for &id in ids {
                    let symbol = definitions[id].symbol;
                    for (other, definition) in definitions.iter().enumerate() {
                        if definition.symbol == symbol {
                            current.set(other, false);
                        }
                    }
                    current.set(id, true);
                }
            }
            terminator_definitions.push(ids_from_bits(&current));
            if let Some(origin) = block.terminator_origin {
                terminator_blocks.insert(origin, block.id);
            }
            statement_definitions.push(per_statement);
        }

        let mut condition_definitions = HashMap::new();
        for block in &cfg.blocks {
            if let Some(origin) = block.branch_origin {
                // An `if` first condition is attached to the block that already
                // contains the statements before it, so use the definitions
                // after those statements. While/repeat headers have no
                // statements, where this is the same as the entry set.
                condition_definitions
                    .entry(origin)
                    .or_insert_with(|| terminator_definitions[block.id.index()].clone());
            }
        }

        let mut loop_initial_definitions = HashMap::new();
        for (origin, header) in &cfg.loop_headers {
            if !matches!(cfg.terminator(*header), Terminator::ForLoop { .. }) {
                continue;
            }
            let init = cfg
                .predecessors(*header)
                .iter()
                .copied()
                .find(|predecessor| !is_loop_body(&cfg, *header, *predecessor));
            if let Some(init) = init {
                let mut current = entry[init.index()].clone();
                for ids in &statement_definitions_per_block[init.index()] {
                    for &id in ids {
                        let symbol = definitions[id].symbol;
                        for (other, definition) in definitions.iter().enumerate() {
                            if definition.symbol == symbol {
                                current.set(other, false);
                            }
                        }
                        current.set(id, true);
                    }
                }
                loop_initial_definitions.insert(*origin, ids_from_bits(&current));
            }
        }

        Ok(Self {
            cfg,
            definitions,
            statement_definitions,
            terminator_definitions,
            loop_initial_definitions,
            condition_definitions,
            statement_blocks,
            terminator_blocks,
        })
    }

    /// Returns the definitions in effect immediately before `origin`.
    pub(crate) fn at_statement(&self, origin: StatementOrigin) -> Option<&[DefinitionId]> {
        let (block, index) = *self.statement_blocks.get(&origin)?;
        self.statement_definitions
            .get(block.index())?
            .get(index)
            .map(Vec::as_slice)
    }

    /// Returns the definitions in effect after the last statement and before
    /// the terminator at `origin`.
    pub(crate) fn at_terminator(&self, origin: StatementOrigin) -> Option<&[DefinitionId]> {
        let block = *self.terminator_blocks.get(&origin)?;
        self.terminator_definitions
            .get(block.index())
            .map(Vec::as_slice)
    }

    /// Returns the definitions in effect at a loop initializer.
    pub(crate) fn at_loop_initial(&self, origin: StatementOrigin) -> Option<&[DefinitionId]> {
        self.loop_initial_definitions
            .get(&origin)
            .map(Vec::as_slice)
    }

    /// Returns the definitions in effect at the entry of a branch condition.
    pub(crate) fn at_condition(&self, origin: ConditionOrigin) -> Option<&[DefinitionId]> {
        self.condition_definitions.get(&origin).map(Vec::as_slice)
    }

    /// Returns whether `symbol` can be written after `origin` on any path.
    ///
    /// The creation point is a statement. A write in the same block after it,
    /// or in any block reachable from it, can run later. A write before it in
    /// the same block can run again when that block is part of a cycle. Writes
    /// in unreachable blocks cannot run after the point.
    pub(crate) fn has_write_after(
        &self,
        symbol: SymbolId,
        origin: StatementOrigin,
    ) -> Option<bool> {
        let (creation_block, creation_index) = *self.statement_blocks.get(&origin)?;
        let mut reachable = HashSet::new();
        let mut stack: Vec<BlockId> = self.cfg.successors(creation_block).to_vec();
        let mut repeats = false;
        while let Some(block) = stack.pop() {
            if block == creation_block {
                repeats = true;
                continue;
            }
            if !reachable.insert(block) {
                continue;
            }
            stack.extend(self.cfg.successors(block));
        }

        for definition in &self.definitions {
            if definition.symbol != symbol {
                continue;
            }
            let Some((block, index)) = self.statement_blocks.get(&definition.origin) else {
                return Some(true);
            };
            if *block == creation_block {
                if *index >= creation_index || repeats {
                    return Some(true);
                }
            } else if reachable.contains(block) {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Filters a reaching set to the definitions of `symbol`.
    pub(crate) fn definitions_of(
        &self,
        ids: &[DefinitionId],
        symbol: SymbolId,
    ) -> Vec<DefinitionId> {
        ids.iter()
            .copied()
            .filter(|id| self.definitions[*id].symbol == symbol)
            .collect()
    }

    /// Returns the definition with the given id.
    pub(crate) fn definition(&self, id: DefinitionId) -> &Definition {
        &self.definitions[id]
    }
}

fn is_loop_body(cfg: &ControlFlowGraph, header: BlockId, predecessor: BlockId) -> bool {
    matches!(
        cfg.terminator(header),
        Terminator::ForLoop { body, .. } if *body == predecessor
    )
}

fn ids_from_bits(bits: &BitVec) -> Vec<DefinitionId> {
    bits.iter_ones().collect()
}

// ----------------------------------------------------------------------
// Definition collection
// ----------------------------------------------------------------------

fn collect_definitions(
    cfg: &ControlFlowGraph,
    model: &SemanticModel,
) -> Result<(Vec<Definition>, StatementDefinitions), ReachingError> {
    let mut definitions = Vec::new();
    let mut blocks: StatementDefinitions = Vec::with_capacity(cfg.blocks.len());

    for block in &cfg.blocks {
        let mut per_statement = Vec::with_capacity(block.statements.len());
        for (index, statement) in block.statements.iter().enumerate() {
            let origin = block.statement_origins.get(index).copied().ok_or(
                ReachingError::MissingStatementOrigin {
                    block: block.id,
                    index,
                },
            )?;
            let mut ids = Vec::new();
            collect_statement(statement, origin, model, &mut definitions, &mut ids)?;
            per_statement.push(ids);
        }
        blocks.push(per_statement);
    }

    Ok((definitions, blocks))
}

fn collect_statement(
    statement: &Statement,
    origin: StatementOrigin,
    model: &SemanticModel,
    definitions: &mut Vec<Definition>,
    ids: &mut Vec<DefinitionId>,
) -> Result<(), ReachingError> {
    match statement {
        Statement::LocalAssign(assignment) => {
            let values: Vec<&Expression> = assignment.iter_values().collect();
            for (index, variable) in assignment.iter_variables().enumerate() {
                let symbol = resolve(model, variable.get_identifier())?;
                if !trackable(model, symbol) {
                    continue;
                }
                let value = initializer_value(&values, index, origin, model)?;
                push_definition(symbol, value, origin, definitions, ids);
            }
        }
        Statement::Assign(assignment) => {
            let values: Vec<&Expression> = assignment.iter_values().collect();
            for (index, variable) in assignment.iter_variables().enumerate() {
                let Variable::Identifier(identifier) = variable else {
                    continue;
                };
                let symbol = resolve(model, identifier)?;
                if !trackable(model, symbol) {
                    continue;
                }
                let value = initializer_value(&values, index, origin, model)?;
                push_definition(symbol, value, origin, definitions, ids);
            }
        }
        Statement::CompoundAssign(assignment) => {
            if let Variable::Identifier(identifier) = assignment.get_variable() {
                let symbol = resolve(model, identifier)?;
                if trackable(model, symbol) {
                    push_definition(symbol, DefinitionValue::Other, origin, definitions, ids);
                }
            }
        }
        Statement::LocalFunction(function) => {
            let symbol = resolve(model, function.get_identifier())?;
            if trackable(model, symbol) {
                push_definition(
                    symbol,
                    DefinitionValue::Function(FunctionKey { origin, slot: 0 }),
                    origin,
                    definitions,
                    ids,
                );
            }
        }
        Statement::Function(function) => {
            let name = function.get_name();
            if !name.get_field_names().is_empty() || name.get_method().is_some() {
                return Ok(());
            }
            let symbol = resolve(model, name.get_name())?;
            if trackable(model, symbol) {
                push_definition(
                    symbol,
                    DefinitionValue::Function(FunctionKey { origin, slot: 0 }),
                    origin,
                    definitions,
                    ids,
                );
            }
        }
        _ => {}
    }
    Ok(())
}

fn push_definition(
    symbol: SymbolId,
    value: DefinitionValue,
    origin: StatementOrigin,
    definitions: &mut Vec<Definition>,
    ids: &mut Vec<DefinitionId>,
) {
    let id = definitions.len();
    definitions.push(Definition {
        symbol,
        value,
        origin,
    });
    ids.push(id);
}

fn trackable(model: &SemanticModel, symbol: SymbolId) -> bool {
    model.symbol(symbol).kind == SymbolKind::Local
}

fn resolve(model: &SemanticModel, identifier: &Identifier) -> Result<SymbolId, ReachingError> {
    model
        .resolve_identifier(identifier)
        .ok_or_else(|| ReachingError::UnresolvedIdentifier {
            name: identifier.get_name().clone(),
            line: identifier
                .get_token()
                .and_then(|token| token.get_line_number()),
        })
}

/// Returns the definition value of the `index`-th variable in a value list.
fn initializer_value(
    values: &[&Expression],
    index: usize,
    origin: StatementOrigin,
    model: &SemanticModel,
) -> Result<DefinitionValue, ReachingError> {
    let Some(expression) = values.get(index) else {
        return Ok(DefinitionValue::Other);
    };

    // Every value except the final one is adjusted to a single value, so a
    // function literal or an identifier is still that value. The final value
    // may expand multiple results; only an expression that is known to produce
    // one value can be classified.
    let single_valued = index + 1 < values.len()
        || !matches!(
            expression,
            Expression::Call(_) | Expression::VariableArguments(_)
        );

    if !single_valued {
        return Ok(DefinitionValue::Other);
    }

    if function_expression(expression).is_some() {
        return Ok(DefinitionValue::Function(FunctionKey {
            origin,
            slot: index as u32,
        }));
    }

    if let Some(identifier) = identifier_expression(expression) {
        return Ok(DefinitionValue::Alias(resolve(model, identifier)?));
    }

    Ok(DefinitionValue::Other)
}

/// Unwraps an expression that denotes exactly one function literal.
pub(crate) fn function_expression(
    expression: &Expression,
) -> Option<&crate::nodes::FunctionExpression> {
    match expression {
        Expression::Function(function) => Some(function),
        Expression::Parenthese(parenthese) => function_expression(parenthese.inner_expression()),
        Expression::TypeCast(type_cast) => function_expression(type_cast.get_expression()),
        _ => None,
    }
}

/// Unwraps an expression that denotes exactly one identifier value.
fn identifier_expression(expression: &Expression) -> Option<&Identifier> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier),
        Expression::Parenthese(parenthese) => identifier_expression(parenthese.inner_expression()),
        Expression::TypeCast(type_cast) => identifier_expression(type_cast.get_expression()),
        _ => None,
    }
}

// ----------------------------------------------------------------------
// Reaching-definition data flow
// ----------------------------------------------------------------------

fn reaching_definitions(
    cfg: &ControlFlowGraph,
    definitions: &[Definition],
    statement_definitions: &[Vec<Vec<DefinitionId>>],
    reachability: &Reachability,
) -> (Vec<BitVec>, Vec<BitVec>) {
    let block_count = cfg.blocks.len();
    let definition_count = definitions.len();

    // A block's transfer keeps only the last write to each symbol in the
    // block and kills every definition of a symbol the block writes. Using the
    // block's ordered definitions (instead of a flat generated set) makes the
    // transfer exact for `f = a; f = b` in one block.
    let mut last_generated: Vec<BitVec> =
        vec![BitVec::repeat(false, definition_count); block_count];
    let mut killed: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];

    for (block_index, statements) in statement_definitions.iter().enumerate() {
        let mut last: HashMap<SymbolId, DefinitionId> = HashMap::new();
        for ids in statements {
            for &id in ids {
                last.insert(definitions[id].symbol, id);
            }
        }
        for (&symbol, &id) in &last {
            last_generated[block_index].set(id, true);
            for (other, definition) in definitions.iter().enumerate() {
                if definition.symbol == symbol {
                    killed[block_index].set(other, true);
                }
            }
        }
    }

    let mut entry: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];
    let mut exit: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];
    let mut new_entry = BitVec::repeat(false, definition_count);

    loop {
        let mut changed = false;
        for index in 0..block_count {
            let block = BlockId(index as u32);
            new_entry.fill(false);
            if block != cfg.entry {
                for &predecessor in cfg.predecessors(block) {
                    if reachability.is_reachable(predecessor) {
                        new_entry |= &exit[predecessor.index()];
                    }
                }
            }

            let mut new_exit = last_generated[index].clone();
            for id in new_entry.iter_ones() {
                if !killed[index][id] {
                    new_exit.set(id, true);
                }
            }

            if new_entry != entry[index] || new_exit != exit[index] {
                entry[index].clone_from(&new_entry);
                exit[index].clone_from(&new_exit);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    (entry, exit)
}
