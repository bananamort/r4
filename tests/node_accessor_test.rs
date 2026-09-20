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
