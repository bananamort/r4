//! In-process Luau execution oracle.
//!
//! Each program is run twice in an embedded [Lune](https://lune-org.github.io/)
//! Luau VM: once as written, and once after `r4` has renamed its symbols and
//! lowered modern Luau syntax. The two runs must produce byte-for-byte identical
//! observable behavior — `print` output, returned values, and the state the
//! program returns.
//!
//! The oracle is deterministic and self-contained: it needs no external Luau
//! binary and no textual comparison against another compiler.

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::rules::desugar_to_lua51;
use r4::rules::rename_variables::rename_variables;
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

/// Wraps a program so that its `print` output and its return values (or the
/// error it raises) are captured into a single, canonical string.
///
/// The program is placed inside a function, so a top-level `return` becomes the
/// captured result. `print` is temporarily replaced with a collector; a string
/// `__encode` renders tables with sorted keys so the rendering is deterministic.
fn harness(program: &str) -> String {
    format!(
        r##"local __log = {{}}
local __print = print
print = function(...)
    local parts = {{}}
    for i = 1, select("#", ...) do
        parts[i] = tostring((select(i, ...)))
    end
    __log[#__log + 1] = table.concat(parts, "\t")
end

local function __encode(value)
    local kind = type(value)
    if kind == "table" then
        local keys = {{}}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys, function(a, b) return __encode(a) < __encode(b) end)
        local rendered = {{}}
        for _, key in ipairs(keys) do
            rendered[#rendered + 1] = __encode(key) .. "=" .. __encode(value[key])
        end
        return "{{" .. table.concat(rendered, ",") .. "}}"
    elseif kind == "string" then
        return string.format("%q", value)
    end
    return tostring(value)
end

local __packed = table.pack(pcall(function()
{program}
end))
print = __print

local __results = {{}}
for i = 1, __packed.n do
    __results[i] = __encode(__packed[i])
end
return table.concat(__results, "\t") .. "\n---log---\n" .. table.concat(__log, "\n")
"##
    )
}

/// Runs `program` in a fresh embedded Luau VM and returns its captured behavior.
fn observe(program: &str) -> String {
    let chunk = harness(program);
    async_io::block_on(async move {
        let mut runtime = lune::Runtime::new().expect("an embedded Luau VM");
        let returned = runtime
            .run_custom("r4-oracle", chunk)
            .await
            .unwrap_or_else(|error| panic!("the oracle program failed to run: {error}"));
        assert!(
            returned.success(),
            "the oracle program reported a failure status"
        );
        returned
            .values
            .into_iter()
            .find_map(|value| match value {
                mlua::Value::String(string) => Some(string.to_string_lossy().to_string()),
                _ => None,
            })
            .expect("the oracle harness always returns a string")
    })
}

/// Lowers and renames `program` with `r4`, returning the generated Luau.
fn transform(program: &str) -> String {
    let (mut block, model) = Parser::default()
        .luau()
        .parse_analyzed(program)
        .unwrap_or_else(|error| panic!("the oracle program must parse as Luau: {error}"));

    rename_variables(&mut block, &model).expect("renaming the oracle program must succeed");
    desugar_to_lua51(&mut block).expect("lowering the oracle program must succeed");

    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .expect("the lowered program must analyze");
    r4::rules::canonicalize(&mut block, &model);

    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(&block);
    generator.into_string()
}

/// Programs that together exercise the syntax `r4` renames or lowers.
const PROGRAMS: &[(&str, &str)] = &[
    (
        "typed function with branches",
        r#"
local function clamp(value: number, minimum: number, maximum: number): number
    if value < minimum then
        return minimum
    elseif value > maximum then
        return maximum
    end
    return value
end
print(clamp(5, 1, 3), clamp(-2, 1, 3), clamp(2, 1, 3))
return clamp(2, 1, 3)
"#,
    ),
    (
        "compound assignment and continue",
        r#"
local total = 0
for i = 1, 10 do
    if i % 2 == 0 then
        continue
    end
    total += i
end
print(total)
return total
"#,
    ),
    (
        "if expression and interpolated string",
        r#"
local value = 7
local label = if value > 5 then "big" else "small"
print(`value is {value}: {label}`)
return label
"#,
    ),
    (
        "closures over mutable upvalues",
        r#"
local function counter(): () -> number
    local count = 0
    return function(): number
        count += 1
        return count
    end
end
local next_value = counter()
print(next_value(), next_value(), next_value())
return next_value()
"#,
    ),
    (
        "tables, metatables and methods",
        r#"
local Vector = {}
Vector.__index = Vector
function Vector.new(x: number, y: number)
    return setmetatable({ x = x, y = y }, Vector)
end
function Vector:length(): number
    return math.sqrt(self.x * self.x + self.y * self.y)
end
local v = Vector.new(3, 4)
print(v:length())
return v:length(), v.x, v.y
"#,
    ),
    (
        "floor division and modulo",
        r#"
local a, b = 17, 5
print(a // b, -a // b, a % b)
return a // b, -a // b, a % b
"#,
    ),
    (
        "short-circuit side effects",
        r#"
local calls = 0
local function touch(value: any)
    calls += 1
    return value
end
local result = touch(false) and touch("and") or touch("or")
print(result, calls)
return result, calls
"#,
    ),
    (
        "type aliases and annotations",
        r#"
type Point = { x: number, y: number }
local function make(x: number, y: number): Point
    return { x = x, y = y }
end
local p: Point = make(2, 3)
print(p.x + p.y)
return p.x + p.y
"#,
    ),
    (
        "table iteration and string building",
        r#"
local values = { 1, 2, 3, 4 }
local sum = 0
for _, value in ipairs(values) do
    sum += value
end
print(tostring(sum) .. "!")
return sum
"#,
    ),
];

#[test]
fn lowered_programs_keep_identical_behavior() {
    for (name, program) in PROGRAMS {
        let before = observe(program);
        let lowered = transform(program);
        let after = observe(&lowered);
        assert_eq!(
            before, after,
            "`{name}` changed behavior after transformation:\n{lowered}"
        );
    }
}

#[test]
fn the_harness_distinguishes_different_behavior() {
    // Guards against a harness that would accept any two programs: two programs
    // with the same shape but different output must not compare equal.
    let left = observe("print('a')\nreturn 1");
    let right = observe("print('a')\nreturn 2");
    assert_ne!(left, right);
}
