use std::mem;

use crate::nodes::{
    BinaryOperator, Block, CompoundOperator, Expression, FieldExpression, FunctionCall, Prefix,
    Statement, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::desugar::remove_compound_assign::replace_compound_assignment;
use crate::rules::NameGenerator;
use crate::semantic::SemanticModel;

const DEFAULT_MATH_LIBRARY: &str = "math";
const DEFAULT_MATH_FLOOR_NAME: &str = "floor";

struct RemoveFloorDivisionProcessor<'a> {
    model: &'a SemanticModel,
    names: NameGenerator,
    math_floor_identifier: Option<String>,
}

impl RemoveFloorDivisionProcessor<'_> {
    fn build_math_floor_call(&mut self, value: Expression) -> Expression {
        let prefix = if self.model.name_is_transparent_global(DEFAULT_MATH_LIBRARY) {
            FieldExpression::new(
                Prefix::from_name(DEFAULT_MATH_LIBRARY),
                DEFAULT_MATH_FLOOR_NAME,
            )
            .into()
        } else {
            let name = self
                .math_floor_identifier
                .get_or_insert_with(|| self.names.generate_with_prefix("__R4_MATH_FLOOR"))
                .clone();
            Prefix::from_name(name)
        };

        FunctionCall::from_prefix(prefix)
            .with_argument(value)
            .into()
    }
}

impl NodeProcessor for RemoveFloorDivisionProcessor<'_> {
    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::CompoundAssign(assign_statement) = statement {
            if assign_statement.get_operator() == CompoundOperator::DoubleSlash {
                replace_compound_assignment(statement, self.model);
            }
        }
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Binary(binary) = expression {
            if binary.operator() == BinaryOperator::DoubleSlash {
                binary.set_operator(BinaryOperator::Slash);

                let value = mem::replace(expression, Expression::nil());

                *expression = self.build_math_floor_call(value);
            }
        }
    }
}

/// Removes floor division (`//`) by replacing it with `math.floor(a / b)`.
///
/// `math` resolves through the semantic model: read-only aliases such as
/// `local math = math` are transparent, so their uses keep calling
/// `math.floor` directly. A collision-free alias (allocated by
/// [`NameGenerator`]) is only introduced when `math` can denote something else
/// (a mutable or non-alias local).
pub fn remove_floor_division(block: &mut Block, model: &SemanticModel) {
    let mut processor = RemoveFloorDivisionProcessor {
        model,
        names: NameGenerator::new(model),
        math_floor_identifier: None,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    if let Some(identifier) = processor.math_floor_identifier {
        block.insert_statement(
            0,
            VariableAssignment::from_variable(identifier).with_value(FieldExpression::new(
                Prefix::from_name(DEFAULT_MATH_LIBRARY),
                DEFAULT_MATH_FLOOR_NAME,
            )),
        );
    }
}
