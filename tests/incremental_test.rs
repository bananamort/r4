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
