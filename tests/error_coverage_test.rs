//! Enforced typed-failure coverage (core analysis errors).
//!
//! Each error enum is mapped to a tag enum with an exhaustive `of` match, and a
//! trigger must actually produce every registered variant. A variant that no
//! input can reach fails the coverage assertion, forcing it to be either given
//! a fail-fast input or removed as dead.

use std::collections::BTreeSet;

use r4::cfg::{build_chunk, CfgError, CfgErrorKind};
use r4::nodes::{Block, BlockIdentity, GotoStatement, Identifier, Statement, TypedIdentifier};
use r4::rules::rename_variables::{rename_symbol, RenameSymbolError};
use r4::semantic::{
    ScopeId, ScopeKind, SemanticAnalyzer, SemanticErrorKind, SemanticModel, SemanticQueryError,
    SymbolId, SymbolKind,
};
use r4::Parser;

macro_rules! variant_registry {
    ($tag:ident, $ast:ty, $( $variant:ident => $pattern:pat ),* $(,)?) => {
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
        #[allow(clippy::enum_variant_names)]
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
    UnknownSymbol => SemanticQueryError::UnknownSymbol { .. },
    UnknownScope => SemanticQueryError::UnknownScope { .. },
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

variant_registry!(RenameSymbolErrorTag, RenameSymbolError,
    UnsupportedIdentifier => RenameSymbolError::UnsupportedIdentifier { .. },
    UnknownSymbol => RenameSymbolError::UnknownSymbol,
    ImplicitSymbol => RenameSymbolError::ImplicitSymbol,
    Conflict => RenameSymbolError::Conflict { .. },
    UnresolvedIdentifier => RenameSymbolError::UnresolvedIdentifier { .. },
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
        model
            .try_can_be_accessed_in(SymbolId(9_999), model.chunk_scope())
            .expect_err("an unknown symbol is rejected"),
        model
            .try_find_variable(ScopeId(9_999), "x", ScopeKind::Block)
            .expect_err("an unknown scope is rejected"),
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

variant_registry!(NavigationErrorTag, r4::navigation::NavigationError,
    MissingTokens => r4::navigation::NavigationError::MissingTokens,
    UnanalyzedBlock => r4::navigation::NavigationError::UnanalyzedBlock,
);

variant_registry!(AnnotationErrorTag, r4::annotation::AnnotationError,
    UnknownBlock => r4::annotation::AnnotationError::UnknownBlock { .. },
    UnknownStatement => r4::annotation::AnnotationError::UnknownStatement { .. },
    UnknownExpression => r4::annotation::AnnotationError::UnknownExpression { .. },
    MissingCondition => r4::annotation::AnnotationError::MissingCondition { .. },
    UnknownIdentifier => r4::annotation::AnnotationError::UnknownIdentifier { .. },
);

variant_registry!(DirectiveErrorTag, r4::semantic::DirectiveError,
    InvalidRange => r4::semantic::DirectiveError::InvalidRange { .. },
    MissingName => r4::semantic::DirectiveError::MissingName,
    ConflictingStrictness => r4::semantic::DirectiveError::ConflictingStrictness { .. },
);

variant_registry!(DiagnosticFormatErrorTag, r4::diagnostic::DiagnosticFormatError,
    UnknownCode => r4::diagnostic::DiagnosticFormatError::UnknownCode { .. },
    InvalidSpan => r4::diagnostic::DiagnosticFormatError::InvalidSpan { .. },
    InvalidLocation => r4::diagnostic::DiagnosticFormatError::InvalidLocation { .. },
);

variant_registry!(DiagnosticQueryErrorTag, r4::diagnostic::DiagnosticQueryError,
    MissingSpan => r4::diagnostic::DiagnosticQueryError::MissingSpan,
);

variant_registry!(TriviaErrorTag, r4::nodes::TriviaError,
    InvalidRange => r4::nodes::TriviaError::InvalidRange { .. },
);

fn annotation_error(
    block: &Block,
    address: r4::annotation::NodeAddress,
) -> r4::annotation::AnnotationError {
    let mut annotations = r4::annotation::SyntaxAnnotations::new();
    annotations.insert(address, r4::annotation::SyntaxAnnotation::new("x"));
    annotations
        .validate(block)
        .expect_err("the address must not resolve")
}

#[test]
fn api_errors_are_all_triggered() {
    use r4::annotation::NodeAddress;
    use r4::diagnostic::{
        Diagnostic, DiagnosticFormatter, DiagnosticQueryError, LinePosition, LinePositionSpan,
        TextSpan,
    };
    use r4::navigation::SyntaxIndex;
    use r4::semantic::{Directive, Directives};
    use r4::Compilation;

    let tokenless = luau("return 1");
    let _ = analyze(&tokenless);
    let tokenful = Parser::default()
        .preserve_tokens()
        .parse("return 1")
        .expect("valid code");
    let navigation_errors = vec![
        SyntaxIndex::build(&tokenless).expect_err("tokens are required"),
        SyntaxIndex::build(&tokenful).expect_err("analysis is required"),
    ];
    let produced: BTreeSet<&'static str> = navigation_errors
        .iter()
        .map(|error| NavigationErrorTag::of(error).name())
        .collect();
    let absent = missing(
        NavigationErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "navigation errors never triggered: {absent:?}"
    );
    for error in &navigation_errors {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }

    let block = luau("local x = 1\nreturn x");
    let _ = analyze(&block);
    let id = block.get_identity().expect("the block is analyzed");
    let annotation_errors = vec![
        annotation_error(&block, NodeAddress::Block(BlockIdentity(9_999))),
        annotation_error(
            &block,
            NodeAddress::Statement {
                block: id,
                index: 99,
            },
        ),
        annotation_error(
            &block,
            NodeAddress::Expression {
                block: id,
                statement: 0,
                index: 99,
            },
        ),
        annotation_error(
            &block,
            NodeAddress::Condition {
                block: id,
                statement: 0,
            },
        ),
        annotation_error(
            &block,
            NodeAddress::Identifier(r4::nodes::BindingId::symbol(9_999)),
        ),
    ];
    let produced: BTreeSet<&'static str> = annotation_errors
        .iter()
        .map(|error| AnnotationErrorTag::of(error).name())
        .collect();
    let absent = missing(
        AnnotationErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "annotation errors never triggered: {absent:?}"
    );
    for error in &annotation_errors {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }

    let directive_source = "--!strict\n--!nonstrict\nreturn 1";
    let directive_block = Parser::default()
        .preserve_tokens()
        .parse(directive_source)
        .expect("valid code");
    let directive_errors = vec![
        Directive::parse("--!").expect_err("a nameless directive is rejected"),
        Directives::from_block(&directive_block, directive_source)
            .expect_err("conflicting strictness is rejected"),
        r4::nodes::TriviaError::InvalidRange {
            start: 0,
            end: 1,
            length: 0,
        }
        .into(),
    ];
    let produced: BTreeSet<&'static str> = directive_errors
        .iter()
        .map(|error| DirectiveErrorTag::of(error).name())
        .collect();
    let absent = missing(
        DirectiveErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "directive errors never triggered: {absent:?}"
    );
    for error in &directive_errors {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }

    let source = "return 1";
    let format_errors = vec![
        DiagnosticFormatter::new(source)
            .render(&Diagnostic::error("x").with_code("bogus/code"))
            .expect_err("an uncataloged code is rejected"),
        DiagnosticFormatter::new(source)
            .render(
                &Diagnostic::error("x")
                    .with_code("semantic/undeclared-symbol")
                    .with_span(TextSpan::new(1000, 1001)),
            )
            .expect_err("a span outside the source is rejected"),
        DiagnosticFormatter::new(source)
            .render(
                &Diagnostic::error("x")
                    .with_code("semantic/undeclared-symbol")
                    .with_location(LinePositionSpan::new(
                        LinePosition::new(99, 1),
                        LinePosition::new(99, 2),
                    )),
            )
            .expect_err("a location outside the source is rejected"),
    ];
    let produced: BTreeSet<&'static str> = format_errors
        .iter()
        .map(|error| DiagnosticFormatErrorTag::of(error).name())
        .collect();
    let absent = missing(
        DiagnosticFormatErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "diagnostic format errors never triggered: {absent:?}"
    );
    for error in &format_errors {
        assert!(!error.to_string().is_empty());
    }

    let mut compilation = Compilation::new();
    compilation.add_tree("return 1").expect("valid code");
    let query_error = compilation
        .diagnostics_for_token(&r4::nodes::Token::from_content("x"))
        .expect_err("a token without a span cannot be queried");
    assert_eq!(DiagnosticQueryErrorTag::ALL.len(), 1);
    assert_eq!(
        DiagnosticQueryErrorTag::of(&query_error).name(),
        DiagnosticQueryErrorTag::MissingSpan.name()
    );
    assert_eq!(query_error.code(), DiagnosticQueryError::MissingSpan.code());

    let tokenful = Parser::default()
        .preserve_tokens()
        .parse("-- comment\nreturn 1")
        .expect("valid code");
    let comment = r4::process::collect_trivia(&tokenful)
        .into_iter()
        .find(|piece| piece.kind().is_comment())
        .expect("a comment");
    let trivia_error = comment
        .try_read_with("--")
        .expect_err("a short source is rejected");
    assert_eq!(TriviaErrorTag::ALL.len(), 1);
    assert_eq!(
        TriviaErrorTag::of(&trivia_error).name(),
        TriviaErrorTag::InvalidRange.name()
    );
    assert!(!trivia_error.to_string().is_empty());
}

variant_registry!(RenameErrorTag, r4::rules::rename_variables::RenameError,
    UnresolvedIdentifier => r4::rules::rename_variables::RenameError::UnresolvedIdentifier { .. },
);

variant_registry!(UnusedVariableErrorTag, r4::rules::canonicalize::UnusedVariableError,
    UnresolvedDeclaration => r4::rules::canonicalize::UnusedVariableError::UnresolvedDeclaration { .. },
);

variant_registry!(UnusedLabelErrorTag, r4::rules::canonicalize::UnusedLabelError,
    UnresolvedLabel => r4::rules::canonicalize::UnusedLabelError::UnresolvedLabel { .. },
);

variant_registry!(ConvertLocalFunctionErrorTag, r4::rules::desugar::ConvertLocalFunctionError,
    UnresolvedFunction => r4::rules::desugar::ConvertLocalFunctionError::UnresolvedFunction { .. },
);

#[test]
fn rule_errors_are_all_triggered() {
    use r4::nodes::{Expression, FunctionAssignment, FunctionCall, VariableAssignment};
    use r4::rules::rename_variables::rename_variables;
    use r4::rules::{canonicalize, desugar};

    // The model is analyzed, but the target blocks were never analyzed against
    // it, so their identifiers are unbound: a malformed state the passes must
    // reject rather than guess.
    let model = analyze(&luau("return 1"));

    let mut rename_target = Block::new(
        vec![Statement::Call(FunctionCall::from_name("never"))],
        None,
    );
    let rename_error = rename_variables(&mut rename_target, &model)
        .expect_err("renaming an unbound identifier is rejected");
    let produced = BTreeSet::from([RenameErrorTag::of(&rename_error).name()]);
    let absent = missing(RenameErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "rename errors never triggered: {absent:?}"
    );
    assert!(!rename_error.to_string().is_empty());

    let mut unused_target = Block::new(
        vec![Statement::LocalAssign(VariableAssignment::new(
            vec![TypedIdentifier::new("never")],
            vec![Expression::from(1)],
        ))],
        None,
    );
    let unused_error = canonicalize::remove_unused_variables(&mut unused_target, &model)
        .expect_err("an unbound declaration is rejected");
    let produced = BTreeSet::from([UnusedVariableErrorTag::of(&unused_error).name()]);
    let absent = missing(
        UnusedVariableErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "unused-variable errors never triggered: {absent:?}"
    );
    assert!(!unused_error.to_string().is_empty());

    // The model records an unused label, but the tree's label no longer matches
    // it, so validation must fail instead of rewriting.
    let mut label_target = lua52("::a::");
    let label_model = analyze(&label_target);
    for statement in label_target.iter_mut_statements() {
        if let Statement::Label(label) = statement {
            label.mutate_label().set_name("b".to_owned());
        }
    }
    let label_error = canonicalize::remove_unused_labels(&mut label_target, &label_model)
        .expect_err("an unbound label is rejected");
    let produced = BTreeSet::from([UnusedLabelErrorTag::of(&label_error).name()]);
    let absent = missing(
        UnusedLabelErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "unused-label errors never triggered: {absent:?}"
    );
    assert!(!label_error.to_string().is_empty());

    let mut local_function_target = Block::new(
        vec![Statement::LocalFunction(Box::new(
            FunctionAssignment::from_name("never", Block::default()),
        ))],
        None,
    );
    let convert_error =
        desugar::convert_local_function_to_assignment(&mut local_function_target, &model)
            .expect_err("an unbound local function is rejected");
    let produced = BTreeSet::from([ConvertLocalFunctionErrorTag::of(&convert_error).name()]);
    let absent = missing(
        ConvertLocalFunctionErrorTag::ALL
            .iter()
            .map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "convert-local-function errors never triggered: {absent:?}"
    );
    assert!(!convert_error.to_string().is_empty());
}

variant_registry!(CompilationErrorTag, r4::CompilationError,
    UnknownTree => r4::CompilationError::UnknownTree(_),
    StaleTree => r4::CompilationError::StaleTree(_),
);

variant_registry!(SessionErrorTag, r4::rules::SessionError<r4::annotation::AnnotationError>,
    Parse => r4::rules::SessionError::Parse(_),
    Semantic => r4::rules::SessionError::Semantic(_),
    Pass => r4::rules::SessionError::Pass(_),
);

variant_registry!(AnalyzedParseErrorTag, r4::AnalyzedParseError,
    Parse => r4::AnalyzedParseError::Parse(_),
    Semantic => r4::AnalyzedParseError::Semantic(_),
);

variant_registry!(InjectGlobalValueErrorTag, r4::rules::InjectGlobalValueError,
    UnrepresentableValue => r4::rules::InjectGlobalValueError::UnrepresentableValue { .. },
);

variant_registry!(ConvertRequireErrorTag, r4::rules::require::ConvertRequireError,
    NonLiteralRequireArgument => r4::rules::require::ConvertRequireError::NonLiteralRequireArgument { .. },
);

#[test]
fn compilation_session_parser_inject_errors_are_all_triggered() {
    use std::collections::HashMap;

    use r4::annotation::AnnotationError;
    use r4::nodes::Expression;
    use r4::process::LuaValue;
    use r4::rules::require::{convert_require, RequirePathConverter};
    use r4::rules::{inject_global_values, Session};
    use r4::{Compilation, TreeId};

    let mut compilation = Compilation::new();
    let tree = compilation.add_tree("return 1").expect("valid code");
    let compilation_errors = [
        compilation
            .model(TreeId(9_999))
            .expect_err("an unknown tree id is rejected"),
        {
            compilation
                .block_mut(tree)
                .expect("tree exists")
                .take_last_statement();
            compilation
                .model(tree)
                .expect_err("a mutated tree's model is stale")
        },
    ];
    let produced: BTreeSet<&'static str> = compilation_errors
        .iter()
        .map(|error| CompilationErrorTag::of(error).name())
        .collect();
    let absent = missing(
        CompilationErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "compilation errors never triggered: {absent:?}"
    );

    let parse_error = Parser::default()
        .parse("local x = ")
        .expect_err("a syntax error");
    let semantic_block = Parser::default()
        .lua52()
        .parse("goto nowhere")
        .expect("the chunk parses");
    let semantic_error = SemanticAnalyzer::new()
        .analyze(&semantic_block)
        .expect_err("a semantic error");
    let mut session = Session::parse("return 1").expect("valid code");
    let session_errors: Vec<r4::rules::SessionError<AnnotationError>> = vec![
        r4::rules::SessionError::Parse(parse_error),
        r4::rules::SessionError::Semantic(semantic_error),
        session
            .apply_semantic_result(|_, _| {
                Err::<usize, AnnotationError>(AnnotationError::UnknownBlock {
                    identity: BlockIdentity(0),
                })
            })
            .expect_err("the pass fails"),
    ];
    let produced: BTreeSet<&'static str> = session_errors
        .iter()
        .map(|error| SessionErrorTag::of(error).name())
        .collect();
    let absent = missing(SessionErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "session errors never triggered: {absent:?}"
    );
    for error in &session_errors {
        assert!(!error.to_string().is_empty());
    }

    let analyzed_errors = [
        Parser::default()
            .parse_analyzed("local x = ")
            .expect_err("a syntax error"),
        Parser::default()
            .lua52()
            .parse_analyzed("goto nowhere")
            .expect_err("a semantic error"),
    ];
    let produced: BTreeSet<&'static str> = analyzed_errors
        .iter()
        .map(|error| AnalyzedParseErrorTag::of(error).name())
        .collect();
    let absent = missing(
        AnalyzedParseErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "analyzed parse errors never triggered: {absent:?}"
    );

    let mut inject_block = luau("return math.pi");
    let inject_model = analyze(&inject_block);
    let inject_error = inject_global_values(
        &mut inject_block,
        &inject_model,
        &HashMap::from([("math".to_owned(), LuaValue::Number(f64::INFINITY))]),
    )
    .expect_err("an unrepresentable value is rejected");
    assert_eq!(InjectGlobalValueErrorTag::ALL.len(), 1);
    assert_eq!(
        InjectGlobalValueErrorTag::of(&inject_error).name(),
        InjectGlobalValueErrorTag::UnrepresentableValue.name()
    );
    assert!(!inject_error.to_string().is_empty());

    struct Echo;
    impl RequirePathConverter for Echo {
        fn convert(&self, path: &str) -> Option<Expression> {
            Some(Expression::String(r4::nodes::StringExpression::from_value(
                path,
            )))
        }
    }

    let mut require_block = luau("return require(name)");
    let require_model = analyze(&require_block);
    let convert_error = convert_require(&mut require_block, &require_model, &Echo)
        .expect_err("a non-literal require argument is rejected");
    assert_eq!(ConvertRequireErrorTag::ALL.len(), 1);
    assert_eq!(
        ConvertRequireErrorTag::of(&convert_error).name(),
        ConvertRequireErrorTag::NonLiteralRequireArgument.name()
    );
    assert!(!convert_error.to_string().is_empty());
}

struct FailingProvider;

impl r4::modules::ModuleProvider for FailingProvider {
    fn load(&self, key: &str) -> Result<Option<String>, r4::modules::ModuleLoadError> {
        Err(r4::modules::ModuleLoadError::Read {
            key: key.to_owned(),
            message: "disk error".to_owned(),
        })
    }
}

variant_registry!(ModuleLoadErrorTag, r4::modules::ModuleLoadError,
    Read => r4::modules::ModuleLoadError::Read { .. },
);

variant_registry!(ResolveErrorTag, r4::modules::ResolveError,
    MissingModule => r4::modules::ResolveError::MissingModule { .. },
    ModuleLoad => r4::modules::ResolveError::ModuleLoad { .. },
    CyclicRequire => r4::modules::ResolveError::CyclicRequire { .. },
    Parse => r4::modules::ResolveError::Parse { .. },
    Analysis => r4::modules::ResolveError::Analysis { .. },
    NonLiteralRequire => r4::modules::ResolveError::NonLiteralRequire { .. },
    Interface => r4::modules::ResolveError::Interface { .. },
);

variant_registry!(ModuleErrorTag, r4::semantic::ModuleError,
    UnboundExport => r4::semantic::ModuleError::UnboundExport { .. },
    UnknownExport => r4::semantic::ModuleError::UnknownExport { .. },
    UnknownImportAlias => r4::semantic::ModuleError::UnknownImportAlias { .. },
    UnknownImportedType => r4::semantic::ModuleError::UnknownImportedType { .. },
    AmbiguousImportedType => r4::semantic::ModuleError::AmbiguousImportedType { .. },
);

variant_registry!(BundleErrorTag, r4::rules::require::BundleError,
    MissingModule => r4::rules::require::BundleError::MissingModule { .. },
    CyclicRequire => r4::rules::require::BundleError::CyclicRequire { .. },
    Parse => r4::rules::require::BundleError::Parse { .. },
    Analysis => r4::rules::require::BundleError::Analysis { .. },
    NonLiteralRequire => r4::rules::require::BundleError::NonLiteralRequire { .. },
    DuplicateExportedType => r4::rules::require::BundleError::DuplicateExportedType { .. },
    UnimportedType => r4::rules::require::BundleError::UnimportedType { .. },
    ModuleLoad => r4::rules::require::BundleError::ModuleLoad { .. },
    Module => r4::rules::require::BundleError::Module(_),
);

variant_registry!(AliasErrorTag, r4::rules::require::AliasError,
    MalformedAlias => r4::rules::require::AliasError::MalformedAlias { .. },
    DuplicateAlias => r4::rules::require::AliasError::DuplicateAlias { .. },
    NestedAlias => r4::rules::require::AliasError::NestedAlias { .. },
    UnresolvedAlias => r4::rules::require::AliasError::UnresolvedAlias { .. },
    Convert => r4::rules::require::AliasError::Convert(_),
);

fn symbol_named(model: &SemanticModel, name: &str) -> r4::semantic::SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("symbol `{name}` must exist"))
        .id
}

#[test]
fn module_and_require_errors_are_all_triggered() {
    use r4::modules::{MemoryModuleProvider, ModuleProvider, ModuleResolver};
    use r4::rules::require::{apply_path_aliases, bundle_modules, AliasRequireConverter};
    use r4::semantic::MultiModuleModel;

    fn resolve<P: ModuleProvider>(
        root: &str,
        provider: &P,
    ) -> Result<r4::modules::ResolvedModules, r4::modules::ResolveError> {
        let block = luau(root);
        let model = analyze(&block);
        ModuleResolver::new(provider).resolve(block, model, "<root>")
    }

    let empty = MemoryModuleProvider::new();
    let missing_error = resolve("return require('./missing')", &empty).expect_err("missing");
    let load_error = resolve("return require('./a')", &FailingProvider).expect_err("load");

    let mut cyclic = MemoryModuleProvider::new();
    cyclic.insert("a", "local b = require('./b')\nreturn b");
    cyclic.insert("b", "local a = require('./a')\nreturn a");
    let cycle_error = resolve("return require('./a')", &cyclic).expect_err("cycle");

    let mut bad_parse = MemoryModuleProvider::new();
    bad_parse.insert("a", "local x = ");
    let parse_error = resolve("return require('./a')", &bad_parse).expect_err("parse");

    let mut non_literal = MemoryModuleProvider::new();
    non_literal.insert("a", "return require(value)");
    let non_literal_error =
        resolve("return require('./a')", &non_literal).expect_err("non-literal");

    let mut strict_module = MemoryModuleProvider::new();
    strict_module.insert("a", "type A = Missing\nreturn 1");
    let strict_root = luau("return require('./a')");
    let strict_root_model = analyze(&strict_root);
    let analysis_error = ModuleResolver::new(&strict_module)
        .with_semantic_options(r4::semantic::SemanticOptions {
            strict_types: true,
            ..Default::default()
        })
        .resolve(strict_root, strict_root_model, "<root>")
        .expect_err("strict types rejects the module");

    // A root block the model does not describe makes interface extraction fail.
    let interface_error = ModuleResolver::new(&empty)
        .resolve(luau("return opaque"), analyze(&luau("return 1")), "<root>")
        .expect_err("the unbound export is rejected");

    let resolve_errors = vec![
        missing_error,
        load_error,
        cycle_error,
        parse_error,
        non_literal_error,
        analysis_error,
        interface_error,
    ];
    let produced: BTreeSet<&'static str> = resolve_errors
        .iter()
        .map(|error| ResolveErrorTag::of(error).name())
        .collect();
    let absent = missing(ResolveErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "resolve errors never triggered: {absent:?}"
    );
    for error in &resolve_errors {
        assert!(!error.code().is_empty(), "{error:?} has no code");
        assert!(!error.to_string().is_empty());
    }

    let r4::modules::ResolveError::ModuleLoad { error } = &resolve_errors[1] else {
        panic!("expected a module load error");
    };
    assert_eq!(ModuleLoadErrorTag::ALL.len(), 1);
    assert_eq!(
        ModuleLoadErrorTag::of(error).name(),
        ModuleLoadErrorTag::Read.name()
    );

    let a_block = luau("export type Point = number\nreturn 1");
    let a_model = analyze(&a_block);
    let c_block = luau("export type Point = string\nreturn 1");
    let c_model = analyze(&c_block);
    let b_block = luau("local a = require('./a')\nlocal c = require('./c')\nreturn a, c");
    let b_model = analyze(&b_block);

    let mut graph = MultiModuleModel::new();
    let a = graph.add_module("a", a_block, a_model).expect("module a");
    let c = graph.add_module("c", c_block, c_model).expect("module c");
    let b = graph.add_module("b", b_block, b_model).expect("module b");
    let alias_a = symbol_named(&graph.module(b).model, "a");
    let alias_c = symbol_named(&graph.module(b).model, "c");

    let unknown_alias = graph
        .resolve_imported_value(b, alias_a, "Point")
        .expect_err("the symbol is not an import alias yet");
    let unknown_type = graph
        .resolve_imported_type(b, "Point")
        .expect_err("the type is not imported");
    graph.bind_import(b, alias_a, a);
    graph.bind_import(b, alias_c, c);
    let ambiguous_type = graph
        .resolve_imported_type(b, "Point")
        .expect_err("the type is ambiguous");
    let unknown_export = graph
        .resolve_imported_value(b, alias_a, "missing")
        .expect_err("the export is unknown");

    let mut interface_graph = MultiModuleModel::new();
    let unanalyzed = luau("return opaque");
    let bound_model = analyze(&luau("return 1"));
    let unbound_export = interface_graph
        .add_module("m", unanalyzed, bound_model)
        .expect_err("an unbound export is rejected");

    let module_errors = vec![
        unbound_export,
        unknown_export,
        unknown_alias,
        unknown_type,
        ambiguous_type,
    ];
    let produced: BTreeSet<&'static str> = module_errors
        .iter()
        .map(|error| ModuleErrorTag::of(error).name())
        .collect();
    let absent = missing(ModuleErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "module errors never triggered: {absent:?}"
    );
    for error in &module_errors {
        assert!(!error.code().is_empty(), "{error:?} has no code");
        assert!(!error.to_string().is_empty());
    }

    let mut bundle_provider = MemoryModuleProvider::new();
    bundle_provider.insert("a", "export type Public = number\nreturn 1");
    bundle_provider.insert("b", "type Local = Public\nreturn 1");
    let mut bundle_block = luau("return require('./a') + require('./b')");
    let bundle_model = analyze(&bundle_block);
    let unimported = bundle_modules(&mut bundle_block, &bundle_model, &bundle_provider)
        .expect_err("an unimported type is rejected");

    let mut ambiguous_provider = MemoryModuleProvider::new();
    ambiguous_provider.insert("a", "export type Point = number\nreturn 1");
    ambiguous_provider.insert("c", "export type Point = string\nreturn 1");
    ambiguous_provider.insert(
        "b",
        "local a = require('./a')\nlocal c = require('./c')\ntype Local = Point\nreturn a, c",
    );
    let mut ambiguous_block = luau("return require('./b')");
    let ambiguous_model = analyze(&ambiguous_block);
    let ambiguous = bundle_modules(&mut ambiguous_block, &ambiguous_model, &ambiguous_provider)
        .expect_err("an ambiguous type is rejected");

    let mut duplicate_provider = MemoryModuleProvider::new();
    duplicate_provider.insert("a", "export type Shared = number\nreturn 1");
    duplicate_provider.insert("c", "export type Shared = string\nreturn 1");
    let mut duplicate_block = luau("return require('./a'), require('./c')");
    let duplicate_model = analyze(&duplicate_block);
    let duplicate = bundle_modules(&mut duplicate_block, &duplicate_model, &duplicate_provider)
        .expect_err("a duplicate exported type is rejected");

    let mut bundle_missing_block = luau("return require('./missing')");
    let bundle_missing_model = analyze(&bundle_missing_block);
    let bundle_missing = bundle_modules(&mut bundle_missing_block, &bundle_missing_model, &empty)
        .expect_err("a missing module is rejected");

    let mut bundle_cycle_block = luau("return require('./a')");
    let bundle_cycle_model = analyze(&bundle_cycle_block);
    let bundle_cycle = bundle_modules(&mut bundle_cycle_block, &bundle_cycle_model, &cyclic)
        .expect_err("a cycle is rejected");

    let mut bundle_parse_block = luau("return require('./a')");
    let bundle_parse_model = analyze(&bundle_parse_block);
    let bundle_parse = bundle_modules(&mut bundle_parse_block, &bundle_parse_model, &bad_parse)
        .expect_err("a parse error is rejected");

    let mut bundle_non_literal_block = luau("return require('./a')");
    let bundle_non_literal_model = analyze(&bundle_non_literal_block);
    let bundle_non_literal = bundle_modules(
        &mut bundle_non_literal_block,
        &bundle_non_literal_model,
        &non_literal,
    )
    .expect_err("a non-literal require is rejected");

    let mut bundle_load_block = luau("return require('./a')");
    let bundle_load_model = analyze(&bundle_load_block);
    let bundle_load = bundle_modules(&mut bundle_load_block, &bundle_load_model, &FailingProvider)
        .expect_err("a load error is rejected");

    let mut bundle_analysis_block = luau("return require('./a')");
    let bundle_analysis_model = analyze(&bundle_analysis_block);
    let bundle_analysis = r4::rules::require::bundle_modules_with_config(
        &mut bundle_analysis_block,
        &bundle_analysis_model,
        &strict_module,
        Parser::default(),
        r4::semantic::SemanticOptions {
            strict_types: true,
            ..Default::default()
        },
    )
    .expect_err("strict types rejects the module during bundling");

    let bundle_errors = vec![
        bundle_missing,
        bundle_cycle,
        bundle_parse,
        bundle_non_literal,
        duplicate,
        unimported,
        bundle_load,
        ambiguous,
        bundle_analysis,
    ];
    let produced: BTreeSet<&'static str> = bundle_errors
        .iter()
        .map(|error| BundleErrorTag::of(error).name())
        .collect();
    let absent = missing(BundleErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "bundle errors never triggered: {absent:?}"
    );
    for error in &bundle_errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }

    let alias_errors = vec![
        AliasRequireConverter::new([("pkg", "x")]).expect_err("malformed"),
        AliasRequireConverter::new([("@pkg", "a"), ("@pkg", "b")]).expect_err("duplicate"),
        AliasRequireConverter::new([("@pkg", "@other")]).expect_err("nested"),
        {
            let mut block = luau("local x = require('@unknown/util')\nreturn x");
            let model = analyze(&block);
            apply_path_aliases(&mut block, &model, [("@pkg", "src/pkg")])
                .expect_err("unresolved alias")
        },
        {
            let mut block = luau("local x = require(name)\nreturn x");
            let model = analyze(&block);
            apply_path_aliases(&mut block, &model, [("@pkg", "src/pkg")])
                .expect_err("non-literal require under aliases")
        },
    ];
    let produced: BTreeSet<&'static str> = alias_errors
        .iter()
        .map(|error| AliasErrorTag::of(error).name())
        .collect();
    let absent = missing(AliasErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(
        absent.is_empty(),
        "alias errors never triggered: {absent:?}"
    );
    for error in &alias_errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }
}

variant_registry!(NumberParsingErrorTag, r4::nodes::NumberParsingError,
    InvalidHexadecimalNumber => r4::nodes::NumberParsingError::InvalidHexadecimalNumber,
    InvalidHexadecimalExponent => r4::nodes::NumberParsingError::InvalidHexadecimalExponent,
    InvalidDecimalNumber => r4::nodes::NumberParsingError::InvalidDecimalNumber,
    InvalidDecimalExponent => r4::nodes::NumberParsingError::InvalidDecimalExponent,
    InvalidBinaryNumber => r4::nodes::NumberParsingError::InvalidBinaryNumber,
);

#[test]
fn number_parsing_errors_are_all_triggered() {
    use r4::nodes::NumberExpression;

    let errors: Vec<r4::nodes::NumberParsingError> = ["", "1e", "0xZZ", "0x1p", "0b1Z"]
        .iter()
        .map(|text| {
            text.parse::<NumberExpression>()
                .expect_err("the literal is invalid")
        })
        .collect();

    let produced: BTreeSet<&'static str> = errors
        .iter()
        .map(|error| NumberParsingErrorTag::of(error).name())
        .collect();
    let absent = missing(
        NumberParsingErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "number parsing errors never triggered: {absent:?}"
    );

    for error in &errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }
}

/// `ConvertDataError` is a struct rather than an enum, so it has no variants to
/// enumerate; this asserts its typed failure path is reachable.
#[test]
fn convert_data_error_is_triggered() {
    struct Failing;

    impl serde::Serialize for Failing {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("cannot serialize this value"))
        }
    }

    let error = r4::convert_data(Failing).expect_err("the value cannot be converted");
    assert!(!error.message().is_empty());
    assert!(!error.to_string().is_empty());
}

variant_registry!(ConstantPropagationErrorTag, r4::cfg::analysis::ConstantPropagationError,
    UnresolvedIdentifier => r4::cfg::analysis::ConstantPropagationError::UnresolvedIdentifier { .. },
);

variant_registry!(LivenessErrorTag, r4::cfg::analysis::LivenessError,
    UnresolvedIdentifier => r4::cfg::analysis::LivenessError::UnresolvedIdentifier { .. },
);

variant_registry!(LicmErrorTag, r4::cfg::analysis::LicmError,
    Liveness => r4::cfg::analysis::LicmError::Liveness(_),
);

variant_registry!(DeadStoreErrorTag, r4::rules::canonicalize::DeadStoreError,
    Cfg => r4::rules::canonicalize::DeadStoreError::Cfg(_),
    Liveness => r4::rules::canonicalize::DeadStoreError::Liveness(_),
);

variant_registry!(UnreachableCodeErrorTag, r4::rules::canonicalize::UnreachableCodeError,
    Cfg => r4::rules::canonicalize::UnreachableCodeError::Cfg(_),
    ConstantPropagation => r4::rules::canonicalize::UnreachableCodeError::ConstantPropagation(_),
    Provenance => r4::rules::canonicalize::UnreachableCodeError::Provenance(_),
);

variant_registry!(HoistLoopInvariantsErrorTag, r4::rules::canonicalize::HoistLoopInvariantsError,
    Provenance => r4::rules::canonicalize::HoistLoopInvariantsError::Provenance(_),
    Cfg => r4::rules::canonicalize::HoistLoopInvariantsError::Cfg(_),
    Liveness => r4::rules::canonicalize::HoistLoopInvariantsError::Liveness(_),
);

/// A block whose local-assignment value is an identifier the model does not
/// bind, replaced after analysis.
fn unbound_local() -> (Block, SemanticModel) {
    use r4::nodes::Expression;

    let mut block = luau("local x = value\nreturn x");
    let model = analyze(&block);
    for statement in block.iter_mut_statements() {
        if let Statement::LocalAssign(assignment) = statement {
            for value in assignment.iter_mut_values() {
                *value = Expression::identifier("never_bound");
            }
        }
    }
    (block, model)
}

#[test]
fn cfg_analysis_errors_are_all_triggered() {
    use r4::cfg::analysis::{ConstantPropagation, Liveness};
    use r4::cfg::build_chunk;
    use r4::nodes::Expression;
    use r4::rules::canonicalize::{
        hoist_loop_invariants, remove_dead_stores, remove_unreachable_code,
    };

    let (block, model) = unbound_local();
    let cfg = build_chunk(&block, &model).expect("the CFG builds");

    let liveness_error = Liveness::analyze(&cfg, &model).expect_err("an unbound read is rejected");
    let produced = BTreeSet::from([LivenessErrorTag::of(&liveness_error).name()]);
    let absent = missing(
        LivenessErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "liveness errors never triggered: {absent:?}"
    );
    assert!(!liveness_error.to_string().is_empty());

    let propagation_error =
        ConstantPropagation::analyze(&cfg, &model).expect_err("an unbound read is rejected");
    let produced = BTreeSet::from([ConstantPropagationErrorTag::of(&propagation_error).name()]);
    let absent = missing(
        ConstantPropagationErrorTag::ALL
            .iter()
            .map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "constant propagation errors never triggered: {absent:?}"
    );
    assert!(!propagation_error.to_string().is_empty());

    // Dead stores: liveness, a CFG that cannot be built, and a stale model.
    let (mut dead_liveness_block, dead_liveness_model) = unbound_local();
    let dead_liveness = remove_dead_stores(&mut dead_liveness_block, &dead_liveness_model)
        .expect_err("an unbound read is rejected");

    let mut dead_cfg_block = luau("break");
    let dead_cfg_model = analyze(&dead_cfg_block);
    let dead_cfg = remove_dead_stores(&mut dead_cfg_block, &dead_cfg_model)
        .expect_err("break outside a loop is rejected");

    let dead_errors = vec![dead_cfg, dead_liveness];
    let produced: BTreeSet<&'static str> = dead_errors
        .iter()
        .map(|error| DeadStoreErrorTag::of(error).name())
        .collect();
    let absent = missing(
        DeadStoreErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "dead store errors never triggered: {absent:?}"
    );
    for error in &dead_errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }

    // Unreachable code: constant propagation, a CFG that cannot be built, and a
    // stale model.
    let (mut unreachable_propagation_block, unreachable_propagation_model) = unbound_local();
    let unreachable_propagation = remove_unreachable_code(
        &mut unreachable_propagation_block,
        &unreachable_propagation_model,
    )
    .expect_err("an unbound read is rejected");

    let mut unreachable_cfg_block = luau("break");
    let unreachable_cfg_model = analyze(&unreachable_cfg_block);
    let unreachable_cfg =
        remove_unreachable_code(&mut unreachable_cfg_block, &unreachable_cfg_model)
            .expect_err("break outside a loop is rejected");

    let mut unreachable_provenance_block = luau("if a then b() end\nreturn 1");
    let analyzed = luau("return 1");
    let model = analyze(&analyzed);
    let unreachable_provenance = remove_unreachable_code(&mut unreachable_provenance_block, &model)
        .expect_err("an unanalyzed block has no provenance");

    let unreachable_errors = vec![
        unreachable_cfg,
        unreachable_propagation,
        unreachable_provenance,
    ];
    let produced: BTreeSet<&'static str> = unreachable_errors
        .iter()
        .map(|error| UnreachableCodeErrorTag::of(error).name())
        .collect();
    let absent = missing(
        UnreachableCodeErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "unreachable code errors never triggered: {absent:?}"
    );
    for error in &unreachable_errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }

    // Loop-invariant hoisting: a CFG that cannot be built, a stale model, and
    // LICM's liveness.
    let mut hoist_cfg_block = luau("break");
    let hoist_cfg_model = analyze(&hoist_cfg_block);
    let hoist_cfg = hoist_loop_invariants(&mut hoist_cfg_block, &hoist_cfg_model)
        .expect_err("break outside a loop is rejected");

    let mut hoist_provenance_block = luau("while a do end");
    let analyzed = luau("return 1");
    let model = analyze(&analyzed);
    let hoist_provenance = hoist_loop_invariants(&mut hoist_provenance_block, &model)
        .expect_err("an unanalyzed block has no provenance");

    let mut hoist_liveness_block = luau("while flag do local y = value end");
    let hoist_liveness_model = analyze(&hoist_liveness_block);
    for statement in hoist_liveness_block.iter_mut_statements() {
        if let Statement::While(while_statement) = statement {
            for inner in while_statement.mutate_block().iter_mut_statements() {
                if let Statement::LocalAssign(assignment) = inner {
                    for value in assignment.iter_mut_values() {
                        *value = Expression::identifier("never_bound");
                    }
                }
            }
        }
    }
    let hoist_liveness = hoist_loop_invariants(&mut hoist_liveness_block, &hoist_liveness_model)
        .expect_err("an unbound read is rejected by LICM");

    let hoist_errors = vec![hoist_cfg, hoist_provenance, hoist_liveness];
    let produced: BTreeSet<&'static str> = hoist_errors
        .iter()
        .map(|error| HoistLoopInvariantsErrorTag::of(error).name())
        .collect();
    let absent = missing(
        HoistLoopInvariantsErrorTag::ALL
            .iter()
            .map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "hoist errors never triggered: {absent:?}"
    );
    for error in &hoist_errors {
        assert!(!error.to_string().is_empty(), "{error:?} has no message");
    }

    let r4::rules::canonicalize::HoistLoopInvariantsError::Liveness(licm) = &hoist_errors[2] else {
        panic!("expected a LICM liveness error");
    };
    let produced = BTreeSet::from([LicmErrorTag::of(licm).name()]);
    let absent = missing(LicmErrorTag::ALL.iter().map(|tag| tag.name()), &produced);
    assert!(absent.is_empty(), "LICM errors never triggered: {absent:?}");
    assert!(!licm.to_string().is_empty());
}

/// The runtime symbol declared as a local named `name`.
fn local_symbol(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.kind != SymbolKind::Global && symbol.name == name)
        .unwrap_or_else(|| panic!("a local named `{name}`"))
        .id
}

#[test]
fn rename_symbol_errors_are_all_triggered() {
    let mut produced = BTreeSet::new();
    let mut codes = BTreeSet::new();

    // UnsupportedIdentifier: the requested name is not a Lua identifier.
    let mut block = luau("local a = 1");
    let model = analyze(&block);
    let original = block.clone();
    let a = local_symbol(&model, "a");
    let error = rename_symbol(&mut block, &model, a, "\u{FEFF}").expect_err("an invalid name");
    assert_eq!(block, original, "the rename must fail before mutating");
    assert!(matches!(
        &error,
        RenameSymbolError::UnsupportedIdentifier { name } if name == "\u{FEFF}"
    ));
    produced.insert(RenameSymbolErrorTag::of(&error).name());
    codes.insert(error.code());
    assert!(!error.to_string().is_empty());

    // UnknownSymbol: the id is not part of the model.
    let mut block = luau("local a = 1");
    let model = analyze(&block);
    let original = block.clone();
    let error = rename_symbol(&mut block, &model, SymbolId(9_999), "b")
        .expect_err("a symbol that is not in the model");
    assert_eq!(block, original, "the rename must fail before mutating");
    produced.insert(RenameSymbolErrorTag::of(&error).name());
    codes.insert(error.code());
    assert!(!error.to_string().is_empty());

    // ImplicitSymbol: a synthesized global has no declaration to rename.
    let mut block = luau("return print");
    let model = analyze(&block);
    let original = block.clone();
    let print = model
        .symbols
        .iter()
        .find(|symbol| symbol.kind == SymbolKind::Global && symbol.name == "print")
        .expect("the implicit global `print`")
        .id;
    assert!(model.symbol(print).is_implicit());
    let error =
        rename_symbol(&mut block, &model, print, "renamed").expect_err("an implicit symbol");
    assert_eq!(block, original, "the rename must fail before mutating");
    produced.insert(RenameSymbolErrorTag::of(&error).name());
    codes.insert(error.code());
    assert!(!error.to_string().is_empty());

    // UnresolvedIdentifier: the tree does not match the model.
    let analyzed = luau("local a = 1\nreturn a");
    let model = analyze(&analyzed);
    let a = local_symbol(&model, "a");
    let mut foreign = luau("return ghost");
    let original = foreign.clone();
    let error = rename_symbol(&mut foreign, &model, a, "renamed")
        .expect_err("an identifier the model does not bind");
    assert_eq!(foreign, original, "the rename must fail before mutating");
    assert!(matches!(
        &error,
        RenameSymbolError::UnresolvedIdentifier { name } if name == "ghost"
    ));
    produced.insert(RenameSymbolErrorTag::of(&error).name());
    codes.insert(error.code());
    assert!(!error.to_string().is_empty());

    // Conflict: the requested name is visible where the renamed symbol is.
    let mut block = luau("local a, b = 1, 2");
    let model = analyze(&block);
    let original = block.clone();
    let a = local_symbol(&model, "a");
    let error = rename_symbol(&mut block, &model, a, "b").expect_err("a visible conflict");
    assert_eq!(block, original, "the rename must fail before mutating");
    assert!(matches!(&error, RenameSymbolError::Conflict { name } if name == "b"));
    produced.insert(RenameSymbolErrorTag::of(&error).name());
    codes.insert(error.code());
    assert!(!error.to_string().is_empty());

    let absent = missing(
        RenameSymbolErrorTag::ALL.iter().map(|tag| tag.name()),
        &produced,
    );
    assert!(
        absent.is_empty(),
        "rename errors never triggered: {absent:?}"
    );

    let expected: BTreeSet<&'static str> = BTreeSet::from([
        "rename/unsupported-identifier",
        "rename/unknown-symbol",
        "rename/implicit-symbol",
        "rename/unresolved-identifier",
        "rename/conflict",
    ]);
    assert_eq!(codes, expected, "unexpected rename error codes");

    let sample = RenameSymbolError::UnknownSymbol;
    let dynamic: &dyn std::error::Error = &sample;
    assert!(
        !dynamic.to_string().is_empty(),
        "the error must be formatted"
    );
}
