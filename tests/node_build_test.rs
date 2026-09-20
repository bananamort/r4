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

#[test]
fn attributes_collection_accessors() {
    use r4::nodes::{Attribute, NamedAttribute};

    let mut attributes = Attributes::new();
    assert!(attributes.is_empty());
    assert_eq!(attributes.len(), 0);
    assert!(!attributes.has_attribute("native"));

    attributes.append_attribute(Attribute::Name(NamedAttribute::new("native")));
    assert_eq!(attributes.len(), 1);
    assert!(!attributes.is_empty());
    assert!(attributes.has_attribute("native"));
    assert!(!attributes.has_attribute("deprecated"));
    assert_eq!(attributes.iter_attributes().count(), 1);

    // A group is reachable by name through the collection.
    attributes.append_attribute(Attribute::Group(AttributeGroup::new(
        AttributeGroupElement::new("config"),
    )));
    assert!(attributes.has_attribute("config"));

    attributes.filter_attributes(|attribute| matches!(attribute, Attribute::Group(_)));
    assert_eq!(attributes.iter_attributes().count(), 1);
    assert!(attributes.has_attribute("config"));
    assert!(!attributes.has_attribute("native"));

    attributes.clear_attributes();
    assert!(attributes.is_empty());
}

#[test]
fn literal_expression_float_conversion_rejects_invalid_values() {
    assert!(LiteralExpression::try_from(f64::NAN).is_err());
    assert!(LiteralExpression::try_from(f64::INFINITY).is_err());
    assert!(LiteralExpression::try_from(-1.0).is_err());

    assert!(matches!(
        LiteralExpression::try_from(0.0).expect("zero"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(0.001).expect("small value"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1000.0).expect("round large value"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1.5).expect("normal value"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(2.5).expect("normal value"),
        LiteralExpression::Number(_)
    ));

    assert!(matches!(
        LiteralExpression::from(false),
        LiteralExpression::False(_)
    ));
    assert!(matches!(
        LiteralExpression::nil(),
        LiteralExpression::Nil(_)
    ));
    assert!(matches!(
        LiteralExpression::from(1u8),
        LiteralExpression::Number(_)
    ));
}

#[test]
fn literal_expression_numeric_conversions() {
    use r4::nodes::NumberExpression;

    // Every integer/float conversion is available.
    assert!(matches!(
        LiteralExpression::from(1u8),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(1u16),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(1u32),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1u64).expect("u64"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1i64).expect("i64"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1i32).expect("i32"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1.5f32).expect("f32"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(NumberExpression::from(r4::nodes::DecimalNumber::new(3.0))),
        LiteralExpression::Number(_)
    ));

    // Negative zero collapses to positive zero.
    assert!(LiteralExpression::try_from(-0.0).is_ok());
    // A round large value exercises the exponent-reduction loop.
    assert!(LiteralExpression::try_from(1200.0).is_ok());
    assert!(LiteralExpression::try_from(12300.0).is_ok());
    // A subnormal/small value uses the exponent branch.
    assert!(LiteralExpression::try_from(1e-5).is_ok());
    // Negative and non-finite values are rejected.
    assert!(LiteralExpression::try_from(-1.0).is_err());
    assert!(LiteralExpression::try_from(f32::NAN).is_err());
}

#[test]
fn token_operations_traverse_attribute_groups() {
    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(Identifier::new("key"), LiteralExpression::from(true));
    literal.append_array_value(LiteralExpression::nil());
    let group =
        AttributeGroup::new(AttributeGroupElement::new("a"))
            .with_attribute(AttributeGroupElement::new("b").with_arguments(
                AttributeArguments::String(StringExpression::from_value("s")),
            ))
            .with_attribute(
                AttributeGroupElement::new("c").with_arguments(AttributeArguments::Table(literal)),
            );
    let attributes = Attributes::new().with_attribute(Attribute::Group(group));
    let mut block = function_with(attributes);

    // Every token operation must reach the group (and its elements/arguments).
    r4::rules::remove_comments(&mut block);
    r4::rules::remove_spaces(&mut block);
    r4::process::detokenize(&mut block);
    let _ = r4::process::collect_trivia(&block);
    r4::rules::shift_token_line(&mut block, 1);
    r4::rules::replace_referenced_tokens(&mut block, "function f() end");
}

#[test]
fn literal_expression_remaining_conversions() {
    use r4::nodes::{BinaryNumber, Expression, HexNumber};

    assert!(matches!(
        LiteralExpression::try_from(1i16).expect("i16"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::try_from(1i8).expect("i8"),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(HexNumber::new(0x10, false)),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(BinaryNumber::new(0b1010, false)),
        LiteralExpression::Number(_)
    ));
    assert!(matches!(
        LiteralExpression::from(Option::<LiteralExpression>::None),
        LiteralExpression::Nil(_)
    ));
    assert!(matches!(
        LiteralExpression::from(Some(LiteralExpression::nil())),
        LiteralExpression::Nil(_)
    ));
    assert!(matches!(
        Expression::from(LiteralExpression::nil()),
        Expression::Nil(_)
    ));
}

#[test]
fn attribute_group_filtering_and_comment_filtering() {
    let mut group = AttributeGroup::new(AttributeGroupElement::new("keep"))
        .with_attribute(AttributeGroupElement::new("drop"));
    group.filter_attributes(|element| element.name().get_name() == "keep");
    assert_eq!(group.iter_attributes().count(), 1);
    group.filter_mut_attributes(|_| false);
    assert_eq!(group.iter_attributes().count(), 0);

    // `remove_comments_where` must reach group attributes.
    let group = AttributeGroup::new(AttributeGroupElement::new("a"));
    let mut block = function_with(Attributes::new().with_attribute(Attribute::Group(group)));
    r4::rules::remove_comments_where(&mut block, |_| true);
    r4::rules::remove_comments_where(&mut block, |_| false);
}
