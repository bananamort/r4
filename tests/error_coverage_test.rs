//! Enforced typed-failure coverage (core analysis errors).
//!
//! Each error enum is mapped to a tag enum with an exhaustive `of` match, and a
//! trigger must actually produce every registered variant. A variant that no
//! input can reach fails the coverage assertion, forcing it to be either given
//! a fail-fast input or removed as dead.

use std::collections::BTreeSet;

use r4::cfg::{build_chunk, CfgError, CfgErrorKind};
use r4::nodes::{Block, BlockIdentity, GotoStatement, Identifier, Statement, TypedIdentifier};
use r4::semantic::{
    ScopeKind, SemanticAnalyzer, SemanticErrorKind, SemanticModel, SemanticQueryError,
};
use r4::Parser;

macro_rules! variant_registry {
    ($tag:ident, $ast:ty, $( $variant:ident => $pattern:pat ),* $(,)?) => {
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
        enum $tag { $( $variant ),* }

        impl $tag {
            const ALL: &'static [$tag] = &[ $( $tag::$variant ),* ];

            fn name(self) -> &'static str {
                match self { $( $tag::$variant => stringify!($variant) ),* }
            }

            fn of(value: &$ast) -> Self {
                match value { $( $pattern => $tag::$variant ),* }
            }
        }
    };
}

variant_registry!(SemanticErrorTag, SemanticErrorKind,
    UndeclaredSymbol => SemanticErrorKind::UndeclaredSymbol { .. },
    DuplicateLabel => SemanticErrorKind::DuplicateLabel { .. },
    UndefinedLabel => SemanticErrorKind::UndefinedLabel { .. },
    GotoIntoLocalScope => SemanticErrorKind::GotoIntoLocalScope { .. },
    UnresolvedType => SemanticErrorKind::UnresolvedType { .. },
);

variant_registry!(QueryErrorTag, SemanticQueryError,
    UnidentifiedBlock => SemanticQueryError::UnidentifiedBlock,
    UnknownBlock => SemanticQueryError::UnknownBlock { .. },
    ForeignBlock => SemanticQueryError::ForeignBlock { .. },
    NoEnclosingScope => SemanticQueryError::NoEnclosingScope { .. },
    UnboundIdentifier => SemanticQueryError::UnboundIdentifier { .. },
    UnboundLabel => SemanticQueryError::UnboundLabel { .. },
    UnboundType => SemanticQueryError::UnboundType { .. },
    UnboundOccurrence => SemanticQueryError::UnboundOccurrence { .. },
);

variant_registry!(CfgErrorTag, CfgErrorKind,
    BreakOutsideLoop => CfgErrorKind::BreakOutsideLoop,
    ContinueOutsideLoop => CfgErrorKind::ContinueOutsideLoop,
    MissingSemanticRecord => CfgErrorKind::MissingSemanticRecord { .. },
    SemanticMismatch => CfgErrorKind::SemanticMismatch { .. },
    UnresolvedGoto => CfgErrorKind::UnresolvedGoto { .. },
    MissingBlockIdentity => CfgErrorKind::MissingBlockIdentity,
    UnresolvedIdentifier => CfgErrorKind::UnresolvedIdentifier { .. },
);

fn luau(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn lua52(code: &str) -> Block {
    Parser::default()
        .lua52()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn analyze(block: &Block) -> SemanticModel {
    SemanticAnalyzer::new()
        .analyze(block)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"))
}

fn missing(
    tags: impl IntoIterator<Item = &'static str>,
    produced: &BTreeSet<&'static str>,
) -> Vec<&'static str> {
    let mut absent: Vec<_> = tags
        .into_iter()
        .filter(|tag| !produced.contains(tag))
        .collect();
    absent.sort_unstable();
    absent
}

#[test]
fn semantic_errors_are_all_triggered() {
    let errors = vec![
        SemanticAnalyzer::new()
            .strict_globals()
            .analyze(&luau("return unknown_name"))
            .expect_err("strict globals rejects the unknown name"),
        SemanticAnalyzer::new()
            .analyze(&lua52("::a::\n::a::"))
            .expect_err("a duplicate label is rejected"),
        SemanticAnalyzer::new()
            .analyze(&lua52("goto nowhere"))
            .expect_err("an undefined label is rejected"),
        SemanticAnalyzer::new()
            .analyze(&lua52("goto skip\nlocal x = 1\n::skip::\nreturn x"))
            .expect_err("goto into a local scope is rejected"),
        SemanticAnalyzer::new()
            .strict_types()
            .analyze(&luau("type A = Missing"))
            .expect_err("strict types rejects the undeclared type"),
    ];

    let produced: BTreeSet<&'static str> = errors
        .iter()
        .map(|error| SemanticErrorTag::of(&error.kind).name())
        .collect();
    let absent = missing(
        SemanticErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "semantic errors never triggered: {absent:?}"
    );

    for error in &errors {
        assert!(!error.code().is_empty(), "{:?} has no code", error.kind);
        assert!(
            !error.to_string().is_empty(),
            "{:?} has no message",
            error.kind
        );
    }
}

#[test]
fn query_errors_are_all_triggered() {
    let block = luau("local x = 1\nreturn x");
    let model = analyze(&block);
    let never = Identifier::new("never");

    let restamped = luau("return 1");
    let restamped_model = analyze(&restamped);
    restamped.set_identity(BlockIdentity(9_999));

    let foreign_block = luau("return 1");
    let _foreign_model = analyze(&foreign_block);

    let errors = vec![
        model
            .try_scope_of(&luau("return 1"))
            .expect_err("an unanalyzed block has no scope"),
        restamped_model
            .try_scope_of(&restamped)
            .expect_err("an unknown identity is rejected"),
        model
            .try_scope_of(&foreign_block)
            .expect_err("a block from another analysis is rejected"),
        model
            .try_find_scope(&block, ScopeKind::Block)
            .expect_err("the chunk has no block scope"),
        model
            .try_declared_symbol(&never)
            .expect_err("the identifier is undeclared"),
        model
            .try_declared_label(&never)
            .expect_err("the label is undeclared"),
        model
            .try_declared_type(&never)
            .expect_err("the type is undeclared"),
        model
            .try_binding(&never)
            .expect_err("the occurrence is unbound"),
    ];

    let produced: BTreeSet<&'static str> = errors
        .iter()
        .map(|error| QueryErrorTag::of(error).name())
        .collect();
    let absent = missing(QueryErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "query errors never triggered: {absent:?}"
    );

    for error in &errors {
        assert!(!error.code().is_empty(), "{error:?} has no code");
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }
}

#[test]
fn cfg_errors_are_all_triggered() {
    let break_block = luau("break");
    let break_model = analyze(&break_block);

    let continue_block = luau("continue");
    let continue_model = analyze(&continue_block);

    let analyzed = luau("return 1");
    let model = analyze(&analyzed);

    let mut mismatch = lua52("::a::\ngoto a");
    let mismatch_model = analyze(&mismatch);
    for statement in mismatch.iter_mut_statements() {
        if let Statement::Goto(goto_statement) = statement {
            goto_statement.mutate_label().set_name("b".to_owned());
        }
    }

    let mut unlabeled = lua52("::a::\ngoto a");
    let unlabeled_model = analyze(&unlabeled);
    unlabeled.remove_statement(0);

    // The model has no goto record for the statement added after analysis.
    let mut extra_goto = lua52("::a::");
    let extra_goto_model = analyze(&extra_goto);
    extra_goto.push_statement(Statement::Goto(GotoStatement::from_name("b")));

    let mut loop_header = luau("for i = 1, 10 do end");
    let loop_model = analyze(&loop_header);
    for statement in loop_header.iter_mut_statements() {
        if let Statement::NumericFor(numeric_for) = statement {
            *numeric_for.mutate_identifier() = TypedIdentifier::new("never_bound");
        }
    }

    let errors: Vec<CfgError> = vec![
        build_chunk(&break_block, &break_model).expect_err("break outside a loop is rejected"),
        build_chunk(&continue_block, &continue_model)
            .expect_err("continue outside a loop is rejected"),
        build_chunk(&luau("return 1"), &model).expect_err("an unanalyzed block is rejected"),
        build_chunk(&mismatch, &mismatch_model).expect_err("a renamed goto no longer matches"),
        build_chunk(&unlabeled, &unlabeled_model).expect_err("a deleted label record is missing"),
        build_chunk(&extra_goto, &extra_goto_model)
            .expect_err("the added goto has no semantic record"),
        build_chunk(&loop_header, &loop_model).expect_err("the iteration variable is unresolved"),
    ];

    let produced: BTreeSet<&'static str> = errors
        .iter()
        .map(|error| CfgErrorTag::of(&error.kind).name())
        .collect();
    let absent = missing(CfgErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(absent.is_empty(), "cfg errors never triggered: {absent:?}");

    for error in &errors {
        assert!(
            !error.to_string().is_empty(),
            "{:?} has no message",
            error.kind
        );
        if let Some(line) = error.line {
            assert!(line >= 1, "{:?} has a zero line", error.kind);
        }
    }
}
