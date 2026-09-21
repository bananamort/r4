//! Traversal parity across the three process visitors.
//!
//! The mutable, post-order and read-only visitors all derive their child
//! recursion from [`r4::nodes::MutableChildren`] / [`r4::nodes::ReadOnlyChildren`].
//! These tests drive all three over a corpus that exercises every statement,
//! expression, type, prefix, variable and child kind, and assert that each
//! visitor reaches the same multiset of nodes.
//!
//! The visitors deliberately differ in *which hook* a child fires
//! (`process_variable_expression` vs `process_identifier`, literals in
//! attribute arguments, string call arguments), so the shared categories are
//! compared three ways while the role-specific categories are compared between
//! the mutable and post-order visitors and checked against the read-only
//! visitor.

use std::mem::discriminant;

use r4::nodes::{
    Arguments, Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement,
    AttributeTupleArguments, Attributes, Block, ChildVisitor, ChildVisitorMut, Expression,
    FunctionCall, FunctionReturnType, FunctionStatement, FunctionVariadicType, GenericParameters,
    GenericParametersWithDefaults, GenericTypePack, Identifier, LastStatement, LiteralExpression,
    LiteralTable, MutableChildren, NumberExpression, Prefix, ReadOnlyChildren, Statement,
    StringExpression, StringType, TableExpression, Type, TypeName, TypePack, TypedIdentifier,
    Variable, VariadicArgumentType, VariadicTypePack,
};
use r4::process::{
    DefaultPostVisitor, DefaultVisitor, ImmutableProcessor, ImmutableVisitor, NodePostProcessor,
    NodePostVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

fn parse(code: &str, lua52: bool) -> Block {
    let parser = if lua52 {
        Parser::default().lua52()
    } else {
        Parser::default()
    };
    parser
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn luau_sources() -> &'static [&'static str] {
    &[
        // statements
        "x = 1\ny.z = 2\nz[1] = 3\nx += 1",
        "do local x = 1 end",
        "print(1)",
        "function M.f(a: number, b: string, ...: any): number return a end",
        "function M:m(x) return x end",
        "function obj.a.b:c(x, ...) return x end",
        "@native function g() end",
        "for i = 1, 10, 2 do print(i) end",
        "for i: number = 1, 10 do end",
        "for k, v in pairs(t) do print(k, v) end",
        "if a then b() elseif c then d() else e() end",
        "local a, b: number = 1, 2",
        "local function h<T>(x: T, ...: T): T return x end",
        "repeat a() until b",
        "while a do break end",
        "while a do continue end",
        "type W = T.Field",
        "type Alias<T, U...> = (T, U...) -> T & U",
        "type Default<T = string, U... = ...number> = T",
        "type function id(x: number): number return x end",
        // expressions
        "return a + b * c",
        "return (a + b) * c",
        "return -a, #b, not c",
        "return a.b.c[1]:d()",
        "return obj:m<<A, B>>(1)",
        "return f<<T>>()",
        "return a and b or c",
        "return `x {y} z {w}`",
        "return `a` .. `b`",
        "return { 1, 2, key = 3, [k] = 4 }",
        "return function<A>(x: A): A return x end",
        "local f = function(a) return a end\nreturn f",
        "return if a then 1 elseif b then 2 else 3",
        "return a :: number",
        "return 1, 'two', true, false, nil, ...",
        "return assert(a, 'message')",
        "return f'string argument'",
        "return f { key = 1 }",
        "local x = { y = z } :: { y: number }\nreturn x",
        // types
        "type T = { a: number, [string]: boolean }",
        "type T = { read x: number, write y: string }",
        "type F = (a: number, ...string) -> (boolean, number)",
        "type G = <T>(value: T) -> T",
        "type U = A | B\ntype V = C & D\ntype W = E?\nreturn 1",
        "local t: typeof(x) = y\nreturn t",
        "type A<T> = T\nlocal a: A<number> = 1\nreturn a",
        "type P = (number) -> string",
        "type Arr = { number }",
        "type S = 'literal'\ntype Str = \"str\"\ntype B = true | false | nil\nreturn 1",
    ]
}

/// A function carrying a grouped attribute whose arguments include a tuple, a
/// string and a literal table, exercising every attribute child kind.
fn attribute_group_block() -> Block {
    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(Identifier::new("k"), LiteralExpression::nil());
    literal.append_array_value(LiteralExpression::from(StringExpression::from_value(
        "deep",
    )));

    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(StringExpression::from_value("arg")));
    tuple.push(LiteralExpression::from(literal));

    let group = AttributeGroup::new(
        AttributeGroupElement::new("tuple").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(AttributeGroupElement::new("string").with_arguments(
        AttributeArguments::String(StringExpression::from_value("arg")),
    ));

    let function = FunctionStatement::from_name("f", Block::default())
        .with_attributes(Attributes::new().with_attribute(Attribute::Group(group)));
    Block::new(vec![Statement::Function(Box::new(function))], None)
}

fn corpus() -> Vec<(String, Block)> {
    let mut blocks: Vec<(String, Block)> = luau_sources()
        .iter()
        .map(|&code| (code.to_owned(), parse(code, false)))
        .collect();
    blocks.push((
        "::top::\ngoto top".to_owned(),
        parse("::top::\ngoto top", true),
    ));
    blocks.push(("grouped attribute".to_owned(), attribute_group_block()));
    blocks
}

/// Collects a tagged entry per visited node, using the mutable and read-only
/// hooks.
#[derive(Default)]
struct Collected(Vec<String>);

impl Collected {
    fn add(&mut self, category: &str, tag: impl std::fmt::Debug) {
        self.0.push(format!("{category}:{tag:?}"));
    }

    fn category(&self, category: &str) -> Vec<String> {
        let prefix = format!("{category}:");
        self.0
            .iter()
            .filter(|entry| entry.starts_with(&prefix))
            .cloned()
            .collect()
    }
}

impl NodeProcessor for Collected {
    fn process_statement(&mut self, node: &mut Statement) {
        self.add("statement", discriminant(node));
    }

    fn process_expression(&mut self, node: &mut Expression) {
        self.add("expression", discriminant(node));
    }

    fn process_type(&mut self, node: &mut Type) {
        self.add("type", discriminant(node));
    }

    fn process_prefix_expression(&mut self, node: &mut Prefix) {
        self.add("prefix", discriminant(node));
    }

    fn process_variable(&mut self, node: &mut Variable) {
        self.add("variable", discriminant(node));
    }

    fn process_variable_expression(&mut self, node: &mut Identifier) {
        self.add("identifier", node.get_name());
    }

    fn process_attributes(&mut self, node: &mut Attributes) {
        self.add("attributes", node.len());
    }

    fn process_function_call(&mut self, _: &mut FunctionCall) {
        self.add("function_call", 0);
    }

    fn process_table_expression(&mut self, node: &mut TableExpression) {
        self.add("table", node.get_entries().len());
    }

    fn process_number_expression(&mut self, _: &mut NumberExpression) {
        self.add("number_expression", 0);
    }

    fn process_string_expression(&mut self, _: &mut StringExpression) {
        self.add("string_expression", 0);
    }

    fn process_literal_expression(&mut self, node: &mut LiteralExpression) {
        self.add("literal_expression", discriminant(node));
    }

    fn process_type_pack(&mut self, node: &mut TypePack) {
        self.add("type_pack", node.len());
    }

    fn process_generic_type_pack(&mut self, node: &mut GenericTypePack) {
        self.add("generic_type_pack", node.get_name().get_name());
    }
}

impl ImmutableProcessor for Collected {
    fn process_statement(&mut self, node: &Statement) {
        self.add("statement", discriminant(node));
    }

    fn process_expression(&mut self, node: &Expression) {
        self.add("expression", discriminant(node));
    }

    fn process_type(&mut self, node: &Type) {
        self.add("type", discriminant(node));
    }

    fn process_prefix(&mut self, node: &Prefix) {
        self.add("prefix", discriminant(node));
    }

    fn process_variable(&mut self, node: &Variable) {
        self.add("variable", discriminant(node));
    }

    fn process_identifier(&mut self, node: &Identifier) {
        self.add("identifier", node.get_name());
    }

    fn process_attributes(&mut self, node: &Attributes) {
        self.add("attributes", node.len());
    }

    fn process_function_call(&mut self, _: &FunctionCall) {
        self.add("function_call", 0);
    }

    fn process_table(&mut self, node: &TableExpression) {
        self.add("table", node.get_entries().len());
    }

    fn process_number_expression(&mut self, _: &NumberExpression) {
        self.add("number_expression", 0);
    }

    fn process_string_expression(&mut self, _: &StringExpression) {
        self.add("string_expression", 0);
    }

    fn process_literal_expression(&mut self, node: &LiteralExpression) {
        self.add("literal_expression", discriminant(node));
    }
}

/// Records using the post-order hooks only. The mutable hooks are left empty so
/// a post-order run does not double-count.
#[derive(Default)]
struct PostCollected(Collected);

impl PostCollected {
    fn category(&self, category: &str) -> Vec<String> {
        self.0.category(category)
    }
}

impl NodeProcessor for PostCollected {}

impl NodePostProcessor for PostCollected {
    fn process_after_statement(&mut self, node: &mut Statement) {
        self.0.add("statement", discriminant(node));
    }

    fn process_after_expression(&mut self, node: &mut Expression) {
        self.0.add("expression", discriminant(node));
    }

    fn process_after_type(&mut self, node: &mut Type) {
        self.0.add("type", discriminant(node));
    }

    fn process_after_prefix_expression(&mut self, node: &mut Prefix) {
        self.0.add("prefix", discriminant(node));
    }

    fn process_after_variable(&mut self, node: &mut Variable) {
        self.0.add("variable", discriminant(node));
    }

    fn process_after_variable_expression(&mut self, node: &mut Identifier) {
        self.0.add("identifier", node.get_name());
    }

    fn process_after_attributes(&mut self, node: &mut Attributes) {
        self.0.add("attributes", node.len());
    }

    fn process_after_function_call(&mut self, _: &mut FunctionCall) {
        self.0.add("function_call", 0);
    }

    fn process_after_table_expression(&mut self, node: &mut TableExpression) {
        self.0.add("table", node.get_entries().len());
    }

    fn process_after_number_expression(&mut self, _: &mut NumberExpression) {
        self.0.add("number_expression", 0);
    }

    fn process_after_string_expression(&mut self, _: &mut StringExpression) {
        self.0.add("string_expression", 0);
    }

    fn process_after_literal_expression(&mut self, node: &mut LiteralExpression) {
        self.0.add("literal_expression", discriminant(node));
    }

    fn process_after_type_pack(&mut self, node: &mut TypePack) {
        self.0.add("type_pack", node.len());
    }

    fn process_after_generic_type_pack(&mut self, node: &mut GenericTypePack) {
        self.0.add("generic_type_pack", node.get_name().get_name());
    }
}

/// Categories that all three visitors surface identically.
const SHARED_CATEGORIES: &[&str] = &[
    "statement",
    "expression",
    "type",
    "prefix",
    "variable",
    "attributes",
    "function_call",
    "table",
    "number_expression",
];

/// Categories that the mutable and post-order visitors surface identically.
/// The read-only visitor uses different hooks (`process_identifier`,
/// `process_literal_expression`, `process_string_expression`).
const MUTABLE_CATEGORIES: &[&str] = &[
    "identifier",
    "literal_expression",
    "string_expression",
    "type_pack",
    "generic_type_pack",
];

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn assert_multiset_eq(left: &[String], right: &[String], category: &str, code: &str) {
    assert_eq!(
        sorted(left.to_vec()),
        sorted(right.to_vec()),
        "visitors disagree on `{category}` for `{code}`"
    );
}

fn assert_sub_multiset(subset: &[String], superset: &[String], category: &str, code: &str) {
    let mut pool = superset.to_vec();
    for value in subset {
        let Some(index) = pool.iter().position(|candidate| candidate == value) else {
            panic!("`{value}` is missing from `{category}` for `{code}`");
        };
        pool.remove(index);
    }
}

fn run_collected(block: &Block) -> (Collected, PostCollected, Collected) {
    let mut mutable_block = block.clone();
    let mut mutable = Collected::default();
    DefaultVisitor::visit_block(&mut mutable_block, &mut mutable);

    let mut post_block = block.clone();
    let mut post = PostCollected::default();
    DefaultPostVisitor::visit_block(&mut post_block, &mut post);

    let mut immutable = Collected::default();
    Walk::visit_block(block, &mut immutable);

    (mutable, post, immutable)
}

#[test]
fn visitors_reach_the_same_shared_nodes() {
    let mut seen = std::collections::BTreeSet::new();
    for (code, block) in corpus() {
        let (mutable, post, immutable) = run_collected(&block);

        for category in SHARED_CATEGORIES {
            let mutable_values = mutable.category(category);
            let post_values = post.category(category);
            let immutable_values = immutable.category(category);

            assert_multiset_eq(&mutable_values, &post_values, category, &code);
            assert_multiset_eq(&mutable_values, &immutable_values, category, &code);
            if !mutable_values.is_empty() {
                seen.insert(*category);
            }
        }
    }

    for category in SHARED_CATEGORIES {
        assert!(
            seen.contains(category),
            "the corpus never reached a `{category}` node"
        );
    }
}

#[test]
fn mutable_and_post_agree_on_every_category() {
    let mut seen = std::collections::BTreeSet::new();
    for (code, block) in corpus() {
        let (mutable, post, _) = run_collected(&block);
        for category in MUTABLE_CATEGORIES {
            let mutable_values = mutable.category(category);
            assert_multiset_eq(&mutable_values, &post.category(category), category, &code);
            if !mutable_values.is_empty() {
                seen.insert(*category);
            }
        }
    }

    for category in MUTABLE_CATEGORIES {
        assert!(
            seen.contains(category),
            "the corpus never reached a `{category}` node"
        );
    }
}

#[test]
fn every_variable_expression_reaches_the_read_only_visitor() {
    for (code, block) in corpus() {
        let (mutable, _, immutable) = run_collected(&block);
        assert_sub_multiset(
            &mutable.category("identifier"),
            &immutable.category("identifier"),
            "identifier",
            &code,
        );
    }
}

/// Recursively records every child, using the authoritative child structure.
struct Structure(Vec<String>);

macro_rules! structure_impl {
    ($trait:ident, $method:ident, $($mutability:tt)?) => {
        impl $trait for Structure {
            fn block(&mut self, node: &$($mutability)? Block) {
                self.0.push("block".into());
                node.$method(self);
            }

            fn statement(&mut self, node: &$($mutability)? Statement) {
                self.0.push(format!("statement:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn last_statement(&mut self, node: &$($mutability)? r4::nodes::LastStatement) {
                self.0.push("last_statement".into());
                node.$method(self);
            }

            fn expression(&mut self, node: &$($mutability)? Expression) {
                self.0.push(format!("expression:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn prefix(&mut self, node: &$($mutability)? Prefix) {
                self.0.push(format!("prefix:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn variable(&mut self, node: &$($mutability)? Variable) {
                self.0.push(format!("variable:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn identifier(&mut self, node: &$($mutability)? Identifier) {
                self.0.push(format!("identifier:{}", node.get_name()));
            }

            fn variable_expression(&mut self, node: &$($mutability)? Identifier) {
                self.0.push(format!("identifier:{}", node.get_name()));
            }

            fn attributes(&mut self, node: &$($mutability)? Attributes) {
                self.0.push(format!("attributes:{}", node.len()));
                node.$method(self);
            }

            fn typed_identifier(&mut self, node: &$($mutability)? TypedIdentifier) {
                self.0.push("typed_identifier".into());
                node.$method(self);
            }

            fn r#type(&mut self, node: &$($mutability)? Type) {
                self.0.push(format!("type:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn type_name(&mut self, node: &$($mutability)? TypeName) {
                self.0.push("type_name".into());
                node.$method(self);
            }

            fn type_pack(&mut self, node: &$($mutability)? TypePack) {
                self.0.push(format!("type_pack:{}", node.len()));
                node.$method(self);
            }

            fn generic_type_pack(&mut self, node: &$($mutability)? GenericTypePack) {
                self.0
                    .push(format!("generic_type_pack:{}", node.get_name().get_name()));
                node.$method(self);
            }

            fn variadic_type_pack(&mut self, node: &$($mutability)? r4::nodes::VariadicTypePack) {
                self.0.push("variadic_type_pack".into());
                node.$method(self);
            }

            fn variadic_argument_type(
                &mut self,
                node: &$($mutability)? r4::nodes::VariadicArgumentType,
            ) {
                self.0.push("variadic_argument_type".into());
                node.$method(self);
            }

            fn function_variadic_type(
                &mut self,
                node: &$($mutability)? r4::nodes::FunctionVariadicType,
            ) {
                self.0.push("function_variadic_type".into());
                node.$method(self);
            }

            fn function_return_type(
                &mut self,
                node: &$($mutability)? r4::nodes::FunctionReturnType,
            ) {
                self.0.push("function_return_type".into());
                node.$method(self);
            }

            fn generic_parameters(
                &mut self,
                node: &$($mutability)? r4::nodes::GenericParameters,
            ) {
                self.0.push(format!("generic_parameters:{}", node.len()));
                node.$method(self);
            }

            fn generic_parameters_with_defaults(
                &mut self,
                node: &$($mutability)? r4::nodes::GenericParametersWithDefaults,
            ) {
                self.0
                    .push(format!("generic_parameters_with_defaults:{}", node.len()));
                node.$method(self);
            }

            fn table(&mut self, node: &$($mutability)? TableExpression) {
                self.0.push(format!("table:{}", node.get_entries().len()));
                node.$method(self);
            }

            fn arguments(&mut self, node: &$($mutability)? Arguments) {
                self.0.push("arguments".into());
                node.$method(self);
            }

            fn string_argument(&mut self, node: &$($mutability)? StringExpression) {
                self.0.push("string_argument".into());
                node.$method(self);
            }

            fn literal_expression(&mut self, node: &$($mutability)? LiteralExpression) {
                self.0
                    .push(format!("literal_expression:{:?}", discriminant(node)));
                node.$method(self);
            }

            fn literal_argument(&mut self, node: &$($mutability)? LiteralExpression) {
                self.0.push("literal_argument".into());
                node.$method(self);
            }

            fn literal_table(&mut self, node: &$($mutability)? LiteralTable) {
                self.0.push(format!("literal_table:{}", node.len()));
                node.$method(self);
            }

            fn string_type(&mut self, node: &$($mutability)? StringType) {
                self.0.push("string_type".into());
                node.$method(self);
            }

            fn number_expression(&mut self, node: &$($mutability)? NumberExpression) {
                self.0.push("number_expression".into());
                node.$method(self);
            }

            fn string_expression(&mut self, node: &$($mutability)? StringExpression) {
                self.0.push("string_expression".into());
                node.$method(self);
            }
        }
    };
}

structure_impl!(ChildVisitorMut, for_each_child_mut, mut);
structure_impl!(ChildVisitor, for_each_child,);

#[test]
fn the_child_structure_is_one_definition() {
    for (code, block) in corpus() {
        let mut mutable_block = block.clone();
        let mut mutable = Structure(Vec::new());
        mutable_block.for_each_child_mut(&mut mutable);

        let mut immutable = Structure(Vec::new());
        block.for_each_child(&mut immutable);

        // The mutable structure cannot reach a handful of identifiers and
        // signature generics because those accessors are read-only today; the
        // read-only structure reaches them. The mutable structure must reach
        // every other child, so it is a sub-multiset of the read-only one.
        assert_sub_multiset(&mutable.0, &immutable.0, "structure", &code);
        assert!(!mutable.0.is_empty(), "no child reached for `{code}`");
    }
}

/// Collects the identifiers the authoritative read-only child walk surfaces in
/// *binding* positions: variable expressions and typed-identifier declarations.
///
/// The `ChildVisitor` callbacks are terminal, so the collector recurses through
/// every container callback to reach nested children.
#[derive(Default)]
struct BindingChildren {
    variable_expressions: Vec<Identifier>,
    declarations: Vec<Identifier>,
}

impl ChildVisitor for BindingChildren {
    fn variable_expression(&mut self, identifier: &Identifier) {
        self.variable_expressions.push(identifier.clone());
    }

    fn typed_identifier(&mut self, identifier: &TypedIdentifier) {
        self.declarations.push(identifier.get_identifier().clone());
        identifier.for_each_child(self);
    }

    fn block(&mut self, node: &Block) {
        node.for_each_child(self);
    }

    fn statement(&mut self, node: &Statement) {
        node.for_each_child(self);
    }

    fn last_statement(&mut self, node: &LastStatement) {
        node.for_each_child(self);
    }

    fn expression(&mut self, node: &Expression) {
        node.for_each_child(self);
    }

    fn prefix(&mut self, node: &Prefix) {
        node.for_each_child(self);
    }

    fn variable(&mut self, node: &Variable) {
        node.for_each_child(self);
    }

    fn attributes(&mut self, node: &Attributes) {
        node.for_each_child(self);
    }

    fn r#type(&mut self, node: &Type) {
        node.for_each_child(self);
    }

    fn type_name(&mut self, node: &TypeName) {
        node.for_each_child(self);
    }

    fn type_pack(&mut self, node: &TypePack) {
        node.for_each_child(self);
    }

    fn generic_type_pack(&mut self, node: &GenericTypePack) {
        node.for_each_child(self);
    }

    fn variadic_type_pack(&mut self, node: &VariadicTypePack) {
        node.for_each_child(self);
    }

    fn variadic_argument_type(&mut self, node: &VariadicArgumentType) {
        node.for_each_child(self);
    }

    fn function_variadic_type(&mut self, node: &FunctionVariadicType) {
        node.for_each_child(self);
    }

    fn function_return_type(&mut self, node: &FunctionReturnType) {
        node.for_each_child(self);
    }

    fn generic_parameters(&mut self, node: &GenericParameters) {
        node.for_each_child(self);
    }

    fn generic_parameters_with_defaults(&mut self, node: &GenericParametersWithDefaults) {
        node.for_each_child(self);
    }

    fn table(&mut self, node: &TableExpression) {
        node.for_each_child(self);
    }

    fn arguments(&mut self, node: &Arguments) {
        node.for_each_child(self);
    }

    fn literal_expression(&mut self, node: &LiteralExpression) {
        node.for_each_child(self);
    }

    fn literal_argument(&mut self, node: &LiteralExpression) {
        node.for_each_child(self);
    }

    fn literal_table(&mut self, node: &LiteralTable) {
        node.for_each_child(self);
    }
}

/// Guards field-level drift in the semantic analyzer.
///
/// The analyzer does not use the shared child definition (its recursion is
/// interleaved with scope and def-use bookkeeping), so a child field added to a
/// node could be missed. Every identifier the authoritative walk surfaces in a
/// binding position must resolve through the model: if the analyzer fails to
/// recurse into a field, the identifiers it contains stay unbound and this
/// fails with the offending name and snippet.
#[test]
fn semantic_walker_binds_every_binding_child() {
    let mut saw_variable_expression = false;
    for (code, block) in corpus() {
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("analysis of `{code}` must succeed: {error}"));

        let mut collector = BindingChildren::default();
        block.for_each_child(&mut collector);

        if !collector.variable_expressions.is_empty() {
            saw_variable_expression = true;
        }

        for identifier in &collector.variable_expressions {
            assert!(
                model.resolve_identifier(identifier).is_some(),
                "the analyzer left the variable expression `{}` unbound in `{code}`",
                identifier.get_name()
            );
        }

        for identifier in &collector.declarations {
            assert!(
                model.resolve_identifier(identifier).is_some(),
                "the analyzer left the declaration `{}` unbound in `{code}`",
                identifier.get_name()
            );
        }
    }

    assert!(
        saw_variable_expression,
        "the corpus never reached a variable expression"
    );
}
