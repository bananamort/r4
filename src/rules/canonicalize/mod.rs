//! Passes that normalize equivalent syntax forms without changing behavior.

pub mod call_parens;
pub mod convert_index_to_field;
pub mod convert_square_root_call;
pub mod empty_do;
pub mod group_local;
pub mod remove_dead_stores;
pub mod remove_nil_declarations;
pub mod remove_unreachable_code;
pub mod remove_unused_labels;
pub mod remove_unused_variables;

pub use call_parens::remove_call_parens;
pub use convert_index_to_field::convert_index_to_field;
pub use convert_square_root_call::{convert_square_root_call, convert_square_root_call_with_model};
pub use empty_do::remove_empty_do;
pub use group_local::{group_local, group_local_with_model};
pub use remove_dead_stores::{remove_dead_stores, DeadStoreError};
pub use remove_nil_declarations::remove_nil_declarations;
pub use remove_unreachable_code::{remove_unreachable_code, UnreachableCodeError};
pub use remove_unused_labels::{remove_unused_labels, UnusedLabelError};
pub use remove_unused_variables::{remove_unused_variables, UnusedVariableError};
