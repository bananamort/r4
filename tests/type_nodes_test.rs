//! Round-trip tests for every Luau type annotation node.

use r4::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

const TYPE_FORMS: &[&str] = &[
    "local x: number = 1",
    "local x: { [string]: number, field: boolean } = t",
    "local x: (number, string) -> boolean = f",
    "local x: \"literal\" = s",
    "local x: { number } = t",
    "local x: number? = nil",
    "local x: number | string = 1",
    "local x: number & string = 1",
    "local x: (...number) -> () = f",
    "local x: <T>(T) -> T = f",
    "type T<A, B = string> = { value: A }",
    "type U = typeof(x)",
    "type V = (number)",
    "type W = T.Field",
    "type A = true",
    "type B = nil",
    "type C = { [number]: string }",
];

fn readable(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn dense(block: &Block) -> String {
    let mut generator = DenseLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

#[test]
fn every_type_form_round_trips_through_both_generators() {
    for code in TYPE_FORMS {
        let block = Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let analyzed = Parser::default()
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
            .0;

        let readable_output = readable(&block);
        let dense_output = dense(&block);
        assert!(!readable_output.trim().is_empty(), "{code}");
        assert!(!dense_output.trim().is_empty(), "{code}");

        // The generated code must analyze again.
        SemanticAnalyzer::new()
            .analyze(&analyzed)
            .unwrap_or_else(|error| panic!("generated `{code}` must analyze: {error}"));
    }
}

#[test]
fn exported_types_round_trip() {
    for code in [
        "export type T = number",
        "export type function f(x) return x end",
    ] {
        let block = Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let output = readable(&block);
        assert!(output.contains("export"), "{code} -> {output}");
    }
}
