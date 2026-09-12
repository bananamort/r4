use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::cfg::analysis::LicmError;
use crate::cfg::{
    build_chunk, validate_block_identities, CfgError, StatementOrigin, UnidentifiedBlock,
};
use crate::nodes::{Block, Identifier, Statement, VariableAssignment};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{SemanticModel, SymbolId};

/// An error produced while hoisting loop-invariant code.
#[derive(Clone, Debug, PartialEq)]
pub enum HoistLoopInvariantsError {
    /// The AST and the semantic model do not match.
    Provenance(UnidentifiedBlock),
    /// The control flow graph could not be built.
    Cfg(CfgError),
    /// Liveness could not be computed.
    Liveness(LicmError),
}

impl From<UnidentifiedBlock> for HoistLoopInvariantsError {
    fn from(error: UnidentifiedBlock) -> Self {
        Self::Provenance(error)
    }
}

impl From<CfgError> for HoistLoopInvariantsError {
    fn from(error: CfgError) -> Self {
        Self::Cfg(error)
    }
}

impl From<LicmError> for HoistLoopInvariantsError {
    fn from(error: LicmError) -> Self {
        Self::Liveness(error)
    }
}

impl fmt::Display for HoistLoopInvariantsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HoistLoopInvariantsError::Provenance(error) => write!(f, "{}", error),
            HoistLoopInvariantsError::Cfg(error) => write!(f, "{}", error),
            HoistLoopInvariantsError::Liveness(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for HoistLoopInvariantsError {}

/// Hoists proven loop-invariant local declarations out of natural loops.
///
/// The analysis in [`crate::cfg::analysis::LoopInvariantAnalysis`] decides what
/// can move; this pass performs the rewrite:
///
/// * each hoisted declaration is inserted immediately before its loop
///   statement, in source order,
/// * the original statement is removed from the loop body,
/// * the hoisted symbol is renamed to a collision-free fresh identifier
///   allocated by [`NameGenerator`], and every occurrence bound to the symbol
///   is renamed with it.
///
/// Returns the number of hoisted statements. Statements that cannot be
/// strictly proven safe are left untouched.
pub fn hoist_loop_invariants(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, HoistLoopInvariantsError> {
    validate_block_identities(block)?;
    let cfg = build_chunk(block, model)?;
    let analysis = crate::cfg::analysis::LoopInvariantAnalysis::analyze(&cfg, model)?;

    if analysis.is_empty() {
        return Ok(0);
    }

    let mut names = NameGenerator::new(model);
    let mut renames: HashMap<SymbolId, String> = HashMap::new();
    let mut names_by_origin: HashMap<StatementOrigin, String> = HashMap::new();
    let mut plan: HashMap<StatementOrigin, Vec<StatementOrigin>> = HashMap::new();
    let mut hoisted_origins: HashSet<StatementOrigin> = HashSet::new();
    let mut count = 0;

    for hoist in analysis.hoists() {
        let mut origins = Vec::with_capacity(hoist.statements.len());
        for statement in &hoist.statements {
            let name = renames
                .entry(statement.symbol)
                .or_insert_with(|| names.generate())
                .clone();
            origins.push(statement.origin);
            names_by_origin.insert(statement.origin, name);
            hoisted_origins.insert(statement.origin);
            count += 1;
        }
        plan.insert(hoist.loop_statement, origins);
    }

    // Phase 1: extract a renamed clone of every hoisted declaration.
    let mut collector = StatementCollector {
        wanted: &hoisted_origins,
        names: &names_by_origin,
        found: HashMap::new(),
    };
    DefaultVisitor::visit_block(block, &mut collector);

    let hoisted: HashMap<StatementOrigin, Statement> = hoisted_origins
        .iter()
        .map(|origin| {
            let statement = collector
                .found
                .remove(origin)
                .expect("every hoisted statement was found by the collector");
            (*origin, statement)
        })
        .collect();

    // Phase 2: rename every occurrence bound to a hoisted symbol.
    let mut renamer = OccurrenceRenamer {
        model,
        renames: &renames,
    };
    DefaultVisitor::visit_block(block, &mut renamer);

    // Phase 3: move the clones and drop the originals.
    let mut mover = LoopHoistMover {
        plan: plan
            .into_iter()
            .map(|(loop_origin, origins)| {
                let statements = origins
                    .into_iter()
                    .map(|origin| {
                        hoisted
                            .get(&origin)
                            .expect("every planned statement was collected")
                            .clone()
                    })
                    .collect();
                (loop_origin, statements)
            })
            .collect(),
        hoisted_origins,
    };
    DefaultVisitor::visit_block(block, &mut mover);

    Ok(count)
}

struct StatementCollector<'a> {
    wanted: &'a HashSet<StatementOrigin>,
    names: &'a HashMap<StatementOrigin, String>,
    found: HashMap<StatementOrigin, Statement>,
}

impl NodeProcessor for StatementCollector<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let identity = block
            .get_identity()
            .expect("block identities were validated before the rewrite");
        for (index, statement) in block.iter_statements().enumerate() {
            let origin = StatementOrigin {
                block: identity,
                index: index as u32,
            };
            if !self.wanted.contains(&origin) {
                continue;
            }

            let mut statement = statement.clone();
            if let Statement::LocalAssign(assignment) = &mut statement {
                for variable in assignment.iter_mut_variables() {
                    if let Some(name) = self.names.get(&origin) {
                        variable.set_name(name.clone());
                    }
                }
            }
            self.found.insert(origin, statement);
        }
    }
}

struct OccurrenceRenamer<'a> {
    model: &'a SemanticModel,
    renames: &'a HashMap<SymbolId, String>,
}

impl OccurrenceRenamer<'_> {
    fn rename(&self, identifier: &mut Identifier) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(name) = self.renames.get(&symbol) {
            identifier.set_name(name.clone());
        }
    }
}

impl NodeProcessor for OccurrenceRenamer<'_> {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        self.rename(identifier);
    }

    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        for variable in assignment.iter_mut_variables() {
            let identifier: &mut Identifier = variable;
            self.rename(identifier);
        }
    }
}

struct LoopHoistMover {
    plan: HashMap<StatementOrigin, Vec<Statement>>,
    hoisted_origins: HashSet<StatementOrigin>,
}

impl NodeProcessor for LoopHoistMover {
    fn process_block(&mut self, block: &mut Block) {
        let identity = block
            .get_identity()
            .expect("block identities were validated before the rewrite");
        let statements = block.take_statements();
        let mut result = Vec::with_capacity(statements.len());

        for (index, statement) in statements.into_iter().enumerate() {
            let origin = StatementOrigin {
                block: identity,
                index: index as u32,
            };
            if self.hoisted_origins.contains(&origin) {
                continue;
            }
            if let Some(hoisted) = self.plan.get(&origin) {
                result.extend(hoisted.iter().cloned());
            }
            result.push(statement);
        }

        block.set_statements(result);
    }
}
