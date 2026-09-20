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

/// Roblox DataModel, services, and engine global identifiers.
pub const ROBLOX: [&str; 58] = [
    "Axes",
    "bit32",
    "BrickColor",
    "CatalogSearchParams",
    "CellId",
    "ColorSequence",
    "ColorSequenceKeypoint",
    "Color3",
    "CFrame",
    "DateTime",
    "DebuggerManager",
    "delay",
    "DockWidgetPluginGuiInfo",
    "elapsedTime",
    "Enum",
    "Faces",
    "Instance",
    "LoadLibrary",
    "game",
    "NumberRange",
    "NumberSequence",
    "NumberSequenceKeypoint",
    "OverlapParams",
    "PathWaypoint",
    "PhysicalProperties",
    "plugin",
    "PluginDrag",
    "PluginManager",
    "printidentity",
    "Random",
    "Ray",
    "RaycastParams",
    "Rect",
    "Region3",
    "Region3int16",
    "script",
    "settings",
    "shared",
    "stats",
    "spawn",
    "task",
    "tick",
    "time",
    "TweenInfo",
    "typeof",
    "UDim",
    "UDim2",
    "UserSettings",
    "utf8",
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
        assert_eq!(ROBLOX.len(), 58);

        assert!(DEFAULT.contains(&"print"));
        assert!(DEFAULT.contains(&"_VERSION"));
        assert!(ROBLOX.contains(&"game"));
        assert!(ROBLOX.contains(&"workspace"));
    }
}
