//! Tests for Roblox/Luau instance-based `require` support.

use r4::modules::{resolve_instance_path, InstanceSegment};
use r4::rules::require::{bundle_modules, BundleError, MemoryModuleProvider};
use r4::Parser;

fn bundle(root: &str, modules: &[(&str, &str)]) -> Result<String, BundleError> {
    let (mut block, model) = Parser::default()
        .parse_analyzed(root)
        .unwrap_or_else(|error| panic!("failed to analyze `{root}`: {error}"));
    let mut provider = MemoryModuleProvider::new();
    for (key, source) in modules {
        provider.insert(*key, *source);
    }
    let count = bundle_modules(&mut block, &model, &provider)?;
    assert!(count > 0);

    let mut generator = r4::generator::ReadableLuaGenerator::default();
    use r4::generator::LuaGenerator;
    generator.write_block(&block);
    Ok(generator.into_string())
}

#[test]
fn resolves_script_relative_instance_paths() {
    assert_eq!(
        resolve_instance_path(
            "pkg/mod",
            &[
                InstanceSegment::Parent,
                InstanceSegment::Child("util".into())
            ]
        ),
        Some("pkg/util".to_owned())
    );
    assert_eq!(
        resolve_instance_path("pkg/mod", &[InstanceSegment::Child("sub".into())]),
        Some("pkg/mod/sub".to_owned())
    );
    assert_eq!(
        resolve_instance_path("a", &[InstanceSegment::Parent]),
        Some(String::new())
    );
    assert_eq!(
        resolve_instance_path("pkg/mod", &[InstanceSegment::Service("X".into())]),
        None
    );
}

#[test]
fn bundles_instance_requires() {
    let output = bundle(
        "local a = require('./pkg/a')\nreturn a.value",
        &[
            (
                "pkg/a",
                "local util = require(script.Parent.util)\nreturn { value = util.value }",
            ),
            ("pkg/util", "return { value = 42 }"),
        ],
    )
    .expect("instance requires must bundle");

    assert!(output.contains("['pkg/a']"), "{output}");
    assert!(output.contains("['pkg/util']"), "{output}");
    assert!(!output.contains("require("), "{output}");
}

#[test]
fn bundles_find_first_child_instance_requires() {
    let output = bundle(
        "local a = require('./pkg/a')\nreturn a.value",
        &[
            (
                "pkg/a",
                "local util = require(script.Parent:FindFirstChild('util'))\nreturn { value = util.value }",
            ),
            ("pkg/util", "return { value = 7 }"),
        ],
    )
    .expect("FindFirstChild instance requires must bundle");

    assert!(output.contains("['pkg/util']"), "{output}");
    assert!(!output.contains("require("), "{output}");
}

#[test]
fn bundles_luau_self_requires() {
    let output = bundle(
        "local a = require('./pkg/a')\nreturn a.value",
        &[
            (
                "pkg/a",
                "local util = require('@self/util')\nreturn { value = util.value }",
            ),
            ("pkg/util", "return { value = 5 }"),
        ],
    )
    .expect("@self requires must bundle");

    assert!(output.contains("['pkg/util']"), "{output}");
    assert!(!output.contains("require("), "{output}");
}

#[test]
fn rejects_unsupported_data_model_require() {
    let error = bundle(
        "local a = require('./pkg/a')\nreturn a",
        &[(
            "pkg/a",
            "return require(game:GetService('ReplicatedStorage').Util)",
        )],
    )
    .expect_err("a data-model require without a project mapping must fail fast");

    assert!(matches!(
        error,
        BundleError::NonLiteralRequire { ref module, .. } if module.as_deref() == Some("pkg/a")
    ));
}
