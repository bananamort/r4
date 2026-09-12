use r4::generator::{LuaGenerator, ReadableLuaGenerator};
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
