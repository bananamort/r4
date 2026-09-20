//! Accessor coverage for small node modules: variables, prefixes, labels, goto
//! statements, parentheses and type nodes.

use r4::nodes::{
    Block, Expression, GotoStatement, LabelStatement, ParentheseExpression, Prefix, Statement,
    Type, Variable,
};
use r4::Parser;

fn parse(code: &str) -> Block {
    Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn parse_lua52(code: &str) -> Block {
    Parser::default()
        .lua52()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn exercise_prefix(prefix: &Prefix) {
    match prefix {
        Prefix::Call(call) => exercise_prefix(call.get_prefix()),
        Prefix::Field(field) => {
            exercise_prefix(field.get_prefix());
            let _ = field.get_field().get_name();
        }
        Prefix::Identifier(identifier) => {
            let _ = identifier.get_name();
        }
        Prefix::Index(index) => {
            exercise_prefix(index.get_prefix());
            let _ = index.get_index();
        }
        Prefix::Parenthese(parenthese) => {
            let _ = parenthese.inner_expression();
        }
        Prefix::TypeInstantiation(instantiation) => exercise_prefix(instantiation.get_prefix()),
    }
}

#[test]
fn variable_accessors() {
    for code in ["x = 1", "t.field = 1", "t[1] = 1"] {
        let mut block = parse(code);
        let statement = block.iter_mut_statements().next().expect("a statement");
        let Statement::Assign(assign) = statement else {
            panic!("expected an assignment for `{code}`");
        };
        for variable in assign.iter_mut_variables() {
            let _ = variable.mutate_first_token();
            let _ = variable.mutate_last_token();
            match variable {
                Variable::Identifier(identifier) => {
                    let _ = identifier.get_name();
                }
                Variable::Field(field) => {
                    let _ = field.get_field().get_name();
                }
                Variable::Index(index) => {
                    let _ = index.get_prefix();
                }
            }
        }

        // The `From<Identifier>` conversion is available.
        let _: Variable = Variable::from(r4::nodes::Identifier::new("converted"));
    }
}

#[test]
fn prefix_accessors() {
    for code in ["a.b.c(1)", "(f)()", "f()()", "a[1](2)", "f<<T>>()"] {
        let mut block = parse(code);
        let statement = block.iter_mut_statements().next().expect("a statement");
        let Statement::Call(call) = statement else {
            panic!("expected a call for `{code}`");
        };
        exercise_prefix(call.get_prefix());
        let mut owned = call.get_prefix().clone();
        let _ = owned.mutate_first_token();
        let _ = owned.mutate_last_token();
    }

    let _ = Prefix::from_name("g");
}

#[test]
fn label_and_goto_accessors() {
    let mut block = parse_lua52("::start::\ngoto start");
    let mut statements = block.iter_mut_statements();
    let label_statement = statements.next().expect("a label");
    let Statement::Label(label) = label_statement else {
        panic!("expected a label");
    };
    let _ = label.get_label();
    let _ = label.get_label_name();
    let _ = label.get_tokens();
    let _ = label.mutate_label();
    let _ = label.mutate_tokens();
    let _ = label.mutate_first_token();
    let _ = label.mutate_last_token();

    let goto_statement = statements.next().expect("a goto");
    let Statement::Goto(goto) = goto_statement else {
        panic!("expected a goto");
    };
    let _ = goto.get_label();
    let _ = goto.get_label_name();
    let _ = goto.get_tokens();
    let _ = goto.mutate_label();
    let _ = goto.mutate_tokens();
    let _ = goto.mutate_first_token();
    let _ = goto.mutate_last_token();

    // Constructors.
    assert_eq!(LabelStatement::from_name("l").get_label_name(), "l");
    assert_eq!(GotoStatement::from_name("l").get_label_name(), "l");
    let mut label = LabelStatement::new("l");
    let _ = label.mutate_first_token();
    let mut goto = GotoStatement::new("l");
    let _ = goto.mutate_last_token();
}

#[test]
fn parenthese_expression_accessors() {
    let expression = ParentheseExpression::new(Expression::nil());
    let _ = expression.inner_expression();
    let mut owned = expression;
    let _ = owned.mutate_inner_expression();
}

#[test]
fn type_node_accessors() {
    let code = "local x: { [string]: number, field: boolean } = t";
    let mut block = parse(code);
    let statement = block.iter_mut_statements().next().expect("a statement");
    let Statement::LocalAssign(assign) = statement else {
        panic!("expected a local assignment");
    };
    let typed = assign.iter_mut_variables().next().expect("a variable");
    let r#type = typed.get_type().expect("a type").clone();

    match &r#type {
        Type::Table(table) => {
            let _ = table.get_tokens();
            let _ = table.has_indexer_type();
            let _ = table.iter_entries().count();
            let _ = table.len();
            let _ = table.is_empty();
        }
        other => panic!("expected a table type, got {other:?}"),
    }

    let _ = Type::nil();
    let _ = r#type.in_parentheses();
}

#[test]
fn generic_parameter_accessors() {
    use r4::nodes::{
        GenericParameters, GenericParametersWithDefaults, GenericTypePack, Identifier,
    };

    let mut plain = GenericParameters::from_type_variable(Identifier::new("A"));
    assert!(!plain.is_empty());
    assert!(plain.type_variables_len() >= 1);
    assert_eq!(plain.generic_type_packs_len(), 0);
    assert_eq!(plain.iter_type_variable().count(), 1);
    let _ = plain.iter_mut_type_variable().count();
    assert_eq!(plain.iter_generic_type_pack().count(), 0);
    let _ = plain.iter_mut_generic_type_pack().count();
    plain.push_type_variable(Identifier::new("B"));
    plain.push_generic_type_pack(GenericTypePack::new("C"));
    assert_eq!(plain.generic_type_packs_len(), 1);
    let _ = plain.len();
    let _ = plain.get_tokens();
    let _ = GenericParameters::from_generic_type_pack(GenericTypePack::new("A"));

    let mut with_defaults = GenericParametersWithDefaults::from_type_variable(Identifier::new("A"));
    assert!(!with_defaults.is_empty());
    let _ = with_defaults.len();
    let _ = with_defaults.iter().count();
    let _ = with_defaults.iter_mut().count();
    let _ = with_defaults.get_tokens();

    let block = parse("type T<A, B...> = A");
    let statement = block.iter_statements().next().expect("a statement");
    let Statement::TypeDeclaration(declaration) = statement else {
        panic!("expected a type declaration");
    };
    let parsed = declaration.get_generic_parameters().expect("generics");
    assert!(!parsed.is_empty());
}

#[test]
fn type_function_statement_accessors() {
    use r4::nodes::{FunctionReturnType, FunctionVariadicType, TypeName, TypedIdentifier};

    let mut function = r4::nodes::TypeFunctionStatement::from_name("f", Block::default());
    assert!(!function.has_parameters());
    assert!(!function.is_variadic());
    assert!(function.get_parameters().is_empty());
    let _ = function.iter_parameters().count();
    let _ = function.parameters_count();
    let _ = function.get_identifier();
    let _ = function.get_block();
    let _ = function.get_tokens();
    let _ = function.get_generic_parameters();
    assert!(!function.is_exported());
    assert!(!function.has_parameter("x"));

    function = function
        .with_parameter(TypedIdentifier::new("x"))
        .variadic();
    assert!(function.is_variadic());
    assert!(function.has_parameter("x"));
    assert!(!function.has_variadic_type());
    function.set_variadic_type(FunctionVariadicType::Type(Box::new(
        TypeName::new("number").into(),
    )));
    assert!(function.has_variadic_type());
    assert!(function.get_variadic_type().is_some());
    let _ = function.mutate_variadic_type();
    function.set_return_type(FunctionReturnType::Type(Box::new(
        TypeName::new("number").into(),
    )));
    assert!(function.has_return_type());
    let _ = function.get_return_type();
    let _ = function.mutate_return_type();
    function.set_generic_parameters(r4::nodes::GenericParameters::from_type_variable(
        r4::nodes::Identifier::new("A"),
    ));
    assert!(function.get_generic_parameters().is_some());
    let _ = function.mutate_identifier();
    let _ = function.mutate_parameters();
    let _ = function.iter_mut_parameters().count();
    let _ = function.mutate_block();
    function.set_exported();
    assert!(function.is_exported());
    function.remove_exported();
    assert!(!function.is_exported());
    function = function.export();
    assert!(function.is_exported());
    let _ = function.mutate_first_token();
    let _ = function.mutate_last_token();
    let _ = function.mutate_tokens();
}

#[test]
fn if_statement_accessors() {
    let block = parse("if a then b() elseif c then d() else e() end");
    let statement = block.iter_statements().next().expect("a statement");
    let Statement::If(if_statement) = statement else {
        panic!("expected an if statement");
    };

    assert!(if_statement.branch_count() >= 1);
    for branch in if_statement.iter_branches() {
        let _ = branch.get_condition();
        let _ = branch.get_block();
    }
    let _ = if_statement.iter_branches().count();
    let _ = if_statement.get_branches();
    let _ = if_statement.get_else_block();
    let _ = if_statement.get_tokens();
}

#[test]
fn interpolated_string_accessors() {
    use r4::nodes::{InterpolatedStringExpression, InterpolationSegment};

    let block = parse("return `a {b} c`");
    let Some(r4::nodes::LastStatement::Return(statement)) = block.get_last_statement() else {
        panic!("expected a return");
    };
    let expression = statement.iter_expressions().next().expect("an expression");
    let Expression::InterpolatedString(string) = expression else {
        panic!("expected an interpolated string");
    };

    assert!(!string.is_empty());
    assert!(!string.is_empty());
    for segment in string.iter_segments() {
        match segment {
            InterpolationSegment::String(_) => {}
            InterpolationSegment::Value(_) => {}
        }
    }
    let _ = string.get_tokens();

    let mut built = InterpolatedStringExpression::new(Vec::new());
    built.push_segment(InterpolationSegment::String(
        r4::nodes::StringSegment::new("x").expect("valid segment"),
    ));
    built = built.with_segment(InterpolationSegment::String(
        r4::nodes::StringSegment::new("y").expect("valid segment"),
    ));
    let _ = built.iter_mut_segments().count();
    let _ = InterpolatedStringExpression::empty();
}
