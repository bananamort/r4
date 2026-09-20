//! Tests for the syntactic token/trivia passes.

use r4::generator::{LuaGenerator, TokenBasedLuaGenerator};
use r4::nodes::{Block, LastStatement, Position, Statement};
use r4::rules::{
    append_text_comment, remove_comments, remove_comments_where, remove_spaces,
    replace_referenced_tokens, shift_token_line, AppendLocation,
};
use r4::Parser;

fn parse(code: &str) -> Block {
    Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn generate(code: &str, block: &Block) -> String {
    let mut generator = TokenBasedLuaGenerator::new(code);
    generator.write_block(block);
    generator.into_string()
}

fn first_identifier_line(block: &Block) -> Option<usize> {
    for statement in block.iter_statements() {
        if let Statement::LocalAssign(assignment) = statement {
            if let Some(variable) = assignment.iter_variables().next() {
                return variable
                    .get_identifier()
                    .get_token()
                    .and_then(|token| token.get_line_number());
            }
        }
    }
    None
}

#[test]
fn remove_comments_strips_every_comment() {
    let code = "-- leading\nlocal x = 1 -- trailing\n-- another\nreturn x -- inline\n";
    let mut block = parse(code);

    remove_comments(&mut block);

    let output = generate(code, &block);
    assert!(!output.contains("--"), "comments must be gone: {output}");
    assert!(output.contains("local x = 1"), "{output}");
    assert!(output.contains("return x"), "{output}");
}

#[test]
fn remove_comments_where_keeps_matching_comments() {
    let code = "-- keep me\nlocal x = 1 -- drop me\n";
    let mut block = parse(code);

    remove_comments_where(&mut block, |trivia| trivia.read(code).contains("keep"));

    let output = generate(code, &block);
    assert!(output.contains("keep me"), "{output}");
    assert!(!output.contains("drop me"), "{output}");
}

#[test]
fn remove_spaces_strips_whitespace_trivia() {
    let code = "local   x   =   1\n";
    let mut block = parse(code);

    remove_spaces(&mut block);

    let output = generate(code, &block);
    assert_eq!(output, "local x=1");
}

#[test]
fn shift_token_line_moves_recorded_lines() {
    let code = "local x = 1\n";
    let mut block = parse(code);
    let before = first_identifier_line(&block).expect("the local assignment must have a line");

    shift_token_line(&mut block, 4);

    let after = first_identifier_line(&block).expect("the local assignment must have a line");
    assert_eq!(after, before + 4);
}

#[test]
fn replace_referenced_tokens_materializes_content() {
    let code = "local x = 1\n";
    let mut block = parse(code);

    replace_referenced_tokens(&mut block, code);

    let statement = block.iter_statements().next().expect("one statement");
    let Statement::LocalAssign(assignment) = statement else {
        panic!("expected a local assignment");
    };
    let token = assignment
        .iter_variables()
        .next()
        .and_then(|variable| variable.get_identifier().get_token())
        .expect("the identifier must have a token");

    assert!(
        matches!(token.get_position(), Position::LineNumber { .. }),
        "the token must be materialized, got {:?}",
        token.get_position()
    );
    assert_eq!(token.read(code), "x");
}

#[test]
fn append_text_comment_adds_start_and_end_comments() {
    let code = "return 1\n";

    let mut start = parse(code);
    append_text_comment(&mut start, "generated", AppendLocation::Start);
    let output = generate(code, &start);
    assert!(output.trim_start().starts_with("--generated"), "{output}");
    assert!(output.contains("return 1"), "{output}");

    let mut end = parse(code);
    append_text_comment(&mut end, "generated", AppendLocation::End);
    let output = generate(code, &end);
    assert!(output.trim_end().ends_with("--generated"), "{output}");
    assert!(output.contains("return 1"), "{output}");
}

#[test]
fn append_text_comment_uses_long_brackets_for_multiline_text() {
    let code = "return 1\n";
    let mut block = parse(code);
    append_text_comment(&mut block, "line one\nline two", AppendLocation::Start);

    let output = generate(code, &block);
    assert!(output.contains("--[[\nline one\nline two\n]]"), "{output}");
}

#[test]
fn formatting_passes_keep_the_last_statement_untouched() {
    let code = "local x = 1 -- comment\n";
    let mut block = parse(code);
    remove_comments(&mut block);

    assert!(block.get_last_statement().is_none());
    assert!(matches!(
        block.iter_statements().next(),
        Some(Statement::LocalAssign(_))
    ));
    // A trivially true assertion to keep the type in scope.
    let _ = LastStatement::new_break();
}

#[test]
fn token_passes_cover_every_node_kind() {
    // A rich program drives each token-bearing node through every token pass, so
    // the per-node token operations are all exercised.
    let code = r#"
        -- leading comment
        local t = { a = 1, [b] = 2, 3 } -- table
        local function f(x, ...)
            if x then
                return x + 1
            elseif x == 2 then
                return 2
            else
                return 0
            end
        end
        for i = 1, 10 do print(i) end
        for k, v in pairs(t) do print(k, v) end
        while true do break end
        repeat until true
        do local y = 1 end
        local s = `interp {x}`
        obj:method(1)
        t.field = t[1] + #t
        local a: number = 1
        type T = { a: number }
        type function id(x) return x end
        return f(1)
    "#;

    for pass in ["comments", "spaces", "shift", "replace"] {
        let mut block = parse(code);
        match pass {
            "comments" => remove_comments(&mut block),
            "spaces" => remove_spaces(&mut block),
            "shift" => shift_token_line(&mut block, -1),
            "replace" => replace_referenced_tokens(&mut block, code),
            other => panic!("unknown pass {other}"),
        }
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&block);
        let _ = generator.into_string();
    }

    // Labels and goto carry their own tokens.
    let lua52 = "::start:: -- label\ngoto start -- goto\n";
    for pass in ["comments", "spaces", "replace"] {
        let mut block = Parser::default()
            .lua52()
            .preserve_tokens()
            .parse(lua52)
            .expect("valid Lua 5.2");
        match pass {
            "comments" => remove_comments(&mut block),
            "spaces" => remove_spaces(&mut block),
            "replace" => replace_referenced_tokens(&mut block, lua52),
            other => panic!("unknown pass {other}"),
        }
        let mut generator = TokenBasedLuaGenerator::new(lua52);
        generator.write_block(&block);
        let _ = generator.into_string();
    }
}

#[test]
fn token_passes_cover_type_nodes() {
    let code = r#"
        local t: { [string]: number, x: boolean } = {} -- table type
        local f: (number, string) -> boolean = g -- function type
        local s: "literal" = x -- string type
        local a: { number } = t -- array type
        local o: number? = nil -- optional type
        local u: number | string = 1 -- union
        local i: number & string = 1 -- intersection
        local v: (...number) -> () = f -- variadic pack
        local z = f<<T, U>>() -- type instantiation
        type T<A, B = string> = { a: A } -- generics
        type U = typeof(x) -- typeof
        type V = (number) -- parenthese
        type W = T.Field -- type field
        return t
    "#;

    for pass in ["comments", "spaces", "shift", "replace"] {
        let mut block = parse(code);
        match pass {
            "comments" => remove_comments(&mut block),
            "spaces" => remove_spaces(&mut block),
            "shift" => shift_token_line(&mut block, 2),
            "replace" => replace_referenced_tokens(&mut block, code),
            other => panic!("unknown pass {other}"),
        }
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&block);
        let _ = generator.into_string();
    }
}

#[test]
fn token_passes_cover_literal_and_type_literal_tokens() {
    let code =
        "local x: true = y -- type literal\nreturn true, false, nil, ... -- value literals\n";
    for pass in ["comments", "spaces", "shift", "replace"] {
        let mut block = parse(code);
        match pass {
            "comments" => remove_comments(&mut block),
            "spaces" => remove_spaces(&mut block),
            "shift" => shift_token_line(&mut block, 1),
            "replace" => replace_referenced_tokens(&mut block, code),
            other => panic!("unknown pass {other}"),
        }
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&block);
        let _ = generator.into_string();
    }

    // The comment attached to a literal token must be removable by the filter.
    let mut block = parse(code);
    remove_comments_where(&mut block, |trivia| trivia.read(code).contains("type"));
    let output = generate(code, &block);
    assert!(output.contains("type literal"), "{output}");
    assert!(!output.contains("value literals"), "{output}");
}
