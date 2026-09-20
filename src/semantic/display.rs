//! Configurable display formatting for semantic symbols.
//!
//! [`SymbolDisplay`] renders a [`Symbol`] (or a unified [`SymbolRef`]) for
//! diagnostics, hover text and error messages. The default rendering is the
//! symbol's name; [`SymbolDisplayFormat`] adds the declaration kind and the
//! declaring scope's kind.
//!
//! Rendering never guesses: it only reads the symbol's recorded name, kind and
//! declaring scope from the model. Nothing is derived from the symbol's name
//! text, so a shadowed name displays with its own declaration.

use std::fmt;

use super::module::GlobalSymbol;
use super::{ScopeKind, SemanticModel, Symbol, SymbolKind};

/// Options controlling how a symbol is rendered.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolDisplayFormat {
    kind: bool,
    scope_kind: bool,
}

impl SymbolDisplayFormat {
    /// Creates a format that renders only the symbol's name.
    pub const fn new() -> Self {
        Self {
            kind: false,
            scope_kind: false,
        }
    }

    /// Includes the declaration kind (`local`, `parameter`, `global`, ...).
    pub const fn with_kind(mut self, include: bool) -> Self {
        self.kind = include;
        self
    }

    /// Includes the declaring scope's kind (`file`, `function`, `block`).
    pub const fn with_scope_kind(mut self, include: bool) -> Self {
        self.scope_kind = include;
        self
    }

    /// Returns whether the declaration kind is included.
    #[inline]
    pub const fn kind(self) -> bool {
        self.kind
    }

    /// Returns whether the declaring scope kind is included.
    #[inline]
    pub const fn scope_kind(self) -> bool {
        self.scope_kind
    }
}

/// A rendered symbol.
///
/// Created by [`SemanticModel::display_symbol`] and
/// [`MultiModuleModel::display_symbol`](crate::semantic::MultiModuleModel::display_symbol).
#[derive(Clone, Debug)]
pub struct SymbolDisplay<'a> {
    name: &'a str,
    kind: SymbolKind,
    scope: Option<ScopeKind>,
    format: SymbolDisplayFormat,
}

impl<'a> SymbolDisplay<'a> {
    pub(crate) fn local(
        symbol: &'a Symbol,
        model: &'a SemanticModel,
        format: SymbolDisplayFormat,
    ) -> Self {
        Self {
            name: &symbol.name,
            kind: symbol.kind,
            scope: Some(model.scope(symbol.declaring_scope).kind),
            format,
        }
    }

    pub(crate) fn global(symbol: &'a GlobalSymbol, format: SymbolDisplayFormat) -> Self {
        Self {
            name: &symbol.name,
            kind: SymbolKind::Global,
            scope: None,
            format,
        }
    }

    /// Returns the symbol's name.
    #[inline]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the symbol's declaration kind.
    #[inline]
    pub fn kind(&self) -> SymbolKind {
        self.kind
    }

    /// Returns the kind of the symbol's declaring scope, when known.
    #[inline]
    pub fn scope_kind(&self) -> Option<ScopeKind> {
        self.scope
    }

    /// Returns the format used by this display.
    #[inline]
    pub fn format(&self) -> SymbolDisplayFormat {
        self.format
    }
}

impl fmt::Display for SymbolDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.format.kind {
            write!(f, "{} ", self.kind.as_str())?;
        }
        f.write_str(self.name)?;
        if self.format.scope_kind {
            if let Some(scope) = self.scope {
                write!(f, " ({} scope)", scope.as_str())?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for Symbol {
    /// Renders the symbol's name.
    ///
    /// For a configurable rendering use
    /// [`SemanticModel::display_symbol`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}
