use std::fmt;
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

/// An error produced while lowering floor division.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveFloorDivisionError {
    /// `math.floor` may have been replaced, so `//` cannot be lowered to it.
    UnstableMathLibrary,
}

impl fmt::Display for RemoveFloorDivisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnstableMathLibrary => write!(
                f,
                "cannot lower `//`: the `math` global may have been replaced"
            ),
        }
    }
}

impl std::error::Error for RemoveFloorDivisionError {}

struct RemoveFloorDivisionProcessor<'a> {
    model: &'a SemanticModel,
    names: NameGenerator,
    math_floor_identifier: Option<String>,
    /// Whether `math.floor` is known to be the builtin.
    stable: bool,
    /// Set when a `//` had to be left in place because `math` is not stable.
    unstable: bool,
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
                .get_or_insert_with(|| {
                    self.names
                        .generate_with_prefix("__R4_MATH_FLOOR")
                        .expect("the reserved prefix is a valid identifier")
                })
                .clone();
            Prefix::from_name(name)
        };

        FunctionCall::from_prefix(prefix)
            .with_argument(value)
            .into()
    }

    /// Lowers a `//` into a `math.floor` call, or records that it could not.
    fn lower(&mut self, expression: &mut Expression) {
        if !self.stable {
            self.unstable = true;
            return;
        }

        let Expression::Binary(binary) = expression else {
            return;
        };
        binary.set_operator(BinaryOperator::Slash);

        let value = mem::replace(expression, Expression::nil());

        *expression = self.build_math_floor_call(value);
    }
}

impl NodeProcessor for RemoveFloorDivisionProcessor<'_> {
    fn process_statement(&mut self, statement: &mut Statement) {
        if let Statement::CompoundAssign(assign_statement) = statement {
            if assign_statement.get_operator() == CompoundOperator::DoubleSlash {
                if !self.stable {
                    self.unstable = true;
                    return;
                }
                replace_compound_assignment(statement, self.model);
            }
        }
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        let is_floor_division = matches!(
            expression,
            Expression::Binary(binary) if binary.operator() == BinaryOperator::DoubleSlash
        );
        if is_floor_division {
            self.lower(expression);
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
///
/// `//` is a builtin operator, so the lowering depends on `math.floor` still
/// being the builtin. When the program can replace `math.floor` — it writes the
/// `math` table, or reaches globals from code the model cannot see — the pass
/// fails with [`RemoveFloorDivisionError::UnstableMathLibrary`] rather than
/// emitting a call that may no longer mean floor division.
pub fn remove_floor_division(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<(), RemoveFloorDivisionError> {
    let mut processor = RemoveFloorDivisionProcessor {
        model,
        names: NameGenerator::new(model),
        math_floor_identifier: None,
        stable: model.global_table_is_stable(DEFAULT_MATH_LIBRARY),
        unstable: false,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    if processor.unstable {
        return Err(RemoveFloorDivisionError::UnstableMathLibrary);
    }

    if let Some(identifier) = processor.math_floor_identifier {
        block.insert_statement(
            0,
            VariableAssignment::from_variable(identifier).with_value(FieldExpression::new(
                Prefix::from_name(DEFAULT_MATH_LIBRARY),
                DEFAULT_MATH_FLOOR_NAME,
            )),
        );
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn converts_floor_division() {
        let (mut block, model) = analyze("return value // 2");

        remove_floor_division(&mut block, &model).expect("the builtin `math.floor` is stable");

        insta::assert_snapshot!(generate(&block).trim(), @"return math.floor(value / 2)");
    }

    #[test]
    fn converts_compound_floor_division() {
        let (mut block, model) = analyze("value //= 2");

        remove_floor_division(&mut block, &model).expect("the builtin `math.floor` is stable");

        insta::assert_snapshot!(generate(&block).trim(), @"value = math.floor(value / 2)");
    }

    #[test]
    fn rejects_a_replaced_math_library() {
        let (mut block, model) = analyze("math.floor = function() end\nreturn value // 2");

        let error =
            remove_floor_division(&mut block, &model).expect_err("`math.floor` was replaced");

        assert_eq!(error, RemoveFloorDivisionError::UnstableMathLibrary);
        let output = generate(&block);
        assert!(
            output.contains("//"),
            "the `//` must be left in place: {output}"
        );
    }

    #[test]
    fn rejects_an_open_global_environment() {
        let (mut block, model) = analyze("load('return 1')\nreturn value // 2");

        let error = remove_floor_division(&mut block, &model).expect_err("the environment is open");

        assert_eq!(error, RemoveFloorDivisionError::UnstableMathLibrary);
    }

    #[test]
    fn accepts_an_unstable_environment_without_floor_division() {
        let (mut block, model) = analyze("load('return 1')\nreturn value");

        remove_floor_division(&mut block, &model).expect("there is no `//` to lower");
    }

    #[test]
    fn accepts_a_program_that_loads_a_module() {
        // `require` loads a separate unit; the lowering still reasons about the
        // block, so `//` lowers when `math` is untouched.
        let (mut block, model) = analyze("require('./module')\nreturn value // 2");

        remove_floor_division(&mut block, &model).expect("`math` is untouched");

        assert!(generate(&block).contains("math.floor(value / 2)"));
    }
}
