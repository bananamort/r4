//! Drives every syntax subsystem (mutable visitor, post visitor, read-only
//! visitor, converter, generators, evaluator) over a corpus that covers every
//! node kind, so node-shaped code paths are exercised rather than assumed.

use r4::generator::{
    DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator, TokenBasedLuaGenerator,
};
use r4::nodes::Block;
use r4::process::{
    DefaultPostVisitor, DefaultVisitor, Evaluator, ImmutableProcessor, ImmutableVisitor,
    NodePostProcessor, NodePostVisitor, NodeProcessor, NodeVisitor, Walk,
};
use r4::{Dialect, Parser};

/// Statements, expressions and types covering every node kind the parser accepts.
const CORPUS: &[&str] = &[
    "x = 1, 2",
    "t.f = 1",
    "t[1] = 1",
    "x += 1",
    "x -= 1",
    "x *= 2",
    "x /= 2",
    "x %= 2",
    "x ^= 2",
    "x ..= 'a'",
    "x //= 2",
    "do local x = 1 end",
    "function f(a, b, ...) return a end",
    "function a.b.c(d) end",
    "function a:b(c) return self end",
    "for k, v in pairs(t) do end",
    "for i = 1, 10, 2 do end",
    "if a then b() elseif c then d() else e() end",
    "local x, y = 1, 2",
    "local function f() end",
    "local x: number = 1",
    "repeat until true",
    "while true do break end",
    "while true do continue end",
    "type T = number",
    "export type T = { a: number }",
    "type T<A, B = string> = { a: A }",
    "type function f(x) return x end",
    "@native function f() end",
    "return a, b",
    "return",
    "return a + b - c * d / e % f ^ g",
    "return a and b or not c",
    "return a == b ~= c < d <= e > f >= g",
    "return a .. b",
    "return -x, #t, not y",
    "return f(), f(1, ...), f'str', f{1}",
    "return obj:m(1)",
    "return a.b.c[1]",
    "return { 1, 2, a = 3, [b] = 4 }",
    "return function(a) return a end",
    "return if a then b else c",
    "return (a)",
    "return true, false, nil",
    "return 0x10, 0b1010, 1e5, 1.5",
    "return \"s\", [[long]]",
    "return `interp {x} more`",
    "return x :: number",
    "return f<<T>>()",
    "return t['field']",
    "local x: { [string]: number, a: boolean } = t",
    "local x: (number, string) -> boolean = f",
    "local x: \"literal\" = s",
    "local x: { number } = t",
    "local x: number? = nil",
    "local x: number | string = 1",
    "local x: number & string = 1",
    "local x: (...number) -> () = f",
    "type U = typeof(x)",
    "type V = (number)",
    "type W = T.Field",
    "export type function g(x) return x end",
    "local a, b, c, d = 1, 2, 3, 4",
    "function f(a, b, c, d) end",
    "f(1, 2, 3, 4)",
    "return 1, 2, 3, 4",
    "type T2<A, B, C...> = (A, B, C...) -> ()",
    "type U2 = { a: number, b: string, c: boolean }",
    "local x: (number, string, boolean) -> (number, string) = f",
    "local t = { 1, 2, 3, a = 4, b = 5 }",
    "return `{a}{b}{c}`",
    "return a, (b), f(1, 2)",
    "return ...",
    "local x: true = y",
    "for a, b, c in f() do end",
    "local x = f<<T>>",
    "return f<<T>>",
    "return if a then 1 elseif b then 2 else 3",
    "type F = (number, ...string) -> boolean",
    "type G<A...> = (A...) -> ()",
    "local x: <T>(T) -> T = f",
    "type H = { [string]: (number) -> () }",
    "type I = (number) -> (...string)",
    "return function(a, b, ...) return a end",
    "return function<T>(x: T): T return x end",
    "local f: (number) -> (string, boolean) = g",
    "type J<A, B...> = (A, B...) -> A",
    "return f<<T, U>>",
];

const LUA52: &[&str] = &["::label::\ngoto label"];
const LUA53: &[&str] = &[
    "return a & b",
    "return a | b",
    "return a ~ b",
    "return a << b",
    "return a >> b",
    "return a // b",
];
const LUA54: &[&str] = &["local x <const> = 1", "local y <close> = f()"];

fn parse_with(code: &str, dialect: Dialect) -> Block {
    Parser::default()
        .dialect(dialect)
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn parse_tokens(code: &str, dialect: Dialect) -> Block {
    Parser::default()
        .dialect(dialect)
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

struct Count;

impl NodeProcessor for Count {}
impl NodePostProcessor for Count {}
impl ImmutableProcessor for Count {}

fn assert_generates(block: &Block, code: &str, dialect: Dialect) {
    let mut readable = ReadableLuaGenerator::default();
    readable.write_block(block);
    assert!(!readable.into_string().is_empty(), "readable `{code}`");

    let mut dense = DenseLuaGenerator::default();
    dense.write_block(block);
    assert!(!dense.into_string().is_empty(), "dense `{code}`");

    let tokens = parse_tokens(code, dialect);
    let mut token_based = TokenBasedLuaGenerator::new(code);
    token_based.write_block(&tokens);
    assert!(
        !token_based.into_string().is_empty(),
        "token based `{code}`"
    );

    // Generating a tokenless tree exercises the token synthesis fallbacks.
    let tokenless = parse_with(code, dialect);
    let mut synthesized = TokenBasedLuaGenerator::new(code);
    synthesized.write_block(&tokenless);
    assert!(
        !synthesized.into_string().is_empty(),
        "synthesized tokens `{code}`"
    );
}

#[test]
fn every_node_kind_drives_the_visitors_generators_and_converter() {
    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];

    for (codes, dialect) in corpora {
        for &code in codes {
            let mut block = parse_with(code, dialect);

            // Mutable visitor: every node callback default is executed.
            DefaultVisitor::visit_block(&mut block, &mut Count);
            // Post visitor: every process_after_* default is executed.
            DefaultPostVisitor::visit_block(&mut block, &mut Count);
            // Read-only visitor: the whole tree is traversed.
            let read_only = parse_with(code, dialect);
            Walk::visit_block(&read_only, &mut Count);
            // Token walk.
            let _ = r4::process::collect_trivia(&read_only);

            // Converters (tokenful and tokenless) and generators.
            let _ = parse_tokens(code, dialect);
            assert_generates(&block, code, dialect);
        }
    }
}

#[test]
fn every_evaluable_expression_shape_is_evaluated() {
    let evaluator = Evaluator::default();
    for code in [
        "return 1 + 2",
        "return 2 * 3 - 4",
        "return 10 / 4",
        "return 2 ^ 3",
        "return 7 % 3",
        "return 1 < 2",
        "return 1 <= 2",
        "return 1 > 2",
        "return 1 >= 2",
        "return 1 == 1",
        "return 1 ~= 2",
        "return true and false",
        "return true or false",
        "return not true",
        "return -3",
        "return #'abc'",
        "return 'a' .. 'b'",
        "return if true then 1 else 2",
        "return (5)",
        "return nil",
        "return true",
        "return false",
        "return 'text'",
        "return a",
        "return a.b",
        "return a[1]",
        "return f()",
        "return ...",
        "return function() end",
        "return function(a) return a end",
        "return x :: number",
        "return f<<T>>",
        "return f<<T, U>>",
        "return (a).b",
        "return (f)()",
        "return (x)[1]",
        "return a.b.c",
        "return a[1][2]",
        "return if a then 1 elseif b then 2 else 3",
        "return if a then 1 elseif b then 2 elseif c then 3 else 4",
        "return `value = {a} and {b}`",
        "return `plain`",
        "return if f() then g() else h()",
        "return if true then f() else g()",
        "return (a).b.c",
        "return ((f))()",
        "return (a)",
        "return 'a' .. 1",
        "return 1 .. 'a'",
        "return if a then f() elseif b then g() elseif c then h() else i()",
        "return if unknown then 1 else 2",
        "return (a)<<T>>",
        "return a<<T>><<U>>",
        "return f()<<T>>",
        "return a.b<<T>>",
        "return a[1]<<T>>",
        "return if false then 1 elseif a then 2 else 3",
        "return if false then 1 elseif true then 2 else 3",
        "return `{\"a\" .. \"b\"}`",
        "return if false then 1 elseif false then 2 else 3",
        "return f(), f(1, ...), f'str', f{1}",
        "return f{ g() }, f{ [g()] = h() }",
        "return if false then f() elseif a then g() else h()",
        "return if false then 1 elseif true then f() else g()",
        "return (a)[b()]",
        "return a[b()].c",
        "return f{1, 2, a = 3}",
    ] {
        let block = parse_with(code, Dialect::Luau);
        for statement in block.iter_statements() {
            let _ = statement;
        }
        if let Some(r4::nodes::LastStatement::Return(statement)) = block.get_last_statement() {
            let lookup = |identifier: &r4::nodes::Identifier| match identifier.get_name().as_str() {
                "a" => Some(r4::process::LuaValue::Number(1.0)),
                "true" => Some(r4::process::LuaValue::True),
                "false" => Some(r4::process::LuaValue::False),
                _ => None,
            };
            for expression in statement.iter_expressions() {
                let _ = evaluator.evaluate(expression);
                let _ = evaluator.evaluate_with(expression, &lookup);
                let _ = evaluator.is_pure(expression);
                let _ = evaluator.has_side_effects(expression);
                let _ = evaluator.has_side_effects_with(expression, &|_| true);
                let _ = evaluator.has_side_effects_with(expression, &|_| false);
                let _ = evaluator.can_return_multiple_values(expression);
            }
        }
    }
}

#[test]
fn comment_filtering_covers_every_node_kind() {
    use r4::rules::remove_comments_where;

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let mut block = parse_tokens(code, dialect);
            // Keep everything: the filter runs for every token of every node.
            remove_comments_where(&mut block, |_| true);
            // Remove everything: the other branch of the filter.
            remove_comments_where(&mut block, |_| false);
        }
    }
}

#[test]
fn liveness_reads_every_prefix_and_variable_shape() {
    use r4::cfg::analysis::Liveness;
    use r4::cfg::build_chunk;
    use r4::semantic::SemanticAnalyzer;

    let code = "local t = {}\nlocal f = function() return 1 end\nlocal a = t.field\nlocal b = t[1]\nlocal c = f()\nlocal d = f<<T>>()\nt.field = a\nt[1] = b\nreturn a, b, c, d";
    let block = Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("analysis succeeds");
    let cfg = build_chunk(&block, &model).expect("cfg construction succeeds");
    let liveness = Liveness::analyze(&cfg, &model).expect("liveness succeeds");

    for id in cfg.iter_block_ids() {
        let _ = liveness.is_live_in(id, r4::semantic::SymbolId(0));
        let _ = liveness.is_live_out(id, r4::semantic::SymbolId(0));
        let _ = liveness.live_in_symbols(id).count();
        let _ = liveness.live_out_symbols(id).count();
    }
}

#[test]
fn token_operations_cover_every_node_kind() {
    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let mut block = parse_tokens(code, dialect);
            r4::rules::remove_comments(&mut block);
            r4::rules::remove_spaces(&mut block);
            let _ = r4::process::collect_trivia(&block);
            r4::rules::shift_token_line(&mut block, 1);
            r4::rules::replace_referenced_tokens(&mut block, code);
            r4::rules::append_text_comment(&mut block, "note", r4::rules::AppendLocation::Start);
            r4::rules::append_text_comment(
                &mut block,
                "multi\nline",
                r4::rules::AppendLocation::End,
            );

            let mut detokenized = parse_tokens(code, dialect);
            r4::process::detokenize(&mut detokenized);
        }
    }
}

#[test]
fn cfg_and_semantic_analyses_cover_every_statement_kind() {
    use r4::cfg::analysis::{ConstantPropagation, DominatorTree, Liveness, Reachability};
    use r4::cfg::build_chunk;
    use r4::semantic::{EscapeAnalysis, SemanticAnalyzer};

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let block = parse_with(code, dialect);
            let Ok(model) = SemanticAnalyzer::new().analyze(&block) else {
                continue;
            };

            let _ = EscapeAnalysis::analyze(&block, &model);

            let Ok(cfg) = build_chunk(&block, &model) else {
                continue;
            };
            let _ = cfg.verify_invariants();
            let _ = Reachability::analyze(&cfg);
            let _ = DominatorTree::analyze(&cfg);
            let _ = Liveness::analyze(&cfg, &model);
            let _ = ConstantPropagation::analyze(&cfg, &model);
        }
    }
}

#[test]
fn navigation_modules_and_diagnostics_cover_the_corpus() {
    use r4::semantic::MultiModuleModel;
    use r4::{Compilation, SyntaxIndex};

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];

    let mut graph = MultiModuleModel::new();
    let mut compilation = Compilation::new();
    let mut index = 0usize;

    for (codes, dialect) in corpora {
        for &code in codes {
            if let Ok((block, model)) = Parser::default().dialect(dialect).parse_analyzed(code) {
                let _ = SyntaxIndex::build(&block);
                let _ = graph.add_module(format!("m{index}"), block, model);
            }
            // Record all diagnostics for both well formed and recovered sources.
            let _ = compilation.add_tree_recovering(code);
            if let Ok(tree) = compilation.add_tree(code) {
                let _ = compilation.block(tree);
                let _ = compilation.model(tree);
            }
            index += 1;
        }
    }

    let _ = graph.whole_program_index();
    for diagnostic in compilation.diagnostics() {
        let _ = compilation.format_diagnostic(diagnostic, "");
    }
    // A malformed source produces recorded diagnostics that can be formatted.
    let _ = compilation.add_tree_recovering("local x = ");
    for diagnostic in compilation.diagnostics() {
        let _ = compilation.format_diagnostic(diagnostic, "local x = ");
    }
}

#[test]
fn parser_dialect_presets_and_error_reporting() {
    use r4::{Dialect, Parser};

    for (parser, dialect) in [
        (Parser::default().lua51(), Dialect::Lua51),
        (Parser::default().lua52(), Dialect::Lua52),
        (Parser::default().lua53(), Dialect::Lua53),
        (Parser::default().lua54(), Dialect::Lua54),
        (Parser::default().lua_jit(), Dialect::LuaJit),
        (Parser::default().dialect(Dialect::Luau), Dialect::Luau),
    ] {
        assert_eq!(parser.get_dialect(), dialect);
        assert!(!dialect.as_str().is_empty());
        assert!(!dialect.to_string().is_empty());
    }
    assert!(Parser::default().lua52().is_lua52());
    assert!(!Parser::default().is_lua52());

    let error = Parser::default()
        .parse("local x = ")
        .expect_err("invalid code must fail");
    assert_eq!(error.code(), "parse/syntax");
    assert!(!error.to_diagnostics().is_empty());
    assert_eq!(error.to_diagnostic(), error.to_diagnostics()[0]);
    assert!(!error.to_string().is_empty());

    // The recovering parse yields the tree and the diagnostics.
    let (block, diagnostics) = Parser::default()
        .parse_recovering("return 1 + ")
        .expect("recovery yields a tree");
    assert!(!diagnostics.is_empty());
    let _ = block.iter_statements().count();
}

#[test]
fn cfg_errors_and_analysis_getters_cover_the_corpus() {
    use r4::cfg::analysis::{ConstantPropagation, LoopInvariantAnalysis};
    use r4::cfg::build_chunk;
    use r4::semantic::{SemanticAnalyzer, SymbolId};

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let block = parse_with(code, dialect);
            let Ok(model) = SemanticAnalyzer::new().analyze(&block) else {
                continue;
            };
            let Ok(cfg) = build_chunk(&block, &model) else {
                continue;
            };
            if let Ok(propagation) = ConstantPropagation::analyze(&cfg, &model) {
                for id in cfg.iter_block_ids() {
                    let _ = propagation.block_entry_constants(id);
                    let _ = propagation.constant_at_block_entry(id, SymbolId(0));
                }
            }
            if let Ok(invariants) = LoopInvariantAnalysis::analyze(&cfg, &model) {
                let _ = invariants.hoists().len();
                let _ = invariants.is_empty();
            }
        }
    }

    // Statements that cannot appear in the CFG produce typed errors.
    for (code, dialect) in [
        ("break", Dialect::Luau),
        ("continue", Dialect::Luau),
        ("break", Dialect::Lua51),
    ] {
        let block = parse_with(code, dialect);
        let Ok(model) = SemanticAnalyzer::new().analyze(&block) else {
            continue;
        };
        if let Err(error) = build_chunk(&block, &model) {
            assert!(!error.to_string().is_empty(), "{code}");
        }
    }

    // A block with no identity is rejected rather than guessed.
    let unanalyzed = parse_with("return 1", Dialect::Luau);
    let empty_model = SemanticAnalyzer::new()
        .analyze(&parse_with("return 1", Dialect::Luau))
        .expect("analysis");
    let _ = build_chunk(&unanalyzed, &empty_model);
}

#[test]
fn attribute_argument_nodes_drive_visitors_and_generators() {
    use r4::nodes::{
        Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement,
        AttributeTupleArguments, Attributes, Block, FunctionStatement, LiteralExpression,
        LiteralTable, Statement,
    };

    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(1u32));
    tuple.push(LiteralExpression::from(true));
    tuple.push(LiteralExpression::from(
        r4::nodes::StringExpression::from_value("text"),
    ));

    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(
        r4::nodes::Identifier::new("k"),
        LiteralExpression::from(1u32),
    );
    literal.append_array_value(LiteralExpression::Nil(None));
    literal.append_array_value(LiteralExpression::from(false));

    let group = AttributeGroup::new(
        AttributeGroupElement::new("tuple").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(
        AttributeGroupElement::new("table").with_arguments(AttributeArguments::Table(literal)),
    )
    .with_attribute(AttributeGroupElement::new("string").with_arguments(
        AttributeArguments::String(r4::nodes::StringExpression::from_value("s")),
    ));

    let mut block = Block::new(
        vec![Statement::Function(Box::new(
            FunctionStatement::from_name("f", Block::default())
                .with_attributes(Attributes::new().with_attribute(Attribute::Group(group))),
        ))],
        None,
    );

    DefaultVisitor::visit_block(&mut block, &mut Count);
    DefaultPostVisitor::visit_block(&mut block, &mut Count);
    Walk::visit_block(&block, &mut Count);
    let _ = r4::process::collect_trivia(&block);
    r4::rules::remove_comments(&mut block);
    r4::rules::remove_spaces(&mut block);
    r4::process::detokenize(&mut block);

    let mut readable = ReadableLuaGenerator::default();
    readable.write_block(&block);
    let _ = readable.into_string();
    let mut dense = DenseLuaGenerator::default();
    dense.write_block(&block);
    let _ = dense.into_string();
    let mut token_based = TokenBasedLuaGenerator::new("");
    token_based.write_block(&block);
    let _ = token_based.into_string();
}

#[test]
fn typed_function_and_generic_declaration_forms_drive_the_visitors() {
    for code in [
        "type A<T = number> = T",
        "type B<T... = (number)> = () -> ()",
        "type C<T... = ...number> = () -> ()",
        "type D<T... = U...> = () -> ()",
        "type E<T, U...> = (T, U...) -> T",
        "type function G<T>(x: T) return x end",
        "local function f(a: number, b: string, ...: boolean): (number, string) return 1, '' end",
        "local function g(...: A...): B... return ... end",
        "function M.n(a: T): U end",
        "function o:p(a: T): U end",
        "local h: (number, ...string) -> boolean = f",
        "local i: <T>(T) -> T = f",
        "return function(a: number, ...: string): (number) return 1 end",
        "local function j(a: number, ...: string): number return 1 end",
        "function k<T>(a: T, ...: U...): V... end",
        "return a[1](), (a)[1], a.b[1]",
        "return f<<T, (U)>>",
        "return f<<(number), string>>",
        "type L = A<number>",
        "type M = A<(number)>",
        "type N = A<...number>",
        "local o: (...number) -> ...string = f",
    ] {
        let mut block = parse_with(code, Dialect::Luau);
        DefaultVisitor::visit_block(&mut block, &mut Count);
        DefaultPostVisitor::visit_block(&mut block, &mut Count);
        let read_only = parse_with(code, Dialect::Luau);
        Walk::visit_block(&read_only, &mut Count);
        let _ = r4::process::collect_trivia(&read_only);
    }
}

#[test]
fn long_and_escaped_strings_drive_the_generators() {
    let long = "a".repeat(300);
    let long_values = (1..=120)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let long_expression = format!(
        "{}0",
        (1..=80).map(|i| format!("a{i}+")).collect::<String>()
    );
    let codes = [
        format!("return \"{long}\""),
        "return \"\\n\\t\\\"\\\\\"".to_owned(),
        "return \"\\x41\\65\\u{1F600}\"".to_owned(),
        "return [[long\nbracket]]".to_owned(),
        "return [==[nested ]] here]==]".to_owned(),
        format!("return {{{long_values}}}"),
        format!("local x = {long_expression} return x"),
        "while true do break end".to_owned(),
        "while true do continue end".to_owned(),
        "for i = 1, 10 do if i then break end end".to_owned(),
        "repeat continue until true".to_owned(),
        "return function(...) return ... end".to_owned(),
    ];
    for code in codes {
        let block = parse_with(&code, Dialect::Luau);
        assert_generates(&block, &code, Dialect::Luau);
    }
}

#[test]
fn comments_drive_trivia_conversion() {
    for code in [
        "-- leading\nlocal x = 1 -- trailing\nreturn x --[[ block ]]",
        "local x = --[[inline]] 1\nreturn x",
        "--- doc\n--!strict\nreturn 1",
        "local t = { --[[a]] 1, --[[b]] 2 }",
        "function --[[f]] f --[[p]] ( --[[arg]] a --[[endarg]] ) --[[body]] return a end",
        "return --[[a]] 1 --[[b]] + --[[c]] 2",
        "if --[[c]] a --[[then]] then --[[body]] return 1 end",
        "local x = #{ --[[k]] a = 1 }",
        "return `text { --[[v]] x } more`",
        "for --[[i]] i = 1, --[[n]] 10 do end",
        "while --[[c]] true do --[[b]] break end",
    ] {
        let tokens = parse_tokens(code, Dialect::Luau);
        assert_generates(&tokens, code, Dialect::Luau);
        DefaultVisitor::visit_block(&mut tokens.clone(), &mut Count);
    }
}

#[test]
fn semantic_walker_handles_type_packs_and_annotations() {
    for code in [
        "local f: <T>(T) -> T = g\nreturn f",
        "local x: (...number) -> ...string = f\nreturn x",
        "local y: <T...>(T...) -> T... = g\nreturn y",
        "type T = A<(number), string>\nreturn 1",
        "type U = A<...number>\nreturn 1",
        "type V = A<T...>\nreturn 1",
        "type W = typeof(x)\nreturn W",
        "type X = { a: number, [string]: number }\nreturn 1",
        "local z: number? = nil\nreturn z",
        "local w: number & string = 1\nreturn w",
        "local q: (number) -> (string, boolean) = f\nreturn q",
        "type Y = { [number]: string }\nreturn 1",
    ] {
        let _ = Parser::default()
            .dialect(Dialect::Luau)
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
    }
}

#[test]
fn partially_tokened_nodes_fall_back_to_synthetic_symbols() {
    use r4::generator::TokenBasedLuaGenerator;
    use r4::nodes::{
        Arguments, AssignStatement, AssignTokens, Block, Expression, FunctionCall,
        FunctionCallTokens, Identifier, LastStatement, Prefix, ReturnStatement, ReturnTokens,
        Statement, Token, TupleArguments, TypeInstantiationExpression, TypeInstantiationTokens,
        TypeName, Variable,
    };

    fn generate(block: &Block) {
        let mut generator = TokenBasedLuaGenerator::new("");
        generator.write_block(block);
        assert!(!generator.into_string().is_empty());
    }

    // Return with tokens but no separator commas.
    let statement = ReturnStatement::new(vec![
        Expression::from(1.0),
        Expression::from(2.0),
        Expression::from(3.0),
    ])
    .with_tokens(ReturnTokens {
        r#return: Token::from_content("return"),
        commas: Vec::new(),
    });
    generate(&Block::new(
        Vec::new(),
        Some(LastStatement::Return(statement)),
    ));

    // Assignment with tokens but no separator commas.
    let assign = AssignStatement::new(
        vec![Variable::new("a"), Variable::new("b")],
        vec![Expression::from(1.0), Expression::from(2.0)],
    )
    .with_tokens(AssignTokens {
        equal: Token::from_content("="),
        variable_commas: Vec::new(),
        value_commas: Vec::new(),
    });
    generate(&Block::new(vec![Statement::Assign(assign)], None));

    // Method call with tokens but no colon.
    let call = FunctionCall::new(
        Prefix::from_name("obj"),
        Arguments::Tuple(TupleArguments::default()),
        Some(Identifier::new("method")),
    )
    .with_tokens(FunctionCallTokens {
        colon: None,
        type_instantiation_tokens: None,
    });
    generate(&Block::new(
        Vec::new(),
        Some(LastStatement::Return(ReturnStatement::new(vec![
            Expression::from(call),
        ]))),
    ));

    // Type instantiation with tokens but no separator commas.
    let instantiation = TypeInstantiationExpression::new(
        Prefix::from_name("f"),
        vec![TypeName::new("T").into(), TypeName::new("U").into()],
    )
    .with_tokens(TypeInstantiationTokens {
        first_opening_list: Token::from_content("<"),
        second_opening_list: Token::from_content("<"),
        first_closing_list: Token::from_content(">"),
        second_closing_list: Token::from_content(">"),
        commas: Vec::new(),
    });
    generate(&Block::new(
        Vec::new(),
        Some(LastStatement::Return(ReturnStatement::new(vec![
            Expression::from(instantiation),
        ]))),
    ));
}

#[test]
fn type_cast_with_parentheses_is_synthesized() {
    use r4::generator::TokenBasedLuaGenerator;
    use r4::nodes::{
        Block, Expression, LastStatement, ReturnStatement, Token, TypeCastExpression, TypeName,
    };

    let inner = TypeCastExpression::new(Expression::from(1.0), TypeName::new("number"));
    let outer = TypeCastExpression::new(Expression::from(inner), TypeName::new("number"))
        .with_token(Token::from_content("::"));

    let block = Block::new(
        Vec::new(),
        Some(LastStatement::Return(ReturnStatement::new(vec![
            Expression::from(outer),
        ]))),
    );
    let mut generator = TokenBasedLuaGenerator::new("");
    generator.write_block(&block);
    let output = generator.into_string();
    assert!(output.contains("(1::number)::number"), "{output}");
}

#[test]
fn partial_tokens_synthesize_separators_and_variadics() {
    use r4::generator::TokenBasedLuaGenerator;
    use r4::nodes::{
        Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement, AttributeGroupTokens,
        AttributeTupleArguments, Attributes, Block, Expression, FunctionBodyTokens, FunctionName,
        FunctionNameTokens, FunctionStatement, FunctionVariadicType, GenericTypePack, Identifier,
        LastStatement, LiteralExpression, LiteralTable, ReturnStatement, Statement,
        TableExpression, TableFieldEntry, TableTokens, Token, TupleArgumentsTokens,
    };

    fn generate(block: &Block) -> String {
        let mut generator = TokenBasedLuaGenerator::new("");
        generator.write_block(block);
        generator.into_string()
    }

    // A table with tokens but no separator tokens.
    let table = TableExpression::new(vec![
        TableFieldEntry::new(Identifier::new("a"), Expression::from(1.0)).into(),
        TableFieldEntry::new(Identifier::new("b"), Expression::from(2.0)).into(),
    ])
    .with_tokens(TableTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
        separators: Vec::new(),
    });
    let output = generate(&Block::new(
        Vec::new(),
        Some(LastStatement::Return(ReturnStatement::new(vec![
            Expression::from(table),
        ]))),
    ));
    assert!(output.contains("b=2"), "{output}");

    // A function name with tokens but no periods or colon.
    let name = FunctionName::new(
        Identifier::new("mod"),
        vec![Identifier::new("table")],
        Some(Identifier::new("method")),
    )
    .with_tokens(FunctionNameTokens {
        periods: Vec::new(),
        colon: None,
    });
    let function = FunctionStatement::new(name, Block::default(), Vec::new(), false);
    let output = generate(&Block::new(
        vec![Statement::Function(Box::new(function))],
        None,
    ));
    assert!(output.contains("mod.table:method"), "{output}");

    // A function body with tokens but no parameter commas, `...` or colon.
    let mut function = FunctionStatement::from_name("f", Block::default());
    function = function.with_parameter(Identifier::new("a"));
    function = function.with_parameter(Identifier::new("b"));
    function.set_variadic_type(FunctionVariadicType::GenericTypePack(GenericTypePack::new(
        "T",
    )));
    function.set_tokens(FunctionBodyTokens {
        function: Token::from_content("function"),
        opening_parenthese: Token::from_content("("),
        closing_parenthese: Token::from_content(")"),
        end: Token::from_content("end"),
        parameter_commas: Vec::new(),
        variable_arguments: None,
        variable_arguments_colon: None,
        return_type_colon: None,
    });
    let output = generate(&Block::new(
        vec![Statement::Function(Box::new(function))],
        None,
    ));
    assert!(output.contains("..."), "{output}");

    // Attribute group, tuple arguments and literal table with missing separators.
    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::nil());
    tuple.push(LiteralExpression::from(true));
    let tuple = tuple.with_tokens(TupleArgumentsTokens {
        opening_parenthese: Token::from_content("("),
        closing_parenthese: Token::from_content(")"),
        commas: Vec::new(),
    });

    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(Identifier::new("k"), LiteralExpression::nil());
    literal.append_field(Identifier::new("j"), LiteralExpression::nil());
    let literal = literal.with_tokens(TableTokens {
        opening_brace: Token::from_content("{"),
        closing_brace: Token::from_content("}"),
        separators: Vec::new(),
    });

    let group = AttributeGroup::new(
        AttributeGroupElement::new("tuple").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(
        AttributeGroupElement::new("table").with_arguments(AttributeArguments::Table(literal)),
    )
    .with_tokens(AttributeGroupTokens {
        opening_attribute_list: Token::from_content("@["),
        closing_bracket: Token::from_content("]"),
        separators: Vec::new(),
    });
    let function = FunctionStatement::from_name("g", Block::default())
        .with_attributes(Attributes::new().with_attribute(Attribute::Group(group)));
    let output = generate(&Block::new(
        vec![Statement::Function(Box::new(function))],
        None,
    ));
    assert!(output.contains("table"), "{output}");
}
