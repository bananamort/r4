//! Token-insensitive views of an AST.
//!
//! [`detokenize`] removes every token from a tree while preserving its
//! structure. Every token becomes the canonical content-free token
//! ([`Token::remove_tokens`](crate::nodes::Token::remove_tokens)), so two trees
//! that differ only in trivia or source positions compare equal afterwards.
//! This is the mechanism behind structural equivalence
//! ([`Equivalent`](crate::Equivalent)).
//!
//! Removing tokens is total and never inspects the model: a token is erased
//! whether or not it was parsed with token data, so a tree parsed with
//! [`Parser::preserve_tokens`](crate::Parser::preserve_tokens) and the same tree
//! parsed without token data become identical.

use crate::nodes::{
    ArrayType, AssignStatement, Attributes, BinaryExpression, Block, CompoundAssignStatement,
    DoStatement, Expression, ExpressionType, FieldExpression, FunctionAssignment, FunctionCall,
    FunctionExpression, FunctionStatement, FunctionType, GenericForStatement, GenericTypePack,
    GotoStatement, Identifier, IfExpression, IfStatement, IndexExpression,
    InterpolatedStringExpression, IntersectionType, LabelStatement, LastStatement,
    LiteralExpression, LiteralTable, NumberExpression, NumericForStatement, OptionalType,
    ParentheseExpression, ParentheseType, RepeatStatement, Statement, StringExpression, StringType,
    TableExpression, TableType, Type, TypeCastExpression, TypeDeclarationStatement, TypeField,
    TypeFunctionStatement, TypeInstantiationExpression, TypeName, TypePack, UnaryExpression,
    UnionType, VariableAssignment, VariadicTypePack, WhileStatement,
};

use super::{DefaultVisitor, NodeProcessor, NodeVisitor};

/// Removes every token from `block` and everything it contains.
pub fn detokenize(block: &mut Block) {
    DefaultVisitor::visit_block(block, &mut DetokenizeProcessor);
}

/// Removes every token from `statement` and everything it contains.
pub fn detokenize_statement(statement: &mut Statement) {
    DefaultVisitor::visit_statement(statement, &mut DetokenizeProcessor);
}

/// Removes every token from `expression` and everything it contains.
pub fn detokenize_expression(expression: &mut Expression) {
    DefaultVisitor::visit_expression(expression, &mut DetokenizeProcessor);
}

struct DetokenizeProcessor;

macro_rules! detokenize_methods {
    ($( $method:ident : $ty:ty ),* $(,)?) => {
        $(
            fn $method(&mut self, node: &mut $ty) {
                node.remove_tokens();
            }
        )*
    };
}

impl NodeProcessor for DetokenizeProcessor {
    detokenize_methods!(
        process_assign_statement: AssignStatement,
        process_compound_assign_statement: CompoundAssignStatement,
        process_do_statement: DoStatement,
        process_function_statement: FunctionStatement,
        process_generic_for_statement: GenericForStatement,
        process_goto_statement: GotoStatement,
        process_if_statement: IfStatement,
        process_label_statement: LabelStatement,
        process_local_assign_statement: VariableAssignment,
        process_local_function_statement: FunctionAssignment,
        process_numeric_for_statement: NumericForStatement,
        process_repeat_statement: RepeatStatement,
        process_while_statement: WhileStatement,
        process_type_declaration: TypeDeclarationStatement,
        process_type_function: TypeFunctionStatement,
        process_attributes: Attributes,
        process_literal_table: LiteralTable,
        process_binary_expression: BinaryExpression,
        process_field_expression: FieldExpression,
        process_function_expression: FunctionExpression,
        process_variable_expression: Identifier,
        process_index_expression: IndexExpression,
        process_if_expression: IfExpression,
        process_number_expression: NumberExpression,
        process_parenthese_expression: ParentheseExpression,
        process_string_expression: StringExpression,
        process_interpolated_string_expression: InterpolatedStringExpression,
        process_table_expression: TableExpression,
        process_unary_expression: UnaryExpression,
        process_type_cast_expression: TypeCastExpression,
        process_type_instantiation: TypeInstantiationExpression,
        process_type_name: TypeName,
        process_type_field: TypeField,
        process_string_type: StringType,
        process_array_type: ArrayType,
        process_table_type: TableType,
        process_expression_type: ExpressionType,
        process_parenthese_type: ParentheseType,
        process_function_type: FunctionType,
        process_optional_type: OptionalType,
        process_intersection_type: IntersectionType,
        process_union_type: UnionType,
        process_type_pack: TypePack,
        process_generic_type_pack: GenericTypePack,
        process_variadic_type_pack: VariadicTypePack,
    );

    fn process_block(&mut self, block: &mut Block) {
        block.remove_tokens();
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        call.remove_tokens();
        call.mutate_arguments().remove_tokens();
    }

    fn process_last_statement(&mut self, statement: &mut LastStatement) {
        match statement {
            LastStatement::Break(token) | LastStatement::Continue(token) => {
                *token = None;
            }
            LastStatement::Return(statement) => statement.remove_tokens(),
        }
    }

    fn process_literal_expression(&mut self, expression: &mut LiteralExpression) {
        match expression {
            LiteralExpression::True(token)
            | LiteralExpression::False(token)
            | LiteralExpression::Nil(token) => {
                *token = None;
            }
            _ => {}
        }
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        match expression {
            Expression::False(token)
            | Expression::Nil(token)
            | Expression::True(token)
            | Expression::VariableArguments(token) => {
                *token = None;
            }
            _ => {}
        }
    }

    fn process_type(&mut self, r#type: &mut Type) {
        match r#type {
            Type::True(token) | Type::False(token) | Type::Nil(token) => {
                *token = None;
            }
            _ => {}
        }
    }
}
