//! Passes that lower modern Luau syntax into vanilla Lua constructs.

pub mod convert_luau_number;
pub mod remove_compound_assign;
pub mod remove_continue;
pub mod remove_floor_division;
pub mod remove_if_expression;
pub mod remove_interpolated_string;
pub mod remove_method_call;
pub mod remove_types;

pub use convert_luau_number::{convert_luau_number, convert_luau_number_with_code};
pub use remove_compound_assign::remove_compound_assign;
pub use remove_continue::remove_continue;
pub use remove_floor_division::remove_floor_division;
pub use remove_if_expression::remove_if_expression;
pub use remove_interpolated_string::{
    remove_interpolated_string, remove_interpolated_string_with_tostring,
};
pub use remove_method_call::remove_method_call;
pub use remove_types::remove_types;
