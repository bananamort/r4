//! A cohesive factory for constructing syntax nodes, tokens and trivia.
//!
//! [`SyntaxFactory`] is the Loretta-style entry point for building an AST
//! programmatically. It groups the node constructors and the `with_*` builders
//! behind one namespace and adds the convenience combinations that otherwise
//! take several calls. Everything it returns is an ordinary node: the factory
//! never attaches tokens or trivia unless a caller adds them, so a built tree
//! is [`is_equivalent_to`](crate::Equivalent::is_equivalent_to) the same source
//! parsed without token data.

use std::borrow::Cow;

use crate::nodes::{
    Arguments, AssignStatement, BinaryExpression, BinaryOperator, Block, DoStatement, Expression,
    FieldExpression, FunctionCall, Identifier, IndexExpression, IntoLuaStringValue, LastStatement,
    NumberExpression, ParentheseExpression, Prefix, ReturnStatement, Statement, StringExpression,
    TableExpression, Token, Trivia, TriviaKind, TupleArguments, TypedIdentifier, UnaryExpression,
    UnaryOperator, Variable, VariableAssignment,
};

/// Constructors for syntax nodes, tokens and trivia.
///
/// ```
/// use r4::{Equivalent, Parser, SyntaxFactory};
///
/// let built = SyntaxFactory::block_with_last(
///     vec![SyntaxFactory::local(
///         vec![SyntaxFactory::typed("x")],
///         vec![SyntaxFactory::number(1.0)],
///     )],
///     Some(SyntaxFactory::return_(vec![SyntaxFactory::variable("x")])),
/// );
///
/// let parsed = Parser::default().parse("local x = 1\nreturn x").unwrap();
/// assert!(built.is_equivalent_to(&parsed));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SyntaxFactory;

impl SyntaxFactory {
    // ------------------------------------------------------------------
    // Tokens and trivia
    // ------------------------------------------------------------------

    /// Creates a token with the given content and no source position.
    pub fn token(content: impl Into<Cow<'static, str>>) -> Token {
        Token::from_content(content)
    }

    /// Creates a comment trivia.
    pub fn comment(text: impl Into<Cow<'static, str>>) -> Trivia {
        TriviaKind::Comment.with_content(text)
    }

    /// Creates a whitespace trivia.
    pub fn whitespace(text: impl Into<Cow<'static, str>>) -> Trivia {
        TriviaKind::Whitespace.with_content(text)
    }

    // ------------------------------------------------------------------
    // Names
    // ------------------------------------------------------------------

    /// Creates an identifier.
    pub fn name(name: impl Into<String>) -> Identifier {
        Identifier::new(name)
    }

    /// Creates an identifier expression (a variable reference).
    pub fn variable(name: impl Into<String>) -> Expression {
        Expression::identifier(name)
    }

    /// Creates a variable reference usable as a prefix.
    pub fn prefix(name: impl Into<String>) -> Prefix {
        Prefix::Identifier(Identifier::new(name))
    }

    /// Creates a typed identifier with no type annotation.
    pub fn typed(name: impl Into<String>) -> TypedIdentifier {
        TypedIdentifier::new(name)
    }

    /// Creates a typed identifier with a type annotation.
    pub fn typed_with(
        name: impl Into<String>,
        r#type: impl Into<crate::nodes::Type>,
    ) -> TypedIdentifier {
        TypedIdentifier::new(name).with_type(r#type)
    }

    /// Creates an assignment target variable.
    pub fn assign_target(name: impl Into<String>) -> Variable {
        Variable::new(name)
    }

    // ------------------------------------------------------------------
    // Literals
    // ------------------------------------------------------------------

    /// Creates the `nil` literal.
    pub fn nil() -> Expression {
        Expression::nil()
    }

    /// Creates a boolean literal.
    pub fn boolean(value: bool) -> Expression {
        value.into()
    }

    /// Creates a floating-point number literal.
    pub fn number(value: f64) -> Expression {
        value.into()
    }

    /// Creates a string literal.
    pub fn string(value: impl IntoLuaStringValue) -> Expression {
        StringExpression::from_value(value).into()
    }

    /// Creates the `...` expression.
    pub fn varargs() -> Expression {
        Expression::variable_arguments()
    }

    /// Creates an empty table constructor.
    pub fn table() -> Expression {
        TableExpression::default().into()
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    /// Creates a field access (`prefix.field`).
    pub fn field(prefix: impl Into<Prefix>, field: impl Into<Identifier>) -> Expression {
        FieldExpression::new(prefix, field).into()
    }

    /// Creates an index access (`prefix[index]`).
    pub fn index(prefix: impl Into<Prefix>, index: impl Into<Expression>) -> Expression {
        IndexExpression::new(prefix, index).into()
    }

    /// Creates a parenthesized expression.
    pub fn parenthese(expression: impl Into<Expression>) -> Expression {
        ParentheseExpression::new(expression).into()
    }

    /// Creates a binary expression.
    pub fn binary(
        operator: BinaryOperator,
        left: impl Into<Expression>,
        right: impl Into<Expression>,
    ) -> Expression {
        BinaryExpression::new(operator, left, right).into()
    }

    /// Creates a unary expression.
    pub fn unary(operator: UnaryOperator, expression: impl Into<Expression>) -> Expression {
        UnaryExpression::new(operator, expression).into()
    }

    /// Creates a function call.
    pub fn call(prefix: impl Into<Prefix>, arguments: impl Into<Arguments>) -> Expression {
        FunctionCall::new(prefix.into(), arguments.into(), None).into()
    }

    /// Creates a method call (`prefix:method(arguments)`).
    pub fn method_call(
        prefix: impl Into<Prefix>,
        method: impl Into<Identifier>,
        arguments: impl Into<Arguments>,
    ) -> Expression {
        FunctionCall::new(prefix.into(), arguments.into(), Some(method.into())).into()
    }

    /// Creates a parenthesized argument tuple.
    pub fn arguments(values: Vec<Expression>) -> Arguments {
        TupleArguments::new(values).into()
    }

    // ------------------------------------------------------------------
    // Statements and blocks
    // ------------------------------------------------------------------

    /// Creates a local variable declaration.
    pub fn local(variables: Vec<TypedIdentifier>, values: Vec<Expression>) -> Statement {
        VariableAssignment::new(variables, values).into()
    }

    /// Creates an assignment statement.
    pub fn assign(variables: Vec<Variable>, values: Vec<Expression>) -> Statement {
        AssignStatement::new(variables, values).into()
    }

    /// Creates a call statement from a function call.
    pub fn call_statement(call: FunctionCall) -> Statement {
        call.into()
    }

    /// Creates a `do ... end` statement.
    pub fn do_block(block: Block) -> Statement {
        DoStatement::new(block).into()
    }

    /// Creates a `return` statement.
    pub fn return_(expressions: Vec<Expression>) -> LastStatement {
        ReturnStatement::new(expressions).into()
    }

    /// Creates a block with the given statements and no final statement.
    pub fn block(statements: Vec<Statement>) -> Block {
        Block::new(statements, None)
    }

    /// Creates a block with the given statements and final statement.
    pub fn block_with_last(
        statements: Vec<Statement>,
        last_statement: Option<LastStatement>,
    ) -> Block {
        Block::new(statements, last_statement)
    }

    /// Creates a number expression from its literal representation, failing on
    /// malformed input.
    ///
    /// This is the fallible counterpart of [`SyntaxFactory::number`] for callers
    /// that only have the source text.
    pub fn parse_number(text: &str) -> Result<NumberExpression, crate::nodes::NumberParsingError> {
        text.parse()
    }
}
