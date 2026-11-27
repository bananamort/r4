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

Aliases are followed when they are immutable (`local g = f`) or when their only
non-declaration write assigns the alias (`local g; g = f`). Each capture must
run before the value it reads: the function's assignment dominates every capture
on the chain, and each capture dominates the next one and the call.

## Goto and labels

A `goto` or label in the function that owns the call defeats the proof, because a
backward jump can reach a call before the assignment (the name would be `nil`
there and the call would raise instead of running the body). Function bodies
nested in expressions are walked too, so their own gotos are accounted for.

## Evaluation order

The preamble runs at the start of the call's statement, so inlining also
requires that everything evaluated before the call is inert or stored first.
When a later expression can be inlined, earlier argument values, return values,
binary operands, and index keys are stored in fresh locals in their original
order, and a call's callee is stored before an inlined argument. An inlined
call's own arguments are rewritten first, so nested calls are inlined too. This
keeps `print(value, set(5))` and `value + set()` correct while still inlining.

Short-circuit operands, `if` expression branches, `elseif` conditions, and
`while`/`repeat` conditions cannot be reordered this way and stop further
inlining. A function containing a `goto` or label refuses inlining because the
preamble's locals would make a forward jump enter their scope. Method calls do
not spill their receiver, because the method lookup must stay between the
receiver and the arguments.

A body that reads a global also refuses when the call site shadows `_ENV`,
because the inlined access would resolve through the caller's environment.

## Remaining refusals

Language limits:

- a body that reads `...` (no expression denotes a foreign vararg list);
- a variable or multi-valued return in a value-consuming position;
- an upvalue, global, or `_ENV` that is shadowed at the call site.

Conservative design limits:

- calls in `while`/`repeat` conditions (evaluated once per iteration);
- calls in method arguments (the method lookup must stay in place);
- calls in a binary operand after a read, or an index key after the table
  (Luau may read the local's register after the other operand);
- table-entry sequences;
- functions containing `goto`/labels (needs label-scope analysis);
- multiple or conditional assignments to the same local (needs
  reaching-definition matching).

## Baseline

The pass returns `Result<usize, InlineError>`, validating block provenance before
collection. Coverage lives in the colocated unit tests, the reparse test, and the
execution-oracle programs (`OPTIMIZED_PROGRAMS`).
