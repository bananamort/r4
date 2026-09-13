//! Tests for position-to-node navigation.

use r4::navigation::{NavigationError, NodeKind, SyntaxIndex};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

const CODE: &str = "local value = compute(input)\nreturn value + 1\n";

fn analyzed_index(code: &str) -> (SyntaxIndex, r4::nodes::Block) {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .expect("valid code");
    SemanticAnalyzer::new()
        .analyze(&block)
        .expect("analysis succeeds");
    let index = SyntaxIndex::build(&block).expect("the index builds");
    (index, block)
}

#[test]
fn finds_the_identifier_at_an_offset() {
    let (index, _block) = analyzed_index(CODE);

    for name in ["value", "compute", "input"] {
        let offset = CODE.find(name).expect("the name is present");
        let entry = index
            .identifier_at(offset)
            .unwrap_or_else(|| panic!("an identifier at `{name}` offset"));
        assert_eq!(entry.kind, NodeKind::Identifier);
        assert!(entry.binding.is_some(), "{name} is bound");
    }
}

#[test]
fn node_at_prefers_the_smallest_span() {
    let (index, _block) = analyzed_index(CODE);
    let offset = CODE.find("input").expect("input is present");

    let id = index.node_at(offset).expect("a node covers the offset");
    let entry = index.entry(id).expect("the node exists");
    assert_eq!(entry.kind, NodeKind::Identifier);
    assert_eq!(index.kind(id), Some(NodeKind::Identifier));

    let span = index.span(id).expect("the node has a span");
    assert_eq!(span.start, offset);
    assert!(span.end > span.start);
}

#[test]
fn ancestors_are_enclosing_nodes() {
    let (index, _block) = analyzed_index(CODE);
    let offset = CODE.find("input").expect("input is present");
    let id = index.node_at(offset).expect("a node");

    let ancestors = index.ancestors(id);
    assert!(!ancestors.is_empty());
    let kinds: Vec<NodeKind> = ancestors
        .iter()
        .filter_map(|ancestor| index.kind(*ancestor))
        .collect();
    assert!(kinds.contains(&NodeKind::Statement), "{kinds:?}");
    assert!(kinds.contains(&NodeKind::Block), "{kinds:?}");
    // Ancestors strictly contain the node.
    let span = index.span(id).unwrap();
    for ancestor in ancestors {
        let ancestor_span = index.span(ancestor).unwrap();
        assert!(ancestor_span.start <= span.start && ancestor_span.end >= span.end);
        assert!(ancestor_span.len() > span.len());
    }
}

#[test]
fn rejects_tokenless_trees() {
    let block = Parser::default().parse(CODE).expect("valid code");
    SemanticAnalyzer::new().analyze(&block).expect("analysis");
    assert_eq!(
        SyntaxIndex::build(&block).expect_err("tokens are required"),
        NavigationError::MissingTokens
    );
    assert_eq!(
        NavigationError::MissingTokens.code(),
        "navigation/missing-tokens"
    );
}

#[test]
fn rejects_unanalyzed_trees() {
    let block = Parser::default()
        .preserve_tokens()
        .parse(CODE)
        .expect("valid code");
    assert_eq!(
        SyntaxIndex::build(&block).expect_err("analysis is required"),
        NavigationError::UnanalyzedBlock
    );
}
