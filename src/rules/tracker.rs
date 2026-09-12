//! Shared identifier bookkeeping for transformation passes.
//!
//! The tracker is collected before a pass mutates the AST. It records every
//! identifier occurrence so that generated temporaries can avoid collisions
//! with any name that already appears in the tree.

use std::collections::HashSet;

use crate::nodes::{
    Block, FunctionAssignment, FunctionExpression, FunctionStatement, GenericForStatement,
    Identifier, NumericForStatement, Statement, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

/// Keeps track of the identifiers used by a block.
#[derive(Debug, Default, Clone)]
pub(crate) struct IdentifierTracker {
    used: HashSet<String>,
}

impl IdentifierTracker {
    /// Collects the identifiers of a block and all its nested nodes.
    ///
    /// The block is taken mutably because the visitor API operates on mutable
    /// AST nodes, but the collection itself does not modify it.
    pub fn collect_block(block: &mut Block) -> Self {
        let mut collector = IdentifierCollector::default();
        DefaultVisitor::visit_block(block, &mut collector);
        collector.tracker
    }

    /// Collects the identifiers of a single statement and its nested nodes.
    pub fn collect_statement(statement: &mut Statement) -> Self {
        let mut collector = IdentifierCollector::default();
        DefaultVisitor::visit_statement(statement, &mut collector);
        collector.tracker
    }

    /// Generates an identifier starting with `prefix` that does not collide with
    /// any identifier of the tracked AST or with a previously generated name.
    pub fn generate_with_prefix(&mut self, prefix: impl Into<String>) -> String {
        let prefix = prefix.into();
        if !self.used.contains(&prefix) {
            self.used.insert(prefix.clone());
            return prefix;
        }

        let mut counter = 1_u32;
        loop {
            let candidate = format!("{prefix}{counter}");
            if !self.used.contains(&candidate) {
                self.used.insert(candidate.clone());
                return candidate;
            }
            counter += 1;
        }
    }

    fn insert_declaration(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }
}

#[derive(Default)]
struct IdentifierCollector {
    tracker: IdentifierTracker,
}

impl IdentifierCollector {
    fn declare(&mut self, identifier: &Identifier) {
        self.tracker.insert_declaration(identifier.get_name());
    }
}

impl NodeProcessor for IdentifierCollector {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        self.tracker.used.insert(identifier.get_name().clone());
    }

    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        for variable in assignment.iter_variables() {
            self.declare(variable.get_identifier());
        }
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        self.declare(function.get_identifier());
        for parameter in function.iter_parameters() {
            self.declare(parameter.get_identifier());
        }
    }

    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        if function.get_name().get_method().is_some() {
            self.tracker.insert_declaration("self");
        }
        for parameter in function.iter_parameters() {
            self.declare(parameter.get_identifier());
        }
    }

    fn process_function_expression(&mut self, function: &mut FunctionExpression) {
        for parameter in function.iter_parameters() {
            self.declare(parameter.get_identifier());
        }
    }

    fn process_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        for identifier in statement.iter_identifiers() {
            self.declare(identifier.get_identifier());
        }
    }

    fn process_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        self.declare(statement.get_identifier().get_identifier());
    }
}
