//! Tests for the derived whole-program index.

use r4::semantic::MultiModuleModel;
use r4::Parser;

fn add_module(graph: &mut MultiModuleModel, key: &str, code: &str) {
    let (block, model) = Parser::default()
        .parse_analyzed(code)
        .expect("the test module analyzes");
    graph
        .add_module(key.to_owned(), block, model)
        .expect("the interface is valid");
}

#[test]
fn dense_rank_space_covers_globals_and_locals() {
    let mut graph = MultiModuleModel::new();
    add_module(&mut graph, "a", "local x = 1\nprint(x)\nreturn x");
    add_module(&mut graph, "b", "local y = 2\nprint(y)\nreturn y");

    let index = graph.whole_program_index();

    // One canonical global (`print`) shared by both modules.
    assert_eq!(index.global_count(), 1);
    let print = graph.global_symbol("print").expect("print is global");
    assert_eq!(index.global_rank(print), 0);
    assert_eq!(index.global_uses(print).len(), 2);

    // Two module-local symbols, one per module.
    assert_eq!(index.local_count(), 2);
    assert_eq!(index.dense_len(), 3);

    // A whole-program bitset indexed by dense rank: canonical globals first,
    // then module-local symbols.
    let mut live = vec![false; index.dense_len()];
    live[index.global_rank(print) as usize] = true;
    let globals = index.global_count() as u32;
    for rank in 0..index.local_count() as u32 {
        let local = index.local(rank).expect("rank is dense");
        assert_eq!(index.dense_rank(local), Some(rank + globals));
        live[(rank + globals) as usize] = true;
    }
    assert!(live.iter().all(|value| *value));
}

#[test]
fn global_uses_distinguish_globals_from_locals() {
    let mut graph = MultiModuleModel::new();
    add_module(
        &mut graph,
        "a",
        "local shadow = 1\nlocal other = shadow\nreturn other",
    );

    let index = graph.whole_program_index();
    let shadow = index
        .locals()
        .iter()
        .copied()
        .find(|qualified| index.symbol(&graph, *qualified).name == "shadow")
        .expect("shadow is a local");

    assert!(!index.is_global(shadow));
    assert_eq!(index.global_of(shadow), None);
    assert_eq!(
        index.dense_rank(shadow),
        index
            .local_rank(shadow)
            .map(|rank| rank + index.global_count() as u32)
    );
}

#[test]
fn index_is_a_read_only_derivation() {
    let mut graph = MultiModuleModel::new();
    add_module(&mut graph, "a", "local x = 1\nreturn x");

    let index = graph.whole_program_index();
    assert_eq!(index.local_count(), 1);
    // The index borrows nothing from the graph: dropping it leaves every
    // per-module query working unchanged.
    drop(index);

    let module = graph.module_by_key("a").expect("module a");
    let model = &graph.module(module).model;
    assert!(model
        .symbols
        .iter()
        .any(|symbol| symbol.name == "x" && !model.is_unused(symbol.id)));
}
