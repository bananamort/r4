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

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate, parse};
    use crate::semantic::{SemanticAnalyzer, SymbolKind};

    #[test]
    fn hoists_pure_loop_invariants() {
        let (mut block, model) = analyze(
            r#"
            local base = 1
            while condition do
                local offset = base + 2
                use(offset)
            end
            "#,
        );

        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(hoisted, 1);
        let output = generate(&block);
        assert!(
            !output.contains("offset"),
            "the hoisted local must be renamed: {output}"
        );
        let declaration = output
            .find("= base + 2")
            .unwrap_or_else(|| panic!("the invariant must be declared: {output}"));
        let loop_start = output
            .find("while")
            .unwrap_or_else(|| panic!("the loop must remain: {output}"));
        assert!(
            declaration < loop_start,
            "the invariant must be hoisted before the loop: {output}"
        );
        assert_eq!(
            output.matches("base + 2").count(),
            1,
            "the invariant must evaluate exactly once: {output}"
        );

        // Semantic oracle: the rewritten program analyzes and reads the hoisted
        // local exactly once.
        let reparsed = parse(&output);
        let new_model = SemanticAnalyzer::new()
            .analyze(&reparsed)
            .expect("the hoisted program must analyze");
        let locals: Vec<_> = new_model
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Local)
            .collect();
        assert_eq!(locals.len(), 2, "base and the hoisted local: {output}");
        assert!(
            locals.iter().any(|symbol| symbol.read_count == 1),
            "the hoisted local must be read: {output}"
        );
    }

    #[test]
    fn leaves_impure_expressions_in_place() {
        for code in [
            "while condition do local x = compute() use(x) end",
            "while condition do local t = {} use(t) end",
            "while condition do local v = object.field use(v) end",
            "while condition do local s = `value {value}` use(s) end",
        ] {
            let (mut block, model) = analyze(code);
            let hoisted = hoist_loop_invariants(&mut block, &model)
                .expect("hoisting must succeed with a matching model");
            assert_eq!(hoisted, 0, "impure expression hoisted for `{code}`");
        }
    }

    #[test]
    fn respects_loop_written_variables() {
        let code = r#"
            local total = 0
            while condition do
                total = total + 1
                local x = total + 1
                use(x)
            end
            "#;

        let (mut block, model) = analyze(code);
        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(
            hoisted, 0,
            "a variable written in the loop is not invariant"
        );
    }

    #[test]
    fn respects_for_iteration_variables() {
        let (mut block, model) = analyze("for i = 1, 10 do local x = i + 1\nuse(x) end");

        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(hoisted, 0, "the iteration variable changes every iteration");
    }

    #[test]
    fn respects_closure_captures() {
        let code = r#"
            local base = 1
            while condition do
                local x = base + 1
                local f = function() return x end
                use(f)
            end
            "#;

        let (mut block, model) = analyze(code);
        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(hoisted, 0, "a captured local keeps per-iteration bindings");
    }

    #[test]
    fn skips_conditional_blocks() {
        let code = r#"
            local base = 1
            while condition do
                if other then
                    local x = base + 1
                    use(x)
                end
            end
            "#;

        let (mut block, model) = analyze(code);
        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(
            hoisted, 0,
            "a block that does not dominate every latch must not be hoisted"
        );
    }

    #[test]
    fn hoists_from_for_loop_bodies() {
        let code = r#"
            local step = 2
            for i = 1, 10 do
                local scaled = step * 2
                use(scaled)
            end
            "#;

        let (mut block, model) = analyze(code);
        let hoisted = hoist_loop_invariants(&mut block, &model)
            .expect("hoisting must succeed with a matching model");

        assert_eq!(hoisted, 1);
        let output = generate(&block);
        let declaration = output
            .find("= step * 2")
            .unwrap_or_else(|| panic!("the invariant must be declared: {output}"));
        let loop_start = output
            .find("for")
            .unwrap_or_else(|| panic!("the loop must remain: {output}"));
        assert!(
            declaration < loop_start,
            "the invariant must be hoisted before the loop: {output}"
        );
        assert!(
            !output.contains("local scaled"),
            "the original declaration must be removed: {output}"
        );
    }
}
