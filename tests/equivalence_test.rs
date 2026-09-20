//! Tests for structural equivalence (`Equivalent`).

use r4::nodes::{Expression, LastStatement, Statement};
use r4::semantic::SemanticAnalyzer;
use r4::{Equivalent, Parser};

fn with_tokens(code: &str) -> r4::nodes::Block {
    Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn without_tokens(code: &str) -> r4::nodes::Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

/// A corpus covering every token-bearing node kind, so a missed token field in
/// detokenization shows up as a false negative.
const CORPUS: &[&str] = &[
    "local x = 1",
    "local x, y = 1, 2",
    "local x: number = 1",
    "local x = nil",
    "const x = 1",
    "x = 1",
    "x, y = 1, 2",
    "x += 1",
    "x -= y",
    "x ..= 'a'",
    "do end",
    "do local a = 1 end",
    "while condition do break end",
    "repeat until condition",
    "if a then b() elseif c then d() else e() end",
    "for i = 1, 10 do print(i) end",
    "for k, v in pairs(t) do print(k, v) end",
    "local function f(a, b) return a + b end",
    "function f(a, b) return a + b end",
    "function a.b.c(x) end",
    "function a:b(x) return self end",
    "return function(...) return ... end",
    "return a + b * c - d / e % f ^ g",
    "return a and b or not c",
    "return a == b or a ~= b or a < b or a <= b or a > b or a >= b",
    "return a .. b .. c",
    "return -x + #t",
    "return value // 2",
    "return t[1]",
    "return t['field']",
    "return t.field",
    "return obj:method(1, 'x')",
    "return f'string argument'",
    "return f{ key = value }",
    "return { 1, 2, key = 3, [expr] = 4, ['x'] = 5 }",
    "return if a then 1 else 2",
    "return (a + b)",
    "return 'single', \"double\", [[long]]",
    "return 0x10, 0b1010, 1e5, 3.14",
    "return true, false, nil",
    "return `interpolated {value} text`",
    "return value :: number",
    "return Vector3.new(1, 2, 3)",
    "return f()()",
    "return f().x[1]",
    "local s = @native function() end",
    "@native function f() end",
    "local t = {} t.x = 1",
    "type T = { a: number }",
    "type T = number",
    "type function identity(x) return x end",
    "export type T = string",
    "local x: T? = nil",
    "local x: number | string = 1",
    "local x: { [string]: number } = {}",
    "local x: () -> number = nil",
    "local x: <T>(T) -> T = nil",
    "return 1;",
    "local a = 1; local b = 2;",
];

#[test]
fn tokenful_and_tokenless_trees_are_equivalent() {
    for code in CORPUS {
        let tokenful = with_tokens(code);
        let tokenless = without_tokens(code);
        assert!(
            tokenful.is_equivalent_to(&tokenless),
            "tokenful and tokenless `{code}` must be equivalent"
        );
        assert!(
            tokenless.is_equivalent_to(&tokenful),
            "equivalence must be symmetric for `{code}`"
        );
        assert!(tokenful.is_equivalent_to(&tokenful), "reflexive: {code}");
    }
}

#[test]
fn trivia_and_positions_are_ignored() {
    let tokenful = with_tokens("local x = 1 -- keep this\n-- and this\nreturn x ");
    let plain = without_tokens("local x=1\nreturn x");
    assert!(tokenful.is_equivalent_to(&plain));
    assert!(plain.is_equivalent_to(&tokenful));
}

#[test]
fn semantic_bindings_are_ignored() {
    let analyzed = with_tokens("local value = 1\nlocal function f() return value end\nreturn f");
    SemanticAnalyzer::new()
        .analyze(&analyzed)
        .expect("analysis must succeed");
    let unanalyzed =
        without_tokens("local value = 1\nlocal function f() return value end\nreturn f");

    assert!(analyzed.is_equivalent_to(&unanalyzed));
    assert!(unanalyzed.is_equivalent_to(&analyzed));
}

#[test]
fn structural_differences_are_detected() {
    let cases = [
        ("return a + b", "return a - b"),
        ("return a + b", "return a + c"),
        ("local x = 1", "local y = 1"),
        ("local x = 1", "const x = 1"),
        ("return 0x10", "return 16"),
        ("return 1", "return 2"),
        ("return f()", "return f(1)"),
        ("return t.a", "return t.b"),
        ("return a and b", "return a or b"),
        ("if a then b() end", "if a then c() end"),
        ("local x = 1", "local x = 1\nlocal y = 2"),
    ];
    for (left, right) in cases {
        let left = with_tokens(left);
        let right = with_tokens(right);
        assert!(
            !left.is_equivalent_to(&right),
            "`{left:?}` and `{right:?}` must not be equivalent"
        );
        assert!(
            !right.is_equivalent_to(&left),
            "equivalence must be symmetric for a negative case"
        );
    }
}

#[test]
fn formatting_only_differences_are_equivalent() {
    let cases = [
        ("local x = 1\nreturn x", "local x=1 return x"),
        ("return a + b + c", "return a  +  b  +  c"),
        ("return { a = 1, b = 2 }", "return {a=1,b=2}"),
        (
            "if a then\n  b()\nelse\n  c()\nend",
            "if a then b() else c() end",
        ),
    ];
    for (left, right) in cases {
        assert!(
            with_tokens(left).is_equivalent_to(&with_tokens(right)),
            "`{left}` and `{right}` must be equivalent"
        );
    }
}

#[test]
fn statement_equivalence_ignores_tokens() {
    let tokenful = with_tokens("local x = 1");
    let tokenless = without_tokens("local  x  =  1");
    let tokenful = tokenful.first_statement().expect("a statement");
    let tokenless = tokenless.first_statement().expect("a statement");
    assert!(tokenful.is_equivalent_to(tokenless));
    assert!(matches!(tokenful, Statement::LocalAssign(_)));
}

#[test]
fn expression_equivalence_ignores_tokens() {
    let expression = |code: &str| -> Expression {
        let block = without_tokens(code);
        let expression = match block.get_last_statement() {
            Some(LastStatement::Return(statement)) => statement.iter_expressions().next().cloned(),
            other => panic!("expected a return, got {other:?}"),
        };
        expression.expect("an expression")
    };

    assert!(expression("return a + b").is_equivalent_to(&expression("return a+b")));
    assert!(!expression("return a + b").is_equivalent_to(&expression("return a * b")));
}

#[test]
fn detokenized_trees_still_generate() {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};
    use r4::process::detokenize;

    let mut block = with_tokens("local x = 1 -- comment\nreturn x");
    detokenize(&mut block);

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(output.contains("local x = 1"), "{output}");
    assert!(!output.contains("comment"), "trivia must be gone: {output}");
}

#[test]
fn equivalence_holds_across_a_transform_that_only_drops_trivia() {
    use r4::rules::remove_comments;

    let original = with_tokens("local x = 1 -- keep\nreturn x");
    let mut stripped = original.clone();
    remove_comments(&mut stripped);

    assert!(stripped.is_equivalent_to(&original));
    assert!(original.is_equivalent_to(&stripped));
}

#[test]
fn dialect_specific_trees_are_equivalent_without_tokens() {
    let lua53 = || Parser::default().lua53();
    for code in [
        "return a & b",
        "return a | b",
        "return a ~ b",
        "return a << b",
        "return a >> b",
        "return a // b",
    ] {
        let tokenful = lua53()
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let tokenless = lua53()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        assert!(tokenful.is_equivalent_to(&tokenless), "{code}");
    }

    let lua54 = || Parser::default().lua54();
    for code in ["local x <const> = 1", "local y <close> = f()"] {
        let tokenful = lua54()
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let tokenless = lua54()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        assert!(tokenful.is_equivalent_to(&tokenless), "{code}");
    }
}
