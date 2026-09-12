use crate::nodes::{
    Block, Expression, FieldExpression, Identifier, IndexExpression, Prefix, TableEntry,
    TableExpression, TableFieldEntry, Variable,
};
use crate::process::utils::is_valid_identifier;
use crate::process::{DefaultVisitor, Evaluator, LuaValue, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

use std::mem;

#[derive(Debug, Clone, Default)]
struct Converter {
    evaluator: Evaluator,
}

impl Converter {
    #[inline]
    fn convert_index_to_field(&self, index: &IndexExpression) -> Option<FieldExpression> {
        self.convert_to_field(index.get_index())
            .map(|key| FieldExpression::new(index.get_prefix().clone(), Identifier::new(key)))
    }

    fn convert_to_field(&self, key_expression: &Expression) -> Option<String> {
        if let LuaValue::String(string) = self.evaluator.evaluate(key_expression) {
            String::from_utf8(string)
                .ok()
                .filter(|string| is_valid_identifier(string))
        } else {
            None
        }
    }
}

impl NodeProcessor for Converter {
    fn process_expression(&mut self, expression: &mut Expression) {
        let field: Option<Expression> = if let Expression::Index(index) = expression {
            self.convert_index_to_field(index).map(Into::into)
        } else {
            None
        };
        if let Some(mut field) = field {
            mem::swap(expression, &mut field);
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        let field: Option<Prefix> = if let Prefix::Index(index) = prefix {
            self.convert_index_to_field(index).map(Into::into)
        } else {
            None
        };
        if let Some(mut field) = field {
            mem::swap(prefix, &mut field);
        }
    }

    fn process_variable(&mut self, variable: &mut Variable) {
        let field: Option<Variable> = if let Variable::Index(index) = variable {
            self.convert_index_to_field(index).map(Into::into)
        } else {
            None
        };
        if let Some(mut field) = field {
            mem::swap(variable, &mut field);
        }
    }

    fn process_table_expression(&mut self, table: &mut TableExpression) {
        for entry in table.iter_mut_entries() {
            let replace_with = match entry {
                TableEntry::Index(entry) => self
                    .convert_to_field(entry.get_key())
                    .map(|key| TableFieldEntry::new(key, entry.get_value().clone()))
                    .map(TableEntry::from),

                TableEntry::Field(_) | TableEntry::Value(_) => None,
            };
            if let Some(new_entry) = replace_with {
                *entry = new_entry;
            }
        }
    }
}

/// Converts index expressions with string-literal keys into field expressions
/// (`t["foo"]` becomes `t.foo`).
pub fn convert_index_to_field(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Converter::default();
    DefaultVisitor::visit_block(block, &mut processor);
}
