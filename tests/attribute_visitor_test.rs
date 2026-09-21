//! Tests that both visitors traverse attribute names and arguments.

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
    let mut names = IdentifierNames(Vec::new());
    DefaultVisitor::visit_block(&mut block, &mut names);
    assert!(names.0.contains(&"f".to_owned()), "{:?}", names.0);
    assert!(names.0.contains(&"native".to_owned()), "{:?}", names.0);

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

    let mut names = IdentifierNames(Vec::new());
    DefaultVisitor::visit_block(&mut attribute_block(), &mut names);
    for name in ["tuple", "table", "string", "k"] {
        assert!(names.0.contains(&name.to_owned()), "{name}: {:?}", names.0);
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
    assert!(names.0.contains(&"native".to_owned()), "{:?}", names.0);

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

    let mut names = Names(Vec::new());
    DefaultPostVisitor::visit_block(&mut attribute_block(), &mut names);
    for name in ["tuple", "table", "string", "k"] {
        assert!(names.0.contains(&name.to_owned()), "{name}: {:?}", names.0);
    }
}
