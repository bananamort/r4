//! Syntactic token and trivia passes.
//!
//! These passes manipulate the raw tokens attached to nodes (comments,
//! whitespace, line numbers and token content). They are deliberately kept
//! separate from the semantic passes in [`crate::rules::canonicalize`] and
//! [`crate::rules::desugar`]: trivia manipulation never consults a
//! [`SemanticModel`](crate::semantic::SemanticModel) and never changes the
//! structure of the syntax tree.
//!
//! The passes are primarily useful with the
//! [`TokenBasedLuaGenerator`](crate::generator::TokenBasedLuaGenerator) and a
//! parser configured with [`Parser::preserve_tokens`](crate::Parser::preserve_tokens)
//! for formatting-preserving transformations. The engine's primary minification
//! pipeline is the non-destructive
//! [`DenseLuaGenerator`](crate::generator::DenseLuaGenerator), which does not
//! require mutating trivia at all.

use crate::nodes::{
    Arguments, ArrayType, AssignStatement, Attributes, BinaryExpression, Block,
    CompoundAssignStatement, DoStatement, Expression, ExpressionType, FieldExpression,
    FunctionAssignment, FunctionCall, FunctionExpression, FunctionStatement, FunctionType,
    GenericForStatement, GenericTypePack, GotoStatement, Identifier, IfExpression, IfStatement,
    IndexExpression, InterpolatedStringExpression, IntersectionType, LabelStatement, LastStatement,
    LiteralExpression, LiteralTable, NumberExpression, NumericForStatement, OptionalType,
    ParentheseExpression, ParentheseType, RepeatStatement, ReturnStatement, StringExpression,
    StringType, TableExpression, TableType, Token, Trivia, TriviaKind, Type, TypeCastExpression,
    TypeDeclarationStatement, TypeField, TypeFunctionStatement, TypeInstantiationExpression,
    TypeName, TypePack, UnaryExpression, UnionType, VariableAssignment, VariadicTypePack,
    WhileStatement,
};

use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

/// The location of an appended text comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AppendLocation {
    /// Prepend the comment before the first token.
    #[default]
    Start,
    /// Append the comment after the last token.
    End,
}

/// The operation applied to every token-bearing node.
#[derive(Clone, Copy)]
enum Mode {
    ClearComments,
    ClearWhitespaces,
    ShiftLine(isize),
    ReplaceTokens,
}

/// A trait implemented by every node that owns tokens.
trait TokenOps {
    fn op_clear_comments(&mut self);
    fn op_clear_whitespaces(&mut self);
    fn op_shift_line(&mut self, amount: isize);
    fn op_replace_tokens(&mut self, code: &str);
}

macro_rules! impl_token_ops {
    ($( $ty:ty ),* $(,)?) => {
        $(
            impl TokenOps for $ty {
                fn op_clear_comments(&mut self) {
                    self.clear_comments();
                }

                fn op_clear_whitespaces(&mut self) {
                    self.clear_whitespaces();
                }

                fn op_shift_line(&mut self, amount: isize) {
                    self.shift_token_line(amount);
                }

                fn op_replace_tokens(&mut self, code: &str) {
                    self.replace_referenced_tokens(code);
                }
            }
        )*
    };
}

impl_token_ops!(
    Token,
    Arguments,
    ReturnStatement,
    Block,
    FunctionCall,
    AssignStatement,
    CompoundAssignStatement,
    DoStatement,
    FunctionStatement,
    GenericForStatement,
    GotoStatement,
    IfStatement,
    LabelStatement,
    VariableAssignment,
    FunctionAssignment,
    NumericForStatement,
    RepeatStatement,
    WhileStatement,
    TypeDeclarationStatement,
    TypeFunctionStatement,
    Attributes,
    LiteralTable,
    BinaryExpression,
    FieldExpression,
    FunctionExpression,
    Identifier,
    IndexExpression,
    IfExpression,
    NumberExpression,
    ParentheseExpression,
    StringExpression,
    InterpolatedStringExpression,
    TableExpression,
    UnaryExpression,
    TypeCastExpression,
    TypeInstantiationExpression,
    TypeName,
    TypeField,
    StringType,
    ArrayType,
    TableType,
    ExpressionType,
    ParentheseType,
    FunctionType,
    OptionalType,
    IntersectionType,
    UnionType,
    TypePack,
    GenericTypePack,
    VariadicTypePack,
);

struct TokenOpProcessor<'a> {
    mode: Mode,
    code: &'a str,
}

impl TokenOpProcessor<'_> {
    fn apply<T: TokenOps>(&self, node: &mut T) {
        match self.mode {
            Mode::ClearComments => node.op_clear_comments(),
            Mode::ClearWhitespaces => node.op_clear_whitespaces(),
            Mode::ShiftLine(amount) => node.op_shift_line(amount),
            Mode::ReplaceTokens => node.op_replace_tokens(self.code),
        }
    }
}

macro_rules! token_op_methods {
    ($( $method:ident : $ty:ty ),* $(,)?) => {
        $(
            fn $method(&mut self, node: &mut $ty) {
                self.apply(node);
            }
        )*
    };
}

impl NodeProcessor for TokenOpProcessor<'_> {
    token_op_methods!(
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
        self.apply(block);
    }

    fn process_literal_expression(&mut self, expression: &mut LiteralExpression) {
        match expression {
            LiteralExpression::True(token)
            | LiteralExpression::False(token)
            | LiteralExpression::Nil(token) => {
                if let Some(token) = token {
                    self.apply(token);
                }
            }
            _ => {}
        }
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        self.apply(call);
        self.apply(call.mutate_arguments());
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        match expression {
            Expression::False(token)
            | Expression::Nil(token)
            | Expression::True(token)
            | Expression::VariableArguments(token) => {
                if let Some(token) = token {
                    self.apply(token);
                }
            }
            _ => {}
        }
    }

    fn process_type(&mut self, r#type: &mut Type) {
        match r#type {
            Type::True(token) | Type::False(token) | Type::Nil(token) => {
                if let Some(token) = token {
                    self.apply(token);
                }
            }
            _ => {}
        }
    }

    fn process_last_statement(&mut self, statement: &mut LastStatement) {
        match statement {
            LastStatement::Break(token) | LastStatement::Continue(token) => {
                if let Some(token) = token {
                    self.apply(token);
                }
            }
            LastStatement::Return(statement) => self.apply(statement),
        }
    }
}

/// Removes every comment attached to the tokens of `block`.
pub fn remove_comments(block: &mut Block) {
    let mut processor = TokenOpProcessor {
        mode: Mode::ClearComments,
        code: "",
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Removes the comments for which `keep` returns `false`.
///
/// `keep` receives the trivia; use [`Trivia::read`] with the original source to
/// inspect the comment text.
pub fn remove_comments_where<F>(block: &mut Block, keep: F)
where
    F: Fn(&Trivia) -> bool,
{
    struct FilterProcessor<F> {
        keep: F,
    }

    impl<F: Fn(&Trivia) -> bool> NodeProcessor for FilterProcessor<F> {
        fn process_block(&mut self, block: &mut Block) {
            block.filter_comments(&self.keep);
        }

        fn process_function_call(&mut self, call: &mut FunctionCall) {
            call.filter_comments(&self.keep);
            call.mutate_arguments().filter_comments(&self.keep);
        }

        fn process_assign_statement(&mut self, node: &mut AssignStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_compound_assign_statement(&mut self, node: &mut CompoundAssignStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_do_statement(&mut self, node: &mut DoStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_function_statement(&mut self, node: &mut FunctionStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_generic_for_statement(&mut self, node: &mut GenericForStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_goto_statement(&mut self, node: &mut GotoStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_if_statement(&mut self, node: &mut IfStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_label_statement(&mut self, node: &mut LabelStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_last_statement(&mut self, statement: &mut LastStatement) {
            match statement {
                LastStatement::Break(token) | LastStatement::Continue(token) => {
                    if let Some(token) = token {
                        token.filter_comments(&self.keep);
                    }
                }
                LastStatement::Return(statement) => statement.filter_comments(&self.keep),
            }
        }

        fn process_local_assign_statement(&mut self, node: &mut VariableAssignment) {
            node.filter_comments(&self.keep);
        }

        fn process_local_function_statement(&mut self, node: &mut FunctionAssignment) {
            node.filter_comments(&self.keep);
        }

        fn process_numeric_for_statement(&mut self, node: &mut NumericForStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_repeat_statement(&mut self, node: &mut RepeatStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_while_statement(&mut self, node: &mut WhileStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_type_declaration(&mut self, node: &mut TypeDeclarationStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_type_function(&mut self, node: &mut TypeFunctionStatement) {
            node.filter_comments(&self.keep);
        }

        fn process_attributes(&mut self, node: &mut Attributes) {
            node.filter_comments(&self.keep);
        }

        fn process_literal_table(&mut self, node: &mut LiteralTable) {
            node.filter_comments(&self.keep);
        }

        fn process_literal_expression(&mut self, expression: &mut LiteralExpression) {
            match expression {
                LiteralExpression::True(token)
                | LiteralExpression::False(token)
                | LiteralExpression::Nil(token) => {
                    if let Some(token) = token {
                        token.filter_comments(&self.keep);
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
                        token.filter_comments(&self.keep);
                    }
                }
                _ => {}
            }
        }

        fn process_binary_expression(&mut self, node: &mut BinaryExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_field_expression(&mut self, node: &mut FieldExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_function_expression(&mut self, node: &mut FunctionExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_variable_expression(&mut self, node: &mut Identifier) {
            node.filter_comments(&self.keep);
        }

        fn process_index_expression(&mut self, node: &mut IndexExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_if_expression(&mut self, node: &mut IfExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_number_expression(&mut self, node: &mut NumberExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_parenthese_expression(&mut self, node: &mut ParentheseExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_string_expression(&mut self, node: &mut StringExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_interpolated_string_expression(
            &mut self,
            node: &mut InterpolatedStringExpression,
        ) {
            node.filter_comments(&self.keep);
        }

        fn process_table_expression(&mut self, node: &mut TableExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_unary_expression(&mut self, node: &mut UnaryExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_type_cast_expression(&mut self, node: &mut TypeCastExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_type_instantiation(&mut self, node: &mut TypeInstantiationExpression) {
            node.filter_comments(&self.keep);
        }

        fn process_type(&mut self, r#type: &mut Type) {
            match r#type {
                Type::True(token) | Type::False(token) | Type::Nil(token) => {
                    if let Some(token) = token {
                        token.filter_comments(&self.keep);
                    }
                }
                _ => {}
            }
        }

        fn process_type_name(&mut self, node: &mut TypeName) {
            node.filter_comments(&self.keep);
        }

        fn process_type_field(&mut self, node: &mut TypeField) {
            node.filter_comments(&self.keep);
        }

        fn process_string_type(&mut self, node: &mut StringType) {
            node.filter_comments(&self.keep);
        }

        fn process_array_type(&mut self, node: &mut ArrayType) {
            node.filter_comments(&self.keep);
        }

        fn process_table_type(&mut self, node: &mut TableType) {
            node.filter_comments(&self.keep);
        }

        fn process_expression_type(&mut self, node: &mut ExpressionType) {
            node.filter_comments(&self.keep);
        }

        fn process_parenthese_type(&mut self, node: &mut ParentheseType) {
            node.filter_comments(&self.keep);
        }

        fn process_function_type(&mut self, node: &mut FunctionType) {
            node.filter_comments(&self.keep);
        }

        fn process_optional_type(&mut self, node: &mut OptionalType) {
            node.filter_comments(&self.keep);
        }

        fn process_intersection_type(&mut self, node: &mut IntersectionType) {
            node.filter_comments(&self.keep);
        }

        fn process_union_type(&mut self, node: &mut UnionType) {
            node.filter_comments(&self.keep);
        }

        fn process_type_pack(&mut self, node: &mut TypePack) {
            node.filter_comments(&self.keep);
        }

        fn process_generic_type_pack(&mut self, node: &mut GenericTypePack) {
            node.filter_comments(&self.keep);
        }

        fn process_variadic_type_pack(&mut self, node: &mut VariadicTypePack) {
            node.filter_comments(&self.keep);
        }
    }

    let mut processor = FilterProcessor { keep };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Removes every whitespace trivia attached to the tokens of `block`.
pub fn remove_spaces(block: &mut Block) {
    let mut processor = TokenOpProcessor {
        mode: Mode::ClearWhitespaces,
        code: "",
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Shifts the recorded line numbers of every token by `amount` lines.
pub fn shift_token_line(block: &mut Block, amount: isize) {
    let mut processor = TokenOpProcessor {
        mode: Mode::ShiftLine(amount),
        code: "",
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Materializes every token that references `code` into its own content.
///
/// Tokens parsed with [`Parser::preserve_tokens`](crate::Parser::preserve_tokens)
/// store a byte range into the original source. This pass resolves those ranges
/// into owned content so the tree no longer depends on the source string.
pub fn replace_referenced_tokens(block: &mut Block, code: &str) {
    let mut processor = TokenOpProcessor {
        mode: Mode::ReplaceTokens,
        code,
    };
    DefaultVisitor::visit_block(block, &mut processor);
}

/// Appends `text` as a comment at the start or end of `block`.
///
/// When `text` spans multiple lines a long-bracket comment is emitted, choosing
/// enough `=` characters to avoid colliding with the content. Line numbers of
/// the existing tokens are shifted by the number of inserted lines so
/// token-based generation keeps the original layout.
pub fn append_text_comment(block: &mut Block, text: &str, location: AppendLocation) {
    if text.is_empty() {
        return;
    }

    let comment = format_comment(text);
    let shift_lines = comment.lines().count();

    shift_token_line(block, shift_lines as isize);

    match location {
        AppendLocation::Start => {
            let token = block.mutate_first_token();
            token.insert_leading_trivia(0, TriviaKind::Comment.with_content(comment));
            token.insert_leading_trivia(1, TriviaKind::Whitespace.with_content("\n"));
        }
        AppendLocation::End => {
            block
                .mutate_last_token()
                .push_trailing_trivia(TriviaKind::Comment.with_content(comment));
        }
    }
}

fn format_comment(text: &str) -> String {
    if !text.contains('\n') {
        return format!("--{text}");
    }

    let mut equal_count = 0;
    loop {
        let close = format!("]{}]", "=".repeat(equal_count));
        if !text.contains(&close) {
            return format!("--[{}[\n{}\n{}", "=".repeat(equal_count), text, close);
        }
        equal_count += 1;
    }
}
