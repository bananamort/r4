# r4

A semantically-aware and control-flow tracking Lua analysis and transformation
engine.

r4 parses Lua and Luau into a clean, self-contained, `Send + Sync` AST, builds a
semantic model (scopes, symbols, types, goto/label resolution), constructs a
control-flow graph, and provides dialect-safe canonicalization, desugaring and
verification passes. It supports Lua 5.1 through 5.4, LuaJIT and Luau, and can
lower modern Luau down to vanilla Lua 5.1.

## Highlights

- **Dialect-safe.** Detects the dialect from the source in a single lexer pass
  and validates transformed output against an explicit target dialect, failing
  fast with typed errors instead of emitting unrepresentable syntax.
- **Semantically aware.** Scope tree, symbol declarations, read/write tracking,
  immutable-alias resolution, goto/label validation, escape analysis and type
  aliases — all on top of the AST without mutating it.
- **Control-flow graph.** Reachability, dominators, liveness, constant
  propagation and loop-invariant analysis, linked back to the AST by structural
  block identities.
- **Typed errors everywhere.** Malformed state (stale models, unresolved
  bindings, unreachable dialects) is a typed error, never a guess or a silent
  drop.

## Usage

```rust
use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::rules::desugar_to_lua51;
use r4::semantic::SemanticAnalyzer;
use r4::Parser;

let mut block = Parser::default()
    .luau()
    .parse("local total = 0\nfor i = 1, 3 do total += i end\nreturn total")
    .expect("valid Luau");

let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
assert!(model.symbols.iter().any(|symbol| symbol.name == "total"));

desugar_to_lua51(&mut block).expect("lowering to Lua 5.1");

let mut generator = ReadableLuaGenerator::default();
generator.write_block(&block);
assert!(generator.into_string().contains("total"));
```

## Toolchain

r4 requires **Rust 1.89 or newer** (see `rust-version` in `Cargo.toml`). CI
builds the crate on that exact version as well as the latest stable.

## Testing

```sh
cargo test --workspace --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo llvm-cov --workspace --all-features --fail-under-lines 94
```

The `luau_execution_oracle_test` integration test runs a corpus of programs in
an embedded Lune VM (Roblox runtime included), both before and after r4's
transformations, and asserts byte-identical observable behavior. It also reruns
the optimized programs and a seeded fuzz corpus under an external Lua 5.4
interpreter, so `lua5.4` must be on `PATH` (or `LUA54` set) for that part.
Local runs skip it when no Lua 5.4 is found; CI installs it and requires it.

## License

MIT — see [LICENSE](LICENSE).
