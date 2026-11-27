# Inlining functions bound by a separate assignment

## Target shape

```lua
local f
f = function(...) ... end
-- calls to f
```

This is the last implementable refusal in `src/rules/canonicalize/inline_local_functions.rs`.
After it, the only refusals left must be hard language limits:

- the callee body reads `...`;
- a variable or multi-valued return sits in a value-consuming position;
- an upvalue used by the callee is shadowed at the call site.

## Decision: syntactic dominance

A CFG is not required. Structured control flow makes lexical dominance sufficient:
the assignment's block must be an ancestor of the call's block, and the statement
that leads from that block toward the call must come *after* the assignment.

The proof is refused when a `goto` or label exists, because a backward jump can
reach a call without passing the assignment (the name would be `nil` there and
the call would raise instead of running the body).

## Implementation steps

1. `inlinable` accepts `write_count == 2` when the single non-declaration write is
   the `f = function(...)` assignment, provided the symbol is a `Local` and is not
   captured.
2. Replace the visitor-based `FunctionCollector` with a block walker that carries
   `(BlockIdentity, statement index)` and records the assignment's
   `StatementOrigin` next to the body. It recurses into nested blocks exactly like
   `rewrite_block`.
3. `Inliner` keeps `frames: Vec<(BlockIdentity, u32)>`. Each `rewrite_block` pushes
   its block, sets the frame index to the statement being rewritten (and to
   `statements.len()` for the block's last statement), and truncates on exit.
4. `try_inline` refuses an assignment-bound callee unless `assignment_dominates`:
   the assignment's block is on the current frame path and
   `assignment.index < frame_index`. This accepts
   `local f; f = function() ... end; return f()` and rejects a conditional
   assignment (`if c then f = ... end; return f()`) or a call before the assignment.
5. `goto`/label anywhere disables the assignment-bound proof.
6. `inline_local_functions` returns `Result<usize, InlineError>` with a provenance
   error, checked before collection, matching the other CFG-backed passes.
   Update `.expect(...)` at the execution oracle and in the unit-test helper.
7. Tests: inlined when dominated; refused when the assignment is conditional, when
   the call precedes the assignment, and when `f` is captured; plus reparse
   coverage and an execution-oracle program.

## Current baseline

`HEAD == origin/main == 3237c27`, 1803 lib tests, 2534 non-oracle tests,
19 inliner tests, clippy/fmt clean, CI run 45910261854 green on all five jobs.
