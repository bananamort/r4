#![deny(clippy::all)]

//! r4: A semantically-aware and control-flow tracking Lua analysis and transformation engine.
//!
//! # Example
//!
//! Parse, analyze, lower and render a Luau snippet:
//!
//! ```
//! use r4::generator::{LuaGenerator, ReadableLuaGenerator};
//! use r4::rules::desugar_to_lua51;
//! use r4::semantic::SemanticAnalyzer;
//! use r4::Parser;
//!
//! let mut block = Parser::default()
//!     .luau()
//!     .parse("local total = 0\nfor i = 1, 3 do total += i end\nreturn total")
//!     .expect("valid Luau");
//!
//! let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
//! assert!(model.symbols.iter().any(|symbol| symbol.name == "total"));
//!
//! desugar_to_lua51(&mut block).expect("lowering to Lua 5.1");
//!
//! let mut generator = ReadableLuaGenerator::default();
//! generator.write_block(&block);
//! assert!(generator.into_string().contains("total"));
//! ```

pub mod annotation;
mod ast_converter;
pub mod cfg;
pub mod compilation;
pub mod convert;
pub mod diagnostic;
mod dialect;
pub mod dialect_support;
pub mod equivalence;
pub mod generator;
pub mod modules;
pub mod navigation;
pub mod nodes;
mod parser;
pub mod process;
pub mod rules;
pub mod semantic;
pub mod syntax_factory;
pub mod utils;

pub use annotation::{AnnotationError, NodeAddress, SyntaxAnnotation, SyntaxAnnotations};
pub use compilation::{Compilation, CompilationError, SyntaxTree, TreeId};
pub use convert::{convert_data, to_expression as convert_expression, ConvertDataError};
pub use diagnostic::{
    descriptor_for, Diagnostic, DiagnosticDescriptor, DiagnosticFormatError, DiagnosticFormatter,
    DiagnosticQueryError, LinePosition, LinePositionSpan, Severity, TextSpan, DIAGNOSTIC_CATALOG,
};
pub use dialect_support::{validate_dialect, DialectSupportError};
pub use equivalence::Equivalent;
pub use navigation::{IndexEntry, NavigationError, NodeId, NodeKind, SyntaxIndex};
pub use parser::{AnalyzedParseError, Dialect, Parser, ParserError};
pub use syntax_factory::SyntaxFactory;
