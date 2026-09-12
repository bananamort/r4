# Architectural Audit — `darklua-0.19.0` vs `r4`

Date: 2026-09-12
Scope: `references/darklua-0.19.0/src/` versus `r4`'s read-only analysis layers
(`src/semantic/`, `src/cfg/`).

**Method.** Every cited file was read end-to-end. The type/visitor plumbing
(`Scope`, `ScopeVisitor`, `IdentifierTracker`, `DefaultVisitor`, `Permutator`)
was traced, and the exact APIs present in `r4::semantic` and `r4::cfg` were
checked. Claims that could not be reproduced from the code are flagged.

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
(`src/semantic/symbol.rs:24-30`) and binds every occurrence to a `SymbolId`
(`src/semantic/model.rs:106-130`; `uses_of` at `154`). Scope resolution walks the
lexical chain (`resolve_in_scope`, `model.rs:121-140`), so shadowed `pcall`
locals are distinct symbols and global reads are separate symbols with their
own use lists and scopes.

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
`read_count`/`write_count` (`src/semantic/symbol.rs:47-49`) and
`SemanticModel::uses_of(symbol) -> &[SymbolUse]` (`src/semantic/model.rs:154`)
with `AccessKind::{Read,Write,Declaration}` and per-use scope/token
(`symbol.rs:68-85`). Unused = `read_count == 0`; deleting an unread symbol
cannot change other symbols' counts, so no fixed-point loop is needed.

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
   `SymbolId` (`resolve_identifier`, `symbol_at_token`, `resolve_in_scope`),
   never by name strings. This structurally prevents Claim D.
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
reachability/dominance/liveness. To eliminate the audited failure modes it
still needs three additions: **initializer/alias metadata on symbols**,
**constant-aware branch folding in the CFG**, and **AST provenance from blocks
back to statements**.

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
  `cfg::{statement_token_key, expression_token_key}` map block contents and
  branch conditions back to AST nodes through source tokens, and
  `rules::canonicalize::remove_unreachable_code` combines constant propagation,
  branch folding and reachability to delete unreachable statements, fold
  constant `if` statements and drop `while false` loops (this change).
- [x] **2. Symbol-keyed rename pass** —
  `rules::rename_variables::rename_variables` renames by `SymbolId`, keeps
  globals untouched, rewrites shadowed locals/parameters/iteration
  variables/captured upvalues consistently, and generates globally unique
  names that avoid host globals, original names and keywords. Fails before
  mutation when any occurrence lacks token data (this change).
- [ ] **7. Resolution-based `require` handling.** (No `convert_require` port
  exists in `r4` yet; implement with symbol resolution when it is added.)
- [x] **8. Freshened synthetic names** —
  `remove_continue` now generates its loop flags through the shared
  `IdentifierTracker` instead of a fixed `__R4_CONTINUE_{n}` counter (this
  change). The bundler/`remove_call_match` do not exist in `r4`.
- [ ] **9. Adversarial semantic-oracle tests** for the rule pipeline
  (partially covered by the rename/remove-invalid/remove-unreachable tests,
  which re-analyze their generated output).
