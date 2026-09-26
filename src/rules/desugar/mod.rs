//! Passes that lower modern Luau syntax into vanilla Lua constructs.

pub mod convert_function_to_assignment;
pub mod convert_local_function_to_assignment;
pub mod convert_luau_number;
pub mod make_assignment_local;
pub mod remove_attribute;
pub mod remove_compound_assign;
pub mod remove_continue;
pub mod remove_floor_division;
pub mod remove_if_expression;
pub mod remove_interpolated_string;
pub mod remove_local_attributes;
pub mod remove_method_call;
pub mod remove_method_definition;
pub mod remove_types;

pub use convert_function_to_assignment::convert_function_to_assignment;
pub use convert_local_function_to_assignment::{
    convert_local_function_to_assignment, ConvertLocalFunctionError,
};
pub use convert_luau_number::{convert_luau_number, convert_luau_number_with_code};
pub use make_assignment_local::make_assignment_local;
pub use remove_attribute::remove_attribute;
pub use remove_compound_assign::remove_compound_assign;
pub use remove_continue::{remove_continue, RemoveContinueError};
pub use remove_floor_division::remove_floor_division;
pub use remove_if_expression::remove_if_expression;
pub use remove_interpolated_string::{
    remove_interpolated_string, remove_interpolated_string_with_tostring,
};
pub use remove_local_attributes::remove_local_attributes;
pub use remove_method_call::remove_method_call;
pub use remove_method_definition::remove_method_definition;
pub use remove_types::remove_types;
