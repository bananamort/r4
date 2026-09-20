//! Tests for the `SyntaxFactory` façade.

use r4::nodes::{BinaryOperator, Expression, UnaryOperator};
use r4::{Equivalent, Parser, SyntaxFactory};

fn parsed(code: &str) -> r4::nodes::Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn returning(expression: Expression) -> r4::nodes::Block {
    SyntaxFactory::block_with_last(Vec::new(), Some(SyntaxFactory::return_(vec![expression])))
}

#[test]
fn builds_a_program_equivalent_to_source() {
    let built = SyntaxFactory::block_with_last(
        vec![SyntaxFactory::local(
            vec![SyntaxFactory::typed("x")],
            vec![SyntaxFactory::number(1.0)],
        )],
        Some(SyntaxFactory::return_(vec![SyntaxFactory::variable("x")])),
    );
    assert!(built.is_equivalent_to(&parsed("local x = 1\nreturn x")));
}

#[test]
fn builds_literals() {
    assert!(returning(SyntaxFactory::nil()).is_equivalent_to(&parsed("return nil")));
    assert!(returning(SyntaxFactory::boolean(true)).is_equivalent_to(&parsed("return true")));
    assert!(returning(SyntaxFactory::boolean(false)).is_equivalent_to(&parsed("return false")));
    assert!(returning(SyntaxFactory::number(42.0)).is_equivalent_to(&parsed("return 42")));
    assert!(returning(SyntaxFactory::string("hello")).is_equivalent_to(&parsed("return 'hello'")));
    assert!(returning(SyntaxFactory::varargs()).is_equivalent_to(&parsed("return ...")));
    assert!(returning(SyntaxFactory::table()).is_equivalent_to(&parsed("return {}")));
}

#[test]
fn builds_access_expressions() {
    let field = SyntaxFactory::field(SyntaxFactory::variable("t"), "x");
    assert!(returning(field).is_equivalent_to(&parsed("return t.x")));

    let index = SyntaxFactory::index(SyntaxFactory::variable("t"), SyntaxFactory::number(1.0));
    assert!(returning(index).is_equivalent_to(&parsed("return t[1]")));

    let nested = SyntaxFactory::field(
        SyntaxFactory::index(SyntaxFactory::variable("t"), SyntaxFactory::number(1.0)),
        "x",
    );
    assert!(returning(nested).is_equivalent_to(&parsed("return t[1].x")));
}

#[test]
fn builds_operator_expressions() {
    let binary = SyntaxFactory::binary(
        BinaryOperator::Plus,
        SyntaxFactory::variable("a"),
        SyntaxFactory::number(1.0),
    );
    assert!(returning(binary).is_equivalent_to(&parsed("return a + 1")));

    let unary = SyntaxFactory::unary(UnaryOperator::Minus, SyntaxFactory::variable("a"));
    assert!(returning(unary).is_equivalent_to(&parsed("return -a")));

    let parenthese = SyntaxFactory::parenthese(SyntaxFactory::variable("a"));
    assert!(returning(parenthese).is_equivalent_to(&parsed("return (a)")));
}

#[test]
fn builds_calls() {
    let call = SyntaxFactory::call(
        SyntaxFactory::prefix("f"),
        SyntaxFactory::arguments(vec![SyntaxFactory::number(1.0)]),
    );
    assert!(returning(call).is_equivalent_to(&parsed("return f(1)")));

    let method = SyntaxFactory::method_call(
        SyntaxFactory::variable("obj"),
        "method",
        SyntaxFactory::arguments(Vec::new()),
    );
    assert!(returning(method).is_equivalent_to(&parsed("return obj:method()")));
}

#[test]
fn builds_statements() {
    let assign = SyntaxFactory::block(vec![SyntaxFactory::assign(
        vec![SyntaxFactory::assign_target("x")],
        vec![SyntaxFactory::number(1.0)],
    )]);
    assert!(assign.is_equivalent_to(&parsed("x = 1")));

    let call_statement = SyntaxFactory::block(vec![SyntaxFactory::call_statement(
        r4::nodes::FunctionCall::new(
            SyntaxFactory::prefix("f"),
            SyntaxFactory::arguments(Vec::new()),
            None,
        ),
    )]);
    assert!(call_statement.is_equivalent_to(&parsed("f()")));

    let do_block = SyntaxFactory::block(vec![SyntaxFactory::do_block(SyntaxFactory::block(vec![
        SyntaxFactory::local(
            vec![SyntaxFactory::typed("x")],
            vec![SyntaxFactory::number(1.0)],
        ),
    ]))]);
    assert!(do_block.is_equivalent_to(&parsed("do local x = 1 end")));
}

#[test]
fn builds_typed_identifiers() {
    let built = SyntaxFactory::block(vec![SyntaxFactory::local(
        vec![SyntaxFactory::typed_with(
            "x",
            r4::nodes::TypeName::new("number"),
        )],
        vec![SyntaxFactory::number(1.0)],
    )]);
    assert!(built.is_equivalent_to(&parsed("local x: number = 1")));
}

#[test]
fn builds_tokens_and_trivia() {
    let token = SyntaxFactory::token("local");
    assert_eq!(
        token.get_position(),
        &r4::nodes::Position::Any {
            content: "local".into()
        }
    );

    let comment = SyntaxFactory::comment("-- note");
    assert_eq!(comment.kind(), r4::nodes::TriviaKind::Comment);
    let shebang = SyntaxFactory::shebang("#!/usr/bin/lua");
    assert_eq!(shebang.kind(), r4::nodes::TriviaKind::Shebang);

    let whitespace = SyntaxFactory::whitespace(" ");
    assert_eq!(whitespace.kind(), r4::nodes::TriviaKind::Whitespace);
}

#[test]
fn parses_number_literals() {
    assert!(SyntaxFactory::parse_number("0x10").is_ok());
    assert!(SyntaxFactory::parse_number("not a number").is_err());
}

#[test]
fn built_trees_generate() {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};

    let built = SyntaxFactory::block_with_last(
        vec![SyntaxFactory::local(
            vec![SyntaxFactory::typed("x")],
            vec![SyntaxFactory::number(1.0)],
        )],
        Some(SyntaxFactory::return_(vec![SyntaxFactory::variable("x")])),
    );
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&built);
    let output = generator.into_string();
    assert!(output.contains("local x = 1"), "{output}");
    assert!(output.contains("return x"), "{output}");
}
