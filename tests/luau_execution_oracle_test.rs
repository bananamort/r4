//! Execution oracles for the optimizer and lowerer.
//!
//! Each program is run twice in an embedded [Lune](https://lune-org.github.io/)
//! VM — which provides the Roblox runtime (`require("@lune/roblox")`: datatypes,
//! instances, `Enum`) — once as written, and once after `r4` has renamed its
//! symbols and lowered modern Luau syntax. The two runs must produce
//! byte-for-byte identical observable behavior: `print` output, returned values,
//! and the state the program returns.
//!
//! A separate dialect oracle reruns the optimized corpus under Lua 5.1, 5.2,
//! 5.3, 5.4, and LuaJIT, and the seeded fuzz corpus under those dialects and the
//! embedded Luau VM. CI runs each dialect in its own matrix job, so the
//! embedded Luau oracle itself remains self-contained.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::rules::canonicalize;
use r4::rules::desugar_to_lua51;
use r4::rules::rename_variables::rename_variables;
use r4::semantic::SemanticAnalyzer;
use r4::{Dialect, Parser};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

/// Builds a program that captures `print` output and return values (or the
/// raised error) into one canonical string.
///
/// `external` selects the Lua 5.4 form, which installs an instruction hook so a
/// non-terminating program is reported instead of hanging, and writes the
/// result to stdout.
fn harness_source(program: &str, external: bool) -> String {
    let hook = if external {
        r#"local __hooked = false
if debug and debug.sethook then
    debug.sethook(function() error("__timeout") end, "", 500000)
    __hooked = true
end
"#
    } else {
        ""
    };
    let unhook = if external {
        "if __hooked then debug.sethook() end\n"
    } else {
        ""
    };
    let ending = if external {
        "io.write(__result)"
    } else {
        "return __result"
    };
    format!(
        r##"local __log = {{}}
local __print = print

local function __encode(value, seen, depth)
    local kind = type(value)
    if kind == "table" then
        seen = seen or {{}}
        depth = depth or 0
        if seen[value] then
            return "<cycle>"
        end
        if depth >= 8 then
            return "<deep>"
        end
        seen[value] = true
        local keys = {{}}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys, function(a, b) return __encode(a) < __encode(b) end)
        local rendered = {{}}
        for _, key in ipairs(keys) do
            rendered[#rendered + 1] = __encode(key) .. "=" .. __encode(value[key], seen, depth + 1)
        end
        seen[value] = nil
        return "{{" .. table.concat(rendered, ",") .. "}}"
    elseif kind == "string" then
        return string.format("%q", value)
    elseif kind == "function" or kind == "thread" or kind == "userdata" then
        return "<" .. kind .. ">"
    end
    return tostring(value)
end

print = function(...)
    local parts = {{}}
    for i = 1, select("#", ...) do
        parts[i] = __encode((select(i, ...)))
    end
    __log[#__log + 1] = table.concat(parts, "\t")
end

local function __pack(...)
    return {{ n = select("#", ...), ... }}
end

{hook}local __packed = __pack(pcall(function()
{program}
end))
{unhook}print = __print

if __packed[1] == false and type(__packed[2]) == "string" then
    __packed[2] = string.gsub(__packed[2], "^.-:%d+: ", "")
end

local __results = {{}}
for i = 1, __packed.n do
    __results[i] = __encode(__packed[i])
end
local __result = table.concat(__results, "\t") .. "\n---log---\n" .. table.concat(__log, "\n")
{ending}
"##
    )
}

fn harness(program: &str) -> String {
    harness_source(program, false)
}

fn lua_harness(program: &str) -> String {
    harness_source(program, true)
}

/// One external Lua implementation covered by the dialect oracle.
struct LuaDialect {
    /// Key used by `R4_LUA_DIALECTS` and the CI matrix.
    key: &'static str,
    /// Human-readable name for failure messages.
    name: &'static str,
    /// Text printed by the interpreter's `-v`.
    marker: &'static str,
    /// Environment variable that overrides the binary.
    env: &'static str,
    /// Binary names tried in order.
    candidates: &'static [&'static str],
    /// Parser and analyzer dialect for programs run by this implementation.
    dialect: Dialect,
}

const LUA_DIALECTS: &[LuaDialect] = &[
    LuaDialect {
        key: "lua51",
        name: "Lua 5.1",
        marker: "Lua 5.1",
        env: "LUA51",
        candidates: &["lua5.1", "lua51"],
        dialect: Dialect::Lua51,
    },
    LuaDialect {
        key: "lua52",
        name: "Lua 5.2",
        marker: "Lua 5.2",
        env: "LUA52",
        candidates: &["lua5.2", "lua52"],
        dialect: Dialect::Lua52,
    },
    LuaDialect {
        key: "lua53",
        name: "Lua 5.3",
        marker: "Lua 5.3",
        env: "LUA53",
        candidates: &["lua5.3", "lua53"],
        dialect: Dialect::Lua53,
    },
    LuaDialect {
        key: "lua54",
        name: "Lua 5.4",
        marker: "Lua 5.4",
        env: "LUA54",
        candidates: &["lua5.4", "lua54", "lua"],
        dialect: Dialect::Lua54,
    },
    LuaDialect {
        key: "luajit",
        name: "LuaJIT",
        marker: "LuaJIT",
        env: "LUAJIT",
        candidates: &["luajit"],
        dialect: Dialect::LuaJit,
    },
];

/// Reads `R4_LUA_DIALECTS`, a comma-separated list of dialect keys. An empty
/// result leaves the external dialect oracle disabled.
fn selected_lua_dialects() -> Vec<&'static LuaDialect> {
    let Ok(selection) = std::env::var("R4_LUA_DIALECTS") else {
        return Vec::new();
    };
    let selected: Vec<&'static LuaDialect> = selection
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| {
            LUA_DIALECTS
                .iter()
                .find(|dialect| dialect.key == key)
                .unwrap_or_else(|| {
                    let keys: Vec<&str> = LUA_DIALECTS.iter().map(|dialect| dialect.key).collect();
                    panic!(
                        "unknown Lua dialect `{key}`; expected one of {}",
                        keys.join(", ")
                    )
                })
        })
        .collect();
    assert!(
        !selected.is_empty(),
        "R4_LUA_DIALECTS does not name a dialect"
    );
    selected
}

fn find_lua_dialect(dialect: &LuaDialect) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os(dialect.env) {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend(
        dialect
            .candidates
            .iter()
            .map(|candidate| PathBuf::from(*candidate)),
    );
    for candidate in candidates {
        let Ok(output) = Command::new(&candidate).arg("-v").output() else {
            continue;
        };
        let mut version = String::from_utf8_lossy(&output.stdout).into_owned();
        version.push_str(&String::from_utf8_lossy(&output.stderr));
        if version.contains(dialect.marker) {
            return Some(candidate);
        }
    }
    None
}

fn required_lua_dialect(dialect: &LuaDialect) -> PathBuf {
    find_lua_dialect(dialect).unwrap_or_else(|| {
        panic!(
            "the {} interpreter is required for R4_LUA_DIALECTS={}; install it or set {}",
            dialect.name, dialect.key, dialect.env
        )
    })
}

/// A temporary chunk file, removed when the run finishes.
struct TempChunk {
    path: PathBuf,
}

impl TempChunk {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "r4-lua-oracle-{}-{}.lua",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, source).expect("the oracle chunk must be written");
        Self { path }
    }
}

impl Drop for TempChunk {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Runs `program` in an external interpreter and returns its captured behavior.
fn observe_with(binary: &Path, program: &str) -> String {
    let chunk = TempChunk::new(&lua_harness(program));
    let output = Command::new(binary)
        .arg(&chunk.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("failed to start `{}`: {error}", binary.display()));
    assert!(
        output.status.success(),
        "{} could not run the program:\n{}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the interpreter output is UTF-8")
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

/// Analyzes `block` in `dialect` for an oracle transformation.
fn analyze_with_dialect(block: &r4::nodes::Block, dialect: Dialect) -> r4::semantic::SemanticModel {
    SemanticAnalyzer::new()
        .with_dialect(dialect)
        .analyze(block)
        .expect("the oracle program must analyze")
}

/// Applies r4's canonicalization and optimizer passes to `program`, re-analyzing
/// the tree before each pass so every pass sees a matching model, and returns
/// the generated Luau.
fn optimize(program: &str) -> String {
    optimize_with_dialect(program, Dialect::Luau)
}

fn optimize_with_dialect(program: &str, dialect: Dialect) -> String {
    let mut block = Parser::default()
        .dialect(dialect)
        .parse(program)
        .unwrap_or_else(|error| {
            panic!(
                "the oracle program must parse as {}: {error}",
                dialect.as_str()
            )
        });

    let model = analyze_with_dialect(&block, dialect);
    canonicalize(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::inline_local_functions(&mut block, &model)
        .expect("inlining local functions must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::propagate_constants(&mut block, &model)
        .expect("propagating constants must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::compute_expression(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::group_local(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_dead_table_stores(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_unused_variables(&mut block, &model)
        .expect("removing unused variables must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_dead_stores(&mut block, &model)
        .expect("removing dead stores must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_unreachable_code(&mut block, &model)
        .expect("removing unreachable code must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_unused_labels(&mut block, &model)
        .expect("removing unused labels must succeed");

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::remove_assertions(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::convert_square_root_call(&mut block, &model);

    let model = analyze_with_dialect(&block, dialect);
    canonicalize::hoist_loop_invariants(&mut block, &model)
        .expect("hoisting loop invariants must succeed");

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

/// Programs that exercise r4's canonicalization and optimizer passes.
///
/// Several of them contain an operation that raises a runtime error but whose
/// value is unused, overwritten, or loop invariant. A pass that deleted or moved
/// that evaluation would change observable behavior. The program catches the
/// error with `pcall` and prints whether it happened, rather than letting the
/// harness capture the error text, because that text carries a source line
/// number which a transformation may legitimately shift.
const OPTIMIZED_PROGRAMS: &[(&str, &str)] = &[
    (
        "constant folding",
        r#"
local value = 1 + 2 * 3
print(value)
return value
"#,
    ),
    (
        "unused variable and dead store",
        r#"
local unused = 42
local kept = 1
kept = 2
print(kept)
return kept
"#,
    ),
    (
        "unreachable loop",
        r#"
local total = 0
while false do
    total = total + 1
end
print(total)
return total
"#,
    ),
    (
        "adjacent locals merge",
        r#"
local a = 1
local b = 2
print(a + b)
return a + b
"#,
    ),
    (
        "index to field and nil declaration",
        r#"
local t = { value = 10, other = 20 }
local first, second = t["value"], nil
print(first, second)
return t["other"]
"#,
    ),
    (
        "dead table store",
        r#"
local t = {}
t.x = 1
t.x = 2
print(t.x)
return t.x
"#,
    ),
    (
        "loop invariant",
        r#"
local total = 0
for i = 1, 3 do
    local scale = 10
    total = total + i * scale
end
print(total)
return total
"#,
    ),
    (
        "square root call",
        r#"
print(math.sqrt(16))
return math.sqrt(9)
"#,
    ),
    (
        "passing assertion",
        r#"
local function check(value)
    assert(value > 0)
    return value
end
print(check(5))
return check(1)
"#,
    ),
    (
        "constant condition",
        r#"
local flag = 1 < 2
if flag then
    print("yes")
else
    print("no")
end
return flag
"#,
    ),
    (
        "raising value in a dead store",
        r#"
local ok = pcall(function()
    local value = 0
    value = "text" + 1
    value = 2
    return value
end)
print(ok)
return ok
"#,
    ),
    (
        "raising initializer of an unused local",
        r#"
local ok = pcall(function()
    local unused = "text" + 1
    return 0
end)
print(ok)
return ok
"#,
    ),
    (
        "raising value in a dead table store",
        r#"
local ok = pcall(function()
    local t = {}
    t.value = "text" + 1
    return 0
end)
print(ok)
return ok
"#,
    ),
    (
        "invariant that would raise in a zero-iteration loop",
        r#"
local function run(flag)
    local base = "text"
    local total = 0
    while flag do
        local offset = base + 1
        total = total + offset
    end
    return total
end
print(run(false))
return run(false)
"#,
    ),
    (
        "loop accumulator that must not be hoisted",
        r#"
local total = 0
for i = 1, 3 do
    local acc = 10
    acc = acc + i
    total = total + acc
end
print(total)
return total
"#,
    ),
    (
        "pure standard-library call with a bad argument",
        r#"
local ok = pcall(function()
    local value = math.floor({})
    return 0
end)
print(ok)
return ok
"#,
    ),
    (
        "constant propagation through locals",
        r#"
local width = 4
local height = width + 3
local area = width * height
print(area)
return width, height, area
"#,
    ),
    (
        "constant that is not provable",
        r#"
local total = 0
for i = 1, 3 do
    total = total + i
end
print(total)
return total
"#,
    ),
    (
        "function value identity is preserved",
        r#"
local f = function() return 1 end
local g = f
print(f == g)
return f == g
"#,
    ),
    (
        "inlined local helper",
        r#"
local function double(value)
    return value * 2
end
print(double(21))
return double(20)
"#,
    ),
    (
        "helper with a statement body is inlined",
        r#"
local function add(a, b)
    local sum = a + b
    return sum
end
print(add(1, 2))
return add(3, 4)
"#,
    ),
    (
        "helper with a return inside a loop is inlined",
        r#"
local function first_positive(limit)
    for i = 1, limit do
        if i > 0 then
            return i
        end
    end
    return 0
end
print(first_positive(3))
return first_positive(4)
"#,
    ),
    (
        "multi-value return in a table",
        r#"
local function pair()
    return 1, 2
end
local values = { pair() }
print(#values, values[1], values[2])
return #values
"#,
    ),
    (
        "call used as a prefix",
        r#"
local function make(value)
    return function() return value end
end
print(make(7)())
return make(8)()
"#,
    ),
    (
        "helper that forwards varargs is not inlined",
        r#"
local function forward(...)
    return ...
end
print(forward(1, 2))
return forward(3, 4)
"#,
    ),
    (
        "observable argument is not duplicated",
        r#"
local calls = 0
local function twice(value)
    return value + value
end
local function bump()
    calls = calls + 1
    return calls
end
print(twice(bump()), calls)
return calls
"#,
    ),
    (
        "function bound by a separate assignment is inlined",
        r#"
local add
add = function(a, b)
    return a + b
end
print(add(1, 2))
return add(3, 4)
"#,
    ),
    (
        "assignment-bound helper with a loop is inlined",
        r#"
local total
total = function(limit)
    local sum = 0
    for i = 1, limit do
        sum = sum + i
    end
    return sum
end
print(total(3))
return total(4)
"#,
    ),
    (
        "assignment-bound helper is read before its assignment",
        r#"
local f
local missing = f == nil
f = function()
    return 7
end
print(missing, f())
return missing and f() or 0
"#,
    ),
    (
        "assignment-bound helper called under a conditional",
        r#"
local f
f = function(value)
    if value > 0 then
        return value
    end
    return -value
end
local result = 0
if f(-2) > 0 then
    result = f(3)
end
print(result)
return result
"#,
    ),
    (
        "an inlined iterator expression keeps its place",
        r#"
local iterations = 0
local value = 1
local function set()
    value = 5
    return 0
end
local function iterator(state, control)
    if control >= state then
        return nil
    end
    return control + 1
end
for item in iterator, value, set() do
    iterations = iterations + 1
end
print(iterations, value)
return iterations
"#,
    ),
    (
        "assigned local alias is inlined",
        r#"
local function add(a, b)
    return a + b
end
local sum
sum = add
print(sum(1, 2))
return sum(3, 4)
"#,
    ),
    (
        "assignment-bound helper alias declared after the assignment",
        r#"
local f
f = function(x)
    return x * 2
end
local g = f
print(g(2))
return g(3)
"#,
    ),
    (
        "assignment-bound helper declared in an unconditional block",
        r#"
local f
do
    f = function(x)
        return x + 1
    end
end
print(f(1))
return f(2)
"#,
    ),
    (
        "upvalue write is not inlined under a shadowing local",
        r#"
local value = 0
local function set()
    value = 5
end
do
    local value = 100
    set()
end
print(value)
return value
"#,
    ),
    (
        "upvalue prefix read is not inlined under a shadowing local",
        r#"
local box = { value = 1 }
local function read()
    return box.value
end
local result
do
    local box = { value = 2 }
    result = read()
end
print(result)
return result
"#,
    ),
    (
        "shadowed global is not inlined",
        r#"
local function hello()
    return tostring(1)
end
local result
do
    local tostring = function() return "local" end
    result = hello()
end
print(result)
return result
"#,
    ),
    (
        "nested return upvalue is not inlined under a shadowing local",
        r#"
local value = 1
local function make()
    return function()
        return value
    end
end
local result
do
    local value = 2
    result = make()()
end
print(result)
return result
"#,
    ),
    (
        "a binary operand keeps its place",
        r#"
local value = 1
local function set()
    value = 5
    return 0
end
print(value + set())
return value
"#,
    ),
    (
        "a global binary operand is read before the other side",
        r#"
X = 1
local function set()
    X = 5
    return 0
end
print(X + set())
return X
"#,
    ),
    (
        "a local comparison operand is read after the other side",
        r#"
local value = 1
local function set()
    value = 2
    return 2
end
print(value == set())
return value
"#,
    ),
    (
        "a concat operand is read before the other side",
        r#"
local value = "a"
local function set()
    value = "b"
    return "c"
end
print(value .. set())
return value
"#,
    ),
    (
        "a local index base is read after the key",
        r#"
local t = { [1] = "one", [2] = "two" }
local function change()
    t = { [1] = "changed", [2] = "changed" }
    return 1
end
print(t[change()])
return 0
"#,
    ),
    (
        "a list table entry keeps its place",
        r#"
local value = 1
local function set()
    value = 5
    return 0
end
local t = { value, set() }
print(t[1], t[2])
return value
"#,
    ),
    (
        "a keyed table entry keeps its place",
        r#"
local value = 1
local function set()
    value = 5
    return 0
end
local t = { item = value, other = set() }
print(t.item, t.other)
return value
"#,
    ),
    (
        "an inlined assignment value keeps its place",
        r#"
local value = 1
local function set()
    value = 5
    return 0
end
local first, second = value, set()
print(first, second)
return value
"#,
    ),
    (
        "an inlined return value keeps its place",
        r#"
local value = 1
local function set()
    value = 5
    return 0
end
local function pair()
    return value, set()
end
print(pair())
return value
"#,
    ),
    (
        "an inlined argument keeps its place among the arguments",
        r#"
local value = 1
local function set(x)
    value = x
    return value
end
print(value, set(5))
return value
"#,
    ),
    (
        "nested argument calls keep their order",
        r#"
local calls = 0
local function bump()
    calls = calls + 1
    return calls
end
local function wrap(x)
    return x
end
print(wrap(bump()))
return calls
"#,
    ),
    (
        "an inlined body is not hoisted out of a short-circuit",
        r#"
local calls = 0
local function bump()
    calls = calls + 1
    return 1
end
print(false and bump())
return calls
"#,
    ),
    (
        "a while condition is re-evaluated every iteration",
        r#"
local calls = 0
local function limit()
    calls = calls + 1
    return calls < 3
end
local iterations = 0
while limit() do
    iterations = iterations + 1
    if iterations > 5 then
        break
    end
end
print(calls, iterations)
return calls
"#,
    ),
    (
        "a repeat condition is re-evaluated every iteration",
        r#"
local calls = 0
local function stop()
    calls = calls + 1
    return calls >= 3
end
local iterations = 0
repeat
    calls = calls + 1
    iterations = iterations + 1
until stop() or iterations > 5
print(calls, iterations)
return calls
"#,
    ),
    (
        "method arguments preserve the dialect's lookup order",
        r#"
local log = {}
local function note(value)
    log[#log + 1] = value
end
local function argument(value)
    note(value)
    return value
end
local target = setmetatable({}, {
    __index = function(_, key)
        note("lookup:" .. key)
        return function(self, value)
            note("call:" .. tostring(value))
        end
    end,
})
target:method(argument("first"), argument("second"))
print(table.concat(log, ","))
return #log
"#,
    ),
    (
        "a while condition sees the outer binding, not the body's shadow",
        r#"
local calls = 0
local function limit(x)
    calls = calls + 1
    return calls < 3 and x
end
local body = 0
while limit(true) do
    body = body + 1
    local calls = 100
    print(body, calls)
end
print(calls)
return calls, body
"#,
    ),
    (
        "a repeat condition sees a body local",
        r#"
local iterations = 0
local function done(value)
    return value >= 3
end
repeat
    iterations = iterations + 1
    local current = iterations
    print("iteration", current)
until done(current)
print("after", iterations)
return iterations
"#,
    ),
    (
        "a luau repeat condition runs after continue",
        r#"
local value = 0
local function done()
    return value >= 4
end
repeat
    value = value + 1
    if value == 2 then continue end
    print(value)
until done()
print("done", value)
return value
"#,
    ),
    (
        "a luau repeat condition reads a local before continue",
        r#"
local value = 0
local function done(step)
    return step >= 3
end
repeat
    value = value + 1
    local step = value
    if value == 2 then continue end
    print(step)
until done(step)
print("done", value)
return value
"#,
    ),
    (
        "conditional assignments with the same body inline",
        r#"
local choose = 1
local f
if choose == 1 then
    f = function(x)
        return x + 10
    end
else
    f = function(x)
        return x + 10
    end
end
local g = function()
    return f(2)
end
print(f(1), g())
return f(3), g()
"#,
    ),
    (
        "an alias keeps the value it captured",
        r#"
local f = function(x)
    return x + 1
end
local g = f
f = function(x)
    return x + 2
end
print(f(1), g(1))
return f(2), g(2)
"#,
    ),
    (
        "an unreachable repeat condition stays in place",
        r#"
local function done()
    return true
end
local function run()
    repeat
        return 1
    until done()
end
print(run())
return 0
"#,
    ),
    (
        "a luau repeat condition handles multiple continues",
        r#"
local value = 0
local function done(step)
    return step >= 5
end
repeat
    value = value + 1
    local step = value
    if step == 2 then continue end
    do
        if step == 3 then continue end
    end
    print(step)
until done(step)
print("done", value)
return value
"#,
    ),
    (
        "a luau while condition runs after continue",
        r#"
local value = 0
local function keep()
    return value < 4
end
while keep() do
    value = value + 1
    if value == 2 then continue end
    print(value)
end
print(value)
return value
"#,
    ),
    (
        "a method call inlines its receiver and arguments",
        r#"
local function make()
    return { m = function(self, value) return value end }
end
local function argument()
    return 7
end
print(make():m(argument()))
return 0
"#,
    ),
    (
        "a repeat condition calls a body-local function",
        r#"
local i = 0
repeat
    i = i + 1
    local function check(value)
        return value >= 2
    end
    local current = i
until check(current)
print(i)
return i
"#,
    ),
    (
        "a luau repeat condition calls a body-local function across continue",
        r#"
local i = 0
repeat
    i = i + 1
    local function check(value)
        return value >= 3
    end
    local current = i
    if i == 2 then continue end
    print(current)
until check(current)
print(i)
return i
"#,
    ),
    (
        "a luau repeat condition leaves a nested loop's continue alone",
        r#"
local outer = 0
local function done(value)
    return value >= 2
end
repeat
    outer = outer + 1
    for inner = 1, 2 do
        if inner == 1 then continue end
        print("inner", inner)
    end
    print("outer", outer)
until done(outer)
print(outer)
return outer
"#,
    ),
    (
        "an elseif condition is not hoisted",
        r#"
local calls = 0
local function check()
    calls = calls + 1
    return true
end
if true then
    print("taken")
elseif check() then
    print("elseif")
end
print(calls)
return calls
"#,
    ),
];

/// Programs that exercise Lua 5.2+ syntax in the external dialect oracle.
/// They are not part of the embedded Luau run because Luau has no `goto`.
const EXTERNAL_ONLY_PROGRAMS: &[(&str, &str)] = &[
    (
        "inlining hoists results around goto",
        r#"
local function bump(value)
    return value + 1
end
goto done
print(bump(1))
::done::
print(bump(41))
return bump(0)
"#,
    ),
    (
        "a callee label does not collide with a caller label",
        r#"
local function value()
    goto done
    ::done::
    return 1
end
::done::
print(value())
return 0
"#,
    ),
    (
        "a reachable call hoists its result around a later goto",
        r#"
local function value(x)
    return x
end
local first = value(1)
goto done
print(first)
::done::
print(first)
return first
"#,
    ),
    (
        "a repeat condition after a nested block-end label is inlined",
        r#"
local function done()
    return true
end
local count = 0
repeat
    count = count + 1
    do
        goto continue
        local skipped = 1
        ::continue::
    end
until done()
print(count)
return count
"#,
    ),
    (
        "a callee with goto and labels is inlined",
        r#"
local function pick(value)
    if value > 0 then
        goto positive
    end
    do
        return -1
    end
    ::positive::
    return 1
end
print(pick(1), pick(-1))
return pick(2)
"#,
    ),
    (
        "a while condition is re-evaluated around a goto label",
        r#"
local function keep(n)
    return n < 3
end
local n = 0
while keep(n) do
    n = n + 1
    if n == 2 then
        goto continue
    end
    print(n)
    ::continue::
end
print(n)
return n
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
fn optimized_programs_keep_identical_behavior() {
    for (name, program) in OPTIMIZED_PROGRAMS {
        let before = observe(program);
        let optimized = optimize(program);
        let after = observe(&optimized);
        assert_eq!(
            before, after,
            "`{name}` changed behavior after optimization:\n{optimized}"
        );
    }
}

/// Number of seeded fuzz programs compared under Lua 5.4.
const FUZZ_PROGRAMS: u64 = 40;

/// Builds a deterministic, terminating Lua 5.4 program from `seed`.
///
/// The programs exercise helper inlining, assignment-bound helpers, aliases,
/// upvalue reads and writes, bounded loops, tables, nested functions, shadowed
/// names, and calls in arguments, operands, index keys, and table entries.
fn generate_fuzz_program(seed: u64) -> String {
    let mut rng = StdRng::seed_from_u64(seed);
    let base = rng.random_range(1..=9);
    let value = rng.random_range(1..=9);
    let tag = ["alpha", "beta", "gamma"][rng.random_range(0..3)];

    let mut lines: Vec<String> = vec![
        format!("local counter = {base}"),
        format!("local value = {value}"),
        format!("local tag = {tag:?}"),
        "local function bump(step)".to_string(),
        "    counter = counter + step".to_string(),
        "    return counter".to_string(),
        "end".to_string(),
        "local function set_value(next)".to_string(),
        "    value = next".to_string(),
        "    return value".to_string(),
        "end".to_string(),
        "local function read_value()".to_string(),
        "    return value".to_string(),
        "end".to_string(),
        "local function combine(a, b)".to_string(),
        "    return a + b".to_string(),
        "end".to_string(),
        "local function choose(n)".to_string(),
        format!("    if n > {} then", rng.random_range(1..=9)),
        "        return n".to_string(),
        "    end".to_string(),
        "    return n + 1".to_string(),
        "end".to_string(),
        "local function sum_to(n)".to_string(),
        "    local total = 0".to_string(),
        "    for i = 1, n do".to_string(),
        "        total = total + i".to_string(),
        "    end".to_string(),
        "    return total".to_string(),
        "end".to_string(),
        "local function make(x)".to_string(),
        "    return function()".to_string(),
        "        return x".to_string(),
        "    end".to_string(),
        "end".to_string(),
        "local later".to_string(),
        format!(
            "later = function(x) return x + {} end",
            rng.random_range(1..=9)
        ),
        "local alias".to_string(),
        "alias = combine".to_string(),
        "local captured = combine".to_string(),
        "combine = function(a, b)".to_string(),
        "    return a * b".to_string(),
        "end".to_string(),
        "local switch".to_string(),
        "if value % 2 == 0 then".to_string(),
        "    switch = function(x) return x + base end".to_string(),
        "else".to_string(),
        "    switch = function(x) return x + base end".to_string(),
        "end".to_string(),
        "local method_log = {}".to_string(),
        "local target = setmetatable({}, {".to_string(),
        "    __index = function(_, key)".to_string(),
        "        method_log[#method_log + 1] = tostring(counter)".to_string(),
        "        return function(self, x) return x end".to_string(),
        "    end,".to_string(),
        "})".to_string(),
        "local function keep_going(n)".to_string(),
        "    return n < 3".to_string(),
        "end".to_string(),
        "local function finished(step)".to_string(),
        "    return step >= 3".to_string(),
        "end".to_string(),
    ];

    let statements = rng.random_range(4..=8);
    let mut next = 0usize;
    for _ in 0..statements {
        let step = rng.random_range(1..=5);
        match rng.random_range(0..15) {
            0 => lines.push(format!("print(counter, bump({step}))")),
            1 => lines.push(format!("print(value + bump({step}))")),
            2 => lines.push(format!("print(tag .. bump({step}))")),
            3 => {
                lines.push(format!(
                    "local t{next} = {{ value, bump({step}), item = value, other = bump({step}) }}"
                ));
                lines.push(format!(
                    "print(t{next}[1], t{next}.item, t{next}[2], t{next}.other)"
                ));
                lines.push(format!("print(t{next})"));
                next += 1;
            }
            4 => {
                lines.push(format!("local n{next} = combine(counter, {step})"));
                lines.push(format!("print(n{next}, choose(n{next}))"));
                next += 1;
            }
            5 => lines.push(format!("print(sum_to({}))", rng.random_range(1..=4))),
            6 => lines.push(format!("print(make({step})())")),
            7 => lines.push(format!("print(later({step}), alias({step}, {step}))")),
            8 => lines.push(format!(
                "do local value = {step}; print(value, read_value(), bump({step})) end"
            )),
            9 => {
                lines.push(format!(
                    "for i = 1, {} do counter = counter + i end",
                    rng.random_range(1..=4)
                ));
                lines.push("print(counter)".to_string());
            }
            10 => {
                lines.push(format!(
                    "print(target:method(bump({step})), table.concat(method_log, \",\"))"
                ));
            }
            11 => {
                lines.push("local n = 0".to_string());
                lines.push("while keep_going(n) do".to_string());
                lines.push("    n = n + 1".to_string());
                lines.push("    counter = counter + n".to_string());
                lines.push("    local n = 0".to_string());
                lines.push("    print(n)".to_string());
                lines.push("end".to_string());
                lines.push("print(counter, n)".to_string());
            }
            12 => {
                lines.push("local r = 0".to_string());
                lines.push("repeat".to_string());
                lines.push("    r = r + 1".to_string());
                lines.push("    local step = r".to_string());
                lines.push("    counter = counter + step".to_string());
                lines.push("    print(step)".to_string());
                lines.push("until finished(step)".to_string());
                lines.push("print(counter, r)".to_string());
            }
            13 => {
                lines.push(format!(
                    "print(switch({step}), captured({step}, {step}), combine({step}, {step}))"
                ));
            }
            _ => {
                lines.push("local closure = function()".to_string());
                lines.push(format!("    return switch({step})"));
                lines.push("end".to_string());
                lines.push("print(closure())".to_string());
            }
        }
    }

    lines.push("print(counter, value, tag)".to_string());
    lines.push("return counter, value".to_string());
    lines.join("\n")
}

#[test]
fn optimized_programs_keep_identical_behavior_in_lua_dialects() {
    let dialects = selected_lua_dialects();
    if dialects.is_empty() {
        eprintln!("skipping the external Lua oracle: set R4_LUA_DIALECTS to run it");
        return;
    }
    for dialect in dialects {
        let binary = required_lua_dialect(dialect);
        let mut compared = 0;
        for (name, program) in OPTIMIZED_PROGRAMS
            .iter()
            .chain(EXTERNAL_ONLY_PROGRAMS.iter())
        {
            if Parser::default()
                .dialect(dialect.dialect)
                .parse(program)
                .is_err()
            {
                continue;
            }
            let optimized = optimize_with_dialect(program, dialect.dialect);
            let before = observe_with(&binary, program);
            let after = observe_with(&binary, &optimized);
            assert_eq!(
                before, after,
                "`{name}` changed behavior under {}:\n{optimized}",
                dialect.name
            );
            compared += 1;
        }
        assert!(compared > 0, "no oracle program is valid {}", dialect.name);
    }
}

#[test]
fn fuzzed_programs_keep_identical_behavior_in_lua_dialects() {
    let dialects = selected_lua_dialects();
    if dialects.is_empty() {
        eprintln!("skipping the external Lua oracle: set R4_LUA_DIALECTS to run it");
        return;
    }
    for dialect in dialects {
        let binary = required_lua_dialect(dialect);
        for seed in 0..FUZZ_PROGRAMS {
            let program = generate_fuzz_program(seed);
            Parser::default()
                .dialect(dialect.dialect)
                .parse(&program)
                .unwrap_or_else(|error| {
                    panic!("fuzz program {seed} is not valid {}: {error}", dialect.name)
                });
            let optimized = optimize_with_dialect(&program, dialect.dialect);
            let before = observe_with(&binary, &program);
            let after = observe_with(&binary, &optimized);
            assert_eq!(
                before, after,
                "fuzzed program {seed} changed behavior under {}:\n--- source ---\n{program}\n--- optimized ---\n{optimized}",
                dialect.name
            );
        }
    }
}

#[test]
fn fuzzed_programs_keep_identical_behavior_in_luau() {
    for seed in 0..FUZZ_PROGRAMS {
        let program = generate_fuzz_program(seed);
        let optimized = optimize(&program);
        let before = observe(&program);
        let after = observe(&optimized);
        assert_eq!(
            before, after,
            "fuzzed program {seed} changed behavior under Luau:\n--- source ---\n{program}\n--- optimized ---\n{optimized}"
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
