//! Tests for incremental parsing and analysis reuse.

use r4::modules::MemoryModuleProvider;
use r4::Compilation;

#[test]
fn identical_sources_share_one_analysis() {
    let mut compilation = Compilation::new();
    let first = compilation.add_tree("return 1").expect("valid code");
    let second = compilation.add_tree("return 1").expect("valid code");

    assert_ne!(first, second);
    assert_eq!(compilation.len(), 2);
    assert_eq!(compilation.cache_len(), 1);
}

#[test]
fn update_tree_analyzes_only_when_the_source_changed() {
    let mut compilation = Compilation::new();
    let tree = compilation.add_tree("return 1").expect("valid code");

    compilation
        .update_tree(tree, "return 1")
        .expect("unchanged source");
    assert_eq!(compilation.cache_len(), 1);

    compilation
        .update_tree(tree, "return 2")
        .expect("changed source");
    assert_eq!(compilation.cache_len(), 2);
    assert_eq!(compilation.source(tree), Some("return 2"));
}

#[test]
fn update_tree_reuses_an_identical_sibling_analysis() {
    let mut compilation = Compilation::new();
    let first = compilation.add_tree("return 1").expect("valid code");
    let second = compilation.add_tree("return 2").expect("valid code");
    assert_eq!(compilation.cache_len(), 2);

    compilation
        .update_tree(second, "return 1")
        .expect("source equals the first tree");
    assert_eq!(compilation.cache_len(), 2, "the analysis is reused");
    assert_eq!(compilation.source(first), Some("return 1"));
    assert_eq!(compilation.source(second), Some("return 1"));
}

#[test]
fn refresh_tree_reanalyzes_mutated_blocks() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("local x = 1\nreturn x")
        .expect("valid code");

    compilation
        .block_mut(tree)
        .expect("tree exists")
        .take_last_statement();
    compilation
        .refresh_tree(tree)
        .expect("the mutated tree analyzes");

    assert!(compilation
        .block(tree)
        .unwrap()
        .get_last_statement()
        .is_none());
    let model = compilation.model(tree).unwrap();
    let x = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "x")
        .expect("x exists");
    assert!(model.is_unused(x.id));
}

#[test]
fn copy_on_write_edits_do_not_leak_between_trees() {
    let mut compilation = Compilation::new();
    let first = compilation.add_tree("return 1").expect("valid code");
    let second = compilation.add_tree("return 1").expect("valid code");

    compilation
        .block_mut(first)
        .expect("tree exists")
        .take_last_statement();
    compilation
        .refresh_tree(first)
        .expect("the mutated tree analyzes");

    assert!(compilation
        .block(first)
        .unwrap()
        .get_last_statement()
        .is_none());
    assert!(compilation
        .block(second)
        .unwrap()
        .get_last_statement()
        .is_some());
}

#[test]
fn module_resolution_reuses_cached_analyses() {
    let mut compilation = Compilation::new();
    let root = compilation
        .add_tree("return require('./a')")
        .expect("valid code");

    let mut provider = MemoryModuleProvider::new();
    provider.insert("a", "return { value = 1 }");

    let first = compilation
        .resolve_modules(root, &provider)
        .expect("resolution succeeds");
    assert!(first.graph.module_by_key("a").is_some());
    let cached = compilation.cache_len();

    let second = compilation
        .resolve_modules(root, &provider)
        .expect("resolution succeeds again");
    assert!(second.graph.module_by_key("a").is_some());
    assert_eq!(
        compilation.cache_len(),
        cached,
        "unchanged dependency sources must reuse the analysis cache"
    );
}

/// Mutating a block in place must detach its model: querying the model while
/// the tree is stale is a typed failure, never a stale answer.
#[test]
fn mutating_a_block_detaches_its_model_until_refreshed() {
    use r4::nodes::Identifier;
    use r4::CompilationError;

    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("local x = 1\nreturn x")
        .expect("valid code");

    assert!(compilation.model(tree).is_ok());
    assert!(!compilation.tree(tree).unwrap().is_stale());

    compilation
        .block_mut(tree)
        .expect("tree exists")
        .take_last_statement();

    assert!(compilation.tree(tree).unwrap().is_stale());
    assert_eq!(
        compilation.model(tree).unwrap_err(),
        CompilationError::StaleTree(tree)
    );

    let identifier = Identifier::new("x");
    assert_eq!(
        compilation.binding_of(tree, &identifier).unwrap_err(),
        CompilationError::StaleTree(tree)
    );

    // Refreshing re-analyzes the mutated block and clears the staleness.
    compilation
        .refresh_tree(tree)
        .expect("the mutated tree analyzes");
    assert!(!compilation.tree(tree).unwrap().is_stale());
    assert!(compilation.model(tree).is_ok());
    assert_eq!(compilation.binding_of(tree, &identifier), Ok(None));

    // An unknown identifier is a distinct typed failure.
    let missing = r4::TreeId(99);
    assert_eq!(
        compilation.model(missing).unwrap_err(),
        CompilationError::UnknownTree(missing)
    );
}

/// `update_tree` must refresh a mutated tree even when the new source text is
/// identical, instead of silently keeping the stale model.
#[test]
fn update_tree_with_the_same_source_refreshes_a_stale_tree() {
    let mut compilation = Compilation::new();
    let tree = compilation
        .add_tree("local x = 1\nreturn x")
        .expect("valid code");

    compilation
        .block_mut(tree)
        .expect("tree exists")
        .take_last_statement();
    assert!(compilation.tree(tree).unwrap().is_stale());

    compilation
        .update_tree(tree, "local x = 1\nreturn x")
        .expect("the update succeeds");

    assert!(!compilation.tree(tree).unwrap().is_stale());
    assert!(compilation.model(tree).is_ok());
}

/// Resolving modules from a tree whose block was mutated but not refreshed must
/// fail with a typed diagnostic, never panic or use the stale model.
#[test]
fn resolving_a_stale_tree_is_a_typed_failure() {
    let mut compilation = Compilation::new();
    let root = compilation
        .add_tree("return require('./a')")
        .expect("valid code");

    compilation
        .block_mut(root)
        .expect("tree exists")
        .take_last_statement();

    let provider = MemoryModuleProvider::new();
    let error = compilation
        .resolve_modules(root, &provider)
        .expect_err("a stale root must not resolve");
    assert_eq!(error.code, Some("compile/stale-tree"));

    compilation
        .refresh_tree(root)
        .expect("the mutated root analyzes");
    compilation
        .resolve_modules(root, &provider)
        .expect("a refreshed root resolves");
}
