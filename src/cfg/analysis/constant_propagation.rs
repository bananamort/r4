//! Reaching-definition based constant propagation.
//!
//! The pass first computes reaching definitions: for every basic block, the set
//! of definitions that may reach its entry and exit. This is a classic forward
//! *may* analysis over definition identifiers, which behaves correctly through
//! loops.
//!
//! A second pass assigns a value to each definition when it can be proven:
//! literals evaluate directly, and definitions whose initializers only read
//! symbols with a unique reaching constant evaluate to that constant. Finally,
//! a symbol is reported as constant at a program point only when *all* of its
//! reaching definitions agree on the same value.
//!
//! Locals captured by nested functions, iteration variables and parameters are
//! never tracked because their values can change or are defined by the runtime.
//! This feeds [`fold_constant_branches_with_propagation`], which turns
//! conditions such as `local enabled = false; if enabled then ... end` into
//! unconditional jumps.

use std::collections::{HashMap, HashSet};
use std::fmt;

use bitvec::prelude::*;

use crate::cfg::{BlockId, ControlFlowGraph, Terminator};
use crate::nodes::{Expression, Identifier, Statement, Variable};
use crate::process::{DefaultVisitor, Evaluator, LuaValue, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId, SymbolKind};

use super::constant_folding::fold_constant_branches_with;
use super::Reachability;

/// Identifies a definition inside the reaching-definition analysis.
type DefId = usize;

/// An error produced while propagating constants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstantPropagationError {
    /// An identifier (a declaration or a read) is not bound in the semantic
    /// model. This means the CFG was not built from the analyzed AST.
    UnresolvedIdentifier {
        /// The name of the unresolved identifier.
        name: String,
        /// The source line of the identifier, if known.
        line: Option<usize>,
    },
}

impl fmt::Display for ConstantPropagationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstantPropagationError::UnresolvedIdentifier { name, line } => match line {
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
        }
    }
}

impl std::error::Error for ConstantPropagationError {}

#[derive(Clone, Debug)]
struct Definition {
    symbol: SymbolId,
    /// The assigned expression when it is simple enough to evaluate.
    expression: Option<Expression>,
    /// A value known regardless of the environment (`function` expressions).
    constant: Option<LuaValue>,
    /// Symbols read by `expression`, for environment-aware evaluation.
    reads: Vec<SymbolId>,
}

/// Constants that must hold at the entry and exit of every block.
#[derive(Clone, Debug)]
pub struct ConstantPropagation {
    entry: Vec<HashMap<SymbolId, LuaValue>>,
    exit: Vec<HashMap<SymbolId, LuaValue>>,
}

impl ConstantPropagation {
    /// Runs the analysis on a graph.
    ///
    /// Declarations and reads are resolved through the structural bindings
    /// recorded by semantic analysis, so the AST does not need to be parsed
    /// with token preservation.
    pub fn analyze(
        cfg: &ControlFlowGraph,
        model: &SemanticModel,
    ) -> Result<Self, ConstantPropagationError> {
        let evaluator = Evaluator::default();
        let reachability = Reachability::analyze(cfg);
        let (definitions, block_definitions) = collect_definitions(cfg, &evaluator, model)?;
        let (entry_reaching, exit_reaching) =
            reaching_definitions(cfg, &definitions, &block_definitions, &reachability);
        let values = propagate_values(
            &definitions,
            &block_definitions,
            &entry_reaching,
            &evaluator,
            model,
        );

        let entry = derive_constants(cfg, &definitions, &entry_reaching, &values);
        let exit = derive_constants(cfg, &definitions, &exit_reaching, &values);

        Ok(Self { entry, exit })
    }

    /// Returns the constants guaranteed at the entry of a block.
    #[inline]
    pub fn block_entry_constants(&self, block: BlockId) -> &HashMap<SymbolId, LuaValue> {
        &self.entry[block.index()]
    }

    /// Returns the constants guaranteed at the terminator of a block.
    #[inline]
    pub fn block_exit_constants(&self, block: BlockId) -> &HashMap<SymbolId, LuaValue> {
        &self.exit[block.index()]
    }

    /// Returns the constant value of a symbol at the entry of a block, when the
    /// value is the same on every reaching path.
    #[inline]
    pub fn constant_at_block_entry(&self, block: BlockId, symbol: SymbolId) -> Option<&LuaValue> {
        self.entry[block.index()].get(&symbol)
    }
}

/// Folds branch conditions using propagated constants.
///
/// Unlike [`fold_constant_branches`](super::fold_constant_branches),
/// identifiers are resolved through the reaching-definition environment, so
/// conditions such as `local enabled = false; if enabled then ... end` are
/// folded too.
pub fn fold_constant_branches_with_propagation(
    cfg: &mut ControlFlowGraph,
    model: &SemanticModel,
) -> Result<usize, ConstantPropagationError> {
    let analysis = ConstantPropagation::analyze(cfg, model)?;
    let evaluator = Evaluator::default();

    let decisions: Vec<Option<bool>> = cfg
        .iter_block_ids()
        .map(|block| match cfg.terminator(block) {
            Terminator::Branch { condition, .. } => {
                let constants = analysis.block_exit_constants(block);
                evaluator
                    .evaluate_with(condition, &|identifier: &Identifier| {
                        model
                            .resolve_identifier(identifier)
                            .and_then(|symbol| constants.get(&symbol).cloned())
                    })
                    .and_then(|value| value.is_truthy())
            }
            _ => None,
        })
        .collect();

    Ok(fold_constant_branches_with(cfg, |block, _| {
        decisions[block.index()]
    }))
}

// ----------------------------------------------------------------------
// Definition collection
// ----------------------------------------------------------------------

fn collect_definitions(
    cfg: &ControlFlowGraph,
    evaluator: &Evaluator,
    model: &SemanticModel,
) -> Result<(Vec<Definition>, Vec<Vec<DefId>>), ConstantPropagationError> {
    let mut definitions = Vec::new();
    let mut block_definitions = Vec::with_capacity(cfg.blocks.len());

    for block in &cfg.blocks {
        let mut ids = Vec::new();
        for statement in &block.statements {
            collect_statement_definitions(statement, evaluator, model, &mut definitions, &mut ids)?;
        }
        block_definitions.push(ids);
    }

    Ok((definitions, block_definitions))
}

fn collect_statement_definitions(
    statement: &Statement,
    evaluator: &Evaluator,
    model: &SemanticModel,
    definitions: &mut Vec<Definition>,
    block_definitions: &mut Vec<DefId>,
) -> Result<(), ConstantPropagationError> {
    match statement {
        Statement::LocalAssign(assignment) => {
            let values: Vec<&Expression> = assignment.iter_values().collect();

            for (index, variable) in assignment.iter_variables().enumerate() {
                let symbol = resolve_declaration(model, variable.get_identifier())?;
                if !trackable(model, symbol) {
                    continue;
                }
                let expression = initializer_at(&values, index, evaluator);
                push_definition(
                    symbol,
                    expression,
                    None,
                    model,
                    definitions,
                    block_definitions,
                )?;
            }
        }
        Statement::Assign(assignment) => {
            let values: Vec<&Expression> = assignment.iter_values().collect();

            for (index, variable) in assignment.iter_variables().enumerate() {
                if let Variable::Identifier(identifier) = variable {
                    let symbol = resolve_declaration(model, identifier)?;
                    if !trackable(model, symbol) {
                        continue;
                    }
                    let expression = initializer_at(&values, index, evaluator);
                    push_definition(
                        symbol,
                        expression,
                        None,
                        model,
                        definitions,
                        block_definitions,
                    )?;
                }
            }
        }
        Statement::CompoundAssign(assignment) => {
            if let Variable::Identifier(identifier) = assignment.get_variable() {
                let symbol = resolve_declaration(model, identifier)?;
                if trackable(model, symbol) {
                    push_definition(symbol, None, None, model, definitions, block_definitions)?;
                }
            }
        }
        Statement::LocalFunction(function) => {
            let symbol = resolve_declaration(model, function.get_identifier())?;
            if trackable(model, symbol) {
                push_definition(
                    symbol,
                    None,
                    Some(LuaValue::Function),
                    model,
                    definitions,
                    block_definitions,
                )?;
            }
        }
        Statement::Function(function) => {
            let name = function.get_name();
            if name.get_field_names().is_empty() && name.get_method().is_none() {
                let symbol = resolve_declaration(model, name.get_name())?;
                if trackable(model, symbol) {
                    push_definition(
                        symbol,
                        None,
                        Some(LuaValue::Function),
                        model,
                        definitions,
                        block_definitions,
                    )?;
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn push_definition(
    symbol: SymbolId,
    mut expression: Option<Expression>,
    constant: Option<LuaValue>,
    model: &SemanticModel,
    definitions: &mut Vec<Definition>,
    block_definitions: &mut Vec<DefId>,
) -> Result<(), ConstantPropagationError> {
    let reads = match &mut expression {
        Some(expression) => collect_reads(expression, model)?,
        None => Vec::new(),
    };

    definitions.push(Definition {
        symbol,
        expression,
        constant,
        reads,
    });
    block_definitions.push(definitions.len() - 1);
    Ok(())
}

/// Returns the expression assigned to the `index`-th assignment target.
///
/// Missing targets receive `nil` unless the last value can expand into them, in
/// which case the assigned value is unknown.
fn initializer_at(
    values: &[&Expression],
    index: usize,
    evaluator: &Evaluator,
) -> Option<Expression> {
    if let Some(value) = values.get(index) {
        Some((*value).clone())
    } else {
        match values.last() {
            Some(last) if evaluator.can_return_multiple_values(last) => None,
            _ => Some(Expression::nil()),
        }
    }
}

fn collect_reads(
    expression: &mut Expression,
    model: &SemanticModel,
) -> Result<Vec<SymbolId>, ConstantPropagationError> {
    struct ReadsCollector<'a> {
        model: &'a SemanticModel,
        symbols: Vec<SymbolId>,
        error: Option<ConstantPropagationError>,
    }

    impl NodeProcessor for ReadsCollector<'_> {
        fn process_variable_expression(&mut self, identifier: &mut Identifier) {
            if self.error.is_some() {
                return;
            }
            match self.model.resolve_identifier(identifier) {
                Some(symbol) => self.symbols.push(symbol),
                None => {
                    self.error = Some(ConstantPropagationError::UnresolvedIdentifier {
                        name: identifier.get_name().clone(),
                        line: identifier
                            .get_token()
                            .and_then(|token| token.get_line_number()),
                    });
                }
            }
        }
    }

    let mut collector = ReadsCollector {
        model,
        symbols: Vec::new(),
        error: None,
    };
    DefaultVisitor::visit_expression(expression, &mut collector);

    if let Some(error) = collector.error {
        return Err(error);
    }

    collector.symbols.sort_unstable();
    collector.symbols.dedup();
    Ok(collector.symbols)
}

// ----------------------------------------------------------------------
// Reaching definitions
// ----------------------------------------------------------------------

fn reaching_definitions(
    cfg: &ControlFlowGraph,
    definitions: &[Definition],
    block_definitions: &[Vec<DefId>],
    reachability: &Reachability,
) -> (Vec<BitVec>, Vec<BitVec>) {
    let block_count = cfg.blocks.len();
    let definition_count = definitions.len();

    let mut definitions_by_symbol: HashMap<SymbolId, Vec<DefId>> = HashMap::new();
    for (id, definition) in definitions.iter().enumerate() {
        definitions_by_symbol
            .entry(definition.symbol)
            .or_default()
            .push(id);
    }

    let mut generated: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];
    let mut killed: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];

    for (index, ids) in block_definitions.iter().enumerate() {
        for &id in ids {
            generated[index].set(id, true);
            let symbol = definitions[id].symbol;
            if let Some(symbol_definitions) = definitions_by_symbol.get(&symbol) {
                for &other in symbol_definitions {
                    killed[index].set(other, true);
                }
            }
        }
    }

    let mut entry: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];
    let mut exit: Vec<BitVec> = vec![BitVec::repeat(false, definition_count); block_count];

    // Reuse scratch buffers across blocks and iterations.
    let mut new_entry = BitVec::repeat(false, definition_count);
    let mut new_exit = BitVec::repeat(false, definition_count);

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

            new_exit.clone_from(&generated[index]);
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

// ----------------------------------------------------------------------
// Value propagation
// ----------------------------------------------------------------------

fn propagate_values(
    definitions: &[Definition],
    block_definitions: &[Vec<DefId>],
    entry_reaching: &[BitVec],
    evaluator: &Evaluator,
    model: &SemanticModel,
) -> Vec<Option<LuaValue>> {
    let mut values: Vec<Option<LuaValue>> = definitions
        .iter()
        .map(|definition| definition.constant.clone())
        .collect();

    // Reuse one scratch reaching-definition bitset across blocks and
    // iterations instead of cloning per block.
    let mut reaching = entry_reaching
        .first()
        .map(|bits| BitVec::repeat(false, bits.len()))
        .unwrap_or_default();

    loop {
        let mut changed = false;

        for (index, ids) in block_definitions.iter().enumerate() {
            reaching.clone_from(&entry_reaching[index]);

            for &id in ids {
                let value = compute_definition_value(
                    &definitions[id],
                    &reaching,
                    definitions,
                    &values,
                    evaluator,
                    model,
                );

                if values[id] != value {
                    values[id] = value;
                    changed = true;
                }

                let symbol = definitions[id].symbol;
                for (other, definition) in definitions.iter().enumerate() {
                    if definition.symbol == symbol {
                        reaching.set(other, false);
                    }
                }
                reaching.set(id, true);
            }
        }

        if !changed {
            break;
        }
    }

    values
}

fn compute_definition_value(
    definition: &Definition,
    reaching: &BitVec,
    definitions: &[Definition],
    values: &[Option<LuaValue>],
    evaluator: &Evaluator,
    model: &SemanticModel,
) -> Option<LuaValue> {
    if let Some(constant) = &definition.constant {
        return Some(constant.clone());
    }

    let expression = definition.expression.as_ref()?;

    let mut environment: HashMap<SymbolId, LuaValue> = HashMap::new();
    for &symbol in &definition.reads {
        let mut found: Option<&LuaValue> = None;

        for id in reaching.iter_ones() {
            if definitions[id].symbol != symbol {
                continue;
            }
            let value = values[id].as_ref()?;
            match found {
                None => found = Some(value),
                Some(previous) if previous == value => {}
                _ => return None,
            }
        }

        environment.insert(symbol, found?.clone());
    }

    let lookup = |identifier: &Identifier| {
        model
            .resolve_identifier(identifier)
            .and_then(|symbol| environment.get(&symbol).cloned())
    };
    evaluator.evaluate_with(expression, &lookup)
}

// ----------------------------------------------------------------------
// Constant derivation
// ----------------------------------------------------------------------

fn derive_constants(
    cfg: &ControlFlowGraph,
    definitions: &[Definition],
    reaching: &[BitVec],
    values: &[Option<LuaValue>],
) -> Vec<HashMap<SymbolId, LuaValue>> {
    cfg.iter_block_ids()
        .map(|block| {
            let mut constants: HashMap<SymbolId, LuaValue> = HashMap::new();
            let mut conflicting: HashSet<SymbolId> = HashSet::new();

            for id in reaching[block.index()].iter_ones() {
                let definition = &definitions[id];
                match &values[id] {
                    Some(value) => match constants.get(&definition.symbol) {
                        Some(previous) if previous != value => {
                            conflicting.insert(definition.symbol);
                        }
                        Some(_) => {}
                        None => {
                            constants.insert(definition.symbol, value.clone());
                        }
                    },
                    None => {
                        conflicting.insert(definition.symbol);
                    }
                }
            }

            for symbol in conflicting {
                constants.remove(&symbol);
            }

            constants
        })
        .collect()
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// Returns whether a symbol's value can be tracked as a constant.
///
/// Captured locals can be modified whenever a nested function runs, and
/// iteration variables and parameters are defined by the runtime.
fn trackable(model: &SemanticModel, symbol: SymbolId) -> bool {
    let symbol = model.symbol(symbol);
    symbol.kind == SymbolKind::Local && symbol.capturing_scopes.is_empty()
}

fn resolve_declaration(
    model: &SemanticModel,
    identifier: &Identifier,
) -> Result<SymbolId, ConstantPropagationError> {
    model.resolve_identifier(identifier).ok_or_else(|| {
        ConstantPropagationError::UnresolvedIdentifier {
            name: identifier.get_name().clone(),
            line: identifier
                .get_token()
                .and_then(|token| token.get_line_number()),
        }
    })
}
