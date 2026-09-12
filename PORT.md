# Porting & Architectural Audit: `darklua-0.19.0` -> `r4`

This document details every single source file in `references/darklua-0.19.0/src`, analyzing its exact function, whether we should **KEEP**, **ADAPT**, or **DISCARD** it, and the technical rationale for building a semantically-aware, control-flow tracking Lua analysis and transformation engine (similar to Roslyn / Loretta).

---

## Executive Summary & Strategy

* **What `darklua` does well**: It provides a rock-solid, production-tested **Frontend** (Lua 5.1/Luau AST definitions, parsing via `full_moon`, and token trivia preservation) and **Backend** (formatted code generators).
* **Where `darklua` falls short**: It is an AST-level peephole rewriter. It lacks a true **Semantic Model** (unique symbol IDs, def-use chains, upvalue capture analysis) and has **no Control Flow Graph (CFG)** or Data Flow Analysis (DFA).
* **Our Plan**:
  1. **KEEP & ADAPT** the AST definitions (`nodes/`), the parser (`parser.rs`, `ast_converter.rs`), the code generators (`generator/`), and the constant evaluator (`process/evaluator/`).
  2. **DISCARD** the ad-hoc transformation rules (`rules/`), CLI/frontend daemons (`cli/`, `frontend/`), and naive scope tracking (`scope_visitor.rs`).
  3. **BUILD** a new compiler middle-end: `semantic/` (Symbol table, scope hierarchy) and `cfg/` (Basic blocks, dominators, liveness, reaching definitions).

---

## 1. Root Files

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `src/ast_converter.rs` | **KEEP** | **Function**: Converts `full_moon`'s concrete syntax tree into darklua's clean `nodes::*` AST.<br>**Rationale**: Essential parser glue. Rewriting this from scratch would take weeks of edge-case handling. |
| `src/parser.rs` | **KEEP** | **Function**: Parser entry point. Invokes `full_moon::parse_fallible` and delegates to `AstConverter`. Has flags for preserving tokens.<br>**Rationale**: Clean API to turn raw Lua string into an AST `Block`. |
| `src/lib.rs` | **ADAPT** | **Function**: Crate root exporting modules.<br>**Rationale**: Needs to be adapted for `r4` to expose our new `semantic` and `cfg` modules instead of darklua's `rules` and `cli`. |
| `src/bin.rs` | **DISCARD** | **Function**: Entry point for darklua's CLI executable.<br>**Rationale**: Darklua CLI arguments and flags are specific to darklua. We will create our own `main.rs` for `r4`. |

---

## 2. AST Nodes (`src/nodes/`)

The AST definitions are clean, well-tested, and cover both Lua 5.1 and Luau syntax.

### Base Node Files (`src/nodes/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `nodes/mod.rs` | **KEEP** | **Function**: Re-exports all AST node modules and definitions.<br>**Rationale**: Necessary module umbrella. |
| `nodes/block.rs` | **KEEP** | **Function**: Defines `Block`, containing statements and an optional `LastStatement` (return/break).<br>**Rationale**: Core root unit of code and function bodies. |
| `nodes/identifier.rs` | **KEEP** | **Function**: Represents variable/identifier names (`Identifier`).<br>**Rationale**: Crucial AST leaf node. |
| `nodes/token.rs` | **KEEP** | **Function**: Implements `Token`, `Trivia`, `Position`, and `TriviaKind` (whitespace/comments).<br>**Rationale**: Enables trivia preservation and exact source code span referencing. |
| `nodes/arguments.rs` | **KEEP** | **Function**: Represents arguments passed to function calls (Tuple `(a, b)`, Table `{...}`, or String `'foo'`).<br>**Rationale**: Standard Lua call syntax representation. |
| `nodes/attributes.rs` | **KEEP** | **Function**: Luau attributes (`@native`, `@checked`).<br>**Rationale**: Needed for modern Luau AST fidelity. |
| `nodes/function_body.rs`| **KEEP** | **Function**: Parameter list, return types, and body `Block` for functions.<br>**Rationale**: Central to closure and function analysis. |
| `nodes/function_call.rs`| **KEEP** | **Function**: Standalone function call and method call (`obj:method()`) structures.<br>**Rationale**: Essential expression and statement. |
| `nodes/literal_expression.rs` | **KEEP** | **Function**: Wrapper enum for literal values (true, false, nil, numbers, strings).<br>**Rationale**: Used for clean AST pattern matching. |
| `nodes/typed_identifier.rs` | **KEEP** | **Function**: An identifier with optional Luau type annotation (`x: number`).<br>**Rationale**: Used in parameters and typed local assignments. |
| `nodes/variable.rs` | **KEEP** | **Function**: Enum for variable reference targets (`Identifier`, `FieldExpression`, `IndexExpression`).<br>**Rationale**: Represents any LHS lvalue in assignments. |

### Expressions (`src/nodes/expressions/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `expressions/mod.rs` | **KEEP** | **Function**: Defines the master `Expression` enum and re-exports all expressions.<br>**Rationale**: Core AST node. |
| `expressions/binary.rs` | **KEEP** | **Function**: Binary operations (`+`, `-`, `*`, `and`, `or`, `..`, etc.) with precedence rules.<br>**Rationale**: Fundamental expression type. |
| `expressions/unary.rs` | **KEEP** | **Function**: Unary operations (`not`, `-`, `#`).<br>**Rationale**: Fundamental expression type. |
| `expressions/field.rs` | **KEEP** | **Function**: Table dot access (`table.field`).<br>**Rationale**: Fundamental expression type. |
| `expressions/index.rs` | **KEEP** | **Function**: Table bracket index access (`table[index]`).<br>**Rationale**: Fundamental expression type. |
| `expressions/function.rs` | **KEEP** | **Function**: Anonymous function expressions (`function(x) ... end`).<br>**Rationale**: Critical for closure and CFG analysis. |
| `expressions/if_expression.rs` | **KEEP** | **Function**: Luau ternary `if cond then val1 else val2`.<br>**Rationale**: Modern Luau syntax support. |
| `expressions/interpolated_string.rs` | **KEEP** | **Function**: Luau backtick string interpolation (`` `hello {name}` ``).<br>**Rationale**: Modern Luau syntax support. |
| `expressions/number.rs` | **KEEP** | **Function**: Float/integer number literals, parsing decimal/hex/binary numbers.<br>**Rationale**: Literal parsing and evaluation. |
| `expressions/string.rs` | **KEEP** | **Function**: String literals with single, double, or multi-line block quotes.<br>**Rationale**: Literal representation. |
| `expressions/string_utils.rs` | **KEEP** | **Function**: Escape sequence parsing and quote conversions.<br>**Rationale**: Helper for string literals. |
| `expressions/table.rs` | **KEEP** | **Function**: Table constructor `{ key = val, [expr] = val, val }`.<br>**Rationale**: Core Lua data structure representation. |
| `expressions/parenthese.rs` | **KEEP** | **Function**: Parenthesized expressions `(expr)`.<br>**Rationale**: Preserves explicit evaluation grouping. |
| `expressions/prefix.rs` | **KEEP** | **Function**: Prefix expressions (calls, variables, parenthesized sub-expressions).<br>**Rationale**: Lua grammar structural representation. |
| `expressions/type_cast.rs` | **KEEP** | **Function**: Luau type casting expression (`expr :: type`).<br>**Rationale**: Luau type system support. |
| `expressions/type_instantiation.rs` | **KEEP** | **Function**: Luau generic type instantiation expression (`fn<T>()`).<br>**Rationale**: Luau type system support. |

### Statements (`src/nodes/statements/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `statements/mod.rs` | **KEEP** | **Function**: Master `Statement` enum re-exporting all statements.<br>**Rationale**: Core AST node. |
| `statements/assign.rs` | **KEEP** | **Function**: Multi-assignment statement (`a, b = 1, 2`).<br>**Rationale**: Key statement for data-flow & def-use tracking. |
| `statements/local_assign.rs` | **KEEP** | **Function**: Local variable declaration (`local x, y = 1, 2`).<br>**Rationale**: Core variable declaration point for symbol tables. |
| `statements/compound_assign.rs` | **KEEP** | **Function**: Luau compound assignments (`+=`, `-=`, `*=`, etc.).<br>**Rationale**: Crucial for def-use and CFG representation. |
| `statements/do_statement.rs` | **KEEP** | **Function**: Explicit lexical scoping block (`do ... end`).<br>**Rationale**: Creates isolated lexical scopes and basic block branches. |
| `statements/function.rs` | **KEEP** | **Function**: Global/table function declaration (`function t.f() ... end`).<br>**Rationale**: Function definition representation. |
| `statements/local_function.rs` | **KEEP** | **Function**: Local recursive function declaration (`local function f() ... end`).<br>**Rationale**: Local symbol declaration with recursive self-binding. |
| `statements/if_statement.rs` | **KEEP** | **Function**: Conditional branch (`if ... elseif ... else ... end`).<br>**Rationale**: Primary branching construct for CFG building. |
| `statements/while_statement.rs` | **KEEP** | **Function**: Loop construct (`while cond do ... end`).<br>**Rationale**: Creates loop back-edges in CFG. |
| `statements/repeat_statement.rs` | **KEEP** | **Function**: Post-condition loop (`repeat ... until cond`).<br>**Rationale**: Creates loop back-edges in CFG. |
| `statements/numeric_for.rs` | **KEEP** | **Function**: Numeric loop (`for i = start, stop, step do ... end`).<br>**Rationale**: Loop construct with implicit local variable. |
| `statements/generic_for.rs` | **KEEP** | **Function**: Iterator loop (`for k, v in pairs(t) do ... end`).<br>**Rationale**: Loop construct with multi-variable declarations. |
| `statements/last_statement.rs` | **KEEP** | **Function**: Terminator statements (`return`, `break`, `continue`).<br>**Rationale**: Basic block terminators for CFG generation. |
| `statements/type_declaration.rs` | **KEEP** | **Function**: Luau type alias declaration (`type Point = { x: number }`).<br>**Rationale**: Luau type system support. |
| `statements/type_function.rs` | **KEEP** | **Function**: Luau type function declaration (`type function F()`).<br>**Rationale**: Luau type system support. |

### Type Definitions (`src/nodes/types/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `types/mod.rs` through `types/*.rs` (16 files) | **KEEP** | **Function**: Contains full Luau type representations: `array.rs`, `expression_type.rs`, `function.rs`, `function_variadic_type.rs`, `generics.rs`, `intersection.rs`, `optional.rs`, `parenthese.rs`, `string_type.rs`, `table.rs`, `type_field.rs`, `type_name.rs`, `type_pack.rs`, `union.rs`, `variadic_type_pack.rs`.<br>**Rationale**: Complete representation of Luau static types. Keeping these enables future type-checking without needing to write type ASTs later. |

---

## 3. Code Generators (`src/generator/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `generator/mod.rs` | **KEEP** | **Function**: Defines the `LuaGenerator` trait.<br>**Rationale**: Core interface for emitting ASTs back to Lua source. |
| `generator/readable.rs` | **KEEP** | **Function**: Pretty-printer formatting code with indentation and standard spacing.<br>**Rationale**: **High value**. Ideal output for deobfuscation, refactoring, and code generation. |
| `generator/dense.rs` | **KEEP** | **Function**: Minified code generator (strips all unnecessary spaces).<br>**Rationale**: Useful if minification output is needed. |
| `generator/token_based.rs` | **KEEP** | **Function**: Emits code respecting original token trivia (comments/newlines).<br>**Rationale**: Useful when lossless trivia preservation is required. |
| `generator/utils.rs` | **KEEP** | **Function**: String indentation helpers, newline counting, and token separators.<br>**Rationale**: Required helper for all generator implementations. |

---

## 4. AST Processors & Evaluator (`src/process/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `process/mod.rs` | **ADAPT** | **Function**: Re-exports process utilities.<br>**Rationale**: Adapt to export only visitors and evaluator, discarding darklua-specific processors. |
| `process/node_processor.rs` | **KEEP** | **Function**: Defines the `NodeProcessor` visitor trait with callbacks for every AST node.<br>**Rationale**: Core visitor interface to drive AST walking. |
| `process/visitors.rs` | **KEEP** | **Function**: Implements recursive pre-order AST traversal (`NodeVisitor`).<br>**Rationale**: Used to traverse the AST to construct Symbol Tables and CFGs. |
| `process/post_visitor.rs` | **KEEP** | **Function**: Implements post-order AST traversal (`NodePostVisitor`).<br>**Rationale**: Useful for bottom-up analysis (e.g., expression type deduction). |
| `process/evaluator/mod.rs` | **KEEP** | **Function**: Evaluator that computes constant expressions at compile time. Includes `has_side_effects()` expression purity analysis.<br>**Rationale**: **Critical for CFG dead-branch elimination and constant folding**. |
| `process/evaluator/lua_value.rs`| **KEEP** | **Function**: Enum `LuaValue` (Number, String, True, False, Nil) and truthiness rules.<br>**Rationale**: Foundation for constant evaluation. |
| `process/expression_serializer.rs` | **KEEP** | **Function**: Converts evaluated `LuaValue` back into AST `Expression` nodes.<br>**Rationale**: Used to rewrite folded constants back into the AST. |
| `process/node_counter.rs` | **KEEP** | **Function**: Counts nodes in an AST.<br>**Rationale**: Useful metric for complexity and optimizations. |
| `process/scope_visitor.rs` | **DISCARD** | **Function**: Naive string-based lexical scope stack (`push`/`pop`).<br>**Rationale**: **Flawed architecture**. String-based stacks cannot distinguish shadowed variables or track def-use chains. Replaced by `semantic/`. |
| `process/processors/find_usage.rs` | **DISCARD** | **Function**: Checks string equality to find variable usage.<br>**Rationale**: Inaccurate because it does not resolve symbols or scopes. Replaced by `SymbolId` def-use chains. |
| `process/processors/find_identifier.rs` | **DISCARD** | **Function**: Checks if an identifier name is present.<br>**Rationale**: Superceded by `SemanticModel`. |
| `process/processors/collect_globals.rs` | **DISCARD** | **Function**: Heuristic collection of global variable names.<br>**Rationale**: Superceded by `SemanticModel`. |
| `process/processors/mod.rs` | **DISCARD** | **Function**: Module root for the discarded processors.<br>**Rationale**: Not needed. |
| `process/utils/mod.rs` | **KEEP** | **Function**: Identifier validity checking (`is_valid_identifier`).<br>**Rationale**: General utility. |
| `process/utils/permutator.rs` | **DISCARD** | **Function**: Identifier generation for variable renaming/obfuscation.<br>**Rationale**: Darklua-specific renaming logic. |

---

## 5. General Utilities (`src/utils/`)

| File | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `utils/mod.rs` | **ADAPT** | **Function**: Re-exports utility functions.<br>**Rationale**: Strip out serde/wasm specific exports. |
| `utils/preserve_arguments_side_effects.rs` | **KEEP** | **Function**: Filters arguments that have side effects when simplifying function calls.<br>**Rationale**: Useful optimization and dead-code elimination tool. |
| `utils/expressions_as_statement.rs` | **KEEP** | **Function**: Converts expressions into statements while preserving evaluation order.<br>**Rationale**: Useful when lowering expressions. |
| `utils/lines.rs` | **KEEP** | **Function**: Line and column counting utilities from byte offsets.<br>**Rationale**: Useful for diagnostics and error reporting. |
| `utils/scoped_hash_map.rs` | **KEEP** | **Function**: Scoped key-value mapping structure.<br>**Rationale**: Useful helper for nested environment lookups. |
| `utils/timer.rs` | **KEEP** | **Function**: Benchmark timer for measuring passes.<br>**Rationale**: Useful for profiling 500K script throughput. |
| `utils/wasm_timer.rs` | **DISCARD** | **Function**: WASM-specific timer fallback.<br>**Rationale**: Unnecessary for native CLI and backend engine. |
| `utils/filter_pattern.rs` | **DISCARD** | **Function**: File glob pattern matching.<br>**Rationale**: Darklua CLI specific. |
| `utils/luau_config.rs` | **DISCARD** | **Function**: Parser for `.luaurc` configuration files.<br>**Rationale**: Unnecessary for core engine. |
| `utils/serde_*.rs` (4 files) | **DISCARD** | **Function**: Custom deserializers for darklua configuration files (`serde_one_or_many.rs`, `serde_string_or_struct.rs`, `serde_usize_from_float.rs`, `serde_vec_of_pairs.rs`).<br>**Rationale**: Unnecessary darklua config parsing code. |

---

## 6. Transformation Rules (`src/rules/` - 48 Files)

| Submodule / Files | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `rules/*` (All 48 files) | **DISCARD** | **Function**: Ad-hoc transformations implemented in darklua:<br>• `rename_variables/*`<br>• `bundle/*`<br>• `convert_require/*`<br>• `require/*`<br>• `remove_comments.rs`, `remove_spaces.rs`<br>• `compute_expression.rs`, `unused_if_branch.rs`, etc.<br>**Rationale**: These rules operate purely on naive AST visitors without semantic tracking or control flow graphs. We are building a formal semantic and CFG transformation pipeline, making these ad-hoc rules obsolete. |

---

## 7. CLI & Frontend Infrastructure (`src/cli/`, `src/frontend/` - 18 Files)

| Submodule / Files | Decision | Function & Rationale |
| :--- | :---: | :--- |
| `cli/*` (7 files) | **DISCARD** | **Function**: Darklua's command-line interface, argument parsing (Clap), and file watcher.<br>**Rationale**: We will build our own focused API and CLI interface. |
| `frontend/*` (11 files) | **DISCARD** | **Function**: Darklua configuration loader (`.darklua.json`), resource caches, and multi-threaded worker tree.<br>**Rationale**: Tied specifically to darklua's bundling workflow. |

---

## Summary Statistics

| Category | Total Files | Keep / Adapt | Discard |
| :--- | :---: | :---: | :---: |
| **Root** | 4 | 3 | 1 |
| **AST Nodes (`nodes/`)** | 58 | 58 | 0 |
| **Generators (`generator/`)** | 5 | 5 | 0 |
| **Process / Evaluator (`process/`)** | 15 | 8 | 7 |
| **Utilities (`utils/`)** | 13 | 6 | 7 |
| **Rules (`rules/`)** | 48 | 0 | 48 |
| **CLI & Frontend (`cli/`, `frontend/`)** | 18 | 0 | 18 |
| **TOTALS** | **161** | **80** | **81** |

---

## Target Project Architecture (`r4`)

The 80 retained files provide the AST, parser, and generator layers. On top of this foundation, we will build the missing semantic and control flow layers:

```
r4/
├── Cargo.toml
├── PORT.md
└── src/
    ├── lib.rs
    │
    ├── ast/             <-- From darklua: nodes/ (58 files)
    ├── parser/          <-- From darklua: parser.rs, ast_converter.rs (2 files)
    ├── generator/       <-- From darklua: readable, dense, token_based (5 files)
    ├── analysis/        <-- From darklua: evaluator, visitors, side_effects (14 files)
    │
    ├── semantic/        <-- [NEW: The Semantic Model]
    │   ├── symbol.rs    (SymbolId, SymbolKind, def-use chains)
    │   ├── scope.rs     (ScopeId, lexical hierarchy, upvalue detection)
    │   └── model.rs     (SemanticModel query API)
    │
    └── cfg/             <-- [NEW: The Control Flow Graph]
        ├── block.rs     (BasicBlock, BlockId, Terminator)
        ├── graph.rs     (ControlFlowGraph, preds/succs)
        ├── builder.rs   (AST -> CFG constructor)
        └── dfa/         (Liveness, Reaching Definitions, Constant Propagation)
```
