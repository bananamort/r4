#![allow(dead_code)]

use crate::nodes::{
    Arguments, Block, Expression, FunctionCall, FunctionReturnType, LastStatement, Prefix,
    Statement, Token, Type, Variable,
};

pub(crate) fn block_total(block: &Block) -> usize {
    last_block_token(block)
        .and_then(get_token_line)
        .unwrap_or(0)
}

pub(crate) fn statement_total(statement: &Statement) -> usize {
    last_statement_token(statement)
        .and_then(get_token_line)
        .unwrap_or(0)
}

pub(crate) fn statement_first(statement: &Statement) -> usize {
    first_statement_token(statement)
        .and_then(get_token_line)
        .unwrap_or(0)
}

fn get_token_line(token: &Token) -> Option<usize> {
    token
        .iter_trailing_trivia()
        .last()
        .and_then(|trivia| {
            trivia.get_line_number().map(|line| {
                line + trivia
                    .try_read()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| *c == '\n')
                    .count()
            })
        })
        .or_else(|| token.get_line_number())
}

fn last_block_token(block: &Block) -> Option<&Token> {
    block
        .get_tokens()
        .and_then(|tokens| tokens.final_token.as_ref())
        .or_else(|| {
            block
                .get_last_statement()
                .and_then(last_last_statement_token)
                .or_else(|| {
                    block
                        .iter_statements()
                        .last()
                        .and_then(last_statement_token)
                })
        })
}

fn last_statement_token(statement: &Statement) -> Option<&Token> {
    match statement {
        Statement::Assign(assign) => assign.last_value().and_then(last_expression_token),
        Statement::Do(do_statement) => do_statement.get_tokens().map(|tokens| &tokens.end),
        Statement::Call(call) => last_call_token(call),
        Statement::CompoundAssign(assign) => last_expression_token(assign.get_value()),
        Statement::Function(function) => function.get_tokens().map(|tokens| &tokens.end),
        Statement::GenericFor(generic_for) => generic_for.get_tokens().map(|tokens| &tokens.end),
        Statement::Goto(goto_statement) => goto_statement.get_label().get_token(),
        Statement::If(if_statement) => if_statement.get_tokens().map(|tokens| &tokens.end),
        Statement::Label(label_statement) => label_statement
            .get_tokens()
            .map(|tokens| &tokens.right_colons),
        Statement::LocalAssign(local_assign) => local_assign
            .iter_values()
            .last()
            .and_then(last_expression_token)
            .or_else(|| {
                local_assign
                    .iter_variables()
                    .last()
                    .and_then(|identifier| identifier.get_token())
            }),
        Statement::LocalFunction(local_function) => {
            local_function.get_tokens().map(|tokens| &tokens.end)
        }
        Statement::NumericFor(numeric_for) => numeric_for.get_tokens().map(|tokens| &tokens.end),
        Statement::Repeat(repeat) => last_expression_token(repeat.get_condition()),
        Statement::While(while_statement) => while_statement.get_tokens().map(|tokens| &tokens.end),
        Statement::TypeDeclaration(type_declaration) => {
            last_type_token(type_declaration.get_type())
        }
        Statement::TypeFunction(type_function) => {
            type_function.get_tokens().map(|tokens| &tokens.end)
        }
    }
}

fn last_last_statement_token(last: &LastStatement) -> Option<&Token> {
    match last {
        LastStatement::Break(token) | LastStatement::Continue(token) => token.as_ref(),
        LastStatement::Return(return_statement) => return_statement
            .iter_expressions()
            .last()
            .and_then(last_expression_token)
            .or_else(|| return_statement.get_tokens().map(|tokens| &tokens.r#return)),
    }
}

fn last_expression_token(expression: &Expression) -> Option<&Token> {
    match expression {
        Expression::Binary(binary) => last_expression_token(binary.right()),
        Expression::Call(call) => last_call_token(call),
        Expression::Field(field) => field.get_field().get_token(),
        Expression::Function(function) => function.get_tokens().map(|tokens| &tokens.end),
        Expression::Identifier(identifier) => identifier.get_token(),
        Expression::If(if_expression) => last_expression_token(if_expression.get_else_result()),
        Expression::Index(index) => index.get_tokens().map(|tokens| &tokens.closing_bracket),
        Expression::Number(number) => number.get_token(),
        Expression::Parenthese(parentheses) => parentheses
            .get_tokens()
            .map(|tokens| &tokens.right_parenthese),
        Expression::String(string) => string.get_token(),
        Expression::InterpolatedString(string) => {
            string.get_tokens().map(|tokens| &tokens.closing_tick)
        }
        Expression::Table(table) => table.get_tokens().map(|tokens| &tokens.closing_brace),
        Expression::Nil(token)
        | Expression::False(token)
        | Expression::True(token)
        | Expression::VariableArguments(token) => token.as_ref(),
        Expression::Unary(unary) => last_expression_token(unary.get_expression()),
        Expression::TypeCast(type_cast) => last_type_token(type_cast.get_type()),
        Expression::TypeInstantiation(type_instantiation) => type_instantiation
            .get_tokens()
            .map(|tokens| &tokens.second_closing_list),
    }
}

fn last_type_token(r#type: &Type) -> Option<&Token> {
    match r#type {
        Type::Name(name) => {
            if let Some(type_params) = name.get_type_parameters() {
                type_params.get_tokens().map(|tokens| &tokens.closing_list)
            } else {
                name.get_type_name().get_token()
            }
        }
        Type::Field(field) => {
            if let Some(type_params) = field.get_type_name().get_type_parameters() {
                type_params.get_tokens().map(|tokens| &tokens.closing_list)
            } else {
                field.get_type_name().get_type_name().get_token()
            }
        }
        Type::True(token) | Type::False(token) | Type::Nil(token) => token.as_ref(),
        Type::String(string) => string.get_token(),
        Type::Array(array) => array.get_tokens().map(|tokens| &tokens.closing_brace),
        Type::Table(table) => table.get_tokens().map(|tokens| &tokens.closing_brace),
        Type::TypeOf(expression_type) => expression_type
            .get_tokens()
            .map(|tokens| &tokens.closing_parenthese),
        Type::Parenthese(parenthese) => parenthese
            .get_tokens()
            .map(|tokens| &tokens.right_parenthese),
        Type::Function(function) => match function.get_return_type() {
            FunctionReturnType::Type(return_type) => last_type_token(return_type),
            FunctionReturnType::TypePack(type_pack) => type_pack
                .get_tokens()
                .map(|tokens| &tokens.right_parenthese),
            FunctionReturnType::GenericTypePack(generic_pack) => generic_pack.get_token(),
            FunctionReturnType::VariadicTypePack(variadic_pack) => {
                last_type_token(variadic_pack.get_type())
            }
        },
        Type::Optional(optional) => optional.get_token(),
        Type::Intersection(intersection) => last_type_token(intersection.last_type()),
        Type::Union(union_type) => last_type_token(union_type.last_type()),
    }
}

fn last_call_token(call: &FunctionCall) -> Option<&Token> {
    match call.get_arguments() {
        Arguments::Tuple(tuple) => tuple.get_tokens().map(|tokens| &tokens.closing_parenthese),
        Arguments::String(string) => string.get_token(),
        Arguments::Table(table) => table.get_tokens().map(|tokens| &tokens.closing_brace),
    }
}

pub(crate) fn first_statement_token(statement: &Statement) -> Option<&Token> {
    match statement {
        Statement::Assign(assign) => assign
            .iter_variables()
            .next()
            .and_then(first_variable_token),
        Statement::Do(do_statement) => do_statement.get_tokens().map(|tokens| &tokens.r#do),
        Statement::Call(call) => first_prefix_token(call.get_prefix()),
        Statement::CompoundAssign(assign) => first_variable_token(assign.get_variable()),
        Statement::Function(function) => function.get_tokens().map(|tokens| &tokens.function),
        Statement::GenericFor(generic_for) => generic_for.get_tokens().map(|tokens| &tokens.r#for),
        Statement::Goto(goto_statement) => goto_statement.get_tokens().map(|tokens| &tokens.r#goto),
        Statement::If(if_statement) => if_statement.get_tokens().map(|tokens| &tokens.r#if),
        Statement::Label(label_statement) => label_statement
            .get_tokens()
            .map(|tokens| &tokens.left_colons),
        Statement::LocalAssign(local_assign) => {
            local_assign.get_tokens().map(|tokens| &tokens.keyword)
        }
        Statement::LocalFunction(local_function) => {
            local_function.get_tokens().map(|tokens| &tokens.keyword)
        }
        Statement::NumericFor(numeric_for) => numeric_for.get_tokens().map(|tokens| &tokens.r#for),
        Statement::Repeat(repeat) => repeat.get_tokens().map(|tokens| &tokens.repeat),
        Statement::While(while_statement) => {
            while_statement.get_tokens().map(|tokens| &tokens.r#while)
        }
        Statement::TypeDeclaration(type_declaration) => {
            type_declaration.get_tokens().and_then(|tokens| {
                if type_declaration.is_exported() {
                    tokens.export.as_ref()
                } else {
                    Some(&tokens.r#type)
                }
            })
        }
        Statement::TypeFunction(type_function) => type_function.get_tokens().and_then(|tokens| {
            if type_function.is_exported() {
                tokens.export.as_ref()
            } else {
                Some(&tokens.r#type)
            }
        }),
    }
}

/// Returns the first token of an expression, when token data is available.
pub(crate) fn first_expression_token(expression: &Expression) -> Option<&Token> {
    match expression {
        Expression::Binary(binary) => first_expression_token(binary.left()),
        Expression::Call(call) => first_prefix_token(call.get_prefix()),
        Expression::Field(field) => first_prefix_token(field.get_prefix()),
        Expression::Function(function) => function.get_tokens().map(|tokens| &tokens.function),
        Expression::Identifier(identifier) => identifier.get_token(),
        Expression::If(if_expression) => first_expression_token(if_expression.get_condition()),
        Expression::Index(index) => first_prefix_token(index.get_prefix()),
        Expression::Number(number) => number.get_token(),
        Expression::Parenthese(parenthese) => parenthese
            .get_tokens()
            .map(|tokens| &tokens.left_parenthese),
        Expression::String(string) => string.get_token(),
        Expression::InterpolatedString(string) => {
            string.get_tokens().map(|tokens| &tokens.opening_tick)
        }
        Expression::Table(table) => table.get_tokens().map(|tokens| &tokens.opening_brace),
        Expression::Nil(token)
        | Expression::False(token)
        | Expression::True(token)
        | Expression::VariableArguments(token) => token.as_ref(),
        Expression::Unary(unary) => unary.get_token(),
        Expression::TypeCast(type_cast) => first_expression_token(type_cast.get_expression()),
        Expression::TypeInstantiation(type_instantiation) => {
            first_prefix_token(type_instantiation.get_prefix())
        }
    }
}

fn first_variable_token(variable: &Variable) -> Option<&Token> {
    match variable {
        Variable::Identifier(identifier) => identifier.get_token(),
        Variable::Field(field_expression) => first_prefix_token(field_expression.get_prefix()),
        Variable::Index(index_expression) => first_prefix_token(index_expression.get_prefix()),
    }
}

fn first_prefix_token(prefix: &Prefix) -> Option<&Token> {
    let mut current = prefix;
    loop {
        match current {
            Prefix::Call(function_call) => {
                current = function_call.get_prefix();
            }
            Prefix::Field(field_expression) => {
                current = field_expression.get_prefix();
            }
            Prefix::Identifier(identifier) => break identifier.get_token(),
            Prefix::Index(index_expression) => {
                current = index_expression.get_prefix();
            }
            Prefix::Parenthese(parenthese_expression) => {
                break parenthese_expression
                    .get_tokens()
                    .map(|tokens| &tokens.left_parenthese)
            }
            Prefix::TypeInstantiation(type_instantiation) => {
                current = type_instantiation.get_prefix();
            }
        }
    }
}
