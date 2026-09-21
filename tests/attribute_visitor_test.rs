//! Tests that both visitors traverse attribute names and arguments.
//!
//! Attribute names and literal-table field names are *not* variable
//! expressions: they are delivered to `process_attributes` /
//! `process_literal_table` (and, for the non-mutating walk, to
//! `process_identifier`). Delivering them to `process_variable_expression`
//! would make every consumer treat a field name as a variable binding.

use r4::nodes::{
    Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement, AttributeTupleArguments,
    Attributes, Block, FunctionStatement, Identifier, LiteralExpression, LiteralTable, Statement,
    StringExpression,
};
use r4::process::{
    DefaultPostVisitor, DefaultVisitor, ImmutableProcessor, ImmutableVisitor, NodePostProcessor,
    NodePostVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::Parser;

struct IdentifierNames(Vec<String>);

impl NodeProcessor for IdentifierNames {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        self.0.push(identifier.get_name().clone());
    }
}

struct ImmutableIdentifierNames(Vec<String>);

impl ImmutableProcessor for ImmutableIdentifierNames {
    fn process_identifier(&mut self, identifier: &Identifier) {
        self.0.push(identifier.get_name().clone());
    }
}

fn collect_attribute_names(attributes: &Attributes, names: &mut Vec<String>) {
    for attribute in attributes.iter_attributes() {
        match attribute {
            Attribute::Name(named) => names.push(named.get_identifier().get_name().clone()),
            Attribute::Group(group) => {
                for element in group.iter_attributes() {
                    names.push(element.name().get_name().clone());
                }
            }
        }
    }
}

fn collect_field_names(table: &mut LiteralTable, names: &mut Vec<String>) {
    for entry in table.iter_mut_entries() {
        if let r4::nodes::LiteralTableEntry::Field(field) = entry {
            names.push(field.mutate_field().get_name().clone());
        }
    }
}

/// Collects attribute names and literal-table field names through the
/// container hooks (the mutable visitor).
struct ContainerNames(Vec<String>);

impl NodeProcessor for ContainerNames {
    fn process_attributes(&mut self, attributes: &mut Attributes) {
        collect_attribute_names(attributes, &mut self.0);
    }

    fn process_literal_table(&mut self, table: &mut LiteralTable) {
        collect_field_names(table, &mut self.0);
    }
}

/// Collects attribute names and literal-table field names through the
/// container hooks (the post-order visitor).
struct PostContainerNames(Vec<String>);

impl NodeProcessor for PostContainerNames {}

impl NodePostProcessor for PostContainerNames {
    fn process_after_attributes(&mut self, attributes: &mut Attributes) {
        collect_attribute_names(attributes, &mut self.0);
    }

    fn process_after_literal_table(&mut self, table: &mut LiteralTable) {
        collect_field_names(table, &mut self.0);
    }
}

fn attribute_block() -> Block {
    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(Identifier::new("k"), LiteralExpression::nil());
    literal.append_array_value(LiteralExpression::from(StringExpression::from_value(
        "deep",
    )));

    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(StringExpression::from_value(
        "tuple-string",
    )));
    tuple.push(LiteralExpression::from(literal));

    let group = AttributeGroup::new(
        AttributeGroupElement::new("tuple").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(
        AttributeGroupElement::new("table")
            .with_arguments(AttributeArguments::Table(LiteralTable::default())),
    )
    .with_attribute(AttributeGroupElement::new("string").with_arguments(
        AttributeArguments::String(StringExpression::from_value("arg")),
    ));

    let function = FunctionStatement::from_name("f", Block::default())
        .with_attributes(Attributes::new().with_attribute(Attribute::Group(group)));
    Block::new(vec![Statement::Function(Box::new(function))], None)
}

#[test]
fn mutable_visitor_visits_attribute_names_and_arguments() {
    let mut block = Parser::default()
        .parse("@native function f() end")
        .expect("valid code");

    // The function name is a variable expression; the attribute name is not.
    let mut names = IdentifierNames(Vec::new());
    DefaultVisitor::visit_block(&mut block, &mut names);
    assert!(names.0.contains(&"f".to_owned()), "{:?}", names.0);
    assert!(!names.0.contains(&"native".to_owned()), "{:?}", names.0);

    // The attribute name is reachable through the container hook.
    let mut container = ContainerNames(Vec::new());
    DefaultVisitor::visit_block(&mut block, &mut container);
    assert!(
        container.0.contains(&"native".to_owned()),
        "{:?}",
        container.0
    );

    struct Literals(Vec<String>);
    impl NodeProcessor for Literals {
        fn process_string_expression(&mut self, string: &mut StringExpression) {
            if let Some(value) = string.get_string_value() {
                self.0.push(value.to_owned());
            }
        }
        fn process_literal_expression(&mut self, _: &mut LiteralExpression) {
            self.0.push("literal".to_owned());
        }
    }

    let mut block = attribute_block();
    let mut literals = Literals(Vec::new());
    DefaultVisitor::visit_block(&mut block, &mut literals);
    assert!(
        literals.0.iter().any(|value| value == "arg"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "tuple-string"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "deep"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "literal"),
        "{:?}",
        literals.0
    );

    // Attribute group names and literal-table field names are reachable
    // through their container hooks, not as variable expressions.
    let mut container = ContainerNames(Vec::new());
    DefaultVisitor::visit_block(&mut attribute_block(), &mut container);
    for name in ["tuple", "table", "string", "k"] {
        assert!(
            container.0.contains(&name.to_owned()),
            "{name}: {:?}",
            container.0
        );
    }
}

#[test]
fn immutable_visitor_visits_attribute_names_and_arguments() {
    let block = Parser::default()
        .parse("@native function f() end")
        .expect("valid code");
    let mut names = ImmutableIdentifierNames(Vec::new());
    Walk::visit_block(&block, &mut names);
    assert!(names.0.contains(&"f".to_owned()), "{:?}", names.0);
    assert!(names.0.contains(&"native".to_owned()), "{:?}", names.0);

    struct Literals(Vec<String>);
    impl ImmutableProcessor for Literals {
        fn process_string_expression(&mut self, string: &StringExpression) {
            if let Some(value) = string.get_string_value() {
                self.0.push(value.to_owned());
            }
        }
        fn process_literal_expression(&mut self, _: &LiteralExpression) {
            self.0.push("literal".to_owned());
        }
        fn process_literal_table(&mut self, _: &LiteralTable) {
            self.0.push("table".to_owned());
        }
    }

    let mut literals = Literals(Vec::new());
    Walk::visit_block(&attribute_block(), &mut literals);
    assert!(
        literals.0.iter().any(|value| value == "arg"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "tuple-string"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "deep"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "table"),
        "{:?}",
        literals.0
    );

    let mut names = ImmutableIdentifierNames(Vec::new());
    Walk::visit_block(&attribute_block(), &mut names);
    for name in ["tuple", "table", "string", "k"] {
        assert!(names.0.contains(&name.to_owned()), "{name}: {:?}", names.0);
    }
}

#[test]
fn post_visitor_visits_attribute_names_and_arguments() {
    struct Names(Vec<String>);
    impl NodeProcessor for Names {}
    impl NodePostProcessor for Names {
        fn process_after_variable_expression(&mut self, identifier: &mut Identifier) {
            self.0.push(identifier.get_name().clone());
        }
    }

    let mut block = Parser::default()
        .parse("@native function f() end")
        .expect("valid code");
    let mut names = Names(Vec::new());
    DefaultPostVisitor::visit_block(&mut block, &mut names);
    assert!(names.0.contains(&"f".to_owned()), "{:?}", names.0);
    assert!(!names.0.contains(&"native".to_owned()), "{:?}", names.0);

    struct Literals(Vec<String>);
    impl NodeProcessor for Literals {}
    impl NodePostProcessor for Literals {
        fn process_after_string_expression(&mut self, string: &mut StringExpression) {
            if let Some(value) = string.get_string_value() {
                self.0.push(value.to_owned());
            }
        }
    }

    let mut block = attribute_block();
    let mut literals = Literals(Vec::new());
    DefaultPostVisitor::visit_block(&mut block, &mut literals);
    assert!(
        literals.0.iter().any(|value| value == "arg"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "tuple-string"),
        "{:?}",
        literals.0
    );
    assert!(
        literals.0.iter().any(|value| value == "deep"),
        "{:?}",
        literals.0
    );

    let mut container = PostContainerNames(Vec::new());
    DefaultPostVisitor::visit_block(&mut attribute_block(), &mut container);
    for name in ["tuple", "table", "string", "k"] {
        assert!(
            container.0.contains(&name.to_owned()),
            "{name}: {:?}",
            container.0
        );
    }
}
