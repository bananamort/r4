use r4::generator::{LuaGenerator, ReadableLuaGenerator, TokenBasedLuaGenerator};
use r4::nodes::*;
use r4::process::{Evaluator, LuaValue};
use r4::Parser;

#[test]
fn test_parse_and_generate() {
    let source = r#"
        local x = 10 + 20
        local function add(a, b)
            return a + b
        end
        if x > 25 then
            print("x is large:", x)
        else
            print("x is small")
        end
    "#;

    let parser = Parser::default();
    let block = parser.parse(source).expect("Failed to parse Lua source");

    assert!(block.iter_statements().count() >= 3);

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();

    assert!(output.contains("local x = 10 + 20"));
    assert!(output.contains("local function add(a, b)"));
    assert!(output.contains("if x > 25 then"));
}

#[test]
fn test_goto_and_label_roundtrip() {
    let source = "goto done\n::done::\nreturn";

    let parser = Parser::default().lua52();
    let block = parser
        .parse(source)
        .expect("Failed to parse Lua 5.2 source");

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();

    assert!(output.contains("goto done"), "generated: {output}");
    assert!(output.contains("::done::"), "generated: {output}");
    parser
        .parse(&output)
        .expect("generated Lua 5.2 code must parse again");

    let block_with_tokens = Parser::default()
        .lua52()
        .preserve_tokens()
        .parse(source)
        .expect("Failed to parse with tokens");
    let mut token_generator = TokenBasedLuaGenerator::new(source);
    token_generator.write_block(&block_with_tokens);
    assert_eq!(token_generator.into_string(), source);
}

#[test]
fn test_constant_evaluator() {
    let source = "return 10 * 5 + 2";
    let parser = Parser::default();
    let block = parser.parse(source).expect("Failed to parse");

    let evaluator = Evaluator::default();
    if let Some(LastStatement::Return(ret)) = block.get_last_statement() {
        let first_expr = ret.iter_expressions().next().unwrap();
        let val = evaluator.evaluate(first_expr);
        assert_eq!(val, LuaValue::Number(52.0));
        assert_eq!(val.is_truthy(), Some(true));
    } else {
        panic!("Expected return statement");
    }
}

#[test]
fn malformed_unicode_escape_is_a_parse_error() {
    // A surrogate code point is not a valid `char`; the converter must report a
    // parse error rather than panicking.
    let error = Parser::default()
        .parse("return \"\\u{D800}\"")
        .expect_err("a surrogate escape must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("invalid unicode scalar value"),
        "unexpected error: {message}"
    );
}
