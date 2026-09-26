use crate::nodes::{AssignmentKind, Block, Expression, VariableAssignment};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor<'a> {
    model: &'a SemanticModel,
    evaluator: Evaluator,
}

impl NodeProcessor for Processor<'_> {
    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        if let AssignmentKind::Const = assignment.get_assignment_kind() {
            return;
        }

        // Dropping a trailing `nil` reorders the declared identifiers; an
        // attributed declaration (`<close>`/`<const>`) must keep its binding, so
        // leave the whole statement untouched.
        if assignment
            .iter_variables()
            .any(|identifier| identifier.get_attribute().is_some())
        {
            return;
        }

        {
            let mut pop_extra_value_at = Vec::new();
            for (index, extra_value) in assignment
                .iter_values()
                .enumerate()
                .skip(assignment.variables_len())
            {
                if !crate::semantic::has_side_effects(self.model, &self.evaluator, extra_value) {
                    pop_extra_value_at.push(index);
                }
            }

            for index in pop_extra_value_at.into_iter().rev() {
                assignment.remove_value(index);
            }
        }

        if assignment.values_len() > assignment.variables_len() {
            return;
        }

        let has_nil_value = assignment
            .iter_values()
            .any(|value| matches!(value, Expression::Nil(_)));

        if !has_nil_value {
            return;
        }

        if assignment.variables_len() > assignment.values_len()
            && assignment
                .last_value()
                .filter(|last_value| self.evaluator.can_return_multiple_values(last_value))
                .is_some()
        {
            return;
        }

        let mut remove_values_at = Vec::new();
        for (index, value) in assignment.iter_values().enumerate() {
            if matches!(value, Expression::Nil(_)) {
                remove_values_at.push(index);
            }
        }

        let insert_variables: Vec<_> = remove_values_at
            .into_iter()
            .rev()
            .filter_map(|index| {
                assignment.remove_value(index);
                assignment.remove_variable(index)
            })
            .collect();

        for variable in insert_variables.into_iter().rev() {
            assignment.push_variable(variable);
        }

        if let Some(last_value) = assignment.last_value() {
            if self.evaluator.can_return_multiple_values(last_value) {
                let new_value = assignment.pop_value().unwrap().in_parentheses();
                assignment.push_value(new_value);
            }
        }
    }
}

/// Removes trailing `nil` values from local assignments.
pub fn remove_nil_declarations(block: &mut Block, model: &SemanticModel) {
    let mut processor = Processor {
        model,
        evaluator: Evaluator::default(),
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn removes_trailing_nil_declarations() {
        let (mut block, model) = analyze("local x, y = 1, nil");

        remove_nil_declarations(&mut block, &model);

        insta::assert_snapshot!(generate(&block).trim(), @"local x, y = 1");
    }
}
