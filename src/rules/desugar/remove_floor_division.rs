use std::mem;

use crate::nodes::{
    BinaryOperator, Block, CompoundOperator, Expression, FieldExpression, FunctionCall, Prefix,
    Statement, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::desugar::remove_compound_assign::replace_compound_assignment;
use crate::rules::shadow::ShadowPolicy;
use crate::semantic::SemanticModel;

const DEFAULT_MATH_LIBRARY: &str = "math";
const DEFAULT_MATH_FLOOR_NAME: &str = "floor";
const MATH_FLOOR_IDENTIFIER: &str = "__R4_MATH_FLOOR";

struct RemoveFloorDivisionProcessor<'a> {
    math_floor_identifier: String,
    define_math_floor: bool,
    shadows: ShadowPolicy<'a>,
}

impl RemoveFloorDivisionProcessor<'_> {
    fn build_math_floor_call(&mut self, value: Expression) -> Expression {
        FunctionCall::from_prefix(if self.shadows.is_transparent(DEFAULT_MATH_LIBRARY) {
            FieldExpression::new(
                Prefix::from_name(DEFAULT_MATH_LIBRARY),
                DEFAULT_MATH_FLOOR_NAME,
            )
            .into()
        } else {
            self.define_math_floor = true;
            Prefix::from_name(&self.math_floor_identifier)
        })
        .with_argument(value)
        .into()
    }
}

impl NodeProcessor for RemoveFloorDivisionProcessor<'_> {
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
///
/// When `math` is declared anywhere in the block, the pass introduces a
/// `__R4_MATH_FLOOR` alias initialized from the global `math` table and routes
/// the rewrites through it.
pub fn remove_floor_division(block: &mut Block) {
    let shadows = ShadowPolicy::from_block(block);
    let mut processor = RemoveFloorDivisionProcessor {
        math_floor_identifier: MATH_FLOOR_IDENTIFIER.to_owned(),
        define_math_floor: false,
        shadows,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    insert_math_floor_alias(block, processor.define_math_floor);
}

/// Like [`remove_floor_division`], but resolves `math` through the semantic
/// model.
///
/// Read-only aliases such as `local math = math` are transparent, so their
/// uses keep calling `math.floor` directly. The synthetic alias is only
/// introduced when `math` can actually denote something else (a mutable or
/// non-alias local).
pub fn remove_floor_division_with_model(block: &mut Block, model: &SemanticModel) {
    let shadows = ShadowPolicy::from_model(model);
    let mut processor = RemoveFloorDivisionProcessor {
        math_floor_identifier: MATH_FLOOR_IDENTIFIER.to_owned(),
        define_math_floor: false,
        shadows,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    insert_math_floor_alias(block, processor.define_math_floor);
}

fn insert_math_floor_alias(block: &mut Block, define_math_floor: bool) {
    if define_math_floor {
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
