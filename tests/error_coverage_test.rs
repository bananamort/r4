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
