//! In-process Luau execution oracle with the Roblox runtime.
//!
//! Each program is run twice in an embedded [Lune](https://lune-org.github.io/)
//! VM — which provides the Roblox runtime (`require("@lune/roblox")`: datatypes,
//! instances, `Enum`) — once as written, and once after `r4` has renamed its
//! symbols and lowered modern Luau syntax. The two runs must produce
//! byte-for-byte identical observable behavior: `print` output, returned values,
//! and the state the program returns.
//!
//! The oracle is deterministic and self-contained: it needs no external Luau
//! binary, no Roblox Studio install, and no textual comparison against another
//! compiler.

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
        // Luau's explicit type instantiation (`f<<T>>(x)`) is behind this fast
        // flag, which embedded VMs leave off by default. r4 implements the
        // flag-on grammar, so the oracle enables it to exercise that path.
        let _ = mlua::Lua::set_fflag("LuauExplicitTypeInstantiationSyntax", true);
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
    (
        "roblox datatype arithmetic",
        r#"
local roblox = require("@lune/roblox")
local Vector3 = roblox.Vector3
local CFrame = roblox.CFrame

local a = Vector3.new(1, 2, 3)
local b = Vector3.new(4, 5, 6)
local sum = a + b
local cross = a:Cross(b)
local dot = a:Dot(b)
print(tostring(sum), cross.X, cross.Y, cross.Z, dot)

local frame = CFrame.new(0, 5, 0) * CFrame.new(1, 0, 0)
local position = frame.Position
print(position.X, position.Y, position.Z)
return sum.X, sum.Y, sum.Z, dot
"#,
    ),
    (
        "roblox instance tree",
        r#"
local roblox = require("@lune/roblox")
local Instance = roblox.Instance

local root = Instance.new("Folder")
root.Name = "Root"
local child = Instance.new("Folder")
child.Name = "Child"
child.Parent = root
local grandchild = Instance.new("Folder")
grandchild.Name = "Grandchild"
grandchild.Parent = child

local names = {}
for _, descendant in ipairs(root:GetDescendants()) do
    names[#names + 1] = descendant.Name
end
table.sort(names)
print(table.concat(names, ","), child.Parent.Name, grandchild.Parent.Name)
return table.concat(names, ",")
"#,
    ),
    (
        "roblox datatypes under luau syntax",
        r#"
local roblox = require("@lune/roblox")

local velocity: Vector3 = roblox.Vector3.new(3, 4, 0)
local speed: number = velocity.Magnitude
local label = if speed > 4 then "fast" else "slow"
print(`speed is {speed}: {label}`)

local offset = velocity
offset += roblox.Vector3.new(0, 0, 5)
print(tostring(offset))
return speed, label
"#,
    ),
    (
        "luau explicit instantiation",
        r#"
local function identity<T>(value: T): T
    return value
end

local number_value = identity<<number>>(42)
local bound = identity<<string>>
local string_value = bound("hello")

local function pair<A, B>(a: A, b: B): (A, B)
    return a, b
end
local first, second = pair<<number, string>>(1, "two")

print(number_value, string_value, first, second)
return number_value, string_value, first, second
"#,
    ),
    (
        "luau instantiation on methods and indexes",
        r#"
local module = {}
function module:convert<T>(value: T): T
    return value
end
function module.map<T>(value: T): T
    return value
end

local converted = module:convert<<number>>(3)
local mapped = module.map<<string>>("x")
print(converted, mapped)
return converted, mapped
"#,
    ),
    (
        "string and table library",
        r#"
local words = { "delta", "alpha", "charlie", "bravo" }
table.sort(words)
table.insert(words, "echo")
local joined = table.concat(words, ",")
print(joined, string.upper(words[1]), #words, string.sub(joined, 1, 3))
return joined, #words
"#,
    ),
    (
        "metatables and operators",
        r#"
local Vec = {}
Vec.__index = Vec
Vec.__add = function(a, b)
    return setmetatable({ x = a.x + b.x, y = a.y + b.y }, Vec)
end
Vec.__eq = function(a, b)
    return a.x == b.x and a.y == b.y
end
function Vec.new(x, y)
    return setmetatable({ x = x, y = y }, Vec)
end
local a = Vec.new(1, 2)
local b = Vec.new(3, 4)
local c = a + b
print(c.x, c.y, a == b, a == Vec.new(1, 2))
return c.x, c.y
"#,
    ),
    (
        "pcall and error handling",
        r#"
local ok = pcall(function()
    return 1
end)
local failed = pcall(function()
    error("boom", 0)
end)
print(ok, failed)
return ok, failed
"#,
    ),
    (
        "recursion and memoization",
        r#"
local cache = {}
local function fib(n)
    if n < 2 then
        return n
    end
    if cache[n] then
        return cache[n]
    end
    local value = fib(n - 1) + fib(n - 2)
    cache[n] = value
    return value
end
print(fib(15))
return fib(15)
"#,
    ),
    (
        "varargs and select",
        r#"
local function count(...)
    return select('#', ...), ...
end
local n, a, b = count(10, 20)
print(n, a, b)
return n, a, b
"#,
    ),
    (
        "repeat until and break",
        r#"
local i = 0
repeat
    i += 1
    if i == 3 then
        break
    end
until i >= 10
print(i)
return i
"#,
    ),
    (
        "bit32 library and binary literals",
        r#"
local band = bit32.band(0b1100, 0b1010)
local bor = bit32.bor(0b1100, 0b1010)
local lshift = bit32.lshift(1, 4)
print(band, bor, lshift)
return band, bor, lshift
"#,
    ),
    (
        "roblox datatypes and enum",
        r#"
local roblox = require("@lune/roblox")
local color = roblox.Color3.fromRGB(10, 20, 30)
local size = roblox.UDim2.new(1, 0, 0, 5)
print(color.R, color.G, color.B, size.X.Scale, size.X.Offset, size.Y.Offset)
local folder = roblox.Instance.new("Folder")
folder.Name = "Root"
return folder.Name, color.B
"#,
    ),
    (
        "counting with sorted iteration",
        r#"
local counts = {}
for _, word in ipairs({ "b", "a", "b", "c", "a", "b" }) do
    counts[word] = (counts[word] or 0) + 1
end
local keys = {}
for key in pairs(counts) do
    keys[#keys + 1] = key
end
table.sort(keys)
local parts = {}
for _, key in ipairs(keys) do
    parts[#parts + 1] = key .. "=" .. tostring(counts[key])
end
print(table.concat(parts, ","))
return table.concat(parts, ",")
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
