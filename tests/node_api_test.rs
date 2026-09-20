//! Direct accessor/builder coverage for the attribute, literal-table and
//! table-type node APIs that the parser cannot produce or exercises indirectly.

use r4::nodes::{
    Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement, AttributeTupleArguments,
    Attributes, Identifier, LiteralExpression, LiteralTable, LiteralTableEntry, NamedAttribute,
    StringExpression, TableEntryType, TableIndexerType, TablePropertyType, TableType, Token, Type,
    TypeName,
};

fn number(name: &str) -> Type {
    TypeName::new(name).into()
}

#[test]
fn attributes_collection_and_group_builders() {
    let mut group = AttributeGroup::new(AttributeGroupElement::new("first"));
    group.append_attribute(AttributeGroupElement::new("second").with_arguments(
        AttributeArguments::String(StringExpression::from_value("text")),
    ));
    assert_eq!(group.iter_attributes().count(), 2);
    assert!(group.has_attribute("first"));
    assert!(group.has_attribute("second"));
    for element in group.iter_mut_attributes() {
        let _ = element.mutate_name();
    }
    let _ = group.get_tokens();

    let mut attributes = Attributes::new();
    attributes.append_attribute(Attribute::Group(group));
    attributes.append_attribute(Attribute::Name(NamedAttribute::new("native")));
    attributes.append_attribute(Attribute::Name(NamedAttribute::new("deprecated")));

    assert_eq!(attributes.len(), 3);
    assert!(attributes.has_attribute("native"));
    assert!(attributes.has_attribute("second"));

    for attribute in attributes.iter_mut_attributes() {
        attribute.clear_comments();
    }
    attributes.filter_mut_attributes(|attribute| matches!(attribute, Attribute::Name(_)));
    assert_eq!(attributes.iter_attributes().count(), 2);

    let built = Attributes::new().with_attribute(Attribute::Name(NamedAttribute::new("x")));
    assert_eq!(built.len(), 1);
}

#[test]
fn attribute_element_and_tuple_arguments_api() {
    let mut element = AttributeGroupElement::new("config");
    let _ = element.name();
    let _ = element.mutate_name();
    assert!(element.get_arguments().is_none());
    assert!(element.mutate_arguments().is_none());

    let tuple = AttributeTupleArguments::default();
    assert!(tuple.is_empty());
    let mut tuple = tuple.with_value(LiteralExpression::from(1u32));
    tuple.push(LiteralExpression::nil());
    tuple.insert(0, LiteralExpression::from(true));
    assert_eq!(tuple.len(), 3);
    for value in tuple.iter_mut_values() {
        let _ = value;
    }
    let _ = tuple.get_tokens();

    element = element.with_arguments(AttributeArguments::Tuple(tuple));
    assert!(element.get_arguments().is_some());
    assert!(element.mutate_arguments().is_some());

    // Named attribute token plumbing.
    let mut named = NamedAttribute::new("native");
    let _ = named.get_identifier();
    let _ = named.mutate_identifier();
    named.set_token(Token::from_content("@"));
    assert!(named.get_token().is_some());
    let named = NamedAttribute::new("x").with_token(Token::from_content("@"));
    assert!(named.get_token().is_some());
}

#[test]
fn literal_table_builders_and_field_entries() {
    let mut table = LiteralTable::from_entries(Vec::new());
    table.append_field(Identifier::new("a"), LiteralExpression::from(true));
    table.append_array_value(LiteralExpression::from(2u32));
    assert_eq!(table.len(), 2);
    assert!(!table.is_empty());
    let _ = table.get_tokens();

    for entry in table.iter_mut_entries() {
        match entry {
            LiteralTableEntry::Field(field) => {
                let _ = field.get_field();
                let _ = field.mutate_field();
                let _ = field.get_value();
                let _ = field.mutate_value();
                let _ = field.mutate_token();
                field.set_token(Token::from_content("="));
                assert!(field.get_token().is_some());
            }
            LiteralTableEntry::Value(_) => {}
        }
    }

    let entry = LiteralTableEntry::from_value(LiteralExpression::nil());
    let mut table = table.with_entry(entry);
    table.append_entry(LiteralTableEntry::from_value(LiteralExpression::from(
        false,
    )));
    assert_eq!(table.len(), 4);
    let _ = table.iter_entries().count();
    assert!(table
        .iter_entries()
        .any(|e| matches!(e, LiteralTableEntry::Value(_))));
}

#[test]
fn table_type_builders_and_entry_accessors() {
    let property = TablePropertyType::new("field", number("number"));
    let indexer = TableIndexerType::new(number("string"), number("boolean"));

    let mut table = TableType::default()
        .with_property(property)
        .with_new_property("other", number("string"))
        .with_indexer_type(indexer);

    assert!(table.has_indexer_type());
    assert!(!table.is_empty());
    assert!(table.len() >= 3);
    let _ = table.get_tokens();

    for entry in table.iter_mut_entries() {
        match entry {
            TableEntryType::Property(property) => {
                let _ = property.get_identifier();
                let _ = property.mutate_identifier();
                let _ = property.get_type();
                let _ = property.mutate_type();
            }
            TableEntryType::Indexer(indexer) => {
                let _ = indexer.get_key_type();
                let _ = indexer.get_value_type();
            }
            TableEntryType::Literal(literal) => {
                let _ = literal.get_string();
                let _ = literal.get_type();
            }
        }
    }

    let _ = table.iter_entries();
}

#[test]
fn token_type_modifiers_are_removable() {
    let mut property = TablePropertyType::new("field", number("number"));
    assert!(property.get_modifier().is_none());
    property.set_modifier(r4::nodes::TablePropertyModifier::Read);
    assert!(property.get_modifier().is_some());
    property.remove_modifier();
    assert!(property.get_modifier().is_none());

    let mut indexer = TableIndexerType::new(number("string"), number("number"));
    indexer.set_modifier(r4::nodes::TablePropertyModifier::Write);
    assert!(indexer.get_modifier().is_some());
    indexer.remove_modifier();
    assert!(indexer.get_modifier().is_none());
}

#[test]
fn table_type_modifier_mutations_and_tokens() {
    use r4::nodes::TablePropertyModifier::Read;

    let mut property = TablePropertyType::new("a", number("number")).with_modifier(Read);
    // Setting the same modifier again hits the early return.
    property.set_modifier(Read);
    assert!(property.get_modifier().is_some());
    property.remove_modifier();
    property.remove_modifier();

    let indexer = TableIndexerType::new(number("string"), number("number")).with_modifier(Read);
    let mut indexer = indexer;
    indexer.set_modifier(r4::nodes::TablePropertyModifier::Write);
    indexer.remove_modifier();

    let mut table = TableType::default()
        .with_property(property)
        .with_indexer_type(indexer);
    assert!(table.has_indexer_type());
    // Setting and replacing the indexer.
    table.set_indexer_type(TableIndexerType::new(number("number"), number("string")));
    assert!(table.has_indexer_type());
    // Removing all field modifiers.
    for entry in table.iter_mut_entries() {
        if let TableEntryType::Property(property) = entry {
            property.remove_modifier();
        }
    }
    let _ = table.iter_entries().count();
}

#[test]
fn attribute_group_token_mutations() {
    use r4::nodes::AttributeGroupTokens;

    let tokens = AttributeGroupTokens {
        opening_attribute_list: Token::from_content("@["),
        closing_bracket: Token::from_content("]"),
        separators: vec![Token::from_content(",")],
    };

    let mut group = AttributeGroup::new(AttributeGroupElement::new("a"));
    group.set_tokens(tokens.clone());
    assert!(group.get_tokens().is_some());
    assert!(group.mutate_tokens().is_some());
    group.append_attribute(AttributeGroupElement::new("b"));
    group.remove(0);
    assert_eq!(group.iter_attributes().count(), 1);

    let group = AttributeGroup::new(AttributeGroupElement::new("a")).with_tokens(tokens);
    assert!(group.get_tokens().is_some());
}

#[test]
fn table_type_modifiers_with_tokens() {
    use r4::nodes::TablePropertyModifier;
    use r4::Parser;

    let block = Parser::default()
        .preserve_tokens()
        .parse("type T = { read a: number, write b: string }")
        .expect("valid Luau");
    let statement = block.iter_statements().next().expect("a statement");
    let r4::nodes::Statement::TypeDeclaration(declaration) = statement else {
        panic!("expected a type declaration");
    };
    let r4::nodes::Type::Table(mut table) = declaration.get_type().clone() else {
        panic!("expected a table type");
    };

    for entry in table.iter_mut_entries() {
        if let TableEntryType::Property(property) = entry {
            // Setting the same modifier early-returns; a different one clears the
            // recorded modifier token.
            let existing = property.get_modifier().cloned();
            if let Some(modifier) = existing {
                property.set_modifier(modifier);
            }
            property.set_modifier(TablePropertyModifier::Read);
            property.set_modifier(TablePropertyModifier::Write);
            property.remove_modifier();
            assert!(property.get_modifier().is_none());
        }
    }
}
