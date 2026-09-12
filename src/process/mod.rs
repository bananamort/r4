//! Defines AST processing and mutation traits, visitors, and constant evaluation.

mod evaluator;
mod node_counter;
mod node_processor;
mod post_visitor;
pub mod utils;
mod visitors;

pub use evaluator::*;
pub use node_counter::NodeCounter;
pub use node_processor::{NodePostProcessor, NodeProcessor};
pub use post_visitor::{DefaultPostVisitor, NodePostVisitor};
pub use visitors::{DefaultVisitor, NodeVisitor};
