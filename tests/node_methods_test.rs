//! Direct method coverage for node accessors and builders that parsing does
//! not reach.

use r4::nodes::*;
use r4::Parser;

fn number(name: &str) -> Type {
    TypeName::new(name).into()
}

#[test]
fn table_type_entry_methods() {
    use r4::nodes::{StringType, TableLiteralPropertyType};

    let literal = TableLiteralPropertyType::new(
        StringType::new("\"key\"").expect("valid string type"),
        number("number"),
    );
    let _ = literal.get_string();
    let _ = literal.get_type();
    let mut literal = literal;
    let _ = literal.mutate_string();
    let _ = literal.mutate_type();
    let _ = literal.get_modifier();
    literal = literal.with_modifier(TablePropertyModifier::Read);

    let mut table = TableType::default().with_property(TableEntryType::from(literal));
    let _ = table.iter_mut_entries().count();
    let _ = table.get_tokens();
    table.remove_tokens();
}

#[test]
fn generic_parameter_pack_methods() {
    use r4::nodes::{
        GenericParametersWithDefaults, GenericTypePack, GenericTypePackWithDefault,
        TypeVariableWithDefault,
    };

    let mut pack = GenericTypePack::new("A");
    let _ = pack.mutate_name();
    let _ = pack.get_token();
    let _ = pack.mutate_last_token();

    let mut variable = TypeVariableWithDefault::new(Identifier::new("A"), number("number"));
    let _ = variable.mutate_type_variable();
    let _ = variable.mutate_default_type();

    let mut pack_default =
        GenericTypePackWithDefault::new(GenericTypePack::new("A"), TypePack::default());
    let _ = pack_default.mutate_generic_type_pack();
    let _ = pack_default.mutate_default_type();

    let mut with_defaults =
        GenericParametersWithDefaults::from_generic_type_pack_with_default(pack_default);
    with_defaults.push_generic_type_pack_with_default(GenericTypePackWithDefault::new(
        GenericTypePack::new("B"),
        TypePack::default(),
    ));
    let _ = with_defaults.get_tokens();
}

#[test]
fn if_statement_methods() {
    use r4::nodes::{IfBranch, IfStatement};

    let statement = IfStatement::new(Vec::new(), None);
    let mut empty = IfStatement::new(
        vec![IfBranch::new(Expression::identifier("b"), Block::default())],
        None,
    );
    let _ = empty.get_tokens();
    let _ = empty.mutate_tokens();
    let _ = empty.mutate_all_blocks();
    empty.push_new_branch(Expression::identifier("c"), Block::default());
    empty.push_branch(IfBranch::new(Expression::identifier("d"), Block::default()));
    let mut branch = IfBranch::new(Expression::identifier("e"), Block::default());
    let _ = branch.take_block();
    let _ = empty.retain_branches_mut(|_| true);
    empty = empty.with_else_block(Block::default());
    empty.set_else_block(Block::default());
    let _ = statement.get_tokens();
}

#[test]
fn interpolated_string_methods() {
    use r4::nodes::{InterpolatedStringExpression, InterpolationSegment, StringSegment};

    let mut string = InterpolatedStringExpression::new(Vec::new());
    assert!(string.is_empty());
    string.push_segment(InterpolationSegment::String(
        StringSegment::new("x").expect("valid"),
    ));
    let _ = string.iter_segments().count();
    let _ = string.mutate_first_token();
    let _ = string.mutate_last_token();
    let _ = string.get_tokens();
}

#[test]
fn attribute_argument_methods() {
    use r4::nodes::{AttributeGroupElement, AttributeTupleArguments};

    let mut element = AttributeGroupElement::new("a");
    element.set_arguments(AttributeArguments::Tuple(AttributeTupleArguments::default()));
    assert!(element.get_arguments().is_some());
    element.remove_arguments();
    assert!(element.get_arguments().is_none());

    let mut attributes = Attributes::new();
    attributes.append_attribute(Attribute::Name(NamedAttribute::new("x")));
    attributes.filter_attributes(|_| true);
}

#[test]
fn local_assignment_methods() {
    use r4::nodes::{TypedIdentifier, VariableAssignment};

    let mut assignment =
        VariableAssignment::new(vec![TypedIdentifier::new("a")], vec![Expression::from(1.0)]);
    assignment.push_value(Expression::from(2.0));
    assignment.extend_values(vec![Expression::from(3.0)]);
    assignment.append_assignment(TypedIdentifier::new("b"), Expression::nil());
    assignment.for_each_assignment(|_, _| {});
    let _ = assignment.mutate_tokens();
    let _ = assignment.get_tokens();
}

#[test]
fn token_methods() {
    use r4::nodes::{Position, Token};

    let token = Token::from_position(Position::line_number("x", 3));
    let _ = token.get_line_number();
    let _ = token.get_end();
    let _ = token.get_start();
    let _ = token.try_read_with("x");
    let mut token = token;
    token.remove_tokens();
}

#[test]
fn function_expression_methods() {
    use r4::nodes::{FunctionExpression, TypedIdentifier};

    let mut function = FunctionExpression::default();
    assert!(!function.has_parameters());
    assert!(!function.is_generic());
    function = function.with_parameters(vec![TypedIdentifier::new("a")]);
    function.set_variadic(true);
    let _ = function.mutate_first_token();
    let _ = function.mutate_last_token();
    let _ = function.get_tokens();
}

#[test]
fn if_expression_methods() {
    use r4::nodes::IfExpression;

    let mut expression = IfExpression::new(
        Expression::identifier("a"),
        Expression::from(1.0),
        Expression::from(2.0),
    );
    expression.clear_elseif_branches();
    expression.retain_elseif_branches_mut(|_| true);
    let _ = expression.remove_branch(0);
    let _ = expression.mutate_first_token();
    let _ = expression.mutate_last_token();
}

#[test]
fn table_expression_methods() {
    use r4::nodes::{TableExpression, TableFieldEntry};

    let mut table = TableExpression::default();
    assert!(table.is_empty());
    let mut entry = TableFieldEntry::new(Identifier::new("a"), Expression::from(1.0));
    let _ = entry.mutate_field();
    table = table.append_entry(entry);
    {
        let entries = table.mutate_entries();
        let _ = entries.len();
    }
    let _ = table.mutate_first_token();
}

#[test]
fn type_name_methods() {
    use r4::nodes::TypeParameters;

    let type_name = TypeName::new("Base");
    let _ = type_name.get_type_parameters();
    let _ = type_name.has_type_parameters();

    let mut parameters = TypeParameters::new(number("number"));
    assert!(!parameters.is_empty());
    parameters.push_parameter(number("string"));
    let _ = parameters.iter().count();
    let _ = parameters.iter_mut().count();
    let _ = parameters.mutate_last_token();
    let _ = parameters.get_tokens();
    let _ = parameters.with_parameter(number("boolean"));
}

#[test]
fn string_and_number_methods() {
    use r4::nodes::{NumberExpression, StringExpression};

    let string = StringExpression::from_value("value");
    let _ = string.clone().into_value();
    let _ = string.into_string();

    let mut number: NumberExpression = "123".parse().expect("valid number");
    let _ = number.mutate_token();
    let _ = number.get_token();
    number = number.with_token(Token::from_content("123"));
    number.set_uppercase(true);
}

#[test]
fn type_pack_and_union_methods() {
    use r4::nodes::{TypePack, UnionType};

    let mut pack = TypePack::default();
    let _ = pack.iter_mut().count();
    let _ = pack.mutate_tokens();
    let _ = pack.mutate_last_token();
    let _ = pack.get_tokens();

    let mut union = UnionType::new(number("number"), number("string"));
    let _ = union.first_type();
    let _ = union.last_type();
    union.remove_leading_token();
    let _ = union.mutate_last_token();
}

#[test]
fn parse_is_referenced() {
    let _ = Parser::default();
}

#[test]
fn table_type_modifier_and_token_methods() {
    use r4::nodes::{
        StringType, TableEntryType, TableIndexTypeTokens, TableIndexerType,
        TableLiteralPropertyType, TablePropertyModifier, TablePropertyType,
        TablePropertyTypeTokens, TableType, TableTypeTokens, Token,
    };

    let read = TablePropertyModifier::Read;
    let write = TablePropertyModifier::Write;

    // Named property (`name: Type`).
    let mut property = TablePropertyType::new("name", number("number"));
    assert!(property.get_modifier().is_none());
    property.set_modifier(read.clone());
    property.set_modifier(read.clone()); // equal modifier: early return
    assert_eq!(property.get_modifier(), Some(&read));
    property = property.with_tokens(TablePropertyTypeTokens {
        colon: Token::from_content(":"),
        modifier: Some(Token::from_content("read")),
    });
    property.set_modifier(write.clone()); // clears the stale modifier token
    property.set_modifier(write.clone());
    property.set_modifier(read.clone());
    assert!(property.get_tokens().is_some());
    property.remove_modifier();
    property.remove_modifier();
    assert!(property.get_modifier().is_none());
    let _ = property.get_identifier();
    let _ = property.get_type();
    let _ = property.mutate_type();

    // String literal property (`"key": Type`).
    let mut literal = TableLiteralPropertyType::new(
        StringType::new("\"key\"").expect("valid string type"),
        number("number"),
    );
    literal.set_modifier(read.clone());
    literal.set_modifier(read.clone());
    literal = literal.with_tokens(TableIndexTypeTokens {
        opening_bracket: Token::from_content("["),
        closing_bracket: Token::from_content("]"),
        colon: Token::from_content(":"),
        modifier: Some(Token::from_content("read")),
    });
    literal.set_modifier(write.clone());
    literal.remove_modifier();
    assert!(literal.get_tokens().is_some());
    let _ = literal.get_string();
    let _ = literal.mutate_string();
    let _ = literal.get_type();
    let _ = literal.mutate_type();

    // Indexer (`[K]: V`).
    let mut indexer = TableIndexerType::new(number("string"), number("number"));
    indexer.set_modifier(read.clone());
    indexer.set_modifier(read.clone());
    let indexer = indexer.with_modifier(write.clone());
    assert_eq!(indexer.get_modifier(), Some(&write));
    let _ = indexer.get_key_type();
    let _ = indexer.get_value_type();

    // Entry dispatch for every variant.
    for mut entry in [
        TableEntryType::Property(property.clone()),
        TableEntryType::Literal(literal.clone()),
        TableEntryType::Indexer(TableIndexerType::new(number("string"), number("number"))),
    ] {
        entry.set_modifier(read.clone());
        entry.set_modifier(read.clone());
        entry.remove_modifier();
        let _ = entry.get_modifier();
        entry.clear_comments();
        entry.clear_whitespaces();
        entry.remove_tokens();
        let mut count = 0;
        entry.for_each_token(&mut |_| count += 1);
        let _ = count;
        let _ = entry.with_modifier(write.clone());
    }

    // The table type container.
    let mut table = TableType::default();
    assert!(table.is_empty());
    table.push_property(TableEntryType::from(property.clone()));
    assert_eq!(table.len(), 1);
    let mut table = table.with_new_property("other", number("string"));
    table = table.with_property(TableEntryType::from(property.clone()));
    // A string key becomes a literal property.
    table = table.with_indexer_type(TableIndexerType::new(number("string"), number("number")));
    // A non-string key installs (and replaces) a real indexer.
    let _ = table.set_indexer_type(TableIndexerType::new(number("number"), number("number")));
    let _ = table.set_indexer_type(TableIndexerType::new(number("number"), number("string")));
    let _ = table.has_indexer_type();
    let _ = table.iter_entries().count();
    let _ = table.iter_mut_entries().count();
    table = table.with_tokens(TableTypeTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
        separators: vec![Token::from_content(",")],
    });
    let _ = table.get_tokens();
    let _ = table.mutate_last_token();
    table.clear_comments();
    table.clear_whitespaces();
    let mut count = 0;
    table.for_each_token(&mut |_| count += 1);
    table.remove_tokens();
    let _ = count;
}

#[test]
fn literal_expression_variant_and_token_methods() {
    use r4::nodes::{
        Expression, LiteralExpression, LiteralTable, LiteralTableEntry, LiteralTableFieldEntry,
        NumberExpression, StringExpression, TableTokens, Token,
    };

    // Every LiteralExpression variant converts into an Expression.
    for variant in [
        LiteralExpression::True(None),
        LiteralExpression::False(None),
        LiteralExpression::nil(),
        LiteralExpression::Number("1".parse::<NumberExpression>().expect("number")),
        LiteralExpression::from(StringExpression::from_value("text")),
        LiteralExpression::from(LiteralTable::default()),
    ] {
        let _: Expression = variant.into();
    }
    let _: Expression = Expression::from(LiteralExpression::nil());

    // Build a field entry, then exercise its accessors.
    let mut table = LiteralTable::from_entries(Vec::new());
    table.append_field(Identifier::new("k"), LiteralExpression::nil());
    table.append_array_value(LiteralExpression::from(true));
    let field = match table.iter_entries().next().expect("one entry") {
        LiteralTableEntry::Field(field) => (**field).clone(),
        LiteralTableEntry::Value(_) => panic!("expected a field entry"),
    };
    let mut field: LiteralTableFieldEntry = field;
    assert!(field.get_token().is_none());
    field = field.with_token(Token::from_content("="));
    assert!(field.get_token().is_some());
    field.set_token(Token::from_content("="));
    let _ = field.mutate_token();
    let _ = field.get_field();
    let _ = field.mutate_field();
    let _ = field.get_value();
    let _ = field.mutate_value();

    // Entry enum conversions and token walks.
    let mut entries = vec![
        LiteralTableEntry::from(field.clone()),
        LiteralTableEntry::from_value(LiteralExpression::nil()),
    ];
    for entry in &mut entries {
        entry.clear_comments();
        entry.clear_whitespaces();
        let mut count = 0;
        entry.for_each_token(&mut |_| count += 1);
        let _ = count;
    }

    // Table accessors and tokens.
    let mut table = LiteralTable::from_entries(entries);
    let _ = table.len();
    let _ = table.is_empty();
    table = table.with_entry(LiteralTableEntry::from_value(LiteralExpression::nil()));
    table.append_entry(LiteralTableEntry::from_value(LiteralExpression::from(
        false,
    )));
    let _ = table.iter_entries().count();
    let _ = table.iter_mut_entries().count();
    table = table.with_tokens(TableTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
        separators: vec![Token::from_content(",")],
    });
    assert!(table.get_tokens().is_some());
    table.set_tokens(TableTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
        separators: Vec::new(),
    });
    let mut count = 0;
    table.for_each_token(&mut |_| count += 1);
    table.clear_comments();
    table.clear_whitespaces();
    table.remove_tokens();
    let _ = count;
}

#[test]
fn interpolated_string_segment_methods() {
    use r4::nodes::{
        InterpolatedStringExpression, InterpolatedStringTokens, InterpolationSegment,
        StringSegment, Token, ValueSegment, ValueSegmentTokens,
    };

    // String segment accessors.
    let mut segment = StringSegment::new("hello").expect("valid segment");
    assert_eq!(segment.get_string_value(), Some("hello"));
    assert!(!segment.is_empty());
    assert_eq!(segment.len(), 5);
    let _ = segment.get_value();
    segment = segment.with_token(Token::from_content("hello"));
    assert!(segment.get_token().is_some());
    segment.set_token(Token::from_content("hello"));
    let _ = format!("{segment:?}");

    // Value segment accessors.
    let mut value = ValueSegment::new(Expression::identifier("a"));
    let _ = value.get_expression();
    let _ = value.mutate_expression();
    value = value.with_tokens(ValueSegmentTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
    });
    assert!(value.get_tokens().is_some());
    value.set_tokens(ValueSegmentTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
    });

    // Segment conversions.
    let segments: Vec<InterpolationSegment> = vec![
        InterpolationSegment::from(segment.clone()),
        InterpolationSegment::from(value.clone()),
        InterpolationSegment::from("literal"),
        InterpolationSegment::from(&String::from("borrowed")),
        InterpolationSegment::from(String::from("owned")),
    ];

    // Interpolated string expression.
    let mut string = InterpolatedStringExpression::new(segments);
    assert!(!string.is_empty());
    assert_eq!(string.len(), 5);
    string = string.with_segment(StringSegment::from_value("tail"));
    let _ = string.iter_segments().count();
    let _ = string.iter_mut_segments().count();
    string = string.with_tokens(InterpolatedStringTokens {
        opening_tick: Token::from_content("`"),
        closing_tick: Token::from_content("`"),
    });
    assert!(string.get_tokens().is_some());
    string.set_tokens(InterpolatedStringTokens {
        opening_tick: Token::from_content("`"),
        closing_tick: Token::from_content("`"),
    });
    let _ = string.mutate_first_token();
    let _ = string.mutate_last_token();

    let collected: InterpolatedStringExpression =
        vec![InterpolationSegment::from("a")].into_iter().collect();
    let _ = collected;
    let _ = InterpolatedStringExpression::empty();
}

#[test]
fn generic_parameters_with_defaults_methods() {
    use r4::nodes::{
        GenericParameter, GenericParametersWithDefaults, GenericTypePack,
        GenericTypePackWithDefault, TypeVariableWithDefault,
    };

    let variable_default = || TypeVariableWithDefault::new(Identifier::new("T"), number("number"));
    let pack = GenericTypePack::new("P");
    let pack_default =
        || GenericTypePackWithDefault::new(GenericTypePack::new("Q"), TypePack::default());

    // A type variable with a default appends; mixing with a pack is rejected.
    let mut parameters = GenericParametersWithDefaults::from_type_variable("A");
    assert!(parameters.push_type_variable_with_default(variable_default()));
    assert!(parameters.push_type_variable_with_default(variable_default()));
    assert!(!parameters.push_generic_type_pack(pack.clone()));
    assert!(parameters
        .with_type_variable_with_default(variable_default())
        .is_some());

    // A generic type pack appends; mixing with a defaulted variable is rejected.
    let mut packs = GenericParametersWithDefaults::from_type_variable("A");
    assert!(packs.push_generic_type_pack(pack.clone()));
    assert!(packs.push_generic_type_pack(GenericTypePack::new("R")));
    assert!(!packs.push_type_variable_with_default(variable_default()));
    assert!(packs.with_generic_type_pack(pack.clone()).is_some());

    let mixed = GenericParametersWithDefaults::from_type_variable_with_default(variable_default());
    assert!(mixed.with_generic_type_pack(pack.clone()).is_none());

    // Type-pack defaults and the container accessors.
    let mut with_defaults =
        GenericParametersWithDefaults::from_generic_type_pack_with_default(pack_default());
    with_defaults.push_generic_type_pack_with_default(pack_default());
    with_defaults.push_type_variable("Y");
    let _ = with_defaults.len();
    let _ = with_defaults.is_empty();
    let _ = with_defaults.get_tokens();
    with_defaults = with_defaults.with_type_variable("X");
    with_defaults = with_defaults.with_generic_type_pack_with_default(pack_default());
    let _ = with_defaults.get_tokens();
    let _ = GenericParametersWithDefaults::from_generic_type_pack(pack.clone());

    // Conversions into `GenericParameter`.
    let _ = GenericParameter::from(Identifier::new("T"));
    let _ = GenericParameter::from(variable_default());
    let _ = GenericParameter::from(pack);
    let _ = GenericParameter::from(pack_default());
}

#[test]
fn type_last_token_mutation_covers_every_variant() {
    use r4::nodes::*;

    let mut types: Vec<Type> = vec![
        Type::True(None),
        Type::False(None),
        Type::Nil(None),
        Type::String(StringType::new("\"s\"").expect("valid string type")),
        Type::Array(ArrayType::new(number("number"))),
        Type::Table(TableType::default()),
        Type::TypeOf(ExpressionType::new(Expression::identifier("x"))),
        TypeName::new("Base").into(),
        TypeField::new("Mod", TypeName::new("T")).into(),
        FunctionType::new(number("number")).into(),
        OptionalType::new(number("number")).into(),
        IntersectionType::new(number("a"), number("b")).into(),
        UnionType::new(number("a"), number("b")).into(),
        ParentheseType::new(number("number")).into(),
    ];
    for r#type in &mut types {
        let _ = r#type.mutate_last_token();
    }

    // `true`/`false`/`nil` synthesize their token on first mutation.
    for mut r#type in [Type::True(None), Type::False(None), Type::Nil(None)] {
        let _ = r#type.mutate_last_token();
        let _ = r#type.mutate_last_token();
    }
}

#[test]
fn type_declaration_token_methods() {
    use r4::nodes::{
        GenericParametersWithDefaults, GenericTypePack, GenericTypePackWithDefault,
        TypeDeclarationStatement, TypePack,
    };

    let mut declaration = TypeDeclarationStatement::new("T", number("number"));
    let _ = declaration.mutate_tokens();
    declaration.clear_comments();
    declaration.clear_whitespaces();
    declaration.remove_tokens();
    declaration.for_each_token(&mut |_| {});

    // Generic type packs with defaults exercise the delegation arms.
    let parameters = GenericParametersWithDefaults::from_generic_type_pack_with_default(
        GenericTypePackWithDefault::new(GenericTypePack::new("A"), TypePack::default()),
    );
    let mut declaration =
        TypeDeclarationStatement::new("U", number("number")).with_generic_parameters(parameters);
    declaration.clear_comments();
    declaration.clear_whitespaces();
    declaration.remove_tokens();
    let _ = format!("{declaration:?}");
}

#[test]
fn attribute_arguments_methods() {
    use r4::nodes::{
        AttributeArguments, AttributeTupleArguments, LiteralExpression, LiteralTable,
        StringExpression,
    };

    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::nil());
    let tuple = tuple.with_value(LiteralExpression::from(true));

    let mut arguments: Vec<AttributeArguments> = vec![
        AttributeArguments::from(tuple),
        AttributeArguments::from(StringExpression::from_value("s")),
        AttributeArguments::from(LiteralTable::default()),
    ];
    for argument in &mut arguments {
        argument.clear_comments();
        argument.clear_whitespaces();
        argument.remove_tokens();
        let mut count = 0;
        argument.for_each_token(&mut |_| count += 1);
        let _ = count;
        let _ = format!("{argument:?}");
    }
}

#[test]
fn arguments_conversions_and_token_methods() {
    use r4::nodes::{
        Arguments, Expression, StringExpression, TableExpression, Token, TupleArguments,
        TupleArgumentsTokens,
    };

    let opening_parenthese = || Token::from_content("(");
    let closing_parenthese = || Token::from_content(")");

    // String and table arguments convert to a one-element tuple.
    let string = Arguments::String(StringExpression::from_value("s"));
    assert_eq!(string.clone().to_expressions().len(), 1);
    assert_eq!(TupleArguments::from(string.clone()).len(), 1);
    assert_eq!(string.clone().with_argument(Expression::from(1.0)).len(), 2);

    let table = Arguments::Table(TableExpression::default());
    assert_eq!(table.clone().to_expressions().len(), 1);
    assert_eq!(TupleArguments::from(table.clone()).len(), 1);
    assert_eq!(table.clone().with_argument(Expression::from(1.0)).len(), 2);

    // In-place mutation of a non-tuple converts it first.
    let mut string = string;
    string.push(Expression::from(3.0));
    string.insert(1, Expression::from(4.0));
    let _ = string.len();
    let _ = string.is_empty();
    let _ = string.mutate_last_token();
    let mut table = table;
    table.push(Expression::from(3.0));
    table.insert(1, Expression::from(4.0));

    // Tuple accessors and token passes.
    let mut tuple = TupleArguments::default().with_argument(Expression::from(1.0));
    tuple.push(Expression::from(2.0));
    tuple.insert(0, Expression::from(0.0));
    let _ = tuple.len();
    let _ = tuple.is_empty();
    let _ = tuple.iter_values().count();
    let _ = tuple.iter_mut_values().count();
    let _ = tuple.mutate_last_token();
    let _ = tuple.get_tokens();
    tuple.set_tokens(TupleArgumentsTokens {
        opening_parenthese: opening_parenthese(),
        closing_parenthese: closing_parenthese(),
        commas: Vec::new(),
    });
    tuple.clear_comments();
    tuple.clear_whitespaces();
    tuple.remove_tokens();
    tuple.for_each_token(&mut |_| {});
    let _ = tuple.clone().to_expressions();
    let _ = tuple.with_tokens(TupleArgumentsTokens {
        opening_parenthese: opening_parenthese(),
        closing_parenthese: closing_parenthese(),
        commas: vec![Token::from_content(",")],
    });
    let collected: TupleArguments = vec![Expression::from(1.0)].into_iter().collect();
    let _ = collected;
}

#[test]
fn block_statement_operations() {
    use r4::nodes::{Expression, ReturnStatement};

    let mut block = Parser::default().parse("a()\nb()\nc()").expect("parse");
    assert_eq!(block.statements_len(), 3);
    assert_eq!(block.reverse_iter_statements().count(), 3);
    block.truncate(2);
    assert_eq!(block.statements_len(), 2);
    let replaced = block.replace_last_statement(ReturnStatement::new(vec![Expression::from(1.0)]));
    assert!(replaced.is_none());
    block.clear_identity();
    assert!(block.get_identity().is_none());
    let _ = block.take_statements();
}
