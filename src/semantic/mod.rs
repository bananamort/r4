//! The semantic model: symbol tables, lexical scopes, definition/use tracking,
//! upvalue capture analysis and goto/label resolution.
//!
//! The entry point is [`SemanticAnalyzer::analyze`], which walks a parsed
//! [`Block`](crate::nodes::Block) and produces a [`SemanticModel`].
//!
//! Analysis also stamps structural bindings on the tree itself: every
//! identifier is bound to the symbol or label it resolves to (exact even for
//! same-scope shadowing), and every block receives a unique identity. Passes
//! consume those bindings instead of source tokens, so analysis and
//! transformations work on token-less ASTs and survive cloning and synthesis.

mod analyzer;
mod error;
mod goto;
mod label;
mod model;
mod module;
mod scope;
mod symbol;
mod type_alias;
mod walker;

pub use analyzer::is_builtin_type_name;
pub use analyzer::SemanticAnalyzer;
pub use error::{SemanticError, SemanticErrorKind, SemanticOptions, SemanticResult};
pub use label::{GotoBinding, GotoLabel, GotoSite, LabelId};
pub use model::SemanticModel;
pub use module::{
    extract_interface, ExportValue, ImportBinding, ModuleError, ModuleId, ModuleInterface,
    ModuleModel, MultiModuleModel,
};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbol::{AccessKind, Symbol, SymbolId, SymbolKind, SymbolUse, TokenKey};
pub use type_alias::{TypeId, TypeSymbol};
