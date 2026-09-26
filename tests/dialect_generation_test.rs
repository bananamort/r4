//! Dialect-safe generation: validated, fallible generator finalization.

use r4::generator::{
    DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator, TokenBasedLuaGenerator,
};
use r4::{Dialect, DialectSupportError, Parser};

fn luau(code: &str) -> r4::nodes::Block {
    Parser::default()
        .luau()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

#[test]
fn readable_generator_rejects_unsupported_constructs() {
    let block = luau("value += 1");
    let mut generator = ReadableLuaGenerator::default().with_dialect(Dialect::Lua51);

    let error = generator
        .try_write_block(&block)
        .expect_err("compound assignment is Luau-only");
    assert_eq!(error.code(), "dialect/compound-assignment");
    assert_eq!(generator.target_dialect(), Dialect::Lua51);

    // Nothing was emitted, and the recorded error surfaces at finalization.
    assert_eq!(generator.clone().into_string(), "");
    assert_eq!(
        generator.try_into_string().unwrap_err(),
        r4::generator::GeneratorError::Dialect(error)
    );
}

#[test]
fn dense_generator_rejects_types_for_lua51() {
    let block = luau("local x: number = 1");
    let mut generator = DenseLuaGenerator::default().with_dialect(Dialect::Lua51);

    assert_eq!(
        generator.try_write_block(&block),
        Err(DialectSupportError::TypesNotSupported {
            dialect: Dialect::Lua51
        })
    );
}

#[test]
fn token_based_generator_validates_too() {
    let source = "value += 1";
    let block = luau(source);
    let mut generator = TokenBasedLuaGenerator::new(source).with_dialect(Dialect::Lua51);

    assert!(generator.try_write_block(&block).is_err());
    assert!(generator.try_into_string().is_err());
}

#[test]
fn token_based_generator_reports_a_mismatched_source() {
    use r4::generator::GeneratorError;

    let block = Parser::default()
        .preserve_tokens()
        .parse("return 1")
        .expect("valid code");
    // Feed a source that does not contain the token's range.
    let mut generator = TokenBasedLuaGenerator::new("");
    generator.write_block(&block);

    let error = generator
        .try_into_string()
        .expect_err("the range is out of bounds");
    assert!(matches!(error, GeneratorError::Source(_)));
    assert!(!error.to_string().is_empty());
}

#[test]
fn supported_blocks_write_and_finalize() {
    let block = luau("local x = 1\nreturn x");
    let mut generator = ReadableLuaGenerator::default().with_dialect(Dialect::Lua51);

    generator
        .try_write_block(&block)
        .expect("plain assignments are valid Lua 5.1");
    let output = generator.try_into_string().expect("no recorded error");
    assert!(output.contains("local x = 1"), "{output}");
}

#[test]
fn the_default_dialect_skips_validation() {
    let block = luau("value += 1");
    let mut generator = ReadableLuaGenerator::default().with_dialect(Dialect::Default);

    generator
        .try_write_block(&block)
        .expect("no target dialect means no validation");
    assert!(!generator.try_into_string().expect("no error").is_empty());
}

#[test]
fn validation_reports_the_construct_and_the_target() {
    let block = Parser::default()
        .lua53()
        .parse("return a & b")
        .expect("Lua 5.3 bitwise");
    let error = r4::validate_dialect(&block, Dialect::Lua51).expect_err("bitwise is Lua 5.3+");

    assert_eq!(error.dialect(), Dialect::Lua51);
    assert_eq!(error.code(), "dialect/bitwise-operators");
    assert!(!error.to_string().is_empty());
}

#[test]
fn desugar_to_lua51_fails_fast_on_unlowered_syntax() {
    let mut block = Parser::default()
        .lua53()
        .parse("return a & b")
        .expect("Lua 5.3 bitwise");
    let error = r4::rules::desugar_to_lua51(&mut block).expect_err("bitwise survives lowering");
    match error {
        r4::rules::DesugarError::UnsupportedDialect(inner) => {
            assert_eq!(inner.code(), "dialect/bitwise-operators");
        }
        other => panic!("expected an unsupported dialect error, got {other:?}"),
    }

    let mut block = Parser::default()
        .lua52()
        .parse("goto start\n::start::")
        .expect("Lua 5.2 goto");
    let error = r4::rules::desugar_to_lua51(&mut block).expect_err("goto survives lowering");
    match error {
        r4::rules::DesugarError::UnsupportedDialect(inner) => {
            assert_eq!(inner.code(), "dialect/goto");
        }
        other => panic!("expected an unsupported dialect error, got {other:?}"),
    }
}

#[test]
fn desugar_to_lua51_accepts_luau() {
    let mut block = luau("local total = 0\nfor i = 1, 3 do total += i end\nreturn total");
    r4::rules::desugar_to_lua51(&mut block).expect("Luau lowers to valid Lua 5.1");
    r4::validate_dialect(&block, Dialect::Lua51).expect("the lowered block is valid Lua 5.1");
}

#[test]
fn desugar_to_lua51_lowers_const_declarations() {
    let mut block = luau("const value = 1\nconst function f() end\nreturn value");
    r4::rules::desugar_to_lua51(&mut block).expect("const declarations lower to locals");

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(
        !output.contains("const"),
        "`const` must be lowered: {output}"
    );
    assert!(output.contains("local value = 1"), "{output}");
    assert!(output.contains("local function f()"), "{output}");
}
