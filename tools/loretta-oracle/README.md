# Loretta scoping oracle

A tiny .NET console app that runs the **published** Loretta compiler over a Lua
source and prints a canonical, index-independent description of its scope tree.
It exists so r4's semantic model can be verified against Loretta's scoping
engine using the canonical package — never a vendored checkout.

## Canonical source

- Package: `Loretta.CodeAnalysis.Lua` **0.2.14-nightly.26**, restored from
  nuget.org (the latest published nightly at the time of writing; verify with
  the flat-container index for `loretta.codeanalysis.lua`).
- The local `references/` directory is **not** a source of truth: it is
  gitignored and no test or build step reads it.

## Usage

```sh
printf "local a = 1\ndo local a = 2 end" | \
  dotnet run --project tools/loretta-oracle -- Luau
```

Presets: `Luau`, `Roblox`, `All`, `Lua51`, `Lua52`, `Lua53`, `Lua54`, `LuaJIT`.

Output:

```json
{"diagnostics":[],"canonical":"Global(vars=[...];labels=[...];children=[...])"}
```

- `vars` lists each declared variable as `name:kind` (`Local`, `Global`,
  `Parameter`, `Iteration`), sorted.
- `labels` lists the scope's goto labels, sorted.
- `children` recurses into contained scopes, sorted.

Loretta declares implicit top-level `...`/`arg` parameters and a `...`
parameter for variadic functions; r4 does not model these as symbols.
`tests/loretta_scoping_test.rs` drops them before comparing, so every other
scope kind, nesting, declaration and label must match exactly.

## Regenerating expectations

`tests/loretta_scoping_test.rs` embeds the canonical trees for its corpus. To
regenerate them, run the oracle over each program (Luau preset), drop the
implicit `...:Parameter` / `arg:Parameter` entries, and replace the
`LORETTA_SCOPE_TREES` table. The embedded values are a snapshot of the packaged
engine's behaviour, so they must be produced by this oracle, not hand-edited.
