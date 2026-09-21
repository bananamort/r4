//! Tests for global value injection.

use std::collections::HashMap;

use r4::process::LuaValue;
use r4::rules::{inject_global_values, InjectGlobalValueError};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn analyze(code: &str) -> (r4::nodes::Block, r4::semantic::SemanticModel) {
    let block = Parser::default().parse(code).expect("valid code");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    (block, model)
}

#[test]
fn injects_representable_globals_and_rejects_others() {
    let (mut block, model) = analyze("return VERSION");
    let mut values = HashMap::new();
    values.insert("VERSION".to_owned(), LuaValue::Number(1.0));
    assert_eq!(
        inject_global_values(&mut block, &model, &values).expect("injects"),
        1
    );

    let (mut block, model) = analyze("return VERSION");
    let mut values = HashMap::new();
    values.insert("VERSION".to_owned(), LuaValue::Function);
    let error =
        inject_global_values(&mut block, &model, &values).expect_err("unrepresentable value");
    assert!(matches!(
        error,
        InjectGlobalValueError::UnrepresentableValue { .. }
    ));
    assert!(!error.to_string().is_empty());

    let (mut block, model) = analyze("return 1");
    assert_eq!(
        inject_global_values(&mut block, &model, &HashMap::new()).expect("nothing to inject"),
        0
    );
}
