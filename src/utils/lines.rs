use crate::nodes::{Prefix, Statement, Token, Variable};

/// Returns the first token of a statement, when token data is available.
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::Parser;

    #[test]
    fn finds_the_first_token_for_every_statement_kind() {
        for code in [
            "x = 1",
            "do end",
            "f()",
            "x += 1",
            "function f() end",
            "for k, v in t do end",
            "if a then end",
            "local x = 1",
            "local function f() end",
            "for i = 1, 2 do end",
            "repeat until true",
            "while true do end",
            "type T = number",
            "export type T = number",
            "type function f(x) return x end",
            "export type function f(x) return x end",
        ] {
            let block = Parser::default()
                .preserve_tokens()
                .parse(code)
                .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
            let statement = block.first_statement().expect("a statement");
            assert!(
                first_statement_token(statement).is_some(),
                "no first token for `{code}`"
            );
        }

        for code in ["::label::\ngoto label", "goto label\n::label::"] {
            let block = Parser::default()
                .lua52()
                .preserve_tokens()
                .parse(code)
                .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
            let statement = block.first_statement().expect("a statement");
            assert!(
                first_statement_token(statement).is_some(),
                "no first token for `{code}`"
            );
        }

        // Prefix forms reached through a call statement.
        for code in ["a.b.c(1)", "a[1]()", "(f)()"] {
            let block = Parser::default()
                .preserve_tokens()
                .parse(code)
                .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
            let statement = block.first_statement().expect("a statement");
            assert!(
                first_statement_token(statement).is_some(),
                "no first token for `{code}`"
            );
        }
    }
}
