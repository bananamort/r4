//! Tests for non-intrusive syntax annotations.

use r4::annotation::{AnnotationError, NodeAddress, SyntaxAnnotation, SyntaxAnnotations};
use r4::nodes::{BindingId, Block, BlockIdentity, Statement};
use r4::rules::Session;
use r4::Parser;

fn analyze(code: &str) -> (Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn identity(block: &Block) -> BlockIdentity {
    block.get_identity().expect("the block is analyzed")
}

fn first_declaration_binding(block: &Block) -> BindingId {
    for statement in block.iter_statements() {
        if let Statement::LocalAssign(assignment) = statement {
            return assignment
                .iter_variables()
                .next()
                .expect("a variable")
                .get_identifier()
                .get_binding()
                .expect("the declaration is bound");
        }
    }
    panic!("the test code must declare a local");
}

#[test]
fn validates_structural_addresses() {
    let (block, model) = analyze("local x = 1\nif x then return x end");
    let root = identity(&block);
    let binding = first_declaration_binding(&block);

    let mut annotations = SyntaxAnnotations::new();
    annotations.insert(NodeAddress::Block(root), SyntaxAnnotation::new("block"));
    annotations.insert(
        NodeAddress::Statement {
            block: root,
            index: 0,
        },
        SyntaxAnnotation::new("statement"),
    );
    annotations.insert(
        NodeAddress::Expression {
            block: root,
            statement: 0,
            index: 0,
        },
        SyntaxAnnotation::with_data("expression", "1"),
    );
    annotations.insert(
        NodeAddress::Condition {
            block: root,
            statement: 1,
        },
        SyntaxAnnotation::new("condition"),
    );
    annotations.insert(
        NodeAddress::Identifier(binding),
        SyntaxAnnotation::new("identifier"),
    );

    assert_eq!(annotations.len(), 5);
    annotations
        .validate(&block)
        .expect("every address resolves");

    let _ = model;
}

#[test]
fn rejects_stale_addresses_with_typed_errors() {
    let (block, _model) = analyze("local x = 1\nif x then return x end");
    let root = identity(&block);

    let cases = [
        (
            NodeAddress::Statement {
                block: r4::nodes::BlockIdentity(u32::MAX),
                index: 0,
            },
            "annotation/unknown-block",
        ),
        (
            NodeAddress::Statement {
                block: root,
                index: 99,
            },
            "annotation/unknown-statement",
        ),
        (
            NodeAddress::Expression {
                block: root,
                statement: 0,
                index: 99,
            },
            "annotation/unknown-expression",
        ),
        (
            NodeAddress::Condition {
                block: root,
                statement: 0,
            },
            "annotation/missing-condition",
        ),
        (
            NodeAddress::Identifier(BindingId::symbol(9999)),
            "annotation/unknown-identifier",
        ),
    ];

    for (address, expected) in cases {
        let mut annotations = SyntaxAnnotations::new();
        annotations.insert(address, SyntaxAnnotation::new("tag"));
        let error = annotations
            .validate(&block)
            .expect_err("the address is stale");
        assert_eq!(error.code(), expected, "{error}");
    }
}

#[test]
fn manages_annotations() {
    let (block, _model) = analyze("local x = 1");
    let root = identity(&block);
    let address = NodeAddress::Block(root);

    let mut annotations = SyntaxAnnotations::new();
    assert!(annotations.is_empty());
    annotations.insert(address.clone(), SyntaxAnnotation::new("a"));
    annotations.insert(address.clone(), SyntaxAnnotation::with_data("b", "payload"));
    assert!(annotations.contains(&address));
    assert_eq!(annotations.get(&address).len(), 2);
    assert_eq!(annotations.iter().count(), 1);
    let removed = annotations.remove(&address);
    assert_eq!(removed.len(), 2);
    assert!(annotations.is_empty());
    assert!(annotations.get(&address).is_empty());
}

#[test]
fn annotation_codes_are_cataloged_and_diagnostic_ready() {
    for code in [
        "annotation/unknown-block",
        "annotation/unknown-statement",
        "annotation/unknown-expression",
        "annotation/missing-condition",
        "annotation/unknown-identifier",
    ] {
        assert!(
            r4::descriptor_for(code).is_some(),
            "missing descriptor {code}"
        );
    }

    let error = AnnotationError::UnknownIdentifier {
        binding: BindingId::symbol(7),
    };
    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.code, Some("annotation/unknown-identifier"));
    r4::DiagnosticFormatter::new("return 0")
        .render(&diagnostic)
        .expect("the diagnostic renders");
}

#[test]
fn session_validates_annotations_against_the_current_tree() {
    let mut session = Session::parse("local x = 1\nreturn x").expect("valid code");
    let root = identity(session.block());
    session.annotate(
        NodeAddress::Statement {
            block: root,
            index: 0,
        },
        SyntaxAnnotation::new("keep"),
    );

    session
        .validate_annotations()
        .expect("the statement exists");

    // Removing every statement makes the address stale; validation must fail
    // with a typed error, never silently drop the annotation.
    session.apply(|block| {
        block.take_statements();
    });
    let error = session
        .validate_annotations()
        .expect_err("the statement no longer exists");
    assert!(matches!(
        error,
        r4::rules::SessionError::Pass(AnnotationError::UnknownStatement { .. })
    ));
    assert!(session.annotations().contains(&NodeAddress::Statement {
        block: root,
        index: 0,
    }));
}
