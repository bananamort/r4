//! Differential fuzzing: random Luau blocks must survive generation, and must
//! survive lowering to Lua 5.1 when the pipeline can lower them.
//!
//! The property checked is **generation idempotence**: rendering a block,
//! re-parsing the output, and rendering again must produce the same text. This
//! is the right notion here because the generator inserts precedence parentheses
//! that do not correspond to explicit nodes; AST equality would be too strong.
//!
//! This file is excluded from coverage builds, like the other fuzz suites.

#![cfg(not(coverage))]

mod ast_fuzzer;

use ast_fuzzer::{AstFuzzer, FuzzBudget};
use r4::cfg::CfgErrorKind;
use r4::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::rules::{desugar_to_lua51, DesugarError};
use r4::{Dialect, DialectSupportError, Parser};

const ITERATIONS: usize = 250;

/// Renders `block` with each generator, checks `parse` accepts the output, and
/// checks that rendering the re-parsed block reproduces the same text.
fn assert_stable(
    iteration: usize,
    label: &str,
    block: &Block,
    parse: impl Fn(&str) -> Result<Block, r4::ParserError>,
) {
    for (name, output) in generated(block) {
        let reparsed = parse(&output).unwrap_or_else(|error| {
            panic!("iteration {iteration} [{label}/{name}] generated code does not parse: {error}\n{output}")
        });

        let regenerated = match name {
            "readable" => {
                let mut generator = ReadableLuaGenerator::default();
                generator.write_block(&reparsed);
                generator.into_string()
            }
            _ => {
                let mut generator = DenseLuaGenerator::default();
                generator.write_block(&reparsed);
                generator.into_string()
            }
        };
        assert_eq!(
            output, regenerated,
            "iteration {iteration} [{label}/{name}] generation is not idempotent"
        );
    }
}

fn generated(block: &Block) -> Vec<(&'static str, String)> {
    let mut readable = ReadableLuaGenerator::default();
    readable.write_block(block);
    let mut dense = DenseLuaGenerator::default();
    dense.write_block(block);
    vec![
        ("readable", readable.into_string()),
        ("dense", dense.into_string()),
    ]
}

#[test]
fn fuzzed_luau_blocks_generate_idempotently() {
    for iteration in 0..ITERATIONS {
        let block = AstFuzzer::new(FuzzBudget::new(3, 6).with_types(2)).fuzz_block();
        assert_stable(iteration, "luau", &block, |output| {
            Parser::default().luau().parse(output)
        });
    }
}

#[test]
fn fuzzed_luau_blocks_lower_to_reparseable_lua51() {
    let mut lowered_count = 0;

    for iteration in 0..ITERATIONS {
        let block = AstFuzzer::new(FuzzBudget::new(3, 6).with_types(2)).fuzz_block();

        let mut lowered = block.clone();
        match desugar_to_lua51(&mut lowered) {
            Ok(()) => {
                lowered_count += 1;
                r4::validate_dialect(&lowered, Dialect::Lua51).unwrap_or_else(|error| {
                    panic!("iteration {iteration} lowered block is not Lua 5.1: {error}")
                });
                assert_stable(iteration, "lua51", &lowered, |output| {
                    Parser::default().lua51().parse(output)
                });
            }
            // The fuzzer places `break`/`continue` in functions that have no
            // loop, which is invalid; r4 reports it. The curated corpus and the
            // execution oracle cover loops that are real.
            Err(DesugarError::Cfg(error))
                if matches!(
                    error.kind,
                    CfgErrorKind::ContinueOutsideLoop | CfgErrorKind::BreakOutsideLoop
                ) => {}
            Err(DesugarError::UnsupportedDialect(DialectSupportError::ContinueNotSupported {
                ..
            })) => {}
            // A `repeat` loop whose `until` condition reads a body local cannot
            // be lowered without changing that condition's meaning.
            Err(DesugarError::Continue(_)) => {}
            Err(DesugarError::Cfg(error)) => {
                panic!("iteration {iteration} lowering reported invalid control flow: {error}")
            }
            Err(DesugarError::UnsupportedDialect(error)) => {
                panic!("iteration {iteration} lowering left unsupported syntax: {error}")
            }
            Err(DesugarError::Semantic(_)) => {}
        }
    }

    assert!(
        lowered_count > 0,
        "no fuzzed block was lowered; the test would be vacuous"
    );
}
