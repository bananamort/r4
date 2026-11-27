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
mod directive;
mod display;
mod error;
mod escape;
mod goto;
mod index;
mod label;
mod model;
mod module;
mod scope;
mod stdlib;
mod symbol;
mod type_alias;
mod walker;

pub use analyzer::is_builtin_type_name;
pub use analyzer::{SemanticAnalysis, SemanticAnalyzer};
pub use directive::{Directive, DirectiveError, Directives, Strictness};
pub use display::{SymbolDisplay, SymbolDisplayFormat};
pub use error::{
    SemanticError, SemanticErrorKind, SemanticOptions, SemanticQueryError, SemanticResult,
};
pub use escape::{EscapeAnalysis, SymbolEscape};
pub use index::{QualifiedLabel, QualifiedSymbol, QualifiedType, WholeProgramIndex};
pub use label::{GotoBinding, GotoLabel, GotoSite, LabelId, StatementPosition, VisibleScopes};
pub use model::{Binding, SemanticModel};
pub use module::{
    extract_interface, ExportValue, GlobalId, GlobalSymbol, ImportBinding, ModuleError, ModuleId,
    ModuleInterface, ModuleModel, MultiModuleModel, ScopeRef, SymbolRef,
};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use stdlib::{call_is_pure, call_is_total, has_observable_effect, has_side_effects};
pub use symbol::{AccessKind, Symbol, SymbolId, SymbolKind, SymbolUse, TokenKey};
pub use type_alias::{TypeId, TypeSymbol};
