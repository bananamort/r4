//! Complete, read-only token and trivia traversal.
//!
//! The node visitors reach syntax nodes but not every *token*: declaration
//! identifiers, attributes, function field names, generic parameters and other
//! tokens live on nodes the visitors do not surface as callbacks. These helpers
//! reach them by combining the read-only [`ImmutableVisitor`] (which walks every
//! syntax child) with each node's generated
//! [`for_each_token`](crate::nodes::Block::for_each_token) (which walks the
//! node's own tokens and the token-bearing children the visitor does not
//! surface).
//!
//! The traversal is read-only and never guesses: it reports exactly the trivia
//! the parser attached, in source order, without duplicates.

use crate::nodes::{
    Block, Expression, LastStatement, Prefix, Statement, Token, Trivia, Type, Variable,
};

use super::{ImmutableProcessor, ImmutableVisitor, Walk};

/// Passes every piece of trivia in `block` to `f`, in source order.
pub fn for_each_trivia(block: &Block, f: &mut impl FnMut(&Trivia)) {
    for trivia in collect_trivia(block) {
        f(&trivia);
    }
}

/// Collects every piece of trivia in `block`, in source order.
///
/// The traversal visits each token exactly once, so every piece of trivia is
/// returned exactly once and distinct trivia are never merged, even when their
/// content is identical. Trivia without source positions (synthesized by a
/// pass) sort last and keep their traversal order.
pub fn collect_trivia(block: &Block) -> Vec<Trivia> {
    let mut trivia = Vec::new();
    for_each_token(block, &mut |token| {
        for piece in token
            .iter_leading_trivia()
            .chain(token.iter_trailing_trivia())
        {
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

/// Passes every token in `block` to `f`, each exactly once.
///
/// The read-only visitor surfaces the syntax children while each node's
/// generated [`for_each_token`](crate::nodes::Block::for_each_token) surfaces
/// the token holders the visitor does not, so the two are disjoint and no token
/// is visited twice.
pub(crate) fn for_each_token(block: &Block, f: &mut impl FnMut(&Token)) {
    Walk::visit_block(block, &mut TokenWalk { f });
}

struct TokenWalk<'a, F> {
    f: &'a mut F,
}

impl<F: FnMut(&Token)> ImmutableProcessor for TokenWalk<'_, F> {
    fn process_block(&mut self, block: &Block) {
        block.for_each_token(self.f);
    }

    fn process_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Assign(node) => node.for_each_token(self.f),
            Statement::Call(node) => node.for_each_token(self.f),
            Statement::CompoundAssign(node) => node.for_each_token(self.f),
            Statement::Do(node) => node.for_each_token(self.f),
            Statement::Function(node) => node.for_each_token(self.f),
            Statement::GenericFor(node) => node.for_each_token(self.f),
            Statement::Goto(node) => node.for_each_token(self.f),
            Statement::If(node) => node.for_each_token(self.f),
            Statement::Label(node) => node.for_each_token(self.f),
            Statement::LocalAssign(node) => node.for_each_token(self.f),
            Statement::LocalFunction(node) => node.for_each_token(self.f),
            Statement::NumericFor(node) => node.for_each_token(self.f),
            Statement::Repeat(node) => node.for_each_token(self.f),
            Statement::TypeDeclaration(node) => node.for_each_token(self.f),
            Statement::TypeFunction(node) => node.for_each_token(self.f),
            Statement::While(node) => node.for_each_token(self.f),
        }
    }

    fn process_last_statement(&mut self, statement: &LastStatement) {
        match statement {
            LastStatement::Break(token) | LastStatement::Continue(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            LastStatement::Return(statement) => statement.for_each_token(self.f),
        }
    }

    fn process_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::Binary(node) => node.for_each_token(self.f),
            Expression::Call(node) => node.for_each_token(self.f),
            Expression::False(token)
            | Expression::Nil(token)
            | Expression::True(token)
            | Expression::VariableArguments(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            Expression::Field(node) => node.for_each_token(self.f),
            Expression::Function(node) => node.for_each_token(self.f),
            Expression::Identifier(node) => node.for_each_token(self.f),
            Expression::If(node) => node.for_each_token(self.f),
            Expression::Index(node) => node.for_each_token(self.f),
            Expression::InterpolatedString(node) => node.for_each_token(self.f),
            Expression::Number(node) => node.for_each_token(self.f),
            Expression::Parenthese(node) => node.for_each_token(self.f),
            Expression::String(node) => node.for_each_token(self.f),
            Expression::Table(_) => {}
            Expression::TypeCast(node) => node.for_each_token(self.f),
            Expression::TypeInstantiation(node) => node.for_each_token(self.f),
            Expression::Unary(node) => node.for_each_token(self.f),
        }
    }

    fn process_prefix(&mut self, prefix: &Prefix) {
        match prefix {
            Prefix::Call(node) => node.for_each_token(self.f),
            Prefix::Field(node) => node.for_each_token(self.f),
            Prefix::Identifier(node) => node.for_each_token(self.f),
            Prefix::Index(node) => node.for_each_token(self.f),
            Prefix::Parenthese(node) => node.for_each_token(self.f),
            Prefix::TypeInstantiation(node) => node.for_each_token(self.f),
        }
    }

    fn process_variable(&mut self, variable: &Variable) {
        match variable {
            Variable::Identifier(node) => node.for_each_token(self.f),
            Variable::Field(node) => node.for_each_token(self.f),
            Variable::Index(node) => node.for_each_token(self.f),
        }
    }

    fn process_function_call(&mut self, call: &crate::nodes::FunctionCall) {
        call.get_arguments().for_each_token(self.f);
    }

    fn process_table(&mut self, table: &crate::nodes::TableExpression) {
        table.for_each_token(self.f);
    }

    fn process_attributes(&mut self, attributes: &crate::nodes::Attributes) {
        attributes.for_each_token(self.f);
    }

    fn process_type(&mut self, r#type: &Type) {
        match r#type {
            Type::Name(node) => node.for_each_token(self.f),
            Type::Field(node) => node.for_each_token(self.f),
            Type::True(token) | Type::False(token) | Type::Nil(token) => {
                if let Some(token) = token {
                    token.for_each_token(self.f);
                }
            }
            Type::String(node) => node.for_each_token(self.f),
            Type::Array(node) => node.for_each_token(self.f),
            Type::Table(node) => node.for_each_token(self.f),
            Type::TypeOf(node) => node.for_each_token(self.f),
            Type::Parenthese(node) => node.for_each_token(self.f),
            Type::Function(node) => node.for_each_token(self.f),
            Type::Optional(node) => node.for_each_token(self.f),
            Type::Intersection(node) => node.for_each_token(self.f),
            Type::Union(node) => node.for_each_token(self.f),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nodes::{
        ArrayType, AssignStatement, Attributes, BinaryExpression, CompoundAssignStatement,
        DoStatement, ExpressionType, FieldExpression, FunctionAssignment, FunctionCall,
        FunctionExpression, FunctionStatement, FunctionType, GenericForStatement, GenericTypePack,
        GotoStatement, Identifier, IfExpression, IfStatement, IndexExpression,
        InterpolatedStringExpression, IntersectionType, LabelStatement, LiteralExpression,
        LiteralTable, NumberExpression, NumericForStatement, OptionalType, ParentheseExpression,
        ParentheseType, RepeatStatement, StringExpression, StringType, TableExpression, TableType,
        TypeCastExpression, TypeDeclarationStatement, TypeField, TypeFunctionStatement,
        TypeInstantiationExpression, TypeName, TypePack, UnaryExpression, UnionType,
        VariableAssignment, VariadicTypePack, WhileStatement,
    };
    use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
    use crate::Parser;

    /// The mutable counterpart of [`for_each_token`], used as an oracle.
    struct MutableWalk<'a, F> {
        f: &'a mut F,
    }

    macro_rules! mutable_methods {
        ($( $method:ident : $ty:ty ),* $(,)?) => {
            $( fn $method(&mut self, node: &mut $ty) { node.for_each_token(self.f); } )*
        };
    }

    impl<F: FnMut(&Token)> NodeProcessor for MutableWalk<'_, F> {
        mutable_methods!(
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

    fn mutable_trivia(block: &mut Block) -> Vec<Trivia> {
        let mut collected: Vec<Trivia> = Vec::new();
        {
            let mut walk = MutableWalk {
                f: &mut |token: &Token| {
                    for piece in token.iter_leading_trivia() {
                        if !collected.contains(piece) {
                            collected.push(piece.clone());
                        }
                    }
                    for piece in token.iter_trailing_trivia() {
                        if !collected.contains(piece) {
                            collected.push(piece.clone());
                        }
                    }
                },
            };
            DefaultVisitor::visit_block(block, &mut walk);
        }
        collected.sort_by_key(|piece| {
            (
                piece.get_start().unwrap_or(usize::MAX),
                piece.get_line_number().unwrap_or(usize::MAX),
            )
        });
        collected
    }

    const CORPUS: &[&str] = &[
        "-- top\nlocal x: number = 1 -- decl\nreturn x",
        "local function f(a: number, b): number -- params\n  return a + b -- body\nend",
        "function a.b.c:m(x) -- method\n  return self, x\nend",
        "local t = { key = 1, [expr] = 2, 3 } -- table",
        "@native function g() end -- attribute",
        "for i = 1, 10 do print(i) -- loop\nend",
        "for k, v in pairs(t) do -- pairs\n  print(k, v)\nend",
        "while c do break end -- while",
        "repeat until c -- repeat",
        "if a then b() elseif c then d() else e() end -- if",
        "type T = { a: number } -- type",
        "type function identity(x) return x end -- type fn",
        "return `value {x}` -- interp",
        "return f'string arg' -- string arg",
        "return f({ key = 1 }) -- table arg",
        "local a, b = 1, 2 -- multi\nreturn a, b",
        "do local inner = 1 end -- do",
        "return 0x10, 0b1010, 1e5 -- numbers",
    ];

    fn assert_read_only_matches(code: &str, parser: Parser) {
        let block = parser
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let mut mutable_block = block.clone();
        assert_eq!(
            collect_trivia(&block),
            mutable_trivia(&mut mutable_block),
            "read-only trivia traversal diverged for `{code}`"
        );
    }

    #[test]
    fn read_only_walk_matches_the_mutable_walk() {
        for code in CORPUS {
            assert_read_only_matches(code, Parser::default());
        }

        assert_read_only_matches("::label:: -- label\ngoto label", Parser::default().lua52());
        assert_read_only_matches(
            "local x <const> = 1 -- attributed",
            Parser::default().lua54(),
        );
    }
    fn assert_unique_tokens(code: &str, parser: Parser) {
        let block = parser
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let mut seen = std::collections::HashSet::new();
        let mut duplicates = 0usize;
        for_each_token(&block, &mut |token| {
            if !seen.insert(token as *const Token as usize) {
                duplicates += 1;
            }
        });
        assert_eq!(duplicates, 0, "duplicate token visits for `{code}`");
    }

    #[test]
    fn read_only_walk_visits_each_token_once() {
        for code in CORPUS {
            assert_unique_tokens(code, Parser::default());
        }
        assert_unique_tokens("::label:: -- label\ngoto label", Parser::default().lua52());
        assert_unique_tokens(
            "local x <const> = 1 -- attributed",
            Parser::default().lua54(),
        );
    }
}
