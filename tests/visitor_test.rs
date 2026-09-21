//! Traversal-completeness tests.
//!
//! Each driver (mutable, post-order and the read-only walk) must reach the same
//! syntax nodes. Comparing the multiset of visited `Type` nodes catches any
//! kind whose children a driver forgets to recurse into -- method-call type
//! arguments (`obj:m<<T>>()`) were dropped by all three.

use r4::nodes::Type;
use r4::process::{
    DefaultPostVisitor, DefaultVisitor, ImmutableProcessor, ImmutableVisitor, NodePostProcessor,
    NodePostVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::Parser;

fn type_corpus() -> &'static [&'static str] {
    &[
        "return 1 :: number",
        "local x: number = 1\nreturn x",
        "type T = { a: number, [string]: boolean }\nreturn 1",
        "type F = (a: number, ...string) -> (boolean, number)\nreturn 1",
        "type U = A | B\ntype V = C & D\ntype W = E?\nreturn 1",
        "local f: (number) -> string = g\nreturn f",
        "return f<<SomeType>>()",
        "return obj:m<<SomeType>>()",
        "return obj:m<<A, B>>(1)",
        "return obj.method<<A>>()",
        "return (obj):m<<A>>()",
        "return obj:m<<A<B>>>()",
        "local t: typeof(x) = y\nreturn t",
        "local function h<T>(x: T): T return x end\nreturn h",
        "local x = { y = z } :: { y: number }\nreturn x",
        "type A<T> = T\nlocal a: A<number> = 1\nreturn a",
        "type R<T, U...> = (T, U...) -> T & U\nreturn 1",
        "local g = function<A>(x: A): A return x end\nreturn g",
        "return `x {y} z` :: string",
    ]
}

fn tag(r#type: &Type) -> String {
    format!("{type:?}")
}

struct MutableTypes(Vec<String>);
impl NodeProcessor for MutableTypes {
    fn process_type(&mut self, r#type: &mut Type) {
        self.0.push(tag(r#type));
    }
}

struct PostTypes(Vec<String>);
impl NodeProcessor for PostTypes {}
impl NodePostProcessor for PostTypes {
    fn process_after_type(&mut self, r#type: &mut Type) {
        self.0.push(tag(r#type));
    }
}

struct ImmutableTypes(Vec<String>);
impl ImmutableProcessor for ImmutableTypes {
    fn process_type(&mut self, r#type: &Type) {
        self.0.push(tag(r#type));
    }
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn program_corpus() -> &'static [&'static str] {
    &[
        "local a, b = 1, 2\nreturn a + b",
        "if a then b() elseif c then d() else e() end",
        "while a do break end",
        "repeat a() until b",
        "for i = 1, 10, 2 do print(i) end",
        "for k, v in pairs(t) do print(k, v) end",
        "local t = { 1, 2, a = 3, [b] = 4 }",
        "local f = function(x) return x end",
        "local g = function<T>(x: T): T return x end",
        "return `a {b} c {d} e`",
        "return a and b or not c",
        "return a.b.c[1]:d()",
        "return obj:m<<T>>(1)",
        "return f<<T>>()",
        "do local x = 1 end",
        "x = 1\nx += 2\ny.x = 3\ny[1] = 4",
        "return (a + b) * c",
        "return -a, #b, not c",
        "return a :: number",
        "local s: string = 'x'\nreturn s",
        "@native function f() end",
        "return if a then 1 else 2",
        "debug.profilebegin('x')\nreturn 1",
        "function obj:method(a, ...) return a, ... end",
        "return assert(a, 'message')",
    ]
}

fn expression_tag(expression: &r4::nodes::Expression) -> String {
    format!("{expression:?}")
}

struct MutableExpressions(Vec<String>);
impl NodeProcessor for MutableExpressions {
    fn process_expression(&mut self, expression: &mut r4::nodes::Expression) {
        self.0.push(expression_tag(expression));
    }
}

struct PostExpressions(Vec<String>);
impl NodeProcessor for PostExpressions {}
impl NodePostProcessor for PostExpressions {
    fn process_after_expression(&mut self, expression: &mut r4::nodes::Expression) {
        self.0.push(expression_tag(expression));
    }
}

struct ImmutableExpressions(Vec<String>);
impl ImmutableProcessor for ImmutableExpressions {
    fn process_expression(&mut self, expression: &r4::nodes::Expression) {
        self.0.push(expression_tag(expression));
    }
}

#[test]
fn every_visitor_reaches_the_same_expressions() {
    for &code in program_corpus() {
        let mut block = Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));

        let mut mutable = MutableExpressions(Vec::new());
        DefaultVisitor::visit_block(&mut block, &mut mutable);

        let mut post = PostExpressions(Vec::new());
        DefaultPostVisitor::visit_block(&mut block, &mut post);

        let immutable_block = Parser::default().parse(code).expect("parse");
        let mut immutable = ImmutableExpressions(Vec::new());
        Walk::visit_block(&immutable_block, &mut immutable);

        assert_eq!(
            sorted(mutable.0.clone()),
            sorted(post.0),
            "the mutable and post-order visitors reached different expressions for `{code}`"
        );
        assert_eq!(
            sorted(mutable.0.clone()),
            sorted(immutable.0),
            "the mutable and read-only visitors reached different expressions for `{code}`"
        );
    }
}

#[test]
fn every_visitor_reaches_the_same_types() {
    for &code in type_corpus() {
        let mut block = Parser::default()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));

        let mut mutable = MutableTypes(Vec::new());
        DefaultVisitor::visit_block(&mut block, &mut mutable);

        let mut post = PostTypes(Vec::new());
        DefaultPostVisitor::visit_block(&mut block, &mut post);

        let immutable_block = Parser::default().parse(code).expect("parse");
        let mut immutable = ImmutableTypes(Vec::new());
        Walk::visit_block(&immutable_block, &mut immutable);

        assert!(
            !mutable.0.is_empty(),
            "no type node reached for `{code}`: {code}"
        );
        assert_eq!(
            sorted(mutable.0.clone()),
            sorted(post.0),
            "the mutable and post-order visitors reached different types for `{code}`"
        );
        assert_eq!(
            sorted(mutable.0.clone()),
            sorted(immutable.0),
            "the mutable and read-only visitors reached different types for `{code}`"
        );
    }
}

#[test]
fn method_call_type_arguments_are_visited() {
    let code = "return obj:m<<SomeType, OtherType>>()";
    let mut block = Parser::default().parse(code).expect("parse");

    let mut mutable = MutableTypes(Vec::new());
    DefaultVisitor::visit_block(&mut block, &mut mutable);
    assert!(
        mutable.0.iter().any(|r#type| r#type.contains("SomeType")),
        "{:?}",
        mutable.0
    );
    assert!(
        mutable.0.iter().any(|r#type| r#type.contains("OtherType")),
        "{:?}",
        mutable.0
    );

    let immutable_block = Parser::default().parse(code).expect("parse");
    let mut immutable = ImmutableTypes(Vec::new());
    Walk::visit_block(&immutable_block, &mut immutable);
    assert!(
        immutable.0.iter().any(|r#type| r#type.contains("SomeType")),
        "{:?}",
        immutable.0
    );
}

/// The read-only token/trivia walk must reach the tokens inside a method's
/// explicit type arguments, exactly once (the syntax walk and the node token
/// walk are disjoint).
#[test]
fn token_walk_reaches_method_type_argument_trivia() {
    use r4::nodes::TriviaKind;

    let block = Parser::default()
        .preserve_tokens()
        .parse("return obj:m<< --[[inner]] T >>()")
        .expect("parse");

    let comments: Vec<_> = r4::process::collect_trivia(&block)
        .into_iter()
        .filter(|piece| piece.kind() == TriviaKind::Comment)
        .collect();
    assert_eq!(
        comments.len(),
        1,
        "the method type argument's comment must be visited exactly once"
    );

    // The type node itself is reachable through the syntax walk.
    let mut types = ImmutableTypes(Vec::new());
    Walk::visit_block(&block, &mut types);
    assert!(
        types.0.iter().any(|r#type| r#type.contains("T")),
        "{:?}",
        types.0
    );
}
