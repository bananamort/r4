//! r4: A semantically-aware and control-flow tracking Lua analysis and transformation engine.

mod ast_converter;
pub mod cfg;
pub mod compilation;
pub mod diagnostic;
pub mod generator;
pub mod modules;
pub mod nodes;
mod parser;
pub mod process;
pub mod rules;
pub mod semantic;
pub mod utils;

pub use compilation::{Compilation, SyntaxTree, TreeId};
pub use diagnostic::{Diagnostic, LinePosition, LinePositionSpan, Severity, TextSpan};
pub use parser::{AnalyzedParseError, Parser, ParserError};
