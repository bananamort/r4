//! Structural dialect validation for generated code.
//!
//! r4's generators serialize whatever AST they are given. When a target
//! [`Dialect`] is configured, that is unsafe: a tree may contain constructs the
//! target grammar cannot express (compound assignment for Lua 5.1, types for
//! any non-Luau dialect, `<close>` for anything but Lua 5.4, and so on).
//! [`validate_dialect`] walks a [`Block`](crate::nodes::Block) and fails fast
//! with a typed
//! [`DialectSupportError`] naming the first unrepresentable construct, so a
//! caller can never silently emit code the target cannot parse.

use std::fmt;

use crate::nodes::{
    Expression, LastStatement, NumberExpression, Prefix, Statement, TypedIdentifier, UnaryOperator,
};
use crate::process::{ImmutableProcessor, ImmutableVisitor, Walk};
use crate::Dialect;

/// Why a [`Block`](crate::nodes::Block) cannot be represented in a target [`Dialect`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialectSupportError {
    /// A compound assignment (`x += 1`), which only Luau accepts.
    CompoundAssignmentNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A `continue` statement, which only Luau accepts.
    ContinueNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// An `if ... then ... else ...` expression, which only Luau accepts.
    IfExpressionNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// An interpolated string (`` `a {b}` ``), which only Luau accepts.
    InterpolatedStringNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A type annotation, declaration, cast, instantiation or generic, which
    /// only Luau accepts.
    TypesNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A function attribute (`@native`), which only Luau accepts.
    FunctionAttributesNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A Lua 5.4 local attribute (`<const>`/`<close>`), which only Lua 5.4
    /// accepts.
    LocalAttributesNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A `goto` statement, which Lua 5.2+ and LuaJIT accept but Lua 5.1 and
    /// Luau do not.
    GotoNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A label (`::name::`), which Lua 5.2+ and LuaJIT accept but Lua 5.1 and
    /// Luau do not.
    LabelsNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A bitwise operator (`&`, `|`, `~`, `<<`, `>>`), which only Lua 5.3 and
    /// 5.4 accept. In Luau `<<`/`>>` is type instantiation, not a shift.
    BitwiseOperatorsNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// Floor division (`//`), which Lua 5.3, Lua 5.4 and Luau accept.
    FloorDivisionNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
    /// A binary number literal (`0b101`), which only Luau accepts.
    BinaryNumberLiteralNotSupported {
        /// The target dialect.
        dialect: Dialect,
    },
}

impl DialectSupportError {
    /// Returns the target dialect the construct could not be represented in.
    #[inline]
    pub fn dialect(&self) -> Dialect {
        match self {
            Self::CompoundAssignmentNotSupported { dialect }
            | Self::ContinueNotSupported { dialect }
            | Self::IfExpressionNotSupported { dialect }
            | Self::InterpolatedStringNotSupported { dialect }
            | Self::TypesNotSupported { dialect }
            | Self::FunctionAttributesNotSupported { dialect }
            | Self::LocalAttributesNotSupported { dialect }
            | Self::GotoNotSupported { dialect }
            | Self::LabelsNotSupported { dialect }
            | Self::BitwiseOperatorsNotSupported { dialect }
            | Self::FloorDivisionNotSupported { dialect }
            | Self::BinaryNumberLiteralNotSupported { dialect } => *dialect,
        }
    }

    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::CompoundAssignmentNotSupported { .. } => "dialect/compound-assignment",
            Self::ContinueNotSupported { .. } => "dialect/continue",
            Self::IfExpressionNotSupported { .. } => "dialect/if-expression",
            Self::InterpolatedStringNotSupported { .. } => "dialect/interpolated-string",
            Self::TypesNotSupported { .. } => "dialect/types",
            Self::FunctionAttributesNotSupported { .. } => "dialect/function-attributes",
            Self::LocalAttributesNotSupported { .. } => "dialect/local-attributes",
            Self::GotoNotSupported { .. } => "dialect/goto",
            Self::LabelsNotSupported { .. } => "dialect/labels",
            Self::BitwiseOperatorsNotSupported { .. } => "dialect/bitwise-operators",
            Self::FloorDivisionNotSupported { .. } => "dialect/floor-division",
            Self::BinaryNumberLiteralNotSupported { .. } => "dialect/binary-number",
        }
    }
}

impl fmt::Display for DialectSupportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dialect = self.dialect();
        match self {
            Self::CompoundAssignmentNotSupported { .. } => {
                write!(f, "compound assignment is not supported by {dialect}")
            }
            Self::ContinueNotSupported { .. } => {
                write!(f, "`continue` is not supported by {dialect}")
            }
            Self::IfExpressionNotSupported { .. } => {
                write!(f, "if-expressions are not supported by {dialect}")
            }
            Self::InterpolatedStringNotSupported { .. } => {
                write!(f, "interpolated strings are not supported by {dialect}")
            }
            Self::TypesNotSupported { .. } => {
                write!(f, "type syntax is not supported by {dialect}")
            }
            Self::FunctionAttributesNotSupported { .. } => {
                write!(f, "function attributes are not supported by {dialect}")
            }
            Self::LocalAttributesNotSupported { .. } => {
                write!(
                    f,
                    "local variable attributes are not supported by {dialect}"
                )
            }
            Self::GotoNotSupported { .. } => {
                write!(f, "`goto` is not supported by {dialect}")
            }
            Self::LabelsNotSupported { .. } => {
                write!(f, "labels are not supported by {dialect}")
            }
            Self::BitwiseOperatorsNotSupported { .. } => {
                write!(f, "bitwise operators are not supported by {dialect}")
            }
            Self::FloorDivisionNotSupported { .. } => {
                write!(f, "floor division is not supported by {dialect}")
            }
            Self::BinaryNumberLiteralNotSupported { .. } => {
                write!(f, "binary number literals are not supported by {dialect}")
            }
        }
    }
}

impl std::error::Error for DialectSupportError {}

const fn accepts_luau_only(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Luau)
}

const fn accepts_local_attributes(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Lua54)
}

const fn accepts_goto_and_labels(dialect: Dialect) -> bool {
    matches!(
        dialect,
        Dialect::Lua52 | Dialect::Lua53 | Dialect::Lua54 | Dialect::LuaJit
    )
}

const fn accepts_bitwise(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Lua53 | Dialect::Lua54)
}

const fn accepts_floor_division(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Lua53 | Dialect::Lua54 | Dialect::Luau)
}

/// Validates that `block` can be represented in `dialect`, returning the first
/// construct the dialect cannot express.
///
/// [`Dialect::Default`] means no target was chosen; nothing is validated.
pub fn validate_dialect(
    block: &crate::nodes::Block,
    dialect: Dialect,
) -> Result<(), DialectSupportError> {
    if dialect == Dialect::Default {
        return Ok(());
    }

    let mut validator = Validator {
        target: dialect,
        error: None,
    };
    Walk::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Validator {
    target: Dialect,
    error: Option<DialectSupportError>,
}

impl Validator {
    fn reject(&mut self, make: impl FnOnce(Dialect) -> DialectSupportError) {
        if self.error.is_none() {
            self.error = Some(make(self.target));
        }
    }
}

impl ImmutableProcessor for Validator {
    fn process_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::CompoundAssign(_) if !accepts_luau_only(self.target) => {
                self.reject(
                    |dialect| DialectSupportError::CompoundAssignmentNotSupported { dialect },
                );
            }
            Statement::Goto(_) if !accepts_goto_and_labels(self.target) => {
                self.reject(|dialect| DialectSupportError::GotoNotSupported { dialect });
            }
            Statement::Label(_) if !accepts_goto_and_labels(self.target) => {
                self.reject(|dialect| DialectSupportError::LabelsNotSupported { dialect });
            }
            Statement::TypeDeclaration(_) | Statement::TypeFunction(_) => {
                self.reject(|dialect| DialectSupportError::TypesNotSupported { dialect });
            }
            _ => {}
        }
    }

    fn process_last_statement(&mut self, statement: &LastStatement) {
        if matches!(statement, LastStatement::Continue(_)) && !accepts_luau_only(self.target) {
            self.reject(|dialect| DialectSupportError::ContinueNotSupported { dialect });
        }
    }

    fn process_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::If(_) if !accepts_luau_only(self.target) => {
                self.reject(|dialect| DialectSupportError::IfExpressionNotSupported { dialect });
            }
            Expression::InterpolatedString(_) if !accepts_luau_only(self.target) => {
                self.reject(
                    |dialect| DialectSupportError::InterpolatedStringNotSupported { dialect },
                );
            }
            Expression::TypeCast(_) | Expression::TypeInstantiation(_) => {
                self.reject(|dialect| DialectSupportError::TypesNotSupported { dialect });
            }
            Expression::Binary(binary) => {
                use crate::nodes::BinaryOperator::*;
                match binary.operator() {
                    BitwiseOr | BitwiseXor | BitwiseAnd | ShiftLeft | ShiftRight
                        if !accepts_bitwise(self.target) =>
                    {
                        self.reject(
                            |dialect| DialectSupportError::BitwiseOperatorsNotSupported { dialect },
                        );
                    }
                    DoubleSlash if !accepts_floor_division(self.target) => {
                        self.reject(|dialect| DialectSupportError::FloorDivisionNotSupported {
                            dialect,
                        });
                    }
                    _ => {}
                }
            }
            Expression::Unary(unary)
                if unary.operator() == UnaryOperator::BitwiseNot
                    && !accepts_bitwise(self.target) =>
            {
                self.reject(
                    |dialect| DialectSupportError::BitwiseOperatorsNotSupported { dialect },
                );
            }
            _ => {}
        }
    }

    fn process_prefix(&mut self, prefix: &Prefix) {
        if matches!(prefix, Prefix::TypeInstantiation(_)) {
            self.reject(|dialect| DialectSupportError::TypesNotSupported { dialect });
        }
    }

    fn process_type(&mut self, _: &crate::nodes::Type) {
        self.reject(|dialect| DialectSupportError::TypesNotSupported { dialect });
    }

    fn process_typed_identifier(&mut self, identifier: &TypedIdentifier) {
        if identifier.get_attribute().is_some() && !accepts_local_attributes(self.target) {
            self.reject(|dialect| DialectSupportError::LocalAttributesNotSupported { dialect });
        }
    }

    fn process_attributes(&mut self, attributes: &crate::nodes::Attributes) {
        if !attributes.is_empty() && !accepts_luau_only(self.target) {
            self.reject(|dialect| DialectSupportError::FunctionAttributesNotSupported { dialect });
        }
    }

    fn process_number_expression(&mut self, number: &NumberExpression) {
        if matches!(number, NumberExpression::Binary(_)) && !accepts_luau_only(self.target) {
            self.reject(|dialect| DialectSupportError::BinaryNumberLiteralNotSupported { dialect });
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Parser;

    fn luau(code: &str) -> crate::nodes::Block {
        Parser::default().luau().parse(code).expect("Luau source")
    }

    fn lua52(code: &str) -> crate::nodes::Block {
        Parser::default()
            .lua52()
            .parse(code)
            .expect("Lua 5.2 source")
    }

    fn lua53(code: &str) -> crate::nodes::Block {
        Parser::default()
            .lua53()
            .parse(code)
            .expect("Lua 5.3 source")
    }

    fn lua54(code: &str) -> crate::nodes::Block {
        Parser::default()
            .lua54()
            .parse(code)
            .expect("Lua 5.4 source")
    }

    #[test]
    fn luau_only_constructs_are_rejected_for_lua51() {
        for (code, expected) in [
            (
                luau("value += 1"),
                DialectSupportError::CompoundAssignmentNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("while true do continue end"),
                DialectSupportError::ContinueNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("return if a then 1 else 2"),
                DialectSupportError::IfExpressionNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("return `a {b}`"),
                DialectSupportError::InterpolatedStringNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("local x: number = 1"),
                DialectSupportError::TypesNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("@native function f() end"),
                DialectSupportError::FunctionAttributesNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                luau("return 0b101"),
                DialectSupportError::BinaryNumberLiteralNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                lua52("goto done\n::done::"),
                DialectSupportError::GotoNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                lua53("return a & b"),
                DialectSupportError::BitwiseOperatorsNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
            (
                lua53("return a // b"),
                DialectSupportError::FloorDivisionNotSupported {
                    dialect: Dialect::Lua51,
                },
            ),
        ] {
            assert_eq!(validate_dialect(&code, Dialect::Lua51), Err(expected));
        }
    }

    #[test]
    fn function_types_casts_and_instantiations_are_types() {
        for code in [
            luau("local function f<T>(x: T): T return x end"),
            luau("local function f(a: number) return a end"),
            luau("return value :: number"),
            luau("return f<<number>>(value)"),
        ] {
            assert_eq!(
                validate_dialect(&code, Dialect::Lua51),
                Err(DialectSupportError::TypesNotSupported {
                    dialect: Dialect::Lua51
                })
            );
        }
    }

    #[test]
    fn labels_are_rejected_for_lua51_on_their_own() {
        let block = lua52("::start::");
        assert_eq!(
            validate_dialect(&block, Dialect::Lua51),
            Err(DialectSupportError::LabelsNotSupported {
                dialect: Dialect::Lua51,
            })
        );
    }

    #[test]
    fn local_attributes_are_lua54_only() {
        let block = lua54("local x <const> = 1\nreturn x");
        assert_eq!(
            validate_dialect(&block, Dialect::Lua53),
            Err(DialectSupportError::LocalAttributesNotSupported {
                dialect: Dialect::Lua53,
            })
        );
        assert_eq!(validate_dialect(&block, Dialect::Lua54), Ok(()));
    }

    #[test]
    fn supported_programs_validate_cleanly() {
        assert_eq!(
            validate_dialect(&luau("local x = 1\nreturn x"), Dialect::Lua51),
            Ok(())
        );
        assert_eq!(
            validate_dialect(&lua52("::a::\ngoto a"), Dialect::Lua52),
            Ok(())
        );
        assert_eq!(
            validate_dialect(&lua53("return a // b"), Dialect::Lua53),
            Ok(())
        );
    }

    #[test]
    fn the_default_dialect_means_no_target() {
        assert_eq!(
            validate_dialect(&luau("value += 1"), Dialect::Default),
            Ok(())
        );
    }
}
