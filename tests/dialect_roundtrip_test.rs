//! Dialect round-trip and differential testing.
//!
//! Two properties are checked across every dialect r4 can parse:
//!
//! * **Round-trip** — a program parsed as dialect `D`, generated, and re-parsed
//!   as `D` yields a structurally equivalent tree, and the tree is
//!   representable in `D` (the dialect validator agrees).
//! * **Differential** — for programs in the common subset, every dialect that
//!   accepts the program reads it as the same tree.

use r4::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};
use r4::nodes::Block;
use r4::{Dialect, Equivalent, Parser};

const ALL: [Dialect; 6] = [
    Dialect::Lua51,
    Dialect::Lua52,
    Dialect::Lua53,
    Dialect::Lua54,
    Dialect::LuaJit,
    Dialect::Luau,
];

fn parser(dialect: Dialect) -> Parser {
    Parser::default().dialect(dialect)
}

fn generate_all(block: &Block) -> Vec<(&'static str, String)> {
    let mut readable = ReadableLuaGenerator::default();
    readable.write_block(block);
    let mut dense = DenseLuaGenerator::default();
    dense.write_block(block);
    vec![
        ("readable", readable.into_string()),
        ("dense", dense.into_string()),
    ]
}

/// Programs that exercise the syntax each dialect adds on top of Lua 5.1.
fn corpus(dialect: Dialect) -> &'static [&'static str] {
    const LUA51: &[&str] = &[
        "local a = 1\nreturn a",
        "local function f(x) return x + 1 end\nreturn f(1)",
        "for i = 1, 10 do if i > 5 then break end end",
        "local t = { 1, 2, x = 3 }\nfor k, v in pairs(t) do print(k, v) end",
        "local s = 'a' .. 'b'\nreturn s",
        "while true do break end",
        "repeat until true",
        "return function(...) return select('#', ...) end",
        "local x = 1\nx = x + 1\nreturn x",
    ];
    const LUA52: &[&str] = &[
        "::top::\ngoto top",
        "do goto done end\n::done::",
        "return 1",
    ];
    const LUA53: &[&str] = &[
        "return 1 & 2 | 3 ~ 4",
        "return 1 << 2 >> 1",
        "return 7 // 2",
        "return ~1",
        "return 0xFF, 0x1p4",
    ];
    const LUA54: &[&str] = &[
        "local x <const> = 1\nreturn x",
        "local f <close> = open()\nreturn f",
        "local a, b <const> = 1, 2\nreturn a + b",
    ];
    const LUAJIT: &[&str] = &["::top::\ngoto top", "return 1"];
    const LUAU: &[&str] = &[
        "local x: number = 1\nreturn x",
        "local x = 1\nx += 2\nreturn x",
        "return if condition then 1 else 2",
        "return `a {b} c`",
        "while true do continue end",
        "local function f<T>(value: T): T return value end\nreturn f<<number>>(1)",
        "type Point = { x: number, y: number }\nreturn 1",
        "return 0b1010",
        "return value :: number",
        "@native function f() end\nreturn f",
    ];

    match dialect {
        Dialect::Lua51 => LUA51,
        Dialect::Lua52 => LUA52,
        Dialect::Lua53 => LUA53,
        Dialect::Lua54 => LUA54,
        Dialect::LuaJit => LUAJIT,
        Dialect::Luau => LUAU,
        // `Default` has no corpus of its own; it is exercised elsewhere.
        Dialect::Default => &[],
    }
}

#[test]
fn every_dialect_corpus_round_trips() {
    for dialect in ALL {
        for program in corpus(dialect) {
            let block = parser(dialect)
                .parse(program)
                .unwrap_or_else(|error| panic!("[{dialect}] failed to parse `{program}`: {error}"));

            r4::validate_dialect(&block, dialect).unwrap_or_else(|error| {
                panic!("[{dialect}] `{program}` is not representable: {error}")
            });

            for (name, output) in generate_all(&block) {
                let reparsed = parser(dialect).parse(&output).unwrap_or_else(|error| {
                    panic!("[{dialect}/{name}] generated code does not parse: {error}\n{output}")
                });
                assert!(
                    block.is_equivalent_to(&reparsed),
                    "[{dialect}/{name}] round-trip changed the tree for `{program}`:\n{output}"
                );
                r4::validate_dialect(&reparsed, dialect).unwrap_or_else(|error| {
                    panic!("[{dialect}/{name}] re-parsed code is not representable: {error}")
                });
            }
        }
    }
}

/// Programs every dialect reads the same way.
const COMMON: &[&str] = &[
    "local a = 1\nreturn a",
    "return 1 + 2 * 3",
    "local t = { 1, 2, 3 }\nreturn #t",
    "local function f(x) return x end\nreturn f(1)",
    "return 'a' .. 'b' .. 'c'",
    "for i = 1, 3 do print(i) end",
    "local x = 1\nif x > 0 then x = x - 1 end\nreturn x",
    "return (1 + 2) * (3 - 4)",
];

#[test]
fn dialects_agree_on_the_common_subset() {
    for program in COMMON {
        let mut baseline: Option<(Dialect, Block)> = None;

        for dialect in ALL {
            let Ok(block) = parser(dialect).parse(program) else {
                continue;
            };

            match &baseline {
                Some((baseline_dialect, baseline_block)) => assert!(
                    baseline_block.is_equivalent_to(&block),
                    "`{program}` was read differently by {baseline_dialect} and {dialect}"
                ),
                None => baseline = Some((dialect, block)),
            }
        }

        assert!(
            baseline.is_some(),
            "no dialect accepted the common program `{program}`"
        );
    }
}
