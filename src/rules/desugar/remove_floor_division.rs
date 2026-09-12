use std::{mem, ops};

use crate::nodes::{
    BinaryOperator, Block, CompoundOperator, Expression, FieldExpression, FunctionCall, Prefix,
    Statement, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::desugar::remove_compound_assign::replace_compound_assignment;
use crate::rules::tracker::IdentifierTracker;

struct RemoveFloorDivisionProcessor {
    math_floor_identifier: String,
    define_math_floor: bool,
    tracker: IdentifierTracker,
}

impl ops::Deref for RemoveFloorDivisionProcessor {
    type Target = IdentifierTracker;

    fn deref(&self) -> &Self::Target {
        &self.tracker
    }
}

impl ops::DerefMut for RemoveFloorDivisionProcessor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tracker
    }
}

const DEFAULT_MATH_LIBRARY: &str = "math";
const DEFAULT_MATH_FLOOR_NAME: &str = "floor";

impl RemoveFloorDivisionProcessor {
    fn new(math_floor_identifier: impl Into<String>, block: &mut Block) -> Self {
        Self {
            math_floor_identifier: math_floor_identifier.into(),
            define_math_floor: false,
            tracker: IdentifierTracker::collect_block(block),
        }
    }

    fn build_math_floor_call(&mut self, value: Expression) -> Expression {
        FunctionCall::from_prefix(if self.is_declared(DEFAULT_MATH_LIBRARY) {
            self.define_math_floor = true;
            Prefix::from_name(&self.math_floor_identifier)
        } else {
            FieldExpression::new(
                Prefix::from_name(DEFAULT_MATH_LIBRARY),
                DEFAULT_MATH_FLOOR_NAME,
            )
            .into()
        })
        .with_argument(value)
        .into()
    }
}

impl NodeProcessor for RemoveFloorDivisionProcessor {
    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::CompoundAssign(assign_statement) = statement {
            if assign_statement.get_operator() == CompoundOperator::DoubleSlash {
                replace_compound_assignment(statement);
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
pub fn remove_floor_division(block: &mut Block) {
    const MATH_FLOOR_IDENTIFIER: &str = "__R4_MATH_FLOOR";

    let mut processor = RemoveFloorDivisionProcessor::new(MATH_FLOOR_IDENTIFIER, block);
    DefaultVisitor::visit_block(block, &mut processor);

    if processor.define_math_floor {
        block.insert_statement(
            0,
            VariableAssignment::from_variable(MATH_FLOOR_IDENTIFIER).with_value(
                FieldExpression::new(
                    Prefix::from_name(DEFAULT_MATH_LIBRARY),
                    DEFAULT_MATH_FLOOR_NAME,
                ),
            ),
        );
    }
}
