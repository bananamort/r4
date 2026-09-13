//! Removal of stores to fields that can never be observed.
//!
//! [`remove_dead_table_stores`] removes `table.field = value` assignments when
//! the table is a *confined* local table (see
//! [`EscapeAnalysis`](crate::semantic::EscapeAnalysis)) whose field is never
//! read, and the assigned value has no side effects.
//!
//! The pass is deliberately conservative. It only fires when every occurrence
//! of the table is a static field access, the table is never captured or
//! reassigned, and it is never read through a dynamic index. Removing the store
//! is then sound: no metatable can be attached (that would require passing the
//! table to another function), no field is observed, and the value has no
//! effects.

use crate::nodes::{Block, Expression, Prefix, Statement, Variable};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{EscapeAnalysis, SemanticModel};

/// Removes assignments to never-read fields of confined local tables.
///
/// Returns the number of removed assignments.
///
/// ```
/// use r4::{Parser, rules::canonicalize::remove_dead_table_stores};
///
/// let (mut block, model) = Parser::default()
///     .parse_analyzed("local t = {}\nt.x = 1\nreturn 0")
///     .unwrap();
/// assert_eq!(remove_dead_table_stores(&mut block, &model), 1);
/// ```
pub fn remove_dead_table_stores(block: &mut Block, model: &SemanticModel) -> usize {
    let analysis = EscapeAnalysis::analyze(block, model);
    let evaluator = Evaluator::default();

    let mut remover = DeadTableStoreRemover {
        model,
        analysis,
        evaluator,
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut remover);
    remover.removed
}

struct DeadTableStoreRemover<'a> {
    model: &'a SemanticModel,
    analysis: EscapeAnalysis,
    evaluator: Evaluator,
    removed: usize,
}

impl DeadTableStoreRemover<'_> {
    fn is_dead_store(&self, statement: &Statement) -> bool {
        let Statement::Assign(assignment) = statement else {
            return false;
        };
        if assignment.variables_len() != 1 || assignment.values_len() != 1 {
            return false;
        }

        let value = assignment
            .iter_values()
            .next()
            .expect("the assignment was checked to have one value");
        if self.evaluator.has_side_effects(value) {
            return false;
        }

        match assignment
            .iter_variables()
            .next()
            .expect("the assignment was checked to have one variable")
        {
            Variable::Field(field) => {
                let Prefix::Identifier(identifier) = field.get_prefix() else {
                    return false;
                };
                let Some(symbol) = self.model.resolve_identifier(identifier) else {
                    return false;
                };
                self.field_is_dead(symbol, field.get_field().get_name())
            }
            Variable::Index(index) => {
                let Prefix::Identifier(identifier) = index.get_prefix() else {
                    return false;
                };
                let Some(symbol) = self.model.resolve_identifier(identifier) else {
                    return false;
                };
                let Some(name) = string_literal(index.get_index()) else {
                    return false;
                };
                self.field_is_dead(symbol, &name)
            }
            Variable::Identifier(_) => false,
        }
    }

    fn field_is_dead(&self, symbol: crate::semantic::SymbolId, name: &str) -> bool {
        self.analysis.is_confined_table(self.model, symbol)
            && !self.analysis.has_dynamic_read(symbol)
            && !self.analysis.is_field_read(symbol, name)
    }
}

impl NodeProcessor for DeadTableStoreRemover<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();
        statements.retain(|statement| {
            if self.is_dead_store(statement) {
                self.removed += 1;
                false
            } else {
                true
            }
        });
        block.set_statements(statements);
    }
}

fn string_literal(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(string) => std::str::from_utf8(string.get_value())
            .ok()
            .map(str::to_owned),
        _ => None,
    }
}
