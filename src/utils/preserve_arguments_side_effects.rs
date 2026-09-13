use crate::{
    nodes::{Arguments, Expression, TableEntry},
    process::Evaluator,
    semantic::{has_side_effects, SemanticModel},
};

pub(crate) fn preserve_arguments_side_effects(
    model: &SemanticModel,
    evaluator: &Evaluator,
    arguments: &Arguments,
) -> Vec<Expression> {
    match arguments {
        Arguments::Tuple(tuple) => tuple
            .iter_values()
            .filter(|value| has_side_effects(model, evaluator, value))
            .cloned()
            .collect(),
        Arguments::Table(table) => {
            let mut expressions = Vec::new();

            for entry in table.iter_entries() {
                match entry {
                    TableEntry::Field(field) => {
                        let expression = field.get_value();
                        if has_side_effects(model, evaluator, expression) {
                            expressions.push(expression.clone());
                        }
                    }
                    TableEntry::Index(index) => {
                        let key = index.get_key();
                        let value = index.get_value();

                        if has_side_effects(model, evaluator, key) {
                            expressions.push(key.clone());
                        }
                        if has_side_effects(model, evaluator, value) {
                            expressions.push(value.clone());
                        }
                    }
                    TableEntry::Value(value) => {
                        if has_side_effects(model, evaluator, value) {
                            expressions.push(*value.clone());
                        }
                    }
                }
            }

            expressions
        }
        Arguments::String(_) => Vec::new(),
    }
}
