# Inlining functions bound by a separate assignment

## Shape

```lua
local f
f = function(...) ... end
-- calls to f
```

`inline_local_functions` inlines calls to a local function that is declared first
and assigned later, alongside the declaration-bound forms (`local function f` and
`local f = function`).

## The proof: syntactic dominance

A function assignment is recorded with its `StatementOrigin`. A call is inlined
only when the assignment provably runs first:

- the assignment's block must contain the call, or contain the statement chain
  that leads to it;
- along that chain, the statement leading toward the call must come after the
  assignment;
- a call inside a nested function is accepted when the assignment precedes the
  function's definition, because the body cannot run before the definition
  completes;
- an assignment inside a `do` body is lifted to the `do` statement, because the
  body always runs to completion before that statement finishes. Loop bodies are
  not lifted: a `break` or `continue` can leave them before the assignment.

This accepts `local f; f = function() ... end; return f()` and rejects a
conditional assignment (`if c then f = ... end; return f()`) or a call that
occurs before the assignment.

A symbol bound this way is accepted only with exactly two writes: its
declaration and the one function assignment. A second write could replace the
function before the call, so it disables the inline.

Immutable aliases (`local g = f`) are followed only when the assignment dominates
the alias's declaration as well, because the alias captures the value where it is
declared.

## Goto and labels

A `goto` or label in the function that owns the call defeats the proof, because a
backward jump can reach a call before the assignment (the name would be `nil`
there and the call would raise instead of running the body). Function bodies
nested in expressions are walked too, so their own gotos are accounted for.

## Remaining refusals

After this shape, the inliner refuses only:

- a body that reads `...` (no expression denotes a foreign vararg list);
- a variable or multi-valued return in a value-consuming position;
- an upvalue that is shadowed at the call site;
- mutable aliases (only immutable aliases are resolved).

## Baseline

The pass returns `Result<usize, InlineError>`, validating block provenance before
collection. Coverage lives in the colocated unit tests, the reparse test, and the
execution-oracle programs (`OPTIMIZED_PROGRAMS`).
