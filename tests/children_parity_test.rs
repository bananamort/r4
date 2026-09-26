//! The mutable and read-only visitors must visit the same nodes.
//!
//! `src/nodes/children.rs` is the single source of child structure for both the
//! mutable `DefaultVisitor`/`DefaultPostVisitor` and the read-only
//! `Walk`. Before this test existed the mutable traversal silently dropped
//! every identifier that is not a variable expression (field names, method
//! names, labels, declaration names, generic parameters) and every generic
//! parameter list. This pins the parity.

use r4::nodes::{Block, Identifier, TypeName};
use r4::process::{
    DefaultVisitor, ImmutableProcessor, ImmutableVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::Parser;

#[derive(Default)]
struct MutableTrace {
    identifiers: Vec<String>,
    type_names: Vec<String>,
    statements: usize,
    expressions: usize,
    blocks: usize,
}

impl NodeProcessor for MutableTrace {
    fn process_identifier(&mut self, identifier: &mut Identifier) {
        self.identifiers.push(identifier.get_name().to_owned());
    }

    fn process_type_name(&mut self, type_name: &mut TypeName) {
        self.type_names
            .push(type_name.get_type_name().get_name().to_owned());
    }

    fn process_statement(&mut self, _: &mut r4::nodes::Statement) {
        self.statements += 1;
    }

    fn process_expression(&mut self, _: &mut r4::nodes::Expression) {
        self.expressions += 1;
    }

    fn process_block(&mut self, _: &mut Block) {
        self.blocks += 1;
    }
}

#[derive(Default)]
struct ReadOnlyTrace {
    identifiers: Vec<String>,
    type_names: Vec<String>,
    statements: usize,
    expressions: usize,
    blocks: usize,
}

impl ImmutableProcessor for ReadOnlyTrace {
    fn process_identifier(&mut self, identifier: &Identifier) {
        self.identifiers.push(identifier.get_name().to_owned());
    }

    fn process_type_name(&mut self, type_name: &TypeName) {
        self.type_names
            .push(type_name.get_type_name().get_name().to_owned());
    }

    fn process_statement(&mut self, _: &r4::nodes::Statement) {
        self.statements += 1;
    }

    fn process_expression(&mut self, _: &r4::nodes::Expression) {
        self.expressions += 1;
    }

    fn process_block(&mut self, _: &Block) {
        self.blocks += 1;
    }
}

/// Programs covering the child shapes where the traversals used to diverge.
const CORPUS: &[&str] = &[
    "local x = 1\nreturn x",
    "local x: Foo.Bar<T> = nil\nreturn x",
    "local x: number, y: string = 1, 'a'\nreturn x, y",
    "type T = number\ntype G<U> = { value: U }\nreturn 1",
    "export type Public = number\nreturn 1",
    "local function f<A, B>(p: A, q: B): (A, B)\n  return p, q\nend\nreturn f",
    "function obj.field<A>(p: A): A\n  return p\nend\nreturn obj",
    "function obj:method<A>(p: A): A\n  return p\nend\nreturn obj",
    "function a.b.c(d) end\nreturn a",
    "local f = function<A>(p: A): A return p end\nreturn f",
    "type function id<T>(value: T): T\n  return value\nend\nreturn 1",
    "return t.field, t.field2, t:method(), t:other()",
    "return require('./x').value, require('./y')()",
    "local t = { key = 1, [computed] = 2, 3 }\nreturn t.key",
    "for k, v in pairs(t) do print(k, v) end",
    "for i = 1, 10, 2 do print(i) end",
    "local a <const> = 1\nreturn a",
    "const b = 2\nreturn b",
    "local c <close> = x\nreturn c",
    "return `hello {name}!`",
    "return if a then b else c",
    "return (a + b) * c",
    "print'string argument'\nprint{ key = value }",
    "do local a = 1 end",
    "::top::\ngoto top",
    "local x = 1 :: number\nreturn x",
    "function ret(): (number, string) return 1, 'a' end\nreturn ret",
    "return a and b or not c",
    "local tbl: { [string]: number } = {}\nreturn tbl",
    "return 0b1010, 0x1p4, 1_000",
];

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

#[test]
fn mutable_and_read_only_visitors_visit_the_same_nodes() {
    for code in CORPUS {
        let mut mutable_block = parse(code);
        let mut mutable = MutableTrace::default();
        DefaultVisitor::visit_block(&mut mutable_block, &mut mutable);

        let read_only_block = parse(code);
        let mut read_only = ReadOnlyTrace::default();
        Walk::visit_block(&read_only_block, &mut read_only);

        assert_eq!(
            mutable.identifiers, read_only.identifiers,
            "identifier traversal differs for `{code}`"
        );
        assert_eq!(
            mutable.type_names, read_only.type_names,
            "type-name traversal differs for `{code}`"
        );
        assert_eq!(
            mutable.statements, read_only.statements,
            "statement traversal differs for `{code}`"
        );
        assert_eq!(
            mutable.expressions, read_only.expressions,
            "expression traversal differs for `{code}`"
        );
        assert_eq!(
            mutable.blocks, read_only.blocks,
            "block traversal differs for `{code}`"
        );
    }
}
