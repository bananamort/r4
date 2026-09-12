//! The semantic model: symbol tables, lexical scopes, definition/use tracking,
//! upvalue capture analysis and goto/label resolution.
//!
//! The entry point is [`SemanticAnalyzer::analyze`], which walks a parsed
//! [`Block`](crate::nodes::Block) and produces a [`SemanticModel`].

mod analyzer;
mod error;
mod goto;
mod label;
mod model;
mod scope;
mod symbol;
mod walker;

pub use analyzer::SemanticAnalyzer;
pub use error::{SemanticError, SemanticErrorKind, SemanticOptions, SemanticResult};
pub use label::{GotoBinding, GotoLabel, GotoSite, LabelId};
pub use model::SemanticModel;
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbol::{AccessKind, Symbol, SymbolId, SymbolKind, SymbolUse, TokenKey};
