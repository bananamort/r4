//! Tests for type alias expansion.

use r4::nodes::Block;
use r4::semantic::{SemanticModel, TypeId};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn type_named(model: &SemanticModel, name: &str) -> TypeId {
    model
        .types
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("type `{name}` must exist"))
        .id
}

#[test]
fn expands_bare_alias_chains() {
    // Type aliases are not order sensitive: `A` may reference `B` before it is
    // declared.
    let (_block, model) = analyze(
        "type A = B\ntype B = C\ntype C = { value: number }\nlocal v: A = { value = 1 }\nreturn v",
    );

    let a = type_named(&model, "A");
    let b = type_named(&model, "B");
    let c = type_named(&model, "C");

    assert_eq!(model.type_symbol(a).alias_of, Some(b));
    assert_eq!(model.type_symbol(b).alias_of, Some(c));
    assert_eq!(model.type_symbol(c).alias_of, None);
    assert_eq!(model.expand_type_alias(a), c);
    assert_eq!(model.expand_type_alias(b), c);
    assert_eq!(model.expand_type_alias(c), c);
}

#[test]
fn forward_and_mutually_recursive_types_resolve() {
    let (_block, model) =
        analyze("type A = { b: B }\ntype B = { a: A }\nlocal v: A = { b = { a = nil } }\nreturn v");
    let a = type_named(&model, "A");
    let b = type_named(&model, "B");
    assert!(a != b);
    assert_eq!(model.type_symbol(a).alias_of, None);
    assert_eq!(model.type_symbol(b).alias_of, None);
}

#[test]
fn structural_and_generic_aliases_do_not_expand() {
    let (_block, model) =
        analyze("type S = { value: number }\ntype G<T> = T\ntype N = number\nreturn 0");

    let s = type_named(&model, "S");
    let g = type_named(&model, "G");
    let n = type_named(&model, "N");

    assert_eq!(model.type_symbol(s).alias_of, None);
    assert_eq!(model.type_symbol(g).alias_of, None);
    assert_eq!(model.type_symbol(n).alias_of, None);
    assert_eq!(model.expand_type_alias(s), s);
    assert_eq!(model.expand_type_alias(g), g);
    assert_eq!(model.expand_type_alias(n), n);
}

#[test]
fn self_alias_terminates() {
    let (_block, model) = analyze("type A = A\nreturn 0");
    let a = type_named(&model, "A");
    assert_eq!(model.type_symbol(a).alias_of, None);
    assert_eq!(model.expand_type_alias(a), a);
}

#[test]
fn scope_lookup_expands_aliases() {
    let (_block, model) =
        analyze("type A = B\ntype B = { value: number }\nlocal v: A = { value = 1 }\nreturn v");
    let a = type_named(&model, "A");
    let b = type_named(&model, "B");
    let scope = model.chunk_scope();
    assert_eq!(model.resolve_type_in_scope(scope, "A"), Some(a));
    assert_eq!(model.resolve_type_in_scope_expanded(scope, "A"), Some(b));
}
