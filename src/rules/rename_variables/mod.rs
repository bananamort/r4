pub mod globals;
mod rename;

pub use globals::{DEFAULT as DEFAULT_GLOBALS, ROBLOX as ROBLOX_GLOBALS};
pub use rename::{
    rename_symbol, rename_variables, rename_variables_with_options, RenameError, RenameOptions,
    RenameSymbolError,
};
