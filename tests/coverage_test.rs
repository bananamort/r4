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
    "local x = 1; local y = 2; return x",
    "do local x = 1; end; return 0",
    "type K<T... = (number)> = () -> ()",
    "type L<T... = ...number> = () -> ()",
    "type M<T... = U...> = () -> ()",
    "type N<T, U... = ...number> = (T, U...) -> T",
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
    // The readable and dense generators may reformat, but their output must be
    // valid source: re-parse it in the same dialect.
    let mut readable = ReadableLuaGenerator::default();
    readable.write_block(block);
    let readable_output = readable.into_string();
    Parser::default()
        .dialect(dialect)
        .parse(&readable_output)
        .unwrap_or_else(|error| {
            panic!("readable output for `{code}` does not reparse: {error}\n{readable_output}")
        });

    let mut dense = DenseLuaGenerator::default();
    dense.write_block(block);
    let dense_output = dense.into_string();
    Parser::default()
        .dialect(dialect)
        .parse(&dense_output)
        .unwrap_or_else(|error| {
            panic!("dense output for `{code}` does not reparse: {error}\n{dense_output}")
        });

    // With the original token data the token-based generator must reproduce the
    // source byte for byte.
    let tokens = parse_tokens(code, dialect);
    let mut token_based = TokenBasedLuaGenerator::new(code);
    token_based.write_block(&tokens);
    assert_eq!(token_based.into_string(), code, "token round trip `{code}`");

    // Generating a tokenless tree exercises the token synthesis fallbacks; the
    // synthesized output must still parse.
    let tokenless = parse_with(code, dialect);
    let mut synthesized = TokenBasedLuaGenerator::new(code);
    synthesized.write_block(&tokenless);
    let synthesized_output = synthesized.into_string();
    Parser::default()
        .dialect(dialect)
        .parse(&synthesized_output)
        .unwrap_or_else(|error| {
            panic!(
                "synthesized output for `{code}` does not reparse: {error}\n{synthesized_output}"
            )
        });
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
    let pure_evaluator = Evaluator::default().assume_pure_metamethods();
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
        "return a and b or not c",
        "return not a",
        "return a and b",
        "return a or b",
        "return #t, -x",
        "return false or b",
        "return true and b",
        "return false and b",
        "return true or b",
        "return a == b",
        "return 1 ~= 2",
        "return 1 ~= 1",
        "return if false then 1 elseif f() then 2 else 3",
        "return if false then 1 elseif true then f() else 3",
        "return if false then 1 elseif a then f() else 3",
        "return if a then 1 elseif f() then 2 else 3",
        "return if a then f() else 3",
        "return a[b()].c",
        "return f().x",
        "return 'a' + 'b'",
        "return 1 + 'a'",
        "return 7 // 2",
        "return a // b",
        "return a ~= b",
        "return a % b",
        "return a ^ b",
        "return -a, #a",
        "return 'x' .. 1",
        "return 1 .. 'x'",
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
                let _ = pure_evaluator.has_side_effects_with(expression, &|_| true);
                let _ = pure_evaluator.has_side_effects_with(expression, &|_| false);
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
            assert!(
                r4::process::collect_trivia(&block)
                    .iter()
                    .all(|trivia| trivia.kind() != r4::nodes::TriviaKind::Comment),
                "`{code}` still has comments after removing them all"
            );
        }
    }

    // With comments present, keeping and removing behave as expected.
    let commented = "-- keep me\nlocal x = 1 -- trailing\nreturn x";
    let mut kept = parse_tokens(commented, Dialect::Luau);
    remove_comments_where(&mut kept, |_| true);
    assert!(r4::process::collect_trivia(&kept)
        .iter()
        .any(|trivia| trivia.kind() == r4::nodes::TriviaKind::Comment));
    let mut removed = parse_tokens(commented, Dialect::Luau);
    remove_comments_where(&mut removed, |_| false);
    assert!(r4::process::collect_trivia(&removed)
        .iter()
        .all(|trivia| trivia.kind() != r4::nodes::TriviaKind::Comment));
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
            assert!(
                r4::process::collect_trivia(&detokenized).is_empty(),
                "`{code}` still has trivia after detokenize"
            );
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
            let model = SemanticAnalyzer::new()
                .analyze(&block)
                .unwrap_or_else(|error| panic!("`{code}` analysis failed: {error}"));

            let _ = EscapeAnalysis::analyze(&block, &model);

            let cfg = build_chunk(&block, &model)
                .unwrap_or_else(|error| panic!("`{code}` cfg failed: {error}"));
            cfg.verify_invariants()
                .unwrap_or_else(|violations| panic!("`{code}` invariants: {violations:?}"));
            let _ = Reachability::analyze(&cfg);
            let _ = DominatorTree::analyze(&cfg);
            Liveness::analyze(&cfg, &model)
                .unwrap_or_else(|error| panic!("`{code}` liveness failed: {error}"));
            ConstantPropagation::analyze(&cfg, &model)
                .unwrap_or_else(|error| panic!("`{code}` constant propagation failed: {error}"));
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
            let (block, model) = Parser::default()
                .dialect(dialect)
                .preserve_tokens()
                .parse_analyzed(code)
                .unwrap_or_else(|error| panic!("`{code}` analysis failed: {error}"));
            let index_result = SyntaxIndex::build(&block);
            assert!(index_result.is_ok(), "`{code}` index: {index_result:?}");
            let _ = graph.add_module(format!("m{index}"), block, model);
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
    tuple.push(LiteralExpression::from(LiteralTable::default()));

    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(
        r4::nodes::Identifier::new("k"),
        LiteralExpression::from(1u32),
    );
    literal.append_array_value(LiteralExpression::Nil(None));
    literal.append_array_value(LiteralExpression::from(false));
    literal.append_array_value(LiteralExpression::from(
        r4::nodes::StringExpression::from_value("nested"),
    ));
    literal.append_array_value(LiteralExpression::from(LiteralTable::default()));

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

    // Every generator must render the attribute names and their arguments.
    for name in ["tuple", "table", "string"] {
        let mut readable = ReadableLuaGenerator::default();
        readable.write_block(&block);
        let output = readable.into_string();
        assert!(output.contains(name), "readable `{name}`: {output}");

        let mut dense = DenseLuaGenerator::default();
        dense.write_block(&block);
        let output = dense.into_string();
        assert!(output.contains(name), "dense `{name}`: {output}");

        let mut token_based = TokenBasedLuaGenerator::new("");
        token_based.write_block(&block);
        let output = token_based.into_string();
        assert!(output.contains(name), "token based `{name}`: {output}");
    }
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
        "type P = A<...number, B>",
        "type Q = A<(number), B>",
        "type R = A<B..., C...>",
        "type W1<A..., B...> = A",
        "type W2<A = number, B... = ...number> = A",
        "type W3<B... = ...number> = B",
        "local y1: { read a: number, write b: string } = t",
        "local y2: { read [number]: string } = t",
        "local y3: { [string]: number } = t",
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

#[test]
fn for_each_trivia_walks_field_call_varargs_and_literals() {
    for code in [
        "return a.b.c",
        "return f().g",
        "return a[1]()",
        "return ...",
        "return function(...) return ... end",
        "local t = { a = 1, [b] = 2 }",
        "return true, false, nil",
        "-- c\nreturn 1 --[[t]]",
    ] {
        let block = parse_tokens(code, Dialect::Luau);
        let mut count = 0;
        r4::process::for_each_trivia(&block, &mut |_| count += 1);
        let _ = count;
    }

    // Literal expressions (attribute arguments) are reachable only by
    // construction.
    use r4::nodes::{
        Attribute, AttributeArguments, AttributeGroup, AttributeGroupElement,
        AttributeTupleArguments, Attributes, Block, FunctionStatement, LiteralExpression,
        Statement, Token,
    };
    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::False(None));
    tuple.push(LiteralExpression::Nil(Some(Token::from_content("nil"))));
    let group = AttributeGroup::new(
        AttributeGroupElement::new("a").with_arguments(AttributeArguments::Tuple(tuple)),
    );
    let function = FunctionStatement::from_name("f", Block::default())
        .with_attributes(Attributes::new().with_attribute(Attribute::Group(group)));
    let block = Block::new(vec![Statement::Function(Box::new(function))], None);
    r4::process::for_each_trivia(&block, &mut |_| {});
}

#[test]
fn generator_spacing_and_line_break_helper_inputs() {
    let long_minus = (1..=80).map(|i| format!("a{i} - ")).collect::<String>() + "0";
    let long_concat = (1..=80).map(|i| format!("a{i} .. ")).collect::<String>() + "''";
    let cases = vec![
        "local a = 1\n(f)()".to_owned(),
        "local b = f()\n(g)()".to_owned(),
        "local c = {}\n(h).x = 1".to_owned(),
        format!("return {long_minus}"),
        format!("return {long_concat}"),
        "local x = 1\nlocal y = 2\nlocal z = x == y".to_owned(),
        "return function(...) return ... end".to_owned(),
        "local t = { 1, 2, 3 }\nreturn t".to_owned(),
        "return f{ 1, 2 }, f's'".to_owned(),
        "return `a\\n{b}\\t`".to_owned(),
        "return \"\\0\\1\\2\\7\\8\\9\\11\\12\\13\"".to_owned(),
        "return \"a\\rb\\nc\\td\\\\e\\\"f\\'g\"".to_owned(),
        "return [==[x]]y]==]".to_owned(),
        "local x\n(f)()".to_owned(),
        "x = 1\n(f)()".to_owned(),
        "if a then end\n(f)()".to_owned(),
        "while a do end\n(f)()".to_owned(),
        "return {1} == t".to_owned(),
        "return {1} and t".to_owned(),
    ];
    for code in cases {
        let block = parse_with(&code, Dialect::Luau);
        assert_generates(&block, &code, Dialect::Luau);
    }
}

#[test]
fn readable_and_dense_generators_wrap_long_signatures_and_tables() {
    let long_params = (1..=40)
        .map(|i| format!("parameter_number_{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let long_chain = (1..=30)
        .map(|i| format!("field_{i}"))
        .collect::<Vec<_>>()
        .join(".");
    let long_arguments = (1..=20)
        .map(|i| format!("argument_number_{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let cases = vec![
        format!("return {long_chain}(1, 2, 3)"),
        format!("return {long_chain}:method({long_arguments})"),
        format!("return outer(inner({long_arguments}))"),
        format!("return {long_chain}[1](2)"),
        format!("local x = {long_chain} + 1\nreturn x"),
        format!("local function f({long_params}) end"),
        format!("function f({long_params}, ...) end"),
        format!("local f = function({long_params}) return 1 end"),
        format!("function f({long_params}): number return 1 end"),
        "return { {}, {}, {}, {} }".to_owned(),
        "return { a = {}, b = {}, c = {}, d = {} }".to_owned(),
        "return { { { 1 } } }".to_owned(),
        "return { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 }".to_owned(),
        "return { a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = 7, h = 8 }".to_owned(),
        "return { 1, 2, 3, a = 4, [5] = 6, 7, 8, 9, 10, 11, 12 }".to_owned(),
        "local a = 1\nlocal b = a + 1\nlocal c = b + 1\nreturn a, b, c".to_owned(),
    ];
    for code in cases {
        let block = parse_with(&code, Dialect::Luau);
        assert_generates(&block, &code, Dialect::Luau);
    }
}

#[test]
fn partial_tokens_synthesize_keywords_and_commas() {
    use r4::generator::TokenBasedLuaGenerator;
    use r4::nodes::{
        Block, Expression, IfBranch, IfStatement, IfStatementTokens, LastStatement,
        ReturnStatement, Statement, Token, TypeDeclarationStatement, TypeDeclarationTokens,
        TypeName, TypedIdentifier, VariableAssignment, VariableAssignmentTokens,
    };

    fn generate(block: &Block) -> String {
        let mut generator = TokenBasedLuaGenerator::new("");
        generator.write_block(block);
        generator.into_string()
    }

    // If statement with tokens but no `else` token.
    let mut statement = IfStatement::new(
        vec![IfBranch::new(Expression::identifier("a"), Block::default())],
        Some(Block::new(
            Vec::new(),
            Some(LastStatement::Return(ReturnStatement::new(vec![
                Expression::from(1.0),
            ]))),
        )),
    );
    statement.set_tokens(IfStatementTokens {
        r#if: Token::from_content("if"),
        then: Token::from_content("then"),
        end: Token::from_content("end"),
        r#else: None,
    });
    let output = generate(&Block::new(vec![Statement::If(statement)], None));
    assert!(output.contains("else"), "{output}");

    // Local assignment with tokens but no separator commas.
    let mut assign = VariableAssignment::new(
        vec![TypedIdentifier::new("a"), TypedIdentifier::new("b")],
        vec![Expression::from(1.0), Expression::from(2.0)],
    );
    assign.set_tokens(VariableAssignmentTokens {
        keyword: Token::from_content("local"),
        equal: Some(Token::from_content("=")),
        variable_commas: Vec::new(),
        value_commas: Vec::new(),
    });
    let output = generate(&Block::new(vec![Statement::LocalAssign(assign)], None));
    assert!(output.contains("a,b"), "{output}");

    // Exported type declaration with tokens but no `export` token.
    let mut declaration = TypeDeclarationStatement::new("T", TypeName::new("number")).export();
    declaration.set_tokens(TypeDeclarationTokens {
        r#type: Token::from_content("type"),
        equal: Token::from_content("="),
        export: None,
    });
    let output = generate(&Block::new(
        vec![Statement::TypeDeclaration(declaration)],
        None,
    ));
    assert!(output.contains("export"), "{output}");
}

#[test]
fn type_instantiation_pack_arguments_are_rejected_clearly() {
    for code in ["return f<<T, U>>", "return f<<(number), string>>"] {
        let _ = parse_with(code, Dialect::Luau);
    }
    for code in ["return f<<A...>>", "return f<<(number), A...>>"] {
        let error = Parser::default()
            .dialect(Dialect::Luau)
            .parse(code)
            .expect_err("a type pack argument is not representable");
        assert!(
            !error.to_string().contains("internal conversion stack"),
            "`{code}`: {error}"
        );
    }
}

#[test]
fn partial_tokens_synthesize_generic_and_for_commas() {
    use r4::generator::TokenBasedLuaGenerator;
    use r4::nodes::{
        Block, Expression, GenericForStatement, GenericForTokens, GenericParametersTokens,
        GenericParametersWithDefaults, Statement, Token, TypeDeclarationStatement, TypeName,
        TypeParameters, TypeParametersTokens, TypedIdentifier,
    };

    fn generate(block: &Block) -> String {
        let mut generator = TokenBasedLuaGenerator::new("");
        generator.write_block(block);
        generator.into_string()
    }

    // Generic for with tokens but no commas.
    let mut generic_for = GenericForStatement::new(
        vec![TypedIdentifier::new("a"), TypedIdentifier::new("b")],
        vec![Expression::identifier("x"), Expression::identifier("y")],
        Block::default(),
    );
    generic_for.set_tokens(GenericForTokens {
        r#for: Token::from_content("for"),
        r#in: Token::from_content("in"),
        r#do: Token::from_content("do"),
        end: Token::from_content("end"),
        identifier_commas: Vec::new(),
        value_commas: Vec::new(),
    });
    let output = generate(&Block::new(vec![Statement::GenericFor(generic_for)], None));
    assert!(output.contains("a,b"), "{output}");

    // Generic parameters with tokens but no commas.
    let mut parameters =
        GenericParametersWithDefaults::from_type_variable("A").with_type_variable("B");
    parameters.set_tokens(GenericParametersTokens {
        opening_list: Token::from_content("<"),
        closing_list: Token::from_content(">"),
        commas: Vec::new(),
    });
    let declaration = TypeDeclarationStatement::new("T", TypeName::new("number"))
        .with_generic_parameters(parameters);
    let output = generate(&Block::new(
        vec![Statement::TypeDeclaration(declaration)],
        None,
    ));
    assert!(output.contains("A,B"), "{output}");

    // Type parameters with tokens but no commas.
    let mut type_parameters = TypeParameters::new(TypeName::new("A"));
    type_parameters.push_parameter(TypeName::new("B"));
    type_parameters.set_tokens(TypeParametersTokens {
        opening_list: Token::from_content("<"),
        closing_list: Token::from_content(">"),
        commas: Vec::new(),
    });
    let type_name = TypeName::new("Outer").with_type_parameters(type_parameters);
    let declaration = TypeDeclarationStatement::new("U", type_name);
    let output = generate(&Block::new(
        vec![Statement::TypeDeclaration(declaration)],
        None,
    ));
    assert!(output.contains("A,B"), "{output}");
}

#[test]
fn type_cast_line_break_helpers_are_driven() {
    for code in [
        "local x = 1\nreturn (x :: (number) -> string)",
        "return (x :: A & B)",
        "return (x :: A | B)",
        "return (x :: A<T>)",
        "return (x :: A)",
        "return (if c then x else y) :: number",
        "return -(x :: number)",
        "return (a + (x :: number))",
        "return f((x :: (A) -> B))",
        "return ((x :: A) :: B)",
        "return { (x :: A) }",
    ] {
        let block = parse_with(code, Dialect::Luau);
        assert_generates(&block, code, Dialect::Luau);
    }
}

#[test]
fn evaluator_folds_constant_expressions_to_their_values() {
    use r4::process::LuaValue;

    let evaluator = Evaluator::default();
    for (code, expected) in [
        ("return 1 + 2", LuaValue::Number(3.0)),
        ("return 7 % 3", LuaValue::Number(1.0)),
        ("return 2 ^ 3", LuaValue::Number(8.0)),
        ("return 10 / 4", LuaValue::Number(2.5)),
        ("return 7 // 2", LuaValue::Number(3.0)),
        ("return 1 < 2", LuaValue::True),
        ("return 1 ~= 2", LuaValue::True),
        ("return 1 == 1", LuaValue::True),
        ("return true and false", LuaValue::False),
        ("return true or false", LuaValue::True),
        ("return not true", LuaValue::False),
        ("return -3", LuaValue::Number(-3.0)),
        ("return #'abc'", LuaValue::Number(3.0)),
        ("return 'a' .. 'b'", LuaValue::String(b"ab".to_vec())),
        ("return (5)", LuaValue::Number(5.0)),
        ("return if true then 1 else 2", LuaValue::Number(1.0)),
        ("return if false then 1 else 2", LuaValue::Number(2.0)),
        ("return nil", LuaValue::Nil),
        ("return true", LuaValue::True),
        ("return false", LuaValue::False),
        ("return 'text'", LuaValue::String(b"text".to_vec())),
        ("return '1' + 2", LuaValue::Number(3.0)),
        ("return 2 * 3 + 4", LuaValue::Number(10.0)),
        ("return 2 + 3 * 4", LuaValue::Number(14.0)),
        ("return (2 + 3) * 4", LuaValue::Number(20.0)),
        ("return 1 and 2", LuaValue::Number(2.0)),
        ("return nil or 3", LuaValue::Number(3.0)),
        ("return true and 1 or 2", LuaValue::Number(1.0)),
        ("return false and 1 or 2", LuaValue::Number(2.0)),
        ("return not nil", LuaValue::True),
        ("return not 1", LuaValue::False),
        ("return 'a' < 'b'", LuaValue::True),
        ("return 'a' == 'a'", LuaValue::True),
        ("return 1 == '1'", LuaValue::False),
        ("return 1 ~= '1'", LuaValue::True),
        (
            "return 'a' .. 'b' .. 'c'",
            LuaValue::String(b"abc".to_vec()),
        ),
        // Non-constant operands decline instead of guessing.
        ("return a", LuaValue::Unknown),
        ("return a + 1", LuaValue::Unknown),
        ("return f()", LuaValue::Unknown),
        ("return 'a' + 'b'", LuaValue::Unknown),
        ("return a and b", LuaValue::Unknown),
    ] {
        let block = parse_with(code, Dialect::Luau);
        let Some(r4::nodes::LastStatement::Return(statement)) = block.get_last_statement() else {
            panic!("`{code}` must return");
        };
        let expression = statement.iter_expressions().next().expect("an expression");
        assert_eq!(evaluator.evaluate(expression), expected, "`{code}`");
    }
}

#[test]
fn generated_output_is_equivalent_to_the_source() {
    use r4::Equivalent;

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let block = parse_with(code, dialect);
            for (name, output) in [
                ("readable", {
                    let mut generator = ReadableLuaGenerator::default();
                    generator.write_block(&block);
                    generator.into_string()
                }),
                ("dense", {
                    let mut generator = DenseLuaGenerator::default();
                    generator.write_block(&block);
                    generator.into_string()
                }),
            ] {
                let reparsed = Parser::default()
                    .dialect(dialect)
                    .parse(&output)
                    .unwrap_or_else(|error| {
                        panic!("{name} `{code}` does not reparse: {error}\n{output}")
                    });
                assert!(
                    block.is_equivalent_to(&reparsed),
                    "{name} changed the tree for `{code}`:\n{output}"
                );
            }
        }
    }
}

#[test]
fn binary_parenthesization_round_trips_for_nested_operators() {
    use r4::Equivalent;

    fn generators(block: &Block) -> Vec<(&'static str, String)> {
        let mut readable = ReadableLuaGenerator::default();
        readable.write_block(block);
        let mut dense = DenseLuaGenerator::default();
        dense.write_block(block);
        // The token-less token-based path synthesizes operators and parentheses.
        let mut synthesized = TokenBasedLuaGenerator::new("");
        synthesized.write_block(block);
        vec![
            ("readable", readable.into_string()),
            ("dense", dense.into_string()),
            ("synthesized", synthesized.into_string()),
        ]
    }

    for code in [
        "local x = a - (b - c)\nreturn x",
        "local x = (a - b) - c\nreturn x",
        "local x = a / (b / c)\nreturn x",
        "local x = (a / b) / c\nreturn x",
        "local x = a ^ (b ^ c)\nreturn x",
        "local x = (a ^ b) ^ c\nreturn x",
        "local x = a .. (b .. c)\nreturn x",
        "local x = (a .. b) .. c\nreturn x",
        "local x = a - (b + c)\nreturn x",
        "local x = a + (b - c)\nreturn x",
        "local x = a * (b / c)\nreturn x",
        "local x = a / (b * c)\nreturn x",
        "local x = a % (b % c)\nreturn x",
        "local x = (a and b) and c\nreturn x",
        "local x = a and (b or c)\nreturn x",
        "local x = (a or b) and c\nreturn x",
        "local x = a == (b < c)\nreturn x",
        "local x = (a < b) == c\nreturn x",
        "local x = -(a ^ b)\nreturn x",
        "local x = a - -b\nreturn x",
        "local x = -a ^ b\nreturn x",
        "local x = (a and b) or c\nreturn x",
        "local x = a and (b and c)\nreturn x",
        "local x = (a .. b) == c\nreturn x",
        "local x = a .. b .. c\nreturn x",
    ] {
        let block = parse_with(code, Dialect::Luau);
        for (name, output) in generators(&block) {
            let reparsed = Parser::default()
                .dialect(Dialect::Luau)
                .parse(&output)
                .unwrap_or_else(|error| panic!("{name} `{code}`: {error}\n{output}"));
            assert!(
                block.is_equivalent_to(&reparsed),
                "{name} changed the tree for `{code}`:\n{output}"
            );
        }
    }
}

#[test]
fn equivalence_is_trivia_insensitive_but_structure_sensitive() {
    use r4::Equivalent;

    let spaced = parse_with("local x = 1 + 2\nreturn x", Dialect::Luau);
    let spaced_more = parse_with("local   x = 1  +   2\nreturn x", Dialect::Luau);
    assert!(spaced.is_equivalent_to(&spaced_more));

    let with_semicolon = parse_with("local x = 1 + 2;\nreturn x;", Dialect::Luau);
    assert!(spaced.is_equivalent_to(&with_semicolon));

    let different_operator = parse_with("local x = 1 - 2\nreturn x", Dialect::Luau);
    assert!(!spaced.is_equivalent_to(&different_operator));

    let different_value = parse_with("local x = 1 + 3\nreturn x", Dialect::Luau);
    assert!(!spaced.is_equivalent_to(&different_value));

    let different_structure = parse_with("local x = (1 + 2) * 3\nreturn x", Dialect::Luau);
    let flat = parse_with("local x = 1 + 2 * 3\nreturn x", Dialect::Luau);
    assert!(!different_structure.is_equivalent_to(&flat));

    let field = parse_with("return t.x", Dialect::Luau);
    let index = parse_with("return t['x']", Dialect::Luau);
    assert!(!field.is_equivalent_to(&index));
}

#[test]
fn tokenful_and_tokenless_trees_are_equivalent() {
    use r4::Equivalent;

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let tokenful = parse_tokens(code, dialect);
            let tokenless = parse_with(code, dialect);
            assert!(
                tokenful.is_equivalent_to(&tokenless),
                "token data changed equivalence for `{code}`"
            );
        }
    }
}

#[test]
fn token_based_generator_preserves_comments_and_spacing_exactly() {
    for code in [
        "-- leading\nlocal x = 1 -- trailing\nreturn x -- tail",
        "local x = --[[inline]] 1\nreturn x",
        "--- doc\n--!strict\nreturn 1",
        "local t = { --[[a]] 1, --[[b]] 2 }",
        "function --[[f]] f --[[p]] ( --[[arg]] a --[[endarg]] ) --[[body]] return a end",
        "return --[[a]] 1 --[[b]] + --[[c]] 2",
        "if --[[c]] a --[[then]] then --[[body]] return 1 end",
        "return `text { --[[v]] x } more`",
        "for --[[i]] i = 1, --[[n]] 10 do end",
        "while --[[c]] true do --[[b]] break end",
        "local x = 1   \n   return x",
        "return 1 -- c1\n-- c2\n",
        "local a,b,c = 1,2,3\nreturn a",
        "\tlocal x\t=\t1\n\treturn\tx",
        "local x = 1;\nreturn x;",
        "return 1,2, --[[t]] 3",
        "local t = { a = 1, ['b'] = 2; c = 3 }",
        "type T = { a: number, [string]: number }",
        "type F < A, B... > = (A, B...) -> A",
        "return a  and  b  or  c",
        "local function f ( a , b ) return a end\nreturn f",
        "return 1 +\n2",
        "return (1 + 2) * 3",
        "return - - 1",
        "return #t",
        "return t [ 1 ] . x",
        "return f ( 1 , 2 )",
        "return a :: number",
        "return `a` .. `b`",
    ] {
        let tokens = parse_tokens(code, Dialect::Luau);
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&tokens);
        assert_eq!(
            generator.into_string(),
            code,
            "token round trip changed `{code}`"
        );
    }
}

#[test]
fn synthesized_token_output_is_equivalent_to_the_source() {
    use r4::Equivalent;

    let corpora: [(&[&str], Dialect); 4] = [
        (CORPUS, Dialect::Luau),
        (LUA52, Dialect::Lua52),
        (LUA53, Dialect::Lua53),
        (LUA54, Dialect::Lua54),
    ];
    for (codes, dialect) in corpora {
        for &code in codes {
            let block = parse_with(code, dialect);
            let mut synthesized = TokenBasedLuaGenerator::new("");
            synthesized.write_block(&block);
            let output = synthesized.into_string();
            let reparsed = Parser::default()
                .dialect(dialect)
                .parse(&output)
                .unwrap_or_else(|error| panic!("`{code}` does not reparse: {error}\n{output}"));
            assert!(
                block.is_equivalent_to(&reparsed),
                "synthesized output changed the tree for `{code}`:\n{output}"
            );
        }
    }
}

#[test]
fn token_based_generator_round_trips_tricky_literals_and_layouts() {
    for code in [
        "return 'a\\n\\t\\\"b'",
        "return [[long\nstring]]",
        "return [==[with ]] inside]==]",
        "return 0xFF, 1e10, 1E-3, 0b1010, 1_000",
        "return { { 1 }, { 2, 3 } }",
        "return { [1] = 'a', [2] = 'b' }",
        "return f ( { a = 1 } , 'x' )",
        "return a [ 1 ] [ 2 ] . b . c",
        "return `a { b } c { d }`",
        "local x = if a then 1 else 2\nreturn x",
        "local x = a :: number\nreturn x",
        "local x = f<<T>>()\nreturn x",
        "type T = { [string]: number }\nreturn 1",
        "type T = (a: number, b: string) -> boolean\nreturn 1",
        "return function ( a : number , ... : string ) : boolean return true end",
        "for i = 1, 10, 2 do print ( i ) end",
        "for k, v in pairs ( t ) do print ( k ) end",
        "repeat print ( 1 ) until true",
        "do end",
        "@native function f () end",
        "local x = 1 --[[a]] + --[[b]] 2 --[[c]]\nreturn x",
        "return ( 1 + 2 )",
        "return - x",
        "return not x",
        "return # x",
        "local a, b = 1, 2\nreturn a, b",
    ] {
        let tokens = parse_tokens(code, Dialect::Luau);
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&tokens);
        assert_eq!(
            generator.into_string(),
            code,
            "token round trip changed `{code}`"
        );
    }
}

#[test]
fn token_based_generator_round_trips_dialect_specific_code() {
    for (code, dialect) in [
        ("::top::\ngoto top", Dialect::Lua52),
        ("goto continue\n::continue::", Dialect::Lua52),
        ("do goto skip end\n::skip::\nreturn 1", Dialect::Lua52),
        ("return 1 // 2", Dialect::Lua53),
        ("return 7 // 2, 1.5, 2.0e3", Dialect::Lua53),
        ("local x <const> = 1\nreturn x", Dialect::Lua54),
        ("local x <close> = make()\nreturn 0", Dialect::Lua54),
        ("::top::\ngoto top", Dialect::LuaJit),
    ] {
        let tokens = parse_tokens(code, dialect);
        let mut generator = TokenBasedLuaGenerator::new(code);
        generator.write_block(&tokens);
        assert_eq!(
            generator.into_string(),
            code,
            "token round trip changed `{code}` ({dialect:?})"
        );
    }
}

#[test]
fn generator_output_is_equivalent_for_tricky_constructs() {
    use r4::Equivalent;

    let generators = |block: &Block| {
        let mut readable = ReadableLuaGenerator::default();
        readable.write_block(block);
        let mut dense = DenseLuaGenerator::default();
        dense.write_block(block);
        let mut synthesized = TokenBasedLuaGenerator::new("");
        synthesized.write_block(block);
        vec![
            ("readable", readable.into_string()),
            ("dense", dense.into_string()),
            ("synthesized", synthesized.into_string()),
        ]
    };

    for code in [
        "return 'a\\n\\t\\\"b'",
        "return { { 1 }, { 2, 3 } }",
        "return { [1] = 'a', [2] = 'b' }",
        "return f ( { a = 1 } , 'x' )",
        "return `a { b } c { d }`",
        "type T = { [string]: number }\nreturn 1",
        "type T = (a: number, b: string) -> boolean\nreturn 1",
        "return function ( a : number , ... : string ) : boolean return true end",
        "local x = if a then 1 elseif b then 2 else 3\nreturn x",
        "return a + b * c - d / e % f ^ g",
        "return a and b or not c",
        "return a .. b .. c",
        "return -x, #t, not y",
        "local t = { 1, 2, a = 3, [b] = 4 }\nreturn t",
        "for k, v in pairs ( t ) do break end",
        "while true do continue end",
    ] {
        let block = parse_with(code, Dialect::Luau);
        for (name, output) in generators(&block) {
            let reparsed = Parser::default()
                .dialect(Dialect::Luau)
                .parse(&output)
                .unwrap_or_else(|error| panic!("{name} `{code}`: {error}\n{output}"));
            assert!(
                block.is_equivalent_to(&reparsed),
                "{name} changed the tree for `{code}`:\n{output}"
            );
        }
    }
}
