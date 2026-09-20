//! Tests for the parser dialect presets and Lua 5.3/5.4/LuaJIT syntax.

use r4::generator::{
    DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator, TokenBasedLuaGenerator,
};
use r4::nodes::{Expression, LastStatement, UnaryOperator};
use r4::process::{Evaluator, LuaValue};
use r4::{Dialect, Parser};

fn generate(block: &r4::nodes::Block) -> String {
    let mut generator = DenseLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn returned_expression(code: &str, parser: Parser) -> Expression {
    let block = parser
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    match block.get_last_statement() {
        Some(LastStatement::Return(statement)) => statement
            .iter_expressions()
            .next()
            .cloned()
            .expect("a returned expression"),
        other => panic!("expected a return, got {other:?}"),
    }
}

#[test]
fn lua53_parses_and_generates_bitwise_operators() {
    let parser = Parser::default().lua53();
    assert_eq!(parser.get_dialect(), Dialect::Lua53);

    for (code, expected) in [
        ("return 1 | 2", "return 1|2"),
        ("return 1 & 2", "return 1&2"),
        ("return 1 ~ 2", "return 1~2"),
        ("return 1 << 2", "return 1<<2"),
        ("return 1 >> 2", "return 1>>2"),
        ("return ~1", "return~1"),
    ] {
        let block = parser
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        assert_eq!(generate(&block), expected, "{code}");
    }
}

#[test]
fn luau_rejects_bitwise_operators() {
    for code in ["return 1 | 2", "return 1 & 2", "return 1 << 2"] {
        assert!(
            Parser::default().parse(code).is_err(),
            "Luau must reject `{code}`"
        );
    }
}

#[test]
fn evaluates_bitwise_operators() {
    let evaluator = Evaluator::default();
    let lookup = |_: &r4::nodes::Identifier| None;

    let cases = [
        ("return 1 | 2", Some(3.0)),
        ("return 6 & 3", Some(2.0)),
        ("return 6 ~ 3", Some(5.0)),
        ("return 1 << 4", Some(16.0)),
        ("return 16 >> 2", Some(4.0)),
        ("return ~0", Some(-1.0)),
        ("return 1 << 64", Some(0.0)),
        // A negative shift reverses direction (Lua 5.3).
        ("return 1 << -1", Some(0.0)),
    ];

    for (code, expected) in cases {
        let expression = returned_expression(code, Parser::default().lua53());
        let value = evaluator.evaluate_with(&expression, &lookup);
        match expected {
            Some(number) => assert_eq!(value, Some(LuaValue::from(number)), "{code}"),
            None => assert_eq!(value, None, "{code}"),
        }
    }
}

#[test]
fn non_integer_bitwise_operands_are_not_folded() {
    let evaluator = Evaluator::default();
    let lookup = |_: &r4::nodes::Identifier| None;
    let expression = returned_expression("return 1.5 & 2", Parser::default().lua53());
    assert_eq!(evaluator.evaluate_with(&expression, &lookup), None);
}

#[test]
fn lua54_parses_local_variable_attributes() {
    let parser = Parser::default().lua54();
    let block = parser
        .parse("local x <const> = 1\nreturn x")
        .expect("Lua 5.4 local attributes parse");
    let output = generate(&block);
    assert!(output.contains("const"), "{output}");
}

#[test]
fn luajit_parses_goto() {
    let block = Parser::default()
        .lua_jit()
        .parse("::top::\ngoto top")
        .expect("LuaJIT goto parses");
    let output = generate(&block);
    assert!(output.contains("goto top"), "{output}");
}

#[test]
fn lua51_rejects_goto() {
    assert!(Parser::default()
        .lua51()
        .parse("::top::\ngoto top")
        .is_err());
}

#[test]
fn unary_operator_string_is_bitwise_not() {
    assert_eq!(UnaryOperator::BitwiseNot.to_str(), "~");
}

#[test]
fn round_trips_goto_and_labels_in_lua52() {
    let source = "goto done\n::done::\nreturn";
    let parser = Parser::default().lua52();

    let block = parser.parse(source).expect("the source must parse");
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(output.contains("goto done"), "generated: {output}");
    assert!(output.contains("::done::"), "generated: {output}");
    parser
        .parse(&output)
        .expect("the generated source must parse again");

    let block_with_tokens = Parser::default()
        .lua52()
        .preserve_tokens()
        .parse(source)
        .expect("the source must parse with tokens");
    let mut token_generator = TokenBasedLuaGenerator::new(source);
    token_generator.write_block(&block_with_tokens);
    assert_eq!(token_generator.into_string(), source);
}
