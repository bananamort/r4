--[====[
Legend:

SAFE_<N>   : Safe to rename (local variables, parameters, loop variables, upvalues, implicit self)
UNSAFE_<N> : Unsafe to rename (table keys, globals, metamethods, external APIs)
]====]

do
    -- Environment and runtime compatibility setup
    local SAFE_1 = _ENV or (getfenv and getfenv()) or _G -- [SAFE: Local Variable]

    -- Bitwise operations (fallback for Lua 5.1/LuaJIT/standalone environments)
    local SAFE_2 = bit32 or (pcall(require, "bit") and require("bit")) or {} -- [SAFE: Local Variable]
    local SAFE_3 = SAFE_2.bxor or function(SAFE_4, SAFE_5) -- [SAFE: Local Variable] -- [SAFE: Function Argument]
        local SAFE_6, SAFE_7 = 1, 0 -- [SAFE: Local Variable]
        while SAFE_4 > 0 or SAFE_5 > 0 do
            local SAFE_8 = SAFE_4 % 2 -- [SAFE: Local Variable]
            local SAFE_9 = SAFE_5 % 2 -- [SAFE: Local Variable]
            if SAFE_8 ~= SAFE_9 then
                SAFE_7 = SAFE_7 + SAFE_6 -- [SAFE: Upvalue Usage]
            end
            SAFE_4 = math.floor(SAFE_4 / 2)
            SAFE_5 = math.floor(SAFE_5 / 2)
            SAFE_6 = SAFE_6 * 2 -- [SAFE: Upvalue Usage]
        end
        return SAFE_7 -- [SAFE: Upvalue Usage]
    end

    -- Roblox API mock / passthrough
    local UNSAFE_game = rawget(_G, "game") or { -- [UNSAFE: Global Variable]
        GetService = function(self, SAFE_10) -- [UNSAFE: External API Field]
            return {
                Create = function(self, SAFE_11, SAFE_12, SAFE_13) -- [UNSAFE: External API Field]
                    return {
                        Play = function(self) -- [UNSAFE: External API Field]
                            return true
                        end
                    }
                end
            }
        end
    }
    local UNSAFE_Instance = rawget(_G, "Instance") or { -- [UNSAFE: Global Variable]
        new = function(SAFE_14) return { ClassName = SAFE_14 } end -- [UNSAFE: External API Field]
    }
    local UNSAFE_TweenInfo = rawget(_G, "TweenInfo") or { -- [UNSAFE: Global Variable]
        new = function(...) return { ... } end -- [UNSAFE: External API Field]
    }

    -- repeat ... until scoping (SAFE_15 remains in scope in until condition)
    local SAFE_15 = "outer_repeat" -- [SAFE: Local Variable]
    local SAFE_16 = 0 -- [SAFE: Local Variable]
    repeat
        local SAFE_15 = "inner_repeat" -- [SAFE: Local Shadowing]
        local SAFE_17 = true -- [SAFE: Local Variable]
        SAFE_16 = SAFE_16 + 1 -- [SAFE: Upvalue Usage]
    until SAFE_17 and SAFE_15 == "inner_repeat" -- [SAFE: Upvalue Usage]
    assert(SAFE_15 == "outer_repeat")
    assert(SAFE_16 == 1)

    -- Numeric for header scope isolation (range expressions resolve to outer scope)
    local SAFE_18 = 10 -- [SAFE: Local Variable]
    local SAFE_19 = 0 -- [SAFE: Local Variable]
    for SAFE_18 = SAFE_18, SAFE_18 + 2, 1 do -- [SAFE: Loop Variable] -- [SAFE: Upvalue Usage]
        SAFE_19 = SAFE_19 + SAFE_18 -- [SAFE: Upvalue Usage]
    end
    assert(SAFE_18 == 10)
    assert(SAFE_19 == 33)

    -- Generic for iterator expression isolation
    local SAFE_20, SAFE_21 = "outer_k", "outer_v" -- [SAFE: Local Variable]
    local SAFE_22 = { [SAFE_20] = SAFE_21 } -- [SAFE: Local Variable] -- [SAFE: Upvalue Usage]
    local SAFE_23 = 0 -- [SAFE: Local Variable]
    for SAFE_20, SAFE_21 in pairs(SAFE_22) do -- [SAFE: Loop Variable]
        assert(SAFE_20 == "outer_k")
        assert(SAFE_21 == "outer_v")
        SAFE_23 = SAFE_23 + 1 -- [SAFE: Upvalue Usage]
    end
    assert(SAFE_20 == "outer_k" and SAFE_21 == "outer_v")
    assert(SAFE_23 == 1)

    -- Recursive local function
    local function SAFE_24(SAFE_25) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
        if SAFE_25 <= 1 then return 1 end
        return SAFE_25 * SAFE_24(SAFE_25 - 1) -- [SAFE: Upvalue Usage]
    end
    assert(SAFE_24(4) == 24)

    -- Anonymous function assigned to local vs global resolution
    UNSAFE_1 = function(SAFE_26) return SAFE_26 * 10 end -- [UNSAFE: Global Variable]
    local SAFE_27 = function(SAFE_28) -- [SAFE: Local Variable] -- [SAFE: Function Argument]
        return UNSAFE_1(SAFE_28) -- [UNSAFE: Global Variable]
    end
    assert(SAFE_27(5) == 50)

    -- Mutually recursive local functions
    local SAFE_29, SAFE_30 -- [SAFE: Local Variable]
    SAFE_29 = function(SAFE_31) -- [SAFE: Upvalue Usage] -- [SAFE: Function Argument]
        if SAFE_31 <= 0 then return true end
        return SAFE_30(SAFE_31 - 1) -- [SAFE: Upvalue Usage]
    end
    SAFE_30 = function(SAFE_32) -- [SAFE: Upvalue Usage] -- [SAFE: Function Argument]
        if SAFE_32 <= 0 then return false end
        return SAFE_29(SAFE_32 - 1) -- [SAFE: Upvalue Usage]
    end
    assert(SAFE_29(4) == true and SAFE_29(3) == false)

    -- The 'local x = x' self-shadowing idiom
    local SAFE_33 = 42 -- [SAFE: Local Variable]
    do
        local SAFE_33 = SAFE_33 + 8 -- [SAFE: Local Shadowing]
        assert(SAFE_33 == 50)
    end
    assert(SAFE_33 == 42)

    -- Shadowing standard library globals into local scope
    local SAFE_34 = print -- [SAFE: Local Variable]
    local SAFE_35 = math  -- [SAFE: Local Variable]
    local SAFE_36 = select -- [SAFE: Local Variable]
    assert(type(SAFE_34) == "function" and type(SAFE_35) == "table")

    -- Multiple assignment and swap evaluation order
    local SAFE_37, SAFE_38 = 100, 200 -- [SAFE: Local Variable]
    SAFE_37, SAFE_38 = SAFE_38, SAFE_37
    assert(SAFE_37 == 200 and SAFE_38 == 100)

    local SAFE_39, SAFE_40, SAFE_41 = 1, 2 -- [SAFE: Local Variable]
    assert(SAFE_39 == 1 and SAFE_40 == 2 and SAFE_41 == nil)

    -- Table key vs. local identifier disambiguation
    local SAFE_42 = "dynamic_table_key" -- [SAFE: Local Variable]
    local SAFE_43 = "property_value_1"  -- [SAFE: Local Variable]
    local SAFE_44 = "property_value_2"  -- [SAFE: Local Variable]

    local SAFE_45 = { -- [SAFE: Local Variable]
        UNSAFE_2 = SAFE_43,      -- [UNSAFE: Table Field Key] -- [SAFE: Upvalue Usage]
        ["UNSAFE_3"] = SAFE_44,  -- [UNSAFE: Table Field Key] -- [SAFE: Upvalue Usage]
        [SAFE_42] = 999,         -- [SAFE: Upvalue Usage]
    }

    local SAFE_46 = SAFE_45.UNSAFE_2 -- [SAFE: Local Variable] -- [UNSAFE: Table Field Key]
    assert(SAFE_46 == "property_value_1")

    local SAFE_47 = SAFE_45["UNSAFE_3"] -- [SAFE: Local Variable] -- [UNSAFE: Table Field Key]
    assert(SAFE_47 == "property_value_2")

    local SAFE_48 = SAFE_45[SAFE_42] -- [SAFE: Local Variable] -- [SAFE: Upvalue Usage]
    assert(SAFE_48 == 999)

    SAFE_45.UNSAFE_4 = "dynamically_added_prop" -- [UNSAFE: Table Field Key]
    SAFE_45["UNSAFE_5"] = "dynamically_added_bracket" -- [UNSAFE: Table Field Key]
    assert(SAFE_45.UNSAFE_4 == "dynamically_added_prop")
    assert(SAFE_45.UNSAFE_5 == "dynamically_added_bracket")

    -- Method syntax and implicit 'self' parameter
    local SAFE_49 = { -- [SAFE: Local Variable]
        UNSAFE_counter = 10 -- [UNSAFE: Table Field Key]
    }

    function SAFE_49:UNSAFE_method_increment(SAFE_50) -- [UNSAFE: Table Field Key] -- [SAFE: Function Argument]
        local SAFE_51 = self -- [SAFE: Local Variable] -- [SAFE: Implicit Parameter]
        SAFE_51.UNSAFE_counter = SAFE_51.UNSAFE_counter + SAFE_50 -- [UNSAFE: Table Field Key]
        return SAFE_51.UNSAFE_counter -- [UNSAFE: Table Field Key]
    end

    function SAFE_49:UNSAFE_method_override(self) -- [UNSAFE: Table Field Key] -- [SAFE: Function Argument]
        return self * 3 -- [SAFE: Upvalue Usage]
    end

    local SAFE_52 = 5 -- [SAFE: Local Variable]
    local SAFE_53 = SAFE_49:UNSAFE_method_increment(SAFE_52) -- [SAFE: Local Variable] -- [UNSAFE: Table Field Key]
    assert(SAFE_53 == 15)
    assert(SAFE_49:UNSAFE_method_override(10) == 30)

    -- Metamethods and external API protections
    local SAFE_54 = {} -- [SAFE: Local Variable]
    local SAFE_55 = { -- [SAFE: Local Variable]
        __index = function(SAFE_56, SAFE_57) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            if SAFE_57 == "UNSAFE_virtual" then -- [UNSAFE: Table Field Key]
                return rawget(SAFE_56, "UNSAFE_backing") or 100 -- [UNSAFE: Table Field Key]
            end
            return rawget(SAFE_56, SAFE_57)
        end,
        __newindex = function(SAFE_58, SAFE_59, SAFE_60) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            rawset(SAFE_58, SAFE_59, SAFE_60)
        end,
        __call = function(SAFE_61, ...) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            local SAFE_62 = select("#", ...) -- [SAFE: Local Variable]
            return SAFE_62 * 10
        end,
        __tostring = function(SAFE_63) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            return "<SafeObject>"
        end,
        __concat = function(SAFE_64, SAFE_65) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            return tostring(SAFE_64) .. ":" .. tostring(SAFE_65)
        end,
        __add = function(SAFE_66, SAFE_67) -- [UNSAFE: Metamethod Name] -- [SAFE: Function Argument]
            return 777
        end,
    }
    setmetatable(SAFE_54, SAFE_55)

    SAFE_54.UNSAFE_backing = 42 -- [UNSAFE: Table Field Key]
    assert(SAFE_54.UNSAFE_virtual == 42)
    assert(SAFE_54(1, 2, 3) == 30)
    assert(tostring(SAFE_54) == "<SafeObject>")
    assert(SAFE_54 + 5 == 777)

    -- External API protection
    local SAFE_68 = {
        Transparency = 0.5, -- [UNSAFE: External API Field]
        Size = 10,          -- [UNSAFE: External API Field]
    }
    local SAFE_69 = UNSAFE_game:GetService("TweenService") -- [UNSAFE: External API Field]
    local SAFE_70 = UNSAFE_Instance.new("Part")             -- [UNSAFE: External API Field]
    local SAFE_71 = UNSAFE_TweenInfo.new(1)                 -- [UNSAFE: External API Field]
    local SAFE_72 = SAFE_69:Create(SAFE_70, SAFE_71, SAFE_68) -- [UNSAFE: External API Field]
    assert(SAFE_72:Play() == true)

    -- False-negative traps (locals named like reserved identifiers)
    do
        local SAFE_73 = 111 -- [SAFE: Local Variable]
        local SAFE_74 = function(SAFE_75) return SAFE_75 * 2 end -- [SAFE: Local Variable]
        local SAFE_76 = "standalone_self" -- [SAFE: Local Variable]
        local SAFE_77 = "local_print"     -- [SAFE: Local Variable]

        assert(SAFE_73 == 111)
        assert(SAFE_74(4) == 8)
        assert(SAFE_76 == "standalone_self")
        assert(SAFE_77 == "local_print")
    end

    -- Obfuscation logic in SAFE_<N> style (byte xor & string decoder)
    local function SAFE_78(SAFE_79, SAFE_80) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
        local SAFE_81 = 0 -- [SAFE: Local Variable]
        local SAFE_82 = 1 -- [SAFE: Local Variable]
        for SAFE_83 = 1, #SAFE_79 do -- [SAFE: Loop Variable]
            local SAFE_84 = SAFE_79:byte(SAFE_83) -- [SAFE: Local Variable]
            SAFE_81 = (SAFE_81 + (SAFE_84 * (SAFE_80 or 1))) % 2^31 -- [SAFE: Upvalue Usage]
            SAFE_82 = (SAFE_82 * ((SAFE_84 % 7) + 3)) % 2^31 -- [SAFE: Upvalue Usage]
        end
        return SAFE_3(SAFE_81, SAFE_82) % 100000 -- [SAFE: Upvalue Usage]
    end

    local function SAFE_85(SAFE_86, SAFE_87) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
        local SAFE_88 = {} -- [SAFE: Local Variable]
        local SAFE_89 = SAFE_87 or 1 -- [SAFE: Local Variable]
        for SAFE_90 = 1, #SAFE_86 do -- [SAFE: Loop Variable]
            local SAFE_91 = SAFE_86[SAFE_90] -- [SAFE: Local Variable]
            SAFE_88[#SAFE_88 + 1] = string.char(SAFE_3(SAFE_91, SAFE_89) % 256) -- [SAFE: Upvalue Usage]
            SAFE_89 = SAFE_3(SAFE_89 * 3, 13) % 256 -- [SAFE: Upvalue Usage]
        end
        return table.concat(SAFE_88) -- [SAFE: Upvalue Usage]
    end

    local SAFE_92 = { 0x25, 0x64, 0x11, 0x7B, 0x15 } -- [SAFE: Local Variable]
    local SAFE_93 = { 0x33, 0x03, 0x6D, 0x01, 0x10 } -- [SAFE: Local Variable]

    local function SAFE_94() -- [SAFE: Local Function Name]
        local SAFE_95 = SAFE_85(SAFE_92, 97) -- [SAFE: Local Variable]
        local SAFE_96 = SAFE_85(SAFE_93, 42) -- [SAFE: Local Variable]
        return SAFE_95, SAFE_96
    end

    local SAFE_97, SAFE_98 = SAFE_94()
    assert(type(SAFE_97) == "string" and type(SAFE_98) == "string")
    assert(SAFE_78("seed", 123) >= 0)

    -- Dense closures, upvalues & alpha-renaming
    local function SAFE_99(SAFE_100) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
        local SAFE_101 = SAFE_100 or 10 -- [SAFE: Local Variable]

        local function SAFE_102(SAFE_103) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
            local SAFE_104 = SAFE_101 + SAFE_103 -- [SAFE: Local Variable]

            return function(SAFE_105) -- [SAFE: Function Argument]
                local SAFE_101 = SAFE_105 * 2 -- [SAFE: Local Shadowing]

                return function(SAFE_106) -- [SAFE: Function Argument]
                    return SAFE_101 + SAFE_104 + SAFE_106 -- [SAFE: Upvalue Usage]
                end
            end
        end

        return SAFE_102 -- [SAFE: Upvalue Usage]
    end

    local SAFE_107 = SAFE_99(10)(5)(3)(2) -- (3*2) + (10+5) + 2 = 23
    assert(SAFE_107 == 23)

    -- Vararg (...) forwarding & select
    local function SAFE_108(...) -- [SAFE: Local Function Name]
        local SAFE_109 = select("#", ...) -- [SAFE: Local Variable]
        local SAFE_110 = 0 -- [SAFE: Local Variable]

        for SAFE_111 = 1, SAFE_109 do -- [SAFE: Loop Variable]
            local SAFE_112 = select(SAFE_111, ...) -- [SAFE: Local Variable]
            if type(SAFE_112) == "number" then
                SAFE_110 = SAFE_110 + SAFE_112 -- [SAFE: Upvalue Usage]
            end
        end

        local function SAFE_113(...) -- [SAFE: Local Function Name]
            return select(1, ...)
        end

        return SAFE_110, SAFE_113(...)
    end

    local SAFE_114, SAFE_115 = SAFE_108(10, 20, "ignore", 30)
    assert(SAFE_114 == 60 and SAFE_115 == 10)

    -- Global state & dynamic code resolution
    UNSAFE_global_state = 12345 -- [UNSAFE: Global Variable]
    local SAFE_116 = UNSAFE_global_state -- [SAFE: Local Variable] -- [UNSAFE: Global Variable]
    assert(SAFE_116 == 12345)

    local function SAFE_117(SAFE_118) -- [SAFE: Local Function Name] -- [SAFE: Function Argument]
        if loadstring then
            local SAFE_119, SAFE_120 = loadstring(SAFE_118) -- [SAFE: Local Variable]
            if SAFE_119 then return SAFE_119() end
        end
        return nil
    end

    local SAFE_121 = SAFE_117("return 40 + 2") -- [SAFE: Local Variable]
    if SAFE_121 then
        assert(SAFE_121 == 42)
    end

    -- Verification summary
    local SAFE_122 = { -- [SAFE: Local Variable]
        UNSAFE_status = "PASS", -- [UNSAFE: Table Field Key]
        UNSAFE_total_tests = 9, -- [UNSAFE: Table Field Key]
    }

    local SAFE_123 = function() -- [SAFE: Local Variable]
        return string.format("[PARSE.LUA %s]: All %d test categories passed successfully.",
            SAFE_122.UNSAFE_status,
            SAFE_122.UNSAFE_total_tests
        )
    end

    print(SAFE_123())
end