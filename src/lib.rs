//! r4: A semantically-aware and control-flow tracking Lua analysis and transformation engine.

mod ast_converter;
pub mod cfg;
pub mod compilation;
pub mod convert;
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
pub use convert::{convert_data, to_expression as convert_expression, ConvertDataError};
pub use diagnostic::{
    descriptor_for, Diagnostic, DiagnosticDescriptor, DiagnosticFormatError, DiagnosticFormatter,
    DiagnosticQueryError, LinePosition, LinePositionSpan, Severity, TextSpan, DIAGNOSTIC_CATALOG,
};
pub use parser::{AnalyzedParseError, Dialect, Parser, ParserError};
