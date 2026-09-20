//! Tests for programmatically constructed nodes that the parser cannot
//! produce, such as Luau attribute groups with arguments and literal tables.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{
    Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement, AttributeTupleArguments,
    Attributes, Block, FunctionStatement, Identifier, LiteralExpression, LiteralTable,
    LiteralTableEntry, Statement, StringExpression,
};
use r4::Equivalent;

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

fn function_with(attributes: Attributes) -> Block {
    let statement = FunctionStatement::from_name("f", Block::default()).with_attributes(attributes);
    Block::new(vec![Statement::Function(Box::new(statement))], None)
}

fn literal_table() -> LiteralTable {
    let mut table = LiteralTable::from_entries(Vec::new());
    table.append_field(Identifier::new("debug"), LiteralExpression::from(true));
    table.append_field(
        Identifier::new("label"),
        LiteralExpression::from(StringExpression::from_value("hi")),
    );
    table.append_array_value(LiteralExpression::from(1u32));
    table.append_array_value(LiteralExpression::Nil(None));
    table
}

#[test]
fn builds_an_attribute_group_with_arguments() {
    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(1u32));
    tuple.push(LiteralExpression::from(false));
    tuple.push(LiteralExpression::Nil(None));
    assert_eq!(tuple.len(), 3);
    assert!(!tuple.is_empty());

    let group = AttributeGroup::new(
        AttributeGroupElement::new("config").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(
        AttributeGroupElement::new("table_config")
            .with_arguments(AttributeArguments::Table(literal_table())),
    )
    .with_attribute(AttributeGroupElement::new("named").with_arguments(
        AttributeArguments::String(StringExpression::from_value("text")),
    ));

    let output = generate(&function_with(
        Attributes::new().with_attribute(Attribute::Group(group)),
    ));

    assert!(output.contains("@["), "{output}");
    assert!(output.contains("config(1, false, nil)"), "{output}");
    assert!(output.contains("debug=true"), "{output}");
    assert!(output.contains("label='hi'"), "{output}");
    assert!(output.contains("named"), "{output}");
}

#[test]
fn literal_table_entries_and_expressions_round_trip_through_generation() {
    let table = literal_table();
    assert_eq!(table.len(), 4);
    assert!(!table.is_empty());

    let entries: Vec<&LiteralTableEntry> = table.iter_entries().collect();
    assert!(matches!(entries[0], LiteralTableEntry::Field(_)));
    assert!(matches!(entries[2], LiteralTableEntry::Value(_)));

    // The same table used as an attribute argument must generate its values.
    let group = AttributeGroup::new(
        AttributeGroupElement::new("t").with_arguments(AttributeArguments::Table(table)),
    );
    let output = generate(&function_with(
        Attributes::new().with_attribute(Attribute::Group(group)),
    ));
    assert!(output.contains("debug=true"), "{output}");
    assert!(output.contains("label='hi'"), "{output}");
    assert!(output.contains(",1,nil}"), "{output}");
}

#[test]
fn literal_expression_conversions_are_available() {
    let from_bool = LiteralExpression::from(true);
    assert!(matches!(from_bool, LiteralExpression::True(_)));

    let from_number = LiteralExpression::from(7u32);
    assert!(matches!(from_number, LiteralExpression::Number(_)));

    let from_string = LiteralExpression::from(StringExpression::from_value("s"));
    assert!(matches!(from_string, LiteralExpression::String(_)));

    let from_table = LiteralExpression::from(literal_table());
    assert!(matches!(from_table, LiteralExpression::Table(_)));
}

#[test]
fn built_attributes_generate_and_can_be_stripped() {
    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(1u32));
    let group = AttributeGroup::new(
        AttributeGroupElement::new("a").with_arguments(AttributeArguments::Tuple(tuple)),
    );
    let attributes = Attributes::new().with_attribute(Attribute::Group(group));

    let first = function_with(attributes.clone());
    let second = function_with(attributes);
    assert!(first.is_equivalent_to(&second));

    let mut stripped = first.clone();
    let model = r4::semantic::SemanticAnalyzer::new()
        .analyze(&stripped)
        .expect("the built tree analyzes");
    r4::rules::desugar::remove_attribute(&mut stripped, &model);

    let output = generate(&stripped);
    assert!(output.contains("function f()"), "{output}");
    assert!(!output.contains('@'), "{output}");
}
