//! Complete token and trivia traversal.
//!
//! The visitors in [`crate::process`] reach syntax nodes; these helpers reach
//! *tokens*, including the tokens attached to nodes the node visitors do not
//! surface as callbacks (declaration identifiers, attributes, generic
//! parameters, function field names, and so on). They are the node-level view
//! of trivia used by directive collection and by callers that need every
//! comment or whitespace in a tree.

use crate::nodes::{
    ArrayType, AssignStatement, Attributes, BinaryExpression, Block, CompoundAssignStatement,
    DoStatement, Expression, ExpressionType, FieldExpression, FunctionAssignment, FunctionCall,
    FunctionExpression, FunctionStatement, FunctionType, GenericForStatement, GenericTypePack,
    GotoStatement, Identifier, IfExpression, IfStatement, IndexExpression,
    InterpolatedStringExpression, IntersectionType, LabelStatement, LastStatement,
    LiteralExpression, LiteralTable, NumberExpression, NumericForStatement, OptionalType,
    ParentheseExpression, ParentheseType, RepeatStatement, StringExpression, StringType,
    TableExpression, TableType, Token, Trivia, Type, TypeCastExpression, TypeDeclarationStatement,
    TypeField, TypeFunctionStatement, TypeInstantiationExpression, TypeName, TypePack,
    UnaryExpression, UnionType, VariableAssignment, VariadicTypePack, WhileStatement,
};

use super::{DefaultVisitor, NodeProcessor, NodeVisitor};

/// Passes every token in `block` to `f`.
///
/// The traversal borrows the tree mutably because r4's only complete syntax
/// visitor is the mutating one; it never changes the tree. Use
/// [`Compilation::block_mut`](crate::Compilation::block_mut) to reach a stored
/// tree.
pub fn for_each_token(block: &mut Block, f: &mut impl FnMut(&Token)) {
    let mut walk = TokenWalk { f };
    DefaultVisitor::visit_block(block, &mut walk);
}

/// Passes every piece of trivia in `block` to `f`.
///
/// Trivia is visited in traversal order (a token's leading trivia, then its
/// trailing trivia, as the tokens are reached).
pub fn for_each_trivia(block: &mut Block, f: &mut impl FnMut(&Trivia)) {
    for_each_token(block, &mut |token| {
        for trivia in token.iter_leading_trivia() {
            f(trivia);
        }
        for trivia in token.iter_trailing_trivia() {
            f(trivia);
        }
    });
}

/// Collects every piece of trivia in `block`, deduplicated and in source order.
///
/// Tokens without source positions (synthesized by a pass) sort last and keep
/// their traversal order.
pub fn collect_trivia(block: &mut Block) -> Vec<Trivia> {
    let mut trivia = Vec::new();
    for_each_trivia(block, &mut |piece| {
        if !trivia.contains(piece) {
            trivia.push(piece.clone());
        }
    });
    trivia.sort_by_key(|piece| {
        (
            piece.get_start().unwrap_or(usize::MAX),
            piece.get_line_number().unwrap_or(usize::MAX),
        )
    });
    trivia
}

struct TokenWalk<'a, F> {
    f: &'a mut F,
}

macro_rules! token_walk_methods {
    ($( $method:ident : $ty:ty ),* $(,)?) => {
        $(
            fn $method(&mut self, node: &mut $ty) {
                node.for_each_token(self.f);
            }
        )*
    };
}

impl<F: FnMut(&Token)> NodeProcessor for TokenWalk<'_, F> {
    token_walk_methods!(
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
        block.for_each_token(self.f);
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        call.for_each_token(self.f);
        call.mutate_arguments().for_each_token(self.f);
    }

    fn process_last_statement(&mut self, statement: &mut LastStatement) {
        match statement {
            LastStatement::Break(token) | LastStatement::Continue(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            LastStatement::Return(statement) => statement.for_each_token(self.f),
        }
    }

    fn process_literal_expression(&mut self, expression: &mut LiteralExpression) {
        match expression {
            LiteralExpression::True(token)
            | LiteralExpression::False(token)
            | LiteralExpression::Nil(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
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
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            _ => {}
        }
    }

    fn process_type(&mut self, r#type: &mut Type) {
        match r#type {
            Type::True(token) | Type::False(token) | Type::Nil(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            _ => {}
        }
    }
}
