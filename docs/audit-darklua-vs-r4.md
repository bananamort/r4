# Architectural Audit — `darklua-0.19.0` vs `r4`

Date: 2026-09-12
Last updated: remaining-open-items mission (mandatory models, type-name
resolution, LICM, and model-first `require` conversion/bundling).
Scope: `references/darklua-0.19.0/src/` versus `r4`'s read-only analysis layers
(`src/semantic/`, `src/cfg/`).

**Method.** Every cited file was read end-to-end. The type/visitor plumbing
(`Scope`, `ScopeVisitor`, `IdentifierTracker`, `DefaultVisitor`, `Permutator`)
was traced, and the exact APIs present in `r4::semantic` and `r4::cfg` were
checked. Claims that could not be reproduced from the code are flagged.

darklua citations are pinned to the frozen reference checkout. `r4` citations
track the current API by name; line numbers were refreshed at the commit above
and may move again.

| Claim | Verdict |
|---|---|
| A — Global blacklist / collision bug | **PARTIALLY TRUE** (mechanism confirmed; stated consequence over-stated; two worse latent bugs found nearby) |
| B — Std-lib caching false positives | **CONFIRMED** (mechanism is order/scope-aware, not flat; alias-blind) |
| C — Dead code & control-flow blindness | **CONFIRMED** |
| D — `convert_require` scope bypass | **CONFIRMED** (genuine dormant semantic bug) |
| E — Quadratic re-traversal | **CONFIRMED** (superlinear + fixed point; r4 comparison corrected) |

---

## A. Global blacklist & `local pcall = pcall`

1. `CollectGlobalsProcessor` is not a naive "everything is global" walker: it
   keeps a scope stack (`process/processors/collect_globals.rs:10-12`) and only
   records an identifier when it is free in every active scope
   (`collect_globals.rs:26-31`, `68-73`). The **output** is one flat
   `HashSet` (`collect_globals.rs:11`, drained at `37-39`), so A.1 is true of
   the output only.
2. The failure case comes from traversal order: `ScopeVisitor::visit_local_assign`
   visits RHS values before `insert_local` (`process/scope_visitor.rs:57-70`).
   In `local pcall = pcall`, the RHS is seen while no local `pcall` is active,
   so `"pcall"` enters the flat set (`collect_globals.rs:70-72`).
3. `RenameVariables::flawless_process` (not `process`) stacks configured globals
   (default `DEFAULT`), local-function names, and the collected set into
   `RenameProcessor::new` (`rules/rename_variables/mod.rs:111-135`, esp.
   `126-133`). A.2 confirmed.
4. The gate is `filter_identifier` (not `is_available`,
   `rules/rename_variables/rename_processor.rs:64-67`) and it is consulted only
   while generating new names (`rename_processor.rs:50-62`). A.3 confirmed
   modulo the name.
5. Actual effect of `local pcall = pcall`: the local is renamed *away* from
   `pcall` (`insert_local` → `replace_identifier`,
   `rename_processor.rs:139-149`, `69-76`); the global RHS is never renamed.
   `pcall` can never be a rename *target* anywhere in the chunk. This is
   avoid-set pollution, not a miscompile in the default configuration.

### Additional bugs found while verifying A

- **A′ — Reuse pool bypasses the avoid filter.** `generate_identifier` returns
  pooled names without `filter_identifier` (`rename_processor.rs:50-52` vs
  `54-60`), while the pool is filled on scope pop (`127-136`). With
  `detect_globals=true` (default) the pre-collected set masks this, but the
  dynamically inserted avoid names (`rename_processor.rs:166`) are not checked
  by the reuse path. With `detect_globals=false`
  (`rename_variables/mod.rs:45-53`) this is a live miscompile: a local can be
  renamed onto a global name and capture later global references.
  `filter_identifier` also never consults active scope mappings, so safety rests
  on permutator monotonicity plus reuse discipline.
- **A″ — `detect_globals = false` is unsound by design.** Any omitted global can
  be generated as a rename target, silently changing resolution.

### r4 contrast

`r4` separates `SymbolKind::Global` from `Local` at declaration time
(`SymbolKind` in `src/semantic/symbol.rs`) and binds every occurrence to a
`SymbolId` (`SemanticModel::resolve_identifier`,
`src/semantic/model.rs:103-125`; `uses_of` at `210`). Scope resolution walks
the lexical chain (`resolve_in_scope`, `model.rs:136-160`), so shadowed
`pcall` locals are distinct symbols and global reads are separate symbols with
their own use lists and scopes.

---

## B. Std-lib caching false positives (`local math = math`)

| Rule | Guard |
|---|---|
| `rules/convert_square_root_call.rs:28-54` | `math` + `!is_identifier_used("math")` |
| `rules/remove_assertions.rs:32-44` | `is_identifier_used("assert") → false` |
| `rules/remove_debug_profiling.rs:31-45` | `is_identifier_used("debug") → false` |
| `rules/remove_floor_division.rs:45-58`, `101-111` | shadowed `math` → `__DARKLUA_MATH_FLOOR = math.floor` alias |
| `rules/remove_interpolated_string.rs:65-145`, `165-199` | shadowed `tostring`/`string` → alias definitions |

**Mechanism correction.** `IdentifierTracker` records declarations only via the
`Scope` impl (`process/scope_visitor.rs:508-531`); references are not recorded.
Because values are visited before `insert_local`, a `math.sqrt` *before* the
declaration is still optimized and one *after* is skipped/aliased. It is
order/scope-aware, but boolean: it cannot distinguish an immutable alias
(`local math = math`) from an overwrite (`local math = {}`) or a reassignment.

**r4 opportunity.** Use the `Symbol` write counts (`symbol.rs:47-49`) plus the
declaration initializer: alias iff `write_count == 1` and the initializer
resolves to the corresponding `SymbolKind::Global`. Then rules can optimize
through read-only aliases instead of skipping or emitting synthetic top-level
aliases.

---

## C. Dead code & control-flow blindness

**`rules/filter_early_return.rs` — CONFIRMED.** `search_remove_after`
(`13-44`) only matches a `do` whose block ends in `Return` (`18-25`) or
recursively contains such a `do` (`27`); `Break`/`Continue` return `None`
(`22-23`); every other statement kind returns `None`, including `If`, `While`,
`Repeat`, `NumericFor`, `GenericFor` (`30-42`). `process_block` then truncates
after the matched `do` and drops the block's last statement (`47-53`). It cannot
see dead code after `if true then return end`, `while true do ... end`,
both-branches-return `if`, loops that always return, or `error(...)`.

**`rules/unused_if_branch.rs` — CONFIRMED.** All decisions come from
`Evaluator::evaluate` plus `has_side_effects` (`21-84`, `86-168`, `175-199`).
It carefully preserves side-effecting conditions, but has no CFG and cannot see
that statements after an if whose branches all return are dead.

**`rules/unused_while.rs` — CONFIRMED.** Constant condition only (`14-29`),
`unwrap_or(true)` keeps unknowns; the body and following code are never
analyzed.

**r4 contrast.** `r4::cfg` builds `Branch`/`ForLoop`/`Return`/`Break`/
`Continue`/`Unreachable` terminators with bidirectional pred/succ edges
(`src/cfg/block.rs:19-60`), and `Reachability` provides `is_reachable` and
`unreachable_blocks` (`src/cfg/analysis/reachability.rs:17,44,58`). Caveats
before replacing the three rules:

1. Reachability is structural; a constant-false `Branch` still keeps both
   edges. Constant-aware edge folding is required so `while false` /
   `while true` expose dead blocks.
2. `BasicBlock::statements` are clones, so CFG analysis needs a provenance
   mapping before it can delete source statements.

Both caveats are now addressed: `cfg::analysis::fold_constant_branches_with`
folds constant edges, and the builder records structural
`StatementOrigin`/`ConditionOrigin`s (see "Structural semantic layer" below).

---

## D. Scope visitor bypasses & dormant bugs

**CONFIRMED — real semantic bug.** `is_require_call` refuses to treat a call as
the global `require` when the tracker says `require` is declared
(`rules/require/match_require.rs:13-24`). `RequireConverter` embeds such a
tracker (`rules/convert_require/mod.rs:101-120`), but `ConvertRequire::process`
runs `DefaultVisitor::visit_block` (`convert_require/mod.rs:207-209`).
`DefaultVisitor` never invokes the `Scope` trait (`process/visitors.rs:7-18`;
only `ScopeVisitor` calls `push`/`insert*`, `process/scope_visitor.rs:31-230`).
The deref-to-`Scope` blanket impl (`scope_visitor.rs:534-569`) is never
exercised, so the tracker is permanently empty and the guard is always `false`.

Consequence: `local require = function(name) ... end; return require("./x")` is
rewritten by `convert_require` — a miscompile. Every other tracker-using rule
uses `ScopeVisitor` correctly, and the bundler's `RequirePathProcessor` does too
(`rules/bundle/path_require_mode/mod.rs:314-315`), which makes the omission easy
to miss.

---

## E. Quadratic re-traversal overhead

**CONFIRMED.** `RemoveUnusedVariableProcessor::process_scope`
(`rules/remove_unused_variable.rs:24-214`):

- collects all local declarations per scope (`27-47`);
- for every declared identifier scans every following statement and the block
  last statement with a fresh `FindUsage` subtree walk (`65-96`, `73-89`;
  `process/processors/find_usage.rs:9-58`), plus the `repeat` condition
  (`49-63`);
- runs the whole thing to a fixed point (`227-235`).

Cost per scope is `O(k · m · avg_subtree)`, repeated per mutation pass —
superlinear, degenerating toward `O(k²·m)`. Shadowing is re-derived by an
ephemeral name-based tracker on every scan.

**r4 correction.** `r4` has no `symbol.references` field. `Symbol` exposes
`read_count`/`write_count` (`src/semantic/symbol.rs:53-56`) and
`SemanticModel::uses_of(symbol) -> &[SymbolUse]` (`src/semantic/model.rs:210`)
with `AccessKind::{Read,Write,Declaration}` and per-use scope/token
(`SymbolUse`, `symbol.rs:90-99`). Unused = `read_count == 0`; deleting an
unread symbol cannot change other symbols' counts, so no fixed-point loop is
needed.

---

## New findings (beyond A–E)

1. **`remove_call_match` reserved names have no collision analysis.**
   `__DARKLUA_REMOVE_CALL_RESERVED_{n}` (`rules/remove_call_match.rs:88-91`,
   `135-147`) is counter-based, and the `local <reserved> = select` statement is
   prepended (`71-86`) without checking existing identifiers.
2. **Bundler hard-codes runtime identifiers.** Default
   `__DARKLUA_BUNDLE_MODULES` (`rules/bundle/mod.rs:143`) is assigned at chunk
   top and referenced by every module function
   (`path_require_mode/module_definitions.rs:142-149`, `255-258`); wrappers
   hard-code `v`, `c`, `__modImpl` (`193-244`); module names come from a
   permutator avoiding only `"cache"` (`152-160`). The identifier is
   configurable but never auto-freshened against the input.
3. **The evaluator has no environment.** `evaluate` returns `LuaValue::Unknown`
   for identifiers/fields/indexes/calls (`process/evaluator/mod.rs:23-77`,
   esp. `71-74`): no reaching definitions, no constant propagation.
   `can_return_multiple_values` misclassifies all `Unary` and non-`and/or`
   `Binary` expressions as multi-valued (`106-111`). Calls are conservatively
   side-effecting (`256-259`); fields/indexes are only pure under the opt-in
   `assume_pure_metamethods` (`272-282`, `18-19`).
4. **Every rule re-implements tracking.** `Scope`/`ScopeVisitor`/
   `IdentifierTracker` are per-rule, ephemeral, and string-keyed. The
   deref-based `Scope` blanket impl makes the Claim D mistake compile silently.
5. **Require path logic is component-based and deterministic** (longest alias
   wins: `rules/require/luau_require_mode.rs:155-182`), so the suspected
   string-prefix alias collision does not exist. Remaining path risks are the
   broken `require` identity check (D) and byte/platform assumptions
   (`match_require.rs:26-31`, `path_utils.rs:55-91`).
6. **`group_local` is a guarded heuristic, not a proof.**
   `should_merge` (`rules/group_local.rs:57-73`) blocks multi-return expansion
   with `variables_len > values_len && values_len != 0` and pads with nils
   (`75-89`). No counterexample was found under that guard, but the correctness
   argument is undocumented and depends on Lua multi-return adjustment rules.
7. **`convert_luau_number` is token/source dependent**
   (`rules/convert_luau_number.rs:17-25`). The r4 port makes this explicit with
   `convert_luau_number` vs `convert_luau_number_with_code`.
8. **`CollectFunctionNames` is another flat, scope-blind avoid list**
   (`rules/rename_variables/function_names.rs:15-19`).

---

## r4 integration recommendations

1. **Make `SemanticModel` a required rule input.** Rules resolve identifiers by
   `SymbolId` (`resolve_identifier`, `resolve_in_scope`), never by name
   strings. This structurally prevents Claim D.
2. **Symbol-keyed renaming with scope-precise collisions.** Reject candidate
   name `G` for local `L` if any `SymbolUse` of a global `G` occurs in a scope
   that is a descendant-or-self of `L.declaring_scope` after `L`'s declaration
   (`uses_of` provides scopes and tokens). `local pcall = pcall` becomes
   renameable, and `pcall` remains usable where no global read can be captured.
3. **Alias/immutability metadata.** Record declaration initializers during
   analysis and expose `is_immutable_alias(symbol)`: `write_count == 1`, the
   sole use is the declaration, and the initializer resolves to a `Global`
   symbol. Then the std-lib rules can optimize through `local math = math`
   instead of skipping, and rebind safely when the alias is written.
4. **CFG-based dead code.** Add a constant-aware branch-folding pass over
   `Terminator::Branch` (evaluate with `Evaluator`, replace with `Jump` on
   constants, rebuild edges), then use `Reachability` to delete unreachable
   statements. This unifies `filter_early_return`, `unused_if_branch`, and
   `unused_while`. Requires AST provenance from blocks back to statements
   because block statements are currently clones.
5. **Reaching definitions / constant propagation.** Reuse the liveness bitset
   scaffolding over `SymbolId`s to compute per-block definition sets, then give
   `Evaluator` an environment (`evaluate_with`) so `local x = false; if x then`
   folds. This fixes the root cause behind B and C.
6. **One-pass unused-variable elimination.** `read_count == 0` plus
   side-effect preservation; no fixed-point loop or per-candidate scans.
7. **Resolution-based require handling.** `require` must resolve to a `Global`
   symbol (or immutable alias) before conversion; fixes D.
8. **Freshened synthetic names.** One `NameGenerator` backed by model symbols
   and keywords for `remove_call_match` reserved names, bundler identifiers, and
   future lowering passes.
9. **Adversarial tests as regression guards.** Shadowing programs
   (`local require = ...`, `local pcall = pcall`, `local math = {}`, closures
   capturing shadowed names) with re-analysis as the semantic oracle instead of
   string checks.

**Summary.** darklua's gaps are systemic: a per-rule string tracker with a
documented bypass bug (D), a flat avoid list bypassed by its own reuse pool
(A/A′), an environment-less evaluator (no reaching definitions), and
AST-pattern dead-code passes with no CFG (C). `r4` already provides
`SymbolId`-keyed def-use with per-use scopes and a verified CFG with
reachability/dominance/liveness. The three additions this audit originally
called out — **initializer/alias metadata on symbols**, **constant-aware
branch folding in the CFG**, and **AST provenance from blocks back to
statements** — are implemented (see "Implementation status in r4").

---

## Implementation status in r4

Tracked against the recommendations above (commits `b9c09ef`, this change, and
the follow-ups):

- [x] **3. Alias/immutability metadata** — `Symbol::initializer`,
  `SemanticModel::{is_unused, is_immutable, immutable_global_alias}`
  (`b9c09ef`).
- [x] **4. Constant-aware branch folding** —
  `ControlFlowGraph::set_terminator`,
  `cfg::analysis::{fold_constant_branches, fold_constant_branches_with}`
  (`b9c09ef`, generalized here).
- [x] **5. Reaching-definition constant propagation** —
  `cfg::analysis::ConstantPropagation`,
  `fold_constant_branches_with_propagation`, `evaluate_with_environment`
  (this change). Captured locals, parameters and iteration variables are
  conservatively excluded.
- [x] **6. One-pass unused-variable elimination** —
  `rules::canonicalize::remove_unused_variables` using O(1) symbol counts
  (`b9c09ef`).
- [x] **4b. AST provenance from CFG blocks to statements** —
  semantic analysis stamps every block with a unique `BlockIdentity`, the CFG
  builder records the structural origin (`StatementOrigin`) of every lowered
  statement and the `ConditionOrigin` of every source branch, and
  `rules::canonicalize::remove_unreachable_code` combines constant propagation,
  branch folding and reachability to delete unreachable statements, fold
  constant `if` statements and drop `while false` loops (originally via token
  keys, migrated to structural origins).
- [x] **2. Symbol-keyed rename pass** —
  `rules::rename_variables::rename_variables` renames by `SymbolId`, keeps
  globals untouched, rewrites shadowed locals/parameters/iteration
  variables/captured upvalues consistently, and generates globally unique
  names that avoid host globals, original names and keywords. Fails before
  mutation when any identifier is unbound in the model (originated with token
  validation, migrated to structural binding validation).
- [x] **7. Resolution-based `require` handling** —
  `rules::require::convert_require` only rewrites `require` calls whose prefix
  resolves to the global `require` symbol or an immutable alias
  (`identifier_resolves_to_global`), so a shadowing local or parameter is
  never rewritten. `rules::require::bundle_modules` inlines statically
  resolvable modules with cycle detection, and every synthesized identifier is
  allocated by `NameGenerator` against the root model and every bundled module.
- [x] **8. Freshened synthetic names** —
  `remove_continue` generates its loop flags and `remove_compound_assign` its
  temporary locals through the model-first `NameGenerator` (the name-scanning
  `IdentifierTracker` was removed), `remove_call_match` allocates
  reserved-global aliases with `NameGenerator`, the standard-library rewrites
  (`remove_floor_division`, `remove_interpolated_string`) allocate their
  aliases through `NameGenerator` instead of fixed `__R4_*` names, and the
  bundler allocates its loader and module-definition names the same way.
- [x] **9. Adversarial semantic-oracle tests** —
  `tests/rules_semantics_test.rs` re-parses and re-analyzes transformed output
  and checks def-use counts, shadowed-global identity, capture relationships,
  generated-name avoidance of host globals, composed dead-code removal, and
  strict-mode binding of renamed programs. `tests/rules_model_test.rs` checks
  the model-mandatory standard-library rewrites, and `tests/rules_test.rs`
  covers require shadowing/conversion/bundling and the LICM preconditions.

### Follow-up upgrades (semantic-first pass migration)

- [x] **Mandatory model across transformation passes** —
  every transformation pass takes a required `&SemanticModel`. The
  standard-library rewrites (`convert_square_root_call`, `group_local`,
  `remove_floor_division`, `remove_interpolated_string`) resolve through
  `SemanticModel::name_is_transparent_global`; their legacy name-based entry
  points and the `ShadowPolicy` name scan are removed, so read-only aliases
  such as `local math = math` are optimized through while mutable or non-alias
  shadowing stays conservative. `tests/rules_model_test.rs` verifies the
  behavior and the collision-free aliases.
- [x] **First-class type-name resolution** — `SemanticModel` owns a
  `TypeId`/`TypeSymbol` arena, `Scope` records declared types, `BindingId` has
  a distinct type domain, and the analyzer binds type declarations, resolves
  type-name occurrences lexically (same-scope redeclarations included) and
  reports `SemanticErrorKind::UnresolvedType` under
  `SemanticAnalyzer::strict_types()`.
- [x] **Sound LICM** — `cfg::analysis::LoopInvariantAnalysis` consumes
  `DominatorTree` natural loops and proves strict purity, loop invariance
  (including `ForLoop` iteration-variable definitions in liveness) and
  non-captured assignment before `rules::canonicalize::hoist_loop_invariants`
  moves the statement behind a fresh `NameGenerator` name.
- [x] **Transformation session + front-end convenience** —
  `rules::Session` caches the model, re-analyzes lazily after model-less passes
  and always hands semantic passes a fresh model; `Parser::parse_analyzed`
  returns a parsed block together with its model.
- [x] **Liveness consumer** — `canonicalize::remove_dead_stores` uses the
  per-statement access data added to `Liveness`, removing stores that are never
  live while keeping captured and side-effecting stores.
- [x] **Label consumer** — `canonicalize::remove_unused_labels` uses the
  model's goto bindings to delete labels that no `goto` targets.
- [x] **Symbol-level local grouping** — `canonicalize::group_local`
  merges consecutive local assignments when the second one reads no symbol
  declared by the first, ignoring unrelated same-name locals.
- [x] **Evaluator arity precision** — `can_return_multiple_values` no longer
  treats unary and binary operators as multi-valued.

### Remaining darklua rules ported

The differential suite exposed the darklua rules that had no r4 counterpart
yet. They are now ported with strict semantics:

- **Assignment localization** — `desugar::make_assignment_local` converts
  Luau `const` declarations to `local` (the darklua rule only changes the
  assigned kind; it does not relocalize `x = 1`).
- **Expression folding** — `canonicalize::compute_expression` folds strictly
  pure constants and short-circuits `and`/`or`. Non-finite results (`2 ^
  10000` to infinity), oversized strings and any impure operand are left
  untouched; darklua rewrites `2 ^ 10000` into a synthetic `1 / 0`.
- **Function syntax lowerings** — `desugar::remove_method_definition`
  (`function a:b()` to `function a.b(self)`),
  `desugar::convert_function_to_assignment`, and
  `desugar::convert_local_function_to_assignment`. The local-function
  conversion resolves references through the semantic model, so an inner local
  shadowing the function name is not mistaken for recursion (darklua's name
  scan refuses to convert).
- **Attribute removal** — `desugar::remove_attribute` drops every `@attribute`
  from function statements, local functions and function expressions.
- **Debug profiling removal** — `canonicalize::remove_debug_profiling` removes
  `debug.profilebegin`/`debug.profileend` calls through the model-first
  `remove_call_matches` framework, preserving argument side effects and never
  matching a shadowed `debug`.

Every one of these is verified in `tests/differential_test.rs` against the
published `darklua` crate, with byte-identical parity on clean inputs and
explicit better-output assertions for the folding and shadowing hazards. The
token/formatting rules (`remove_comments`, `remove_spaces`, `append_text_comment`,
`replace_referenced_tokens`, `shift_token_line`) are implemented in
`rules::formatting` rather than as configuration-driven rules, and
`ConvertRequire`/`Bundle` are replaced by the model-first
`convert_require`/`bundle_modules` APIs. The dead-code rules
`FilterAfterEarlyReturn`/`RemoveUnusedIfBranch`/`RemoveUnusedWhile` are
superseded by `remove_unreachable_code`.

### Production hardening audit

An audit of panic sites, memory, allocation and dead code across `r4` found and
fixed the following:

- **CFG builder crash on valid Lua 5.2.** `::l:: goto l return` unwound in
  `CfgBuilder::terminate` because the unreachable `return` was lowered after
  the `goto` had already closed the block. `lower_last_statement` now installs
  a terminator only while a block is active; the `break`/`continue` loop
  checks still run so invalid jumps are still rejected.
- **Surrogate unicode escapes panicked during string parsing.**
  `\u{D800}`/`\u{DFFF}` are not valid `char`s and previously hit
  `char::from_u32(...).expect(...)`. They are now reported as a typed
  `StringError` ("invalid unicode scalar value"). `ConvertError::String` also
  carries the underlying reason instead of discarding it.
- **Third-party parser panics are contained.** `full_moon` panics on a few
  malformed inputs (for example a surrogate escape inside an interpolated
  string). `Parser::parse` wraps only the third-party parse in
  `catch_unwind` and returns a typed `ParserError` carrying the panic message;
  `r4`'s own conversion and analysis paths remain panic-free by construction.
- **Dead code removed.** The unused `scoped_hash_map` module, the
  `#![allow(dead_code)]` masks, the migrated-away `lines.rs` token helpers,
  `generate_identifier`, and `KEYWORDS` were deleted. `cargo clippy
  --all-targets --all-features -- -D warnings` is clean with no blanket dead
  code allowances.
- **Allocation reductions in hot paths.** The liveness and
  constant-propagation fixpoints reuse scratch bitsets instead of allocating
  two bitsets per block on every iteration, and the analyzer no longer clones
  each identifier name before scope resolution (`read_identifier`,
  `write_identifier`, compound writes).
- **Adversarial coverage.** `tests/no_panic_test.rs` runs every transformation
  pass over a corpus of malformed, shadowing, goto, type, attribute and
  constant-folding snippets, then re-analyzes the transformed trees and
  verifies symbol/scope arena integrity, block identity uniqueness and CFG
  edge/origin invariants.

`GotoSite::visible_symbols` and `GotoLabel::visible_symbols` are recorded as
`VisibleScopes` cursor snapshots (the number of visible symbols per active
scope), so recording and checking goto visibility is `O(active scopes)` instead
of `O(locals)` per goto and no longer quadratic on goto-heavy files.

### Compiler-as-a-service API expansion

The programmatic API now covers the remaining functional darklua capabilities
without a configuration file or filesystem coupling:

- **Semantic constant injection** — `rules::inject_global_values(block, model,
  values)` replaces reads of configured globals with literal values. Every
  occurrence resolves through the `SemanticModel`, so shadowed locals,
  parameters and captures are never touched, globals that are written are left
  alone, and values with no literal representation are rejected up front with
  `InjectGlobalValueError::UnrepresentableValue`.
- **Exported-type hoisting** — `rules::require::bundle_modules` hoists each
  module's `export type` declarations (plus the local types they depend on)
  into the root scope using fresh `NameGenerator` names that reserve every
  value symbol, type symbol and host global across the bundle. References are
  retargeted through the module models (a shadowing local type keeps its
  name), cross-module references are retargeted to the hoisted type, and two
  modules exporting the same type name is a typed
  `BundleError::DuplicateExportedType`.
- **Path aliases** — `rules::require::apply_path_aliases` and
  `AliasRequireConverter` map `@pkg/...` prefixes to normalized paths through
  the existing `RequirePathConverter` trait. Malformed, duplicate and
  recursive mappings are typed errors, and an aliased require with no mapping
  is `AliasError::UnresolvedAlias` rather than a silent skip.
- **Generator-level minification** — `Session::generate`,
  `Session::generate_minified` and `Session::generate_with` expose formatting
  as code generation (`DenseLuaGenerator`), so minification never mutates
  tokens or trivia.

### Multi-module architecture

Multi-file analysis is split into three layers with clean boundaries:

- **Semantic layer** (`semantic::MultiModuleModel`) owns the syntax trees and
  their single-file `SemanticModel`s, an explicit `ModuleInterface` per module
  (exported values and `TypeId`s) and `ImportBinding`s that bind a local alias
  symbol directly to a target module. Cross-module queries
  (`resolve_imported_value`, `resolve_imported_type`, `modules_exporting_type`)
  resolve by identity and never consult a name dictionary. Export extraction
  supports inline returned tables, tables built by mutation (`t.k = v`,
  `function t.f()`), single-value returns, and `export type` declarations.
  This layer is free of paths, I/O and code generation.
- **Resolution layer** (`modules::ModuleResolver`) owns everything
  path-related: a `ModuleProvider`, relative-path normalization, `require`
  classification, cycle detection and topological ordering. It parses each
  discovered file, delegates analysis to `SemanticAnalyzer`, and builds the
  `MultiModuleModel`, including binding alias imports to the modules the
  resolver resolved.
- **Bundler** (`rules::require::bundle_modules`) is a pure consumer. It inlines
  modules in topological order, hoists and renames exported types through the
  graph's identities, rewrites `require` calls with the loader, and validates
  cross-module type references.

Strict cross-module rules: a type reference resolves only through an imported
module's interface; a reference to a type exported by an unimported module is
`BundleError::UnimportedType`, and a name exported by several imported modules
is `ModuleError::AmbiguousImportedType`. A value field access through an
import must be a declared export (`ModuleError::UnknownExport`), and a symbol
that is not an import alias is `UnknownImportAlias`. Lexical shadowing wins: a
module-local type with the same name as an imported export is never retargeted.
New names are minted by `NameGenerator`, which reserves every value symbol,
type symbol and host global across every bundled module.

Scope arenas remain per module (the multi-tree container links them through
import bindings rather than merging them into one flat arena). This keeps
standalone `SemanticAnalyzer`/`SemanticModel` analysis lightweight and
unchanged while still providing identity-based cross-module resolution.

### Token/formatting parity and public API surface

The remaining darklua token-level rules are now public passes in
`rules::formatting`, kept strictly separate from semantic passes (they never
take a `SemanticModel` and never restructure the tree):

- `remove_comments(block)` and `remove_comments_where(block, keep)` clear all
  comments, or keep the trivia for which the predicate returns `true`.
- `remove_spaces(block)` clears whitespace trivia.
- `shift_token_line(block, amount)` shifts recorded line numbers.
- `replace_referenced_tokens(block, code)` materializes tokens that reference the
  original source into owned content.
- `append_text_comment(block, text, location)` prepends or appends a text
  comment, choosing long-bracket delimiters when the text spans lines and
  shifting line numbers to keep token-based generation aligned.

These complement, rather than replace, the non-destructive
`DenseLuaGenerator` minification pipeline.

The public API surface was completed with:

- `diagnostic` (`Diagnostic`, `Severity`, `TextSpan`, `LinePosition`,
  `LinePositionSpan`), plus `resolve_location` to derive line/column from a byte
  span and source.
- `SemanticError` now carries an optional `TextSpan` and exposes `code()` and
  `to_diagnostic()`; `ParserError`, `ResolveError` and `ModuleError` expose the
  same conversion. `Token::get_span()` provides byte ranges.
- `semantic::Binding` and `SemanticModel::{binding_of, symbol_of, label_of,
  type_of}` give a uniform query over value, label and type occurrences.
- `Compilation`/`TreeId`/`SyntaxTree`: an orchestrator that owns trees, records
  diagnostics and delegates parsing to `Parser`, analysis to `SemanticAnalyzer`
  and module resolution to `ModuleResolver`.

### Composite value tracking

Semantic tracking now goes beyond single scalars:

- `SemanticModel::immutable_alias_root` follows immutable declaration
  initializers to the symbol a chain ultimately refers to (generalizing the
  previous one-step `immutable_global_alias`). `local math = math`, `local m =
  math` and deeper chains all resolve to the original global, while a
  reassignment breaks the chain.
- `semantic::EscapeAnalysis` conservatively classifies every local: whether it
  is initialized from a table constructor, whether it escapes (passed,
  returned, stored, compared, called, method-called, dynamically indexed or
  captured), and which static fields are read or written. It never claims a
  table is confined unless every occurrence is a static field access, so it can
  only miss optimizations, never remove a live store.
- `rules::canonicalize::remove_dead_table_stores` (opt-in, not part of the
  default `canonicalize` pipeline) removes `table.field = value` assignments
  when the table is a confined local table, the field is never read, there is
  no dynamic read, and the value has no side effects. Removing the store is
  sound because a confined table can never receive a metatable, so no
  `__newindex` can observe the write.

Static linking of exported module values is implemented in
`rules::require` (`link::link_exported_values`, run by `bundle_modules`). When
a module returns an inline table whose fields are backed by immutable locals,
and every local alias of the required module is immutable, uncaptured, never
field-written, never dynamically written and never escapes (including through a
non-alias `require(...)` use), the bundler mints a root-level local per linked
export, captures the value right before the exporting module returns, and
rewrites `alias.field`/`alias["field"]` reads to that local. The runtime loader
and the returned tables are left in place, so method calls, dynamic indexing and
loader caching keep working exactly as before; a rewrite that cannot be proven
equivalent is simply skipped.

### Unified hierarchy, incremental analysis and frontend parity

- **Unified scope hierarchy.** `MultiModuleModel` unifies the implicit globals
  of every module into canonical `GlobalId`s (`GlobalSymbol`, `global_of`,
  `same_global`) and exposes a single scope hierarchy (`ScopeRef`, `SymbolRef`,
  `root_scope`, `file_scope`, `parent_scope`, `resolve_in_scope`,
  `resolve_identifier`). A module's global root scope is a child of the one
  shared global scope, so the same global has one identity everywhere.
- **Type environment.** Type declarations are hoisted per block and type-name
  resolution is position aware: an occurrence binds to the nearest preceding
  declaration, or forward-references the next declaration in the same scope, so
  forward and mutually recursive Luau type aliases resolve under strict typing
  while same-scope redeclarations keep positional bindings. Bare aliases record
  their target (`TypeSymbol::alias_of`) and expand without re-analysis
  (`expand_type_alias`, `resolve_type_expanded`).
- **Incremental analysis.** `modules::AnalysisCache` stores parsed and analyzed
  sources keyed by a re-validated content hash; `resolve_with_cache` reuses
  unchanged dependency analyses, and `Compilation` caches `add_tree`/
  `update_tree`, exposes copy-on-write `block_mut` plus `refresh_tree` for
  in-place edits, and resolves module graphs through the shared cache.
- **Data conversion.** `convert::convert_data`/`to_expression` turn any
  `serde::Serialize` value into a Lua expression/module (the library counterpart
  of darklua's `convert` command), covering sequences, maps, structs, tuples and
  externally tagged enums.
- **Filesystem and Roblox requires.** `modules::FileSystemModuleProvider`
  resolves `<root>/<key>.lua|.luau` and folder modules (`<root>/<key>/init.lua`),
  rejecting keys that escape the root, and `normalize_require_path` supports the
  Luau RFC `@self` prefix next to `./`/`../`. `classify_require_call` recognizes
  Roblox/Luau instance requires (`require(script.Parent.Util)`,
  `:FindFirstChild`/`:WaitForChild`) and `resolve_instance_path` maps them to
  module keys using the Rojo file layout; the resolver, alias binding, bundler
  and static linker all handle them end to end. A `game:GetService` root cannot
  be resolved without a project mapping, so it fails fast as a non-literal
  require rather than being guessed.
- **Goto visibility.** `VisibleScopes` records the visible symbol count per
  active scope, replacing the per-goto symbol snapshots.

### Scope notes (outside the audited items)

All open items from this audit are implemented. The notes below record
deliberate scope boundaries of the new features, not outstanding audit work.

- **Type declarations** — declarations, exported types, generic type parameters
  and lexical type shadowing are bound and resolved, and strict typing rejects
  undeclared type names. Generic type-pack defaults and function-type generic
  parameters are walked conservatively; this is name resolution, not a type
  checker.
- **LICM depth** — `hoist_loop_invariants` moves strictly pure, loop-invariant
  single-value local declarations whose block dominates every latch. It
  intentionally does not perform preheader synthesis for load-bearing
  expressions, strength reduction, or speculative hoisting of
  potentially-raising arithmetic.
- **Uniform model requirement** — every transformation pass takes
  `&SemanticModel`. The purely syntactic lowerings (`remove_types`,
  `remove_if_expression`, `remove_method_call`, `convert_luau_number`,
  `remove_nil_declarations`, `remove_empty_do`, `remove_call_parens`,
  `convert_index_to_field`) do not resolve identifiers, but the API requires an
  analyzed AST so a pass can never run on an unanalyzed or stale block. The
  passes that synthesize identifiers (`remove_compound_assign`,
  `remove_continue`) and all standard-library rewrites resolve and allocate
  through the model.

### Structural semantic layer (token-free passes)

Semantic information now lives on the AST rather than in source coordinates:

- **Identifier bindings** — `Identifier` carries a thread-safe binding cell
  written by `SemanticAnalyzer`. `SemanticModel::resolve_identifier` and
  `resolve_label` read and validate that binding, so resolution is exact for
  same-scope shadowing (`local x = 1; print(x); local x = 2; print(x)` binds
  each occurrence to its own declaration) and works with
  `Parser::preserve_tokens(false)`. `resolve_in_scope_at` adds positional
  name lookup for callers that only track declaration counts.
- **Block identities and CFG origins** — every analyzed block gets a unique
  `BlockIdentity`; the CFG builder records `StatementOrigin`/`ConditionOrigin`
  for lowered statements and branch conditions, with a
  `CfgErrorKind::MissingBlockIdentity` error when the model does not match the
  tree. `validate_block_identities` gates the provenance consumers.
- **Token data is passive metadata** — the model stores no token→symbol
  lookup tables. `SymbolUse.token`, `GotoSite.token`, `GotoLabel.token` and
  the AST token fields remain for diagnostics, line reporting and formatting,
  and are populated exactly when the parser preserves tokens.
- **Consumers migrated** — liveness, constant propagation, unused-variable and
  unused-label removal, dead-store removal, unreachable-code removal and
  variable renaming all resolve structurally. `rules::Session` now parses
  without token preservation, and `tests/tokenless_test.rs` verifies
  byte-identical pipelines with and without tokens.

### Strict semantics upgrade

- [x] **First-class evaluator environment** — `Evaluator::evaluate_with`
  evaluates against a strict `Fn(&Identifier) -> Option<LuaValue>` lookup:
  missing entries, calls, fields, indexes and `...` propagate as `None`
  (no `Unknown` fallback). All recursive arms (`evaluate_binary_with`,
  `evaluate_unary_with`, `evaluate_if_with`, `evaluate_prefix_with`,
  interpolated strings, type instantiations) pass the lookup through, and the
  CFG constant folder/ propagation now call it directly.
- [x] **`resolve_in_scope` / `captured_symbols` coverage** — exact tests for
  innermost-shadow resolution, global fallback with strict `None` for
  never-analyzed names, direct/transitive upvalue marking, and the invariant
  that unreferenced outer variables never appear in `captured_symbols`.
- [x] **Dominator-tree primitives** — `BackEdge`, exact natural-loop detection
  (back edges plus dominated reach-back sets), `is_loop_header`,
  `natural_loop_blocks` (panics on non-back-edges; `try_` for fallible use),
  `dominator_subtree`, `is_provably_unreachable`, and
  `prune_after_removed_edge` (exact: only seeds when the removed target has no
  other predecessor, then closes over the dominator subtree).
- [x] **Model-first `NameGenerator`** — reserves every model symbol, the
  default/Roblox host globals and all previously generated names, so generated
  identifiers cannot collide in any visible scope.
- [x] **`remove_call_match` + `remove_assertions`** — the framework resolves
  calls through the model (`identifier_resolves_to_global` recognizes read-only
  aliases), preserves argument side effects, and allocates reserved-global
  aliases with `NameGenerator` instead of counter-based names.
