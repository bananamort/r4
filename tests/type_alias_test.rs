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

#[test]
fn type_function_parameters_and_generics_are_analyzed() {
    // A type function introduces a function scope with its parameters and
    // generic type parameters. Its own name is a type-level function, not a
    // lexically declared type: the reference implementation does not declare it
    // as a symbol either, so it must not appear in `model.types`.
    let (_, model) = analyze("type function id<T>(value: T): T\n  return value\nend\nreturn 1");

    assert!(
        model
            .symbols
            .iter()
            .any(|symbol| symbol.name == "value"
                && symbol.kind == r4::semantic::SymbolKind::Parameter),
        "the type function parameter must be declared as a parameter"
    );
    assert!(
        !model.types.iter().any(|symbol| symbol.name == "id"),
        "the type function name is not a lexical type"
    );
}

#[test]
fn generic_alias_of_a_named_type_does_not_expand() {
    // `G` is generic, so it has no single expansion even though its body names
    // the bare alias `B`.
    let (_block, model) = analyze("type B = number\ntype G<T> = B\nreturn 0");
    let b = type_named(&model, "B");
    let g = type_named(&model, "G");

    assert_eq!(model.type_symbol(b).alias_of, None);
    assert_eq!(
        model.type_symbol(g).alias_of,
        None,
        "a generic alias must not collapse onto a bare alias"
    );
    assert_eq!(model.expand_type_alias(g), g);
}

#[test]
fn a_generic_parameter_does_not_resolve_to_an_outer_alias() {
    // The `T` on the right-hand side is `A`'s own generic parameter, not the
    // outer alias `type T = number`.
    let (_block, model) = analyze("type T = number\ntype A<T> = T\nreturn 0");
    let a = type_named(&model, "A");

    assert_eq!(
        model.type_symbol(a).alias_of,
        None,
        "A's parameter must shadow the outer alias"
    );
    assert_eq!(model.expand_type_alias(a), a);
}

#[test]
fn a_generic_parameter_shadows_a_same_named_alias_occurrence() {
    use r4::nodes::TypeName;
    use r4::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

    #[derive(Default)]
    struct TypeNameBindings {
        names: Vec<String>,
        bound: Vec<bool>,
    }

    impl NodeProcessor for TypeNameBindings {
        fn process_type_name(&mut self, name: &mut TypeName) {
            let identifier = name.get_type_name();
            self.names.push(identifier.get_name().to_owned());
            self.bound.push(identifier.get_binding().is_some());
        }
    }

    let (mut block, _model) =
        analyze("type T = number\nlocal function f<T>(x: T): T\n  return x\nend\nreturn f");

    let mut collector = TypeNameBindings::default();
    DefaultVisitor::visit_block(&mut block, &mut collector);

    let mut saw_generic_t = false;
    for (name, bound) in collector.names.iter().zip(&collector.bound) {
        if name == "T" {
            saw_generic_t = true;
            assert!(
                !bound,
                "the generic `T` occurrence must not bind to the alias"
            );
        }
    }
    assert!(saw_generic_t, "the signature declares and uses `T`");
}

#[test]
fn a_namespace_qualified_type_does_not_bind_to_a_local_alias() {
    use r4::nodes::Type;
    use r4::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

    #[derive(Default)]
    struct FieldBindings {
        fields: Vec<(bool, bool)>,
    }

    impl NodeProcessor for FieldBindings {
        fn process_type(&mut self, r#type: &mut Type) {
            if let Type::Field(field) = r#type {
                self.fields.push((
                    field.get_namespace().get_binding().is_some(),
                    field
                        .get_type_name()
                        .get_type_name()
                        .get_binding()
                        .is_some(),
                ));
            }
        }
    }

    let (mut block, _model) = analyze(
        "type Point = string\nlocal Types = {}\nlocal value: Types.Point = nil\nreturn value",
    );
    let mut collector = FieldBindings::default();
    DefaultVisitor::visit_block(&mut block, &mut collector);

    assert_eq!(collector.fields.len(), 1, "one namespace-qualified type");
    let (namespace_bound, member_bound) = collector.fields[0];
    assert!(
        namespace_bound,
        "the namespace `Types` must resolve as a value binding"
    );
    assert!(
        !member_bound,
        "`Point` must not bind to the local alias of the same name"
    );
}
