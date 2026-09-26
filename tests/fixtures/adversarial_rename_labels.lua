--[====[
Lua 5.2 label/goto companion fixture for the renaming oracle.

`goto` and `::labels::` are Lua 5.2 syntax, while Luau-only constructs (compound
assignment, type packs, `continue`) are not accepted by the Lua 5.2 parser, so
this lives in its own fixture rather than the Luau one.

Labels occupy their own namespace: a label and a runtime variable may share a
name, and renaming the variable must leave every label and every `goto` target
byte-identical. Labels are not runtime symbols.
]====]

do
    -- A variable and a label share a name; they are different namespaces.
    local SAFE_mark = 0
    ::SAFE_mark::
    SAFE_mark = SAFE_mark + 1
    if SAFE_mark < 3 then
        goto SAFE_mark
    end

    -- A forward jump over an assignment (no local is entered).
    local SAFE_skipped = 0
    goto UNSAFE_after
    SAFE_skipped = 100
    ::UNSAFE_after::

    -- A jump out of a nested block to an enclosing label.
    local SAFE_escaped = 0
    do
        SAFE_escaped = 1
        goto UNSAFE_outer
    end
    ::UNSAFE_outer::

    return SAFE_mark + SAFE_skipped + SAFE_escaped
end
