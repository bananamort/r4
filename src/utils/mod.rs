mod expressions_as_statement;
pub(crate) mod lines;
mod preserve_arguments_side_effects;
mod scoped_hash_map;
mod timer;

pub(crate) use expressions_as_statement::{expressions_as_expression, expressions_as_statement};
pub(crate) use preserve_arguments_side_effects::preserve_arguments_side_effects;
pub use timer::Timer;
