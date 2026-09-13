//! Tests for node-keyed semantic queries and their typed errors.

use r4::nodes::{Block, Identifier, Statement};
use r4::semantic::{SemanticModel, SemanticQueryError};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn first_do_block(block: &Block) -> &Block {
    for statement in block.iter_statements() {
        if let Statement::Do(do_statement) = statement {
            return do_statement.get_block();
        }
    }
    panic!("the test code must contain a do block");
}

#[test]
fn maps_blocks_to_their_scopes() {
    let (block, model) = analyze("do local x = 1 end\nif c then do end end");

    let chunk = model.try_scope_of(&block).expect("the chunk has a scope");
    assert_eq!(chunk, model.chunk_scope());
    assert_eq!(
        model.try_find_scope(&block, r4::semantic::ScopeKind::File),
        Ok(chunk)
    );

    let do_block = first_do_block(&block);
    let do_scope = model
        .try_scope_of(do_block)
        .expect("the do block has a scope");
    assert_ne!(do_scope, chunk);
    assert_eq!(
        model.try_find_scope(do_block, r4::semantic::ScopeKind::Block),
        Ok(do_scope)
    );
}

#[test]
fn missing_scope_kind_is_a_typed_error() {
    let (block, model) = analyze("local x = 1\nreturn x");
    let error = model
        .try_find_scope(&block, r4::semantic::ScopeKind::Block)
        .expect_err("the chunk has no block scope");
    assert!(matches!(error, SemanticQueryError::NoEnclosingScope { .. }));
    assert_eq!(error.code(), "query/no-enclosing-scope");
}

#[test]
fn foreign_models_are_rejected() {
    let (first_block, first_model) = analyze("local x = 1\nreturn x");
    let (second_block, _second_model) = analyze("local x = 1\nreturn x");

    let error = first_model
        .try_scope_of(&second_block)
        .expect_err("a block from another model must be rejected");
    assert!(matches!(error, SemanticQueryError::ForeignBlock { .. }));
    assert_eq!(error.code(), "query/foreign-block");

    // The same block in its own model is fine.
    assert!(first_model.try_scope_of(&first_block).is_ok());
}

#[test]
fn unanalyzed_blocks_and_identifiers_are_rejected() {
    let (block, model) = analyze("local x = 1\nreturn x");

    let fresh = Parser::default()
        .parse("return 1")
        .expect("the fresh block parses");
    assert!(matches!(
        model.try_scope_of(&fresh),
        Err(SemanticQueryError::UnidentifiedBlock)
    ));

    let identifier = Identifier::new("never_bound");
    assert!(matches!(
        model.try_declared_symbol(&identifier),
        Err(SemanticQueryError::UnboundIdentifier { .. })
    ));
    assert!(matches!(
        model.try_binding(&identifier),
        Err(SemanticQueryError::UnboundOccurrence { .. })
    ));

    let _ = block;
}

#[test]
fn resolves_declarations_and_their_scopes() {
    let (block, model) = analyze("local x = 1\nreturn x");

    let statement = block.iter_statements().next().expect("a statement");
    let Statement::LocalAssign(assignment) = statement else {
        panic!("expected a local assignment");
    };
    let identifier = assignment
        .iter_variables()
        .next()
        .expect("a variable")
        .get_identifier();

    let symbol = model
        .try_declared_symbol(identifier)
        .expect("the declaration is bound");
    assert_eq!(model.symbol(symbol).name, "x");
    assert_eq!(
        model.try_scope_of_identifier(identifier),
        Ok(model.symbol(symbol).declaring_scope)
    );
    assert!(matches!(
        model.try_binding(identifier),
        Ok(r4::semantic::Binding::Symbol(_))
    ));
}
