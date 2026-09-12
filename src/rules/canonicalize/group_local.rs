use crate::nodes::{Block, Expression, Identifier, Statement, VariableAssignment};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

use std::collections::HashSet;
use std::iter;

/// Finds whether any of the tracked variable names is read by an expression.
#[derive(Debug, Clone, Default)]
struct FindVariables {
    names: HashSet<String>,
    found: bool,
}

impl FindVariables {
    fn has_found_usage(&self) -> bool {
        self.found
    }
}

impl<'a> FromIterator<&'a str> for FindVariables {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        Self {
            names: iter.into_iter().map(str::to_owned).collect(),
            found: false,
        }
    }
}

impl NodeProcessor for FindVariables {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        if !self.found && self.names.contains(identifier.get_name()) {
            self.found = true;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct GroupLocalProcessor {}

impl GroupLocalProcessor {
    fn filter_statements(&self, block: &mut Block) -> Vec<Statement> {
        let mut statements = block.take_statements();
        let mut filter_statements = Vec::new();
        let mut iter = statements.drain(..);
        let mut previous_statement = iter.next();
        let mut current_statement = iter.next();

        while let Some(current) = current_statement {
            previous_statement = if let Some(previous) = previous_statement {
                use Statement::LocalAssign;

                match (previous, current) {
                    (LocalAssign(mut previous), LocalAssign(mut current)) => {
                        if self.should_merge(&previous, &mut current) {
                            self.merge(&mut previous, current);

                            Some(LocalAssign(previous))
                        } else {
                            filter_statements.push(LocalAssign(previous));
                            Some(LocalAssign(current))
                        }
                    }
                    (previous, current) => {
                        filter_statements.push(previous);
                        Some(current)
                    }
                }
            } else {
                None
            };

            current_statement = iter.next();
        }

        if let Some(previous) = previous_statement {
            filter_statements.push(previous);
        }

        filter_statements
    }

    fn should_merge(&self, first: &VariableAssignment, next: &mut VariableAssignment) -> bool {
        let first_value_count = first.values_len();

        if first.variables_len() > first_value_count && first_value_count != 0 {
            return false;
        }

        let mut find_variables: FindVariables = first
            .iter_variables()
            .map(|variable| variable.get_name().as_str())
            .collect();

        next.iter_mut_values().all(|expression| {
            DefaultVisitor::visit_expression(expression, &mut find_variables);
            !find_variables.has_found_usage()
        })
    }

    fn merge(&self, first: &mut VariableAssignment, mut other: VariableAssignment) {
        if first.values_len() == 0 && other.values_len() != 0 {
            let variable_count = first.variables_len();
            first.extend_values(iter::repeat_n(Expression::nil(), variable_count));
        }

        if other.values_len() == 0 && first.values_len() != 0 {
            let variable_count = other.variables_len();
            other.extend_values(iter::repeat_n(Expression::nil(), variable_count));
        }

        let (mut variables, mut values) = other.into_assignments();
        first.append_variables(&mut variables);
        first.append_values(&mut values);
    }
}

impl NodeProcessor for GroupLocalProcessor {
    fn process_block(&mut self, block: &mut Block) {
        let filter_statements = self.filter_statements(block);

        block.set_statements(filter_statements);
    }
}

/// Groups consecutive local assignments into a single statement when the
/// second statement does not read the variables of the first one.
pub fn group_local(block: &mut Block) {
    let mut processor = GroupLocalProcessor::default();
    DefaultVisitor::visit_block(block, &mut processor);
}
