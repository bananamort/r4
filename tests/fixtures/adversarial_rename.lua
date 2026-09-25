--[====[
Legend:

SAFE_<name>   : a runtime binding — a local, a parameter, a loop variable, an
                upvalue, or the receiver a colon method introduces. Renaming it
                must not change program behavior.

UNSAFE_<name> : a name that is not a renamable runtime binding — a global, a
                table key, a string key, a field name, a method name, a
                metamethod, a type name, a generic parameter or an attribute.
                It must be byte-identical after renaming.

The suite is checked by tests/adversarial_rename_test.rs, which renames the whole
program and asserts, from the semantic model, that every SAFE_ binding is renamed
consistently, every UNSAFE_ name survives, the symbol graph is unchanged and the
output re-parses and re-analyzes.
]====]

-- Type aliases and generics are type bindings, never runtime symbols.
type UNSAFE_Alias = { value: number }
export type UNSAFE_Exported = { n: number }

do
    ----------------------------------------------------------------------
    -- 1. Globals, and locals that shadow their names.
    --
    -- A global is looked up by name in the environment, so it must never be
    -- renamed. A local with the same name as a global is an ordinary local.
    ----------------------------------------------------------------------
    local SAFE_env = _ENV or _G
    local SAFE_print = print
    local SAFE_math = math
    local SAFE_string = string
    local SAFE_table = table
    local SAFE_select = select
    local SAFE_type = type
    local SAFE_pairs = pairs
    local SAFE_ipairs = ipairs
    local SAFE_setmetatable = setmetatable
    local SAFE_rawget = rawget
    local SAFE_rawset = rawset
    local SAFE_tostring = tostring
    local SAFE_pcall = pcall
    local SAFE_assert = assert

    SAFE_assert(SAFE_print == print and SAFE_math == math)
    SAFE_assert(SAFE_type(SAFE_env) == "table")

    -- Writing a global creates a global; it must keep its name.
    UNSAFE_global_written = 1
    UNSAFE_global_function = function(SAFE_g1) return SAFE_g1 end
    function UNSAFE_global_declared(SAFE_g2) return SAFE_g2 end
    UNSAFE_global_table = {}
    function UNSAFE_global_table.UNSAFE_global_field() end
    UNSAFE_global_table.UNSAFE_global_key = 2
    SAFE_assert(UNSAFE_global_written == 1)
    SAFE_assert(UNSAFE_global_function(2) == 2)
    SAFE_assert(UNSAFE_global_declared(3) == 3)

    ----------------------------------------------------------------------
    -- 2. Implicit `self`, explicit `self`, and a local named `self`.
    ----------------------------------------------------------------------
    local SAFE_obj = { UNSAFE_counter = 10 }

    function SAFE_obj:UNSAFE_increment(SAFE_delta)
        local SAFE_this = self -- the implicit receiver must stay `self`
        SAFE_this.UNSAFE_counter = SAFE_this.UNSAFE_counter + SAFE_delta
        return SAFE_this.UNSAFE_counter
    end

    function SAFE_obj:UNSAFE_scale(self) -- explicit `self` shadows the receiver
        return self * 3
    end

    function SAFE_obj.UNSAFE_dot(self) -- dot syntax: `self` is an ordinary parameter
        return self
    end

    local SAFE_n = 5
    SAFE_assert(SAFE_obj:UNSAFE_increment(SAFE_n) == 15)
    SAFE_assert(SAFE_obj:UNSAFE_scale(10) == 30)
    SAFE_assert(SAFE_obj.UNSAFE_dot(7) == 7)

    do
        local self = 10 -- a local named `self` is renamed like any other local
        SAFE_assert(self == 10)
    end

    ----------------------------------------------------------------------
    -- 3. Table keys, string keys, field access and method names.
    ----------------------------------------------------------------------
    local SAFE_dynamic_key = "UNSAFE_dynamic"
    local SAFE_dict = {
        UNSAFE_field = 1,
        ["UNSAFE_string_key"] = 2,
        [SAFE_dynamic_key] = 3,
        -- a positional entry, not a key
        4,
    }
    SAFE_assert(SAFE_dict.UNSAFE_field == 1)
    SAFE_assert(SAFE_dict["UNSAFE_string_key"] == 2)
    SAFE_assert(SAFE_dict[SAFE_dynamic_key] == 3)

    SAFE_dict.UNSAFE_added_field = 5
    SAFE_dict["UNSAFE_added_string_key"] = 6
    SAFE_assert(SAFE_dict.UNSAFE_added_field == 5)
    SAFE_assert(SAFE_dict["UNSAFE_added_string_key"] == 6)

    local SAFE_ns = {}
    function SAFE_ns.UNSAFE_field_function() end
    SAFE_ns.UNSAFE_namespace = {}
    function SAFE_ns.UNSAFE_namespace.UNSAFE_nested_function() end
    function SAFE_ns:UNSAFE_method(SAFE_arg) return SAFE_arg end
    SAFE_assert(SAFE_ns:UNSAFE_method(1) == 1)

    ----------------------------------------------------------------------
    -- 4. Metamethods. The keys are never variables.
    ----------------------------------------------------------------------
    local SAFE_subject = {}
    local SAFE_meta = {
        __index = function(SAFE_t, SAFE_k)
            return SAFE_rawget(SAFE_t, SAFE_k)
        end,
        __newindex = function(SAFE_t, SAFE_k, SAFE_v)
            SAFE_rawset(SAFE_t, SAFE_k, SAFE_v)
        end,
        __call = function(SAFE_t, ...)
            return SAFE_select("#", ...)
        end,
        __tostring = function(SAFE_t)
            return "<object>"
        end,
        __concat = function(SAFE_a, SAFE_b)
            return SAFE_tostring(SAFE_a) .. SAFE_tostring(SAFE_b)
        end,
        __add = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __sub = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __mul = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __div = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __mod = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __pow = function(SAFE_a, SAFE_b)
            return SAFE_a
        end,
        __unm = function(SAFE_a)
            return SAFE_a
        end,
        __len = function(SAFE_a)
            return 0
        end,
        __eq = function(SAFE_a, SAFE_b)
            return SAFE_a == SAFE_b
        end,
        __lt = function(SAFE_a, SAFE_b)
            return false
        end,
        __le = function(SAFE_a, SAFE_b)
            return true
        end,
    }
    SAFE_setmetatable(SAFE_subject, SAFE_meta)

    ----------------------------------------------------------------------
    -- 5. Scoping: self-reference, shadowing, sibling scopes, loop headers.
    ----------------------------------------------------------------------
    local SAFE_outer = 42
    do
        local SAFE_outer = SAFE_outer + 8 -- the initializer sees the outer local
        SAFE_assert(SAFE_outer == 50)
    end
    SAFE_assert(SAFE_outer == 42)

    local SAFE_sibling = 1
    do
        local SAFE_sibling = 2
        SAFE_assert(SAFE_sibling == 2)
    end
    do
        local SAFE_sibling = 3
        SAFE_assert(SAFE_sibling == 3)
    end
    SAFE_assert(SAFE_sibling == 1)

    local SAFE_index = 10
    local SAFE_total = 0
    for SAFE_index = SAFE_index, SAFE_index + 2 do -- the header sees the outer local
        SAFE_total = SAFE_total + SAFE_index
    end
    SAFE_assert(SAFE_index == 10 and SAFE_total == 33)

    local SAFE_key, SAFE_value = "UNSAFE_outer_key", "UNSAFE_outer_value"
    local SAFE_source = { [SAFE_key] = SAFE_value }
    local SAFE_seen = 0
    for SAFE_key, SAFE_value in SAFE_pairs(SAFE_source) do
        SAFE_assert(SAFE_key == "UNSAFE_outer_key")
        SAFE_assert(SAFE_value == "UNSAFE_outer_value")
        SAFE_seen = SAFE_seen + 1
    end
    SAFE_assert(SAFE_key == "UNSAFE_outer_key")
    SAFE_assert(SAFE_value == "UNSAFE_outer_value")
    SAFE_assert(SAFE_seen == 1)

    local SAFE_rounds = 0
    repeat
        local SAFE_more = true -- body locals are visible in `until`
        SAFE_rounds = SAFE_rounds + 1
    until not SAFE_more or SAFE_rounds >= 3
    SAFE_assert(SAFE_rounds == 3)

    ----------------------------------------------------------------------
    -- 6. Closures, upvalues, recursion and shadowed captures.
    ----------------------------------------------------------------------
    local function SAFE_factorial(SAFE_x)
        if SAFE_x <= 1 then
            return 1
        end
        return SAFE_x * SAFE_factorial(SAFE_x - 1)
    end
    SAFE_assert(SAFE_factorial(4) == 24)

    local SAFE_is_even, SAFE_is_odd
    SAFE_is_even = function(SAFE_x)
        if SAFE_x <= 0 then
            return true
        end
        return SAFE_is_odd(SAFE_x - 1)
    end
    SAFE_is_odd = function(SAFE_x)
        if SAFE_x <= 0 then
            return false
        end
        return SAFE_is_even(SAFE_x - 1)
    end
    SAFE_assert(SAFE_is_even(4) and not SAFE_is_even(3))

    local function SAFE_adder(SAFE_base)
        local SAFE_start = SAFE_base or 10
        local function SAFE_middle(SAFE_step)
            local SAFE_shared = SAFE_start + SAFE_step
            return function(SAFE_factor)
                local SAFE_start = SAFE_factor * 2 -- shadows the captured SAFE_start
                return function(SAFE_extra)
                    return SAFE_start + SAFE_shared + SAFE_extra
                end
            end
        end
        return SAFE_middle
    end
    SAFE_assert(SAFE_adder(10)(5)(3)(2) == 23)

    ----------------------------------------------------------------------
    -- 7. Varargs, multiple assignment, swaps and short-circuit.
    ----------------------------------------------------------------------
    local function SAFE_sum_numbers(...)
        local SAFE_count = SAFE_select("#", ...)
        local SAFE_sum = 0
        for SAFE_i = 1, SAFE_count do
            local SAFE_item = SAFE_select(SAFE_i, ...)
            if SAFE_type(SAFE_item) == "number" then
                SAFE_sum = SAFE_sum + SAFE_item
            end
        end
        return SAFE_sum
    end
    SAFE_assert(SAFE_sum_numbers(1, "x", 2) == 3)

    local SAFE_left, SAFE_right = 1, 2
    SAFE_left, SAFE_right = SAFE_right, SAFE_left
    SAFE_assert(SAFE_left == 2 and SAFE_right == 1)

    local SAFE_first, SAFE_second = 1
    SAFE_assert(SAFE_first == 1 and SAFE_second == nil)

    local SAFE_choice = SAFE_left and SAFE_right or 0
    SAFE_assert(SAFE_choice == 1)

    ----------------------------------------------------------------------
    -- 8. Compound assignment on locals and on fields.
    ----------------------------------------------------------------------
    local SAFE_counter = 1
    SAFE_counter += 2
    SAFE_counter *= 3
    SAFE_assert(SAFE_counter == 9)

    local SAFE_holder = { UNSAFE_count = 0 }
    SAFE_holder.UNSAFE_count += 4
    SAFE_assert(SAFE_holder.UNSAFE_count == 4)

    ----------------------------------------------------------------------
    -- 9. Luau types, casts, generics, if-expressions, continue, attributes.
    --    Type names and generic parameters are never runtime symbols.
    ----------------------------------------------------------------------
    local SAFE_record: UNSAFE_Alias = { value = 1 }
    local SAFE_cast = SAFE_record :: UNSAFE_Alias
    SAFE_assert(SAFE_cast.value == 1)

    local SAFE_exported: UNSAFE_Exported = { n = 1 }
    SAFE_assert(SAFE_exported.n == 1)

    local function SAFE_identity<UNSAFE_T>(SAFE_x: UNSAFE_T): UNSAFE_T
        return SAFE_x
    end
    SAFE_assert(SAFE_identity(5) == 5)

    local SAFE_conditional = if SAFE_left then SAFE_right else 0
    SAFE_assert(SAFE_conditional == 1)

    local SAFE_evens = 0
    for SAFE_scan = 1, 5 do
        if SAFE_scan % 2 == 0 then
            continue
        end
        SAFE_evens = SAFE_evens + SAFE_scan
    end
    SAFE_assert(SAFE_evens == 9)

    local SAFE_attributed = @native function()
        return 1
    end
    SAFE_assert(SAFE_attributed() == 1)

    ----------------------------------------------------------------------
    -- 10. Interpolated strings, string literals and comments are not code.
    ----------------------------------------------------------------------
    local SAFE_message = `count={SAFE_counter} label=UNSAFE_literal_text`
    SAFE_assert(SAFE_type(SAFE_message) == "string")

    local SAFE_literal = "UNSAFE_string_literal_marker"
    SAFE_assert(SAFE_literal == "UNSAFE_string_literal_marker")

    -- SAFE_counter UNSAFE_global_written self print are only words in a comment.

    ----------------------------------------------------------------------
    -- 11. Renaming must survive a name that is also a global field.
    ----------------------------------------------------------------------
    local SAFE_shadowed = SAFE_print -- reused alias
    SAFE_assert(SAFE_shadowed == print)

    ----------------------------------------------------------------------
    -- 12. Remaining binding hazards: parameter shadowing a global, a receiver
    --     captured by a nested closure, a vararg method, chained access and a
    --     string key that is not an identifier.
    ----------------------------------------------------------------------
    local function SAFE_param_shadow(SAFE_print)
        return SAFE_print
    end
    SAFE_assert(SAFE_param_shadow(1) == 1)

    local SAFE_chain = { UNSAFE_inner = { UNSAFE_leaf = 5, ["UNSAFE not an identifier"] = 6 } }
    SAFE_assert(SAFE_chain.UNSAFE_inner.UNSAFE_leaf == 5)
    SAFE_assert(SAFE_chain.UNSAFE_inner["UNSAFE not an identifier"] == 6)

    local SAFE_sized = { 1, 2, 3 }
    SAFE_assert(#SAFE_sized == 3)

    local SAFE_receiver = {}
    function SAFE_receiver:UNSAFE_capture(...)
        local SAFE_count = SAFE_select("#", ...)
        local function SAFE_inner()
            return self -- `self` captured by a nested closure must stay `self`
        end
        return SAFE_count, SAFE_inner()
    end
    local SAFE_captured_count, SAFE_captured_self = SAFE_receiver:UNSAFE_capture(1, 2)
    SAFE_assert(SAFE_captured_count == 2)
    SAFE_assert(SAFE_captured_self == SAFE_receiver)

    UNSAFE_final_status = "PASS"
    SAFE_assert(UNSAFE_final_status == "PASS")

    SAFE_print(SAFE_string.format("[adversarial_rename] status=%s", UNSAFE_final_status))
end
