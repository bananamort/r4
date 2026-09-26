use crate::nodes::{Block, Expression, Identifier, Statement, VariableAssignment};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId};

use std::collections::HashSet;
use std::iter;

/// Finds whether an expression reads any of the tracked symbols.
struct SymbolReads<'a> {
    model: &'a SemanticModel,
    symbols: HashSet<SymbolId>,
    found: bool,
}

impl NodeProcessor for SymbolReads<'_> {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        if self.found {
            return;
        }
        match self.model.resolve_identifier(identifier) {
            Some(symbol) if self.symbols.contains(&symbol) => self.found = true,
            // An unresolvable identifier cannot be proven unrelated.
            None => self.found = true,
            Some(_) => {}
        }
    }
}

#[derive(Clone, Debug)]
struct GroupLocalProcessor<'a> {
    model: &'a SemanticModel,
}

impl GroupLocalProcessor<'_> {
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
        // A `<close>` local becomes active only after its whole statement
        // completes; merging declarations would change when it closes. Keep any
        // attributed declaration in its own statement.
        if first
            .iter_variables()
            .chain(next.iter_variables())
            .any(|identifier| identifier.get_attribute().is_some())
        {
            return false;
        }

        // Merging appends `next`'s values after `first`'s, so the binding must
        // stay aligned: `first` has to consume exactly its own values. A
        // declaration with more values than variables (`local a = 1, 2`) would
        // otherwise shift every following variable onto a discarded value. A
        // declaration with no values is fine, because `merge` pads it with
        // `nil` before appending the next values.
        let first_value_count = first.values_len();
        if first_value_count != 0 && first.variables_len() != first_value_count {
            return false;
        }

        let mut symbols = HashSet::new();
        for variable in first.iter_variables() {
            match self.model.resolve_identifier(variable.get_identifier()) {
                Some(symbol) => {
                    symbols.insert(symbol);
                }
                // Without a binding we cannot prove the merge safe.
                None => return false,
            }
        }

        let mut reads = SymbolReads {
            model: self.model,
            symbols,
            found: false,
        };
        next.iter_mut_values().all(|expression| {
            DefaultVisitor::visit_expression(expression, &mut reads);
            !reads.found
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

impl NodeProcessor for GroupLocalProcessor<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let filter_statements = self.filter_statements(block);

        block.set_statements(filter_statements);
    }
}

/// Groups consecutive local assignments into a single statement when the
/// second statement does not read the variables of the first one.
///
/// Every read resolves through the semantic model, so unrelated same-name
/// locals in nested scopes do not block a merge. An identifier that cannot be
/// resolved is treated as an unproven read and prevents the merge.
pub fn group_local(block: &mut Block, model: &SemanticModel) {
    let mut processor = GroupLocalProcessor { model };
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate, parse};

    #[test]
    fn groups_adjacent_local_assignments() {
        let (mut block, model) = analyze(
            r#"
            local a = 1
            local b = 2
            "#,
        );

        group_local(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"local a, b = 1, 2");
    }

    #[test]
    fn does_not_merge_when_extra_values_would_shift_the_binding() {
        let (mut block, model) = analyze(
            r#"
            local a = 1, 2
            local b = 3
            "#,
        );

        group_local(&mut block, &model);

        let output = generate(&block);
        assert!(output.contains("local a = 1, 2"), "unexpected: {output}");
        assert!(output.contains("local b = 3"), "unexpected: {output}");
        parse(&output);
    }

    #[test]
    fn merges_extra_values_that_are_already_last() {
        let (mut block, model) = analyze(
            r#"
            local a = 1
            local b = 2, 3
            "#,
        );

        group_local(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"local a, b = 1, 2, 3");
    }
}
