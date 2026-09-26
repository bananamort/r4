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
            Parser::default().luau().parse(code).is_err(),
            "Luau must reject `{code}`"
        );
        // The default detects Lua 5.3 from the operator instead.
        Parser::default().parse(code).unwrap_or_else(|error| {
            panic!("the default must detect Lua 5.3 for `{code}`: {error}")
        });
    }
}

#[test]
fn evaluates_bitwise_operators() {
    let evaluator = Evaluator::default();
    let lookup = |_: &r4::nodes::Identifier| None;

    let cases = [
        ("return 1 | 2", Some(LuaValue::Integer(3))),
        ("return 6 & 3", Some(LuaValue::Integer(2))),
        ("return 6 ~ 3", Some(LuaValue::Integer(5))),
        ("return 1 << 4", Some(LuaValue::Integer(16))),
        ("return 16 >> 2", Some(LuaValue::Integer(4))),
        ("return ~0", Some(LuaValue::Integer(-1))),
        ("return 1 << 64", Some(LuaValue::Integer(0))),
        // A negative shift reverses direction (Lua 5.3).
        ("return 1 << -1", Some(LuaValue::Integer(0))),
    ];

    for (code, expected) in cases {
        let expression = returned_expression(code, Parser::default().lua53());
        let value = evaluator.evaluate_with(&expression, &lookup);
        match expected {
            Some(number) => assert_eq!(value, Some(number), "{code}"),
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

#[test]
fn parser_error_kinds_render() {
    let error = Parser::default()
        .parse_analyzed("local x = ")
        .expect_err("a syntax error must be reported");
    assert!(!error.to_string().is_empty());
    assert!(!format!("{error:?}").is_empty());

    let panic_error = Parser::default()
        .luau()
        .parse_analyzed("return 3 & 1")
        .expect_err("the upstream panic must become an error");
    assert!(
        panic_error.to_string().contains("panicked"),
        "{panic_error}"
    );
}

#[test]
fn default_dialect_detects_the_source_dialect() {
    assert_eq!(Parser::default().get_dialect(), Dialect::Default);

    // Luau-only constructs select Luau.
    for code in [
        "local x = 1\nx += 2\nreturn x",
        "return `a {1}`",
        "local function f<T>(x: T): T\n\treturn x\nend\nreturn f(1)",
    ] {
        let block = Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("the default must parse `{code}`: {error}"));
        r4::semantic::SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("`{code}` must analyze: {error}"));
        Parser::default()
            .luau()
            .parse(code)
            .unwrap_or_else(|error| panic!("explicit Luau must parse `{code}`: {error}"));
    }

    // Lua 5.3 bitwise operators select Lua 5.3.
    Parser::default()
        .parse("return 6 & 3")
        .expect("the default detects Lua 5.3");
    assert!(Parser::default().luau().parse("return 6 & 3").is_err());

    // Labels and goto select a Lua 5.2 or later dialect.
    Parser::default()
        .parse("::again::\nlocal s = 1\ngoto again")
        .expect("the default detects Lua 5.2");
    assert!(Parser::default().luau().parse("::again::").is_err());
}

#[test]
fn default_dialect_reports_a_syntactic_overlap() {
    // `a << b >> (c)` is a Luau type instantiation and a Lua 5.3 shift chain.
    let error = Parser::default()
        .parse("local x = a << b >> (c)")
        .expect_err("the overlap is ambiguous");
    assert_eq!(error.code(), "parse/ambiguous-dialect");
    assert!(!error.to_string().is_empty());

    // Choosing either dialect resolves it.
    Parser::default()
        .luau()
        .parse("local x = a << b >> (c)")
        .expect("Luau type instantiation");
    Parser::default()
        .lua53()
        .parse("local x = a << b >> (c)")
        .expect("the Lua 5.3 shift chain");

    // Unambiguous `::` is read by position, not reported as ambiguous.
    Parser::default()
        .parse("local x = y :: number")
        .expect("a Luau cast");
    Parser::default()
        .parse("::again::\nlocal s = 1\ngoto again")
        .expect("Lua 5.2 labels");
}

#[test]
fn default_detects_luau_type_instantiations() {
    // `<<`/`>>` that Lua 5.3/5.4 cannot read names Luau.
    for code in [
        "return f<<T>>",
        "return f<<T>>.x",
        "return f<<T>>:m()",
        "f<<T>>(x)",
        "foo.bar<<T>>()",
        "f<<T, U>>(x)",
        "f<<T>>(x, y)",
        "local d = f<<T>>()",
        "return obj:m<<T>>(1)",
        "local x = 1\nx += 2\nreturn f<<T>>(x)",
    ] {
        Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("the default must parse `{code}`: {error}"));
    }

    // A `>>` with a shift operand on its right stays an ordinary Lua 5.3 shift.
    Parser::default()
        .parse("return a << b >> c")
        .expect("a Lua 5.3 shift chain");

    // A closing `)` before the `>>` means the `<<` was a shift inside brackets,
    // not the start of a type instantiation.
    Parser::default()
        .parse("return (a << b) >> (c)")
        .expect("a parenthesized Lua 5.3 shift chain");

    // `a << b >> (c)` is both a shift and an instantiation call in expression
    // position, so it cannot be resolved without a choice.
    for code in [
        "local x = a << b >> (c)",
        "local y = f<<T>>(x)",
        "return f<<T>>(x)",
    ] {
        let error = Parser::default()
            .parse(code)
            .expect_err("the call form is ambiguous");
        assert_eq!(error.code(), "parse/ambiguous-dialect", "{code}");
    }
}

#[test]
fn default_reads_goto_as_an_identifier_for_lua51() {
    // Lua 5.1 and Luau treat `goto` as an ordinary name; the dialects between
    // them reserve it, so a non-label use selects the pair and parses as Luau.
    for code in [
        "local goto = 1\nreturn goto",
        "t.goto = 1\nreturn t.goto",
        "goto()",
    ] {
        Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("the default must parse `{code}`: {error}"));
    }

    // A label after `goto` is still a Lua 5.2 statement.
    Parser::default()
        .parse("::again::\ngoto again")
        .expect("a Lua 5.2 goto statement");
}
