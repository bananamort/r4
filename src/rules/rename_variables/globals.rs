//! Standard host environment global identifiers for Lua and Roblox.

/// Standard Lua 5.1 and Luau global identifiers.
pub const DEFAULT: [&str; 40] = [
    "arg",
    "assert",
    "collectgarbage",
    "coroutine",
    "debug",
    "dofile",
    "error",
    "gcinfo",
    "getfenv",
    "getmetatable",
    "io",
    "ipairs",
    "load",
    "loadfile",
    "loadstring",
    "math",
    "module",
    "newproxy",
    "next",
    "os",
    "package",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawset",
    "require",
    "select",
    "setfenv",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
    "_G",
    "_VERSION",
];

/// Roblox DataModel, services and engine globals, plus the Luau standard library
/// types that the engine exposes (`buffer`, `vector`).
pub const ROBLOX: [&str; 70] = [
    "Axes",
    "bit32",
    "BrickColor",
    "buffer",
    "CatalogSearchParams",
    "CellId",
    "CFrame",
    "Color3",
    "ColorSequence",
    "ColorSequenceKeypoint",
    "Content",
    "DateTime",
    "DebuggerManager",
    "delay",
    "DockWidgetPluginGuiInfo",
    "elapsedTime",
    "Enum",
    "Faces",
    "FloatCurveKey",
    "Font",
    "game",
    "Instance",
    "LoadLibrary",
    "NumberRange",
    "NumberSequence",
    "NumberSequenceKeypoint",
    "OverlapParams",
    "PathfindingLink",
    "PathfindingModifier",
    "PathWaypoint",
    "PhysicalProperties",
    "plugin",
    "PluginDrag",
    "PluginManager",
    "printidentity",
    "Random",
    "Ray",
    "RaycastParams",
    "RaycastResult",
    "Rect",
    "Region3",
    "Region3int16",
    "RotationCurveKey",
    "script",
    "Secret",
    "SecurityCapabilities",
    "settings",
    "shared",
    "SharedTable",
    "spawn",
    "stats",
    "task",
    "tick",
    "time",
    "TweenInfo",
    "typeof",
    "UDim",
    "UDim2",
    "UserSettings",
    "utf8",
    "vector",
    "Vector2",
    "Vector2int16",
    "Vector3",
    "Vector3int16",
    "version",
    "wait",
    "warn",
    "workspace",
    "ypcall",
];

#[cfg(test)]
mod test {
    use super::{DEFAULT, ROBLOX};

    #[test]
    fn exposes_the_host_global_sets() {
        assert_eq!(DEFAULT.len(), 40);
        assert_eq!(ROBLOX.len(), 70);

        assert!(DEFAULT.contains(&"print"));
        assert!(DEFAULT.contains(&"_VERSION"));
        assert!(ROBLOX.contains(&"game"));
        assert!(ROBLOX.contains(&"workspace"));

        // The modern Luau/Roblox globals must be present so renaming never
        // generates an identifier that collides with an engine global.
        for name in [
            "buffer",
            "vector",
            "Font",
            "Content",
            "Secret",
            "SecurityCapabilities",
            "SharedTable",
            "RaycastResult",
            "FloatCurveKey",
            "RotationCurveKey",
            "PathfindingLink",
            "PathfindingModifier",
        ] {
            assert!(ROBLOX.contains(&name), "missing Roblox global `{name}`");
        }
    }

    #[test]
    fn roblox_globals_are_sorted_and_unique() {
        let mut sorted: Vec<&str> = ROBLOX.to_vec();
        sorted.sort_by_key(|name| name.to_ascii_lowercase());
        assert_eq!(
            sorted,
            ROBLOX.to_vec(),
            "the Roblox global set must stay sorted"
        );

        let mut unique: Vec<&str> = ROBLOX.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            ROBLOX.len(),
            "the Roblox global set must be unique"
        );
    }
}
