//! Regression tests for token-less semantic execution.
//!
//! Semantic passes resolve identifiers and statements through the structural
//! bindings recorded by the analyzer, so they must behave identically on ASTs
//! parsed with and without token preservation.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, Statement};
use r4::rules::rename_variables::rename_variables;
use r4::rules::{canonicalize, Session};
use r4::semantic::{SemanticAnalyzer, SemanticModel, SymbolKind};
use r4::Parser;

fn parse(code: &str, preserve_tokens: bool, lua52: bool) -> Block {
    let mut parser = Parser::default();
    if preserve_tokens {
        parser = parser.preserve_tokens();
    }
    if lua52 {
        parser = parser.lua52();
    }
    parser
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze(block: &Block) -> SemanticModel {
    SemanticAnalyzer::new()
        .analyze(block)
        .unwrap_or_else(|error| panic!("semantic analysis failed: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn pipeline(code: &str, preserve_tokens: bool) -> String {
    let mut block = parse(code, preserve_tokens, false);
    let mut model = analyze(&block);

    canonicalize::remove_unused_variables(&mut block, &model)
        .expect("unused variable removal must succeed");
    model = analyze(&block);

    canonicalize::remove_dead_stores(&mut block, &model).expect("dead store removal must succeed");
    model = analyze(&block);

    canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("unreachable code removal must succeed");
    model = analyze(&block);

    rename_variables(&mut block, &model).expect("renaming must succeed");

    generate(&block)
}

#[test]
fn semantic_pipeline_is_identical_without_tokens() {
    let cases = [
        "local unused = 1\nreturn 2",
        "local x\nx = 1\nx = 2\nreturn x",
        "local running = false\nwhile running do work() end\nafter()",
        "local x = 1\nprint(x)\nlocal x = 2\nreturn x",
        "local a = 1\nlocal b = a + 1\nreturn b",
    ];

    for code in cases {
        let with_tokens = pipeline(code, true);
        let without_tokens = pipeline(code, false);
        assert_eq!(
            with_tokens, without_tokens,
            "token-less output differs for `{code}`"
        );
    }
}

#[test]
fn same_scope_shadowing_renames_each_binding() {
    let code = "local x = 1\nprint(x)\nlocal x = 2\nprint(x)";
    let mut block = parse(code, false, false);
    let model = analyze(&block);

    let renamed = rename_variables(&mut block, &model).expect("renaming must succeed");
    assert_eq!(renamed, 2, "both locals must be renamed");

    let output = generate(&block);
    let reparsed = parse(&output, false, false);
    let new_model = analyze(&reparsed);

    let locals: Vec<_> = new_model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Local)
        .collect();
    assert_eq!(locals.len(), 2, "unexpected output: {output}");
    for symbol in locals {
        assert_eq!(
            symbol.read_count, 1,
            "each renamed local must be read exactly once: {output}"
        );
    }
    assert!(
        new_model
            .symbols
            .iter()
            .any(|symbol| symbol.kind == SymbolKind::Global && symbol.name == "print"),
        "the global call must keep its name: {output}"
    );
}

#[test]
fn label_removal_is_identical_without_tokens() {
    let cases = ["::unused::\nreturn 1", "::keep::\ngoto keep"];

    for code in cases {
        let mut with_tokens = parse(code, true, true);
        let model = analyze(&with_tokens);
        let removed_with_tokens =
            canonicalize::remove_unused_labels(&mut with_tokens, &model).expect("removal");

        let mut without_tokens = parse(code, false, true);
        let model = analyze(&without_tokens);
        let removed_without_tokens =
            canonicalize::remove_unused_labels(&mut without_tokens, &model).expect("removal");

        assert_eq!(removed_with_tokens, removed_without_tokens, "{code}");
        assert_eq!(generate(&with_tokens), generate(&without_tokens), "{code}");
    }
}

#[test]
fn session_parses_without_tokens_and_transforms() {
    let mut session =
        Session::parse("local unused = 1\nreturn 2").expect("session must parse and analyze");

    let Statement::LocalAssign(assignment) = session
        .block()
        .first_statement()
        .expect("the block must start with a local assignment")
    else {
        panic!("unexpected first statement");
    };
    assert!(
        assignment
            .iter_variables()
            .next()
            .expect("the assignment has a variable")
            .get_identifier()
            .get_token()
            .is_none(),
        "the session AST must not retain token data"
    );

    let removed = session
        .apply_semantic_result(canonicalize::remove_unused_variables)
        .expect("token-less removal must succeed");
    assert_eq!(removed, 1);

    let output = generate(session.block());
    assert!(!output.contains("unused"), "unexpected: {output}");
}
