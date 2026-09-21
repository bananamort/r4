//! Tests for require path aliases.

use r4::nodes::Block;
use r4::rules::require::{
    apply_path_aliases, AliasError, AliasRequireConverter, RequirePathConverter,
};
use r4::Parser;

fn analyze(code: &str) -> (Block, r4::semantic::SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

#[test]
fn alias_converter_validation_and_resolution() {
    let converter = AliasRequireConverter::new([("@pkg", "src/pkg")]).expect("valid aliases");
    assert!(AliasRequireConverter::is_aliased("@pkg/x"));
    assert!(!AliasRequireConverter::is_aliased("pkg/x"));
    assert_eq!(converter.resolve("@pkg"), Some("src/pkg".to_owned()));
    assert_eq!(
        converter.resolve("@pkg/util"),
        Some("src/pkg/util".to_owned())
    );
    assert_eq!(converter.resolve("other"), None);
    assert!(converter.convert("@pkg/util").is_some());
    assert!(converter.convert("other").is_none());

    // The longest matching prefix wins.
    let nested =
        AliasRequireConverter::new([("@pkg", "a"), ("@pkg/extra", "b")]).expect("valid aliases");
    assert_eq!(nested.resolve("@pkg/extra/x"), Some("b/x".to_owned()));
    assert_eq!(nested.resolve("@pkg/x"), Some("a/x".to_owned()));

    for error in [
        AliasRequireConverter::new([("pkg", "x")]).expect_err("malformed prefix"),
        AliasRequireConverter::new([("@", "x")]).expect_err("too short"),
        AliasRequireConverter::new([("@pkg/", "x")]).expect_err("trailing slash"),
        AliasRequireConverter::new([("@pkg", "")]).expect_err("empty target"),
        AliasRequireConverter::new([("@pkg", "@other")]).expect_err("nested alias"),
        AliasRequireConverter::new([("@pkg", "a"), ("@pkg", "b")]).expect_err("duplicate"),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn applies_aliases_and_rejects_unresolved_paths() {
    let (mut block, model) = analyze("local x = require('@pkg/util')\nreturn x");
    let converted =
        apply_path_aliases(&mut block, &model, [("@pkg", "src/pkg")]).expect("aliases apply");
    assert_eq!(converted, 1);

    let (mut block, model) = analyze("local x = require('@unknown/util')\nreturn x");
    let error = apply_path_aliases(&mut block, &model, [("@pkg", "src/pkg")])
        .expect_err("an unresolved alias is reported");
    assert!(matches!(error, AliasError::UnresolvedAlias { .. }));
    assert!(!error.to_string().is_empty());

    // A malformed configuration is rejected before conversion.
    let (mut block, model) = analyze("local x = require('@pkg/util')\nreturn x");
    let error = apply_path_aliases(&mut block, &model, [("pkg", "x")])
        .expect_err("malformed aliases are rejected");
    assert!(matches!(error, AliasError::MalformedAlias { .. }));
}
