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
