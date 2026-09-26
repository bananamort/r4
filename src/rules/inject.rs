use std::collections::HashMap;
use std::fmt;

use crate::nodes::{Block, Expression, Identifier, Prefix};
use crate::process::{DefaultVisitor, LuaValue, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolKind};

/// An error produced while injecting global constant values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectGlobalValueError {
    /// A configured value has no faithful literal representation, so injecting
    /// it would emit a broken or unparseable node.
    UnrepresentableValue {
        /// The configured global name.
        name: String,
    },
}

impl fmt::Display for InjectGlobalValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InjectGlobalValueError::UnrepresentableValue { name } => write!(
                f,
                "the value configured for global `{}` cannot be represented as a literal",
                name
            ),
        }
    }
}

impl std::error::Error for InjectGlobalValueError {}

struct Processor<'a> {
    model: &'a SemanticModel,
    replacements: &'a HashMap<String, Expression>,
    replaced: usize,
}

impl Processor<'_> {
    /// Resolves a read of `identifier` to an injectable global value.
    ///
    /// A local, parameter or captured name never matches unless it is an
    /// immutable alias of the global, and a global that is written anywhere is
    /// not a constant and is left alone.
    fn replacement(&self, identifier: &Identifier) -> Option<Expression> {
        let symbol = self.model.resolve_identifier(identifier)?;

        let global = match self.model.symbol(symbol).kind {
            SymbolKind::Global => symbol,
            _ => self.model.immutable_global_alias(symbol)?,
        };

        let global_symbol = self.model.symbol(global);
        if global_symbol.write_count != 0 {
            return None;
        }

        self.replacements.get(&global_symbol.name).cloned()
    }
}

impl NodeProcessor for Processor<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Identifier(identifier) = expression {
            if let Some(replacement) = self.replacement(identifier) {
                *expression = replacement;
                self.replaced += 1;
            }
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        if let Prefix::Identifier(identifier) = prefix {
            if let Some(replacement) = self.replacement(identifier) {
                *prefix = Prefix::from(replacement);
                self.replaced += 1;
            }
        }
    }
}

/// Replaces reads of configured globals with constant literal expressions.
///
/// Every occurrence is resolved through the [`SemanticModel`], so a local,
/// parameter or closure capture that shadows the name is never touched. Only a
/// global (or an immutable alias of it) that is never written is injected.
/// Values that cannot be represented as literals are rejected up front with
/// [`InjectGlobalValueError::UnrepresentableValue`]; no partial rewrite is
/// performed.
pub fn inject_global_values(
    block: &mut Block,
    model: &SemanticModel,
    values: &HashMap<String, LuaValue>,
) -> Result<usize, InjectGlobalValueError> {
    let mut replacements: HashMap<String, Expression> = HashMap::new();
    for (name, value) in values {
        let expression = literal_expression(value)
            .ok_or_else(|| InjectGlobalValueError::UnrepresentableValue { name: name.clone() })?;
        replacements.insert(name.clone(), expression);
    }

    if replacements.is_empty() {
        return Ok(0);
    }

    let mut processor = Processor {
        model,
        replacements: &replacements,
        replaced: 0,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    Ok(processor.replaced)
}

/// Returns a literal expression for `value`, or `None` when it is not
/// representable (non-finite numbers, functions, tables, unknown values).
fn literal_expression(value: &LuaValue) -> Option<Expression> {
    match value {
        LuaValue::Number(number) if !number.is_finite() => None,
        LuaValue::True
        | LuaValue::False
        | LuaValue::Integer(_)
        | LuaValue::Nil
        | LuaValue::Number(_)
        | LuaValue::String(_) => value.clone().to_expression(),
        LuaValue::Function | LuaValue::Table | LuaValue::Unknown => None,
    }
}
