//! Tests for bundling error surfaces.

use r4::modules::{MemoryModuleProvider, ModuleLoadError, ModuleProvider, ResolveError};
use r4::nodes::Block;
use r4::rules::require::{bundle_modules, BundleError};
use r4::semantic::{ModuleError, SemanticAnalyzer};
use r4::Parser;

fn analyze(code: &str) -> (Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

struct FailingProvider;

impl ModuleProvider for FailingProvider {
    fn load(&self, key: &str) -> Result<Option<String>, ModuleLoadError> {
        Err(ModuleLoadError::Read {
            key: key.to_owned(),
            message: "disk error".to_owned(),
        })
    }
}

#[test]
fn bundling_reports_parse_and_load_errors() {
    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "local x = ");
    let (mut block, model) = analyze("return require('./a')");
    let error = bundle_modules(&mut block, &model, &provider).expect_err("a parse error");
    assert!(matches!(error, BundleError::Parse { .. }), "{error:?}");
    assert!(!error.to_string().is_empty());

    let (mut block, model) = analyze("return require('./a')");
    let error = bundle_modules(&mut block, &model, &FailingProvider).expect_err("a load error");
    assert!(matches!(error, BundleError::ModuleLoad { .. }), "{error:?}");
    assert!(!error.to_string().is_empty());
}

#[test]
fn bundling_a_failing_graph_leaves_the_input_block_untouched() {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};

    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "local x = ");
    let (mut block, model) = analyze("local before = require('./a')\nreturn before");

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    let before = generator.into_string();

    bundle_modules(&mut block, &model, &provider).expect_err("a parse error");

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    assert_eq!(
        before,
        generator.into_string(),
        "a failed bundle must not clear the caller's block"
    );
}

#[test]
fn bundle_error_conversions_render() {
    let parse_error = Parser::default()
        .parse("local x = ")
        .expect_err("a syntax error");
    let analysis_error = SemanticAnalyzer::new()
        .analyze(
            &Parser::default()
                .lua52()
                .parse("goto missing")
                .expect("parse"),
        )
        .expect_err("a semantic error");

    for resolve in [
        ResolveError::Parse {
            path: "a".to_owned(),
            error: parse_error,
        },
        ResolveError::Analysis {
            path: "a".to_owned(),
            error: analysis_error,
        },
        ResolveError::ModuleLoad {
            error: ModuleLoadError::Read {
                key: "a".to_owned(),
                message: "io".to_owned(),
            },
        },
        ResolveError::Interface(ModuleError::UnboundExport {
            name: "x".to_owned(),
        }),
    ] {
        let bundle: BundleError = resolve.into();
        assert!(!bundle.to_string().is_empty());
    }
}
