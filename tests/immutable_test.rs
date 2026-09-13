//! Tests for the read-only, result-carrying visitor.

use r4::nodes::{Expression, Statement};
use r4::process::{
    any_expression, any_statement, collect_identifier_names, count_nodes, ImmutableProcessor,
    ImmutableVisitor, Walk,
};
use r4::Parser;

fn parse(code: &str) -> r4::nodes::Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

#[test]
fn traversal_is_read_only() {
    let block = parse("local x = 1\nreturn x");
    let before = block.clone();
    let _ = collect_identifier_names(&block);
    assert_eq!(block, before, "the visitor must not mutate the tree");
}

#[test]
fn collects_identifier_names() {
    let block = parse("local x = 1\nreturn x");
    assert_eq!(collect_identifier_names(&block), vec!["x", "x"]);
}

#[test]
fn searches_statements_and_expressions() {
    let block = parse("local x = f(1)\nreturn x");

    assert!(any_expression(&block, |expression| matches!(
        expression,
        Expression::Call(_)
    )));
    assert!(any_statement(&block, |statement| matches!(
        statement,
        Statement::LocalAssign(_)
    )));
    assert!(!any_expression(&block, |expression| matches!(
        expression,
        Expression::Table(_)
    )));
}

#[test]
fn reaches_types_and_signatures() {
    // The identifier `x` only occurs inside `typeof`, and `T` only inside the
    // function signature, so both must be found by the traversal.
    let block = parse(
        "type A = typeof(x)\nlocal function f<T>(value: T): A\n    return value\nend\nreturn x",
    );
    let names = collect_identifier_names(&block);
    assert!(names.iter().any(|name| name == "x"), "{names:?}");
    assert!(names.iter().any(|name| name == "A"), "{names:?}");
    assert!(names.iter().any(|name| name == "T"), "{names:?}");
    assert!(names.iter().any(|name| name == "value"), "{names:?}");
}

#[test]
fn counts_nodes_across_nesting() {
    let block = parse("local x = 1 + 2\nif x then\n    return x\nend");
    let (statements, expressions) = count_nodes(&block);
    assert!(statements >= 2, "{statements}");
    assert!(expressions >= 2, "{expressions}");
}

#[test]
fn reaches_function_names_methods_and_attribute_names() {
    let block = parse("local M = {}\nfunction M.f() end\nfunction M:m() end\nreturn M");
    let names = collect_identifier_names(&block);
    for expected in ["M", "f", "m"] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
}

#[test]
fn reaches_function_type_argument_names() {
    let block =
        parse("local f: (value: number) -> number = function(value) return value end\nreturn f");
    let names = collect_identifier_names(&block);
    assert!(
        names.iter().filter(|name| *name == "value").count() >= 3,
        "{names:?}"
    );
}

#[test]
fn reaches_goto_and_label_names() {
    let block = Parser::default()
        .lua52()
        .parse("::top::\ngoto top\nreturn 0")
        .expect("Lua 5.2 code parses");
    let names = collect_identifier_names(&block);
    assert_eq!(
        names.iter().filter(|name| *name == "top").count(),
        2,
        "{names:?}"
    );
}

#[test]
fn reaches_type_parameters_and_typeof() {
    let block = parse(
        "type Alias<T> = typeof(build)\nlocal function build<T>(value: T): T\n    return value\nend\nreturn Alias",
    );
    let names = collect_identifier_names(&block);
    for expected in ["Alias", "T", "build", "value"] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
}

#[test]
fn custom_processor_accumulates_a_result() {
    struct CallNames {
        names: Vec<String>,
    }

    impl ImmutableProcessor for CallNames {
        fn process_function_call(&mut self, call: &r4::nodes::FunctionCall) {
            if let r4::nodes::Prefix::Identifier(identifier) = call.get_prefix() {
                self.names.push(identifier.get_name().clone());
            }
        }
    }

    let block = parse("print(1)\nformat('%d', 2)\nreturn 0");
    let mut collector = CallNames { names: Vec::new() };
    Walk::visit_block(&block, &mut collector);
    assert_eq!(collector.names, vec!["print", "format"]);
}
