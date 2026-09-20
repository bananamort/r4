//! Defines AST processing and mutation traits, visitors, and constant evaluation.

mod detokenize;
mod evaluator;
mod immutable;
mod node_counter;
mod node_processor;
mod post_visitor;
pub mod utils;
mod visitors;

pub use detokenize::{detokenize, detokenize_expression, detokenize_statement};
pub use evaluator::*;
pub use immutable::{
    any_expression, any_statement, collect_identifier_names, count_nodes, ImmutableProcessor,
    ImmutableVisitor, Walk,
};
pub use node_counter::NodeCounter;
pub use node_processor::{NodePostProcessor, NodeProcessor};
pub use post_visitor::{DefaultPostVisitor, NodePostVisitor};
pub use visitors::{DefaultVisitor, NodeVisitor};
