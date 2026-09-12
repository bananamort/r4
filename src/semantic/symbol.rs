use crate::nodes::{BindingId, Token};

use super::scope::ScopeId;

/// Identifies a symbol declared in the [`SemanticModel`](super::SemanticModel).
///
/// The identifier is an index into the model's flat `symbols` arena.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u32);

impl SymbolId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the structural binding corresponding to this symbol.
    #[inline]
    pub const fn as_binding(self) -> BindingId {
        BindingId::symbol(self.0)
    }
}

/// The kind of a declared symbol.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// A local variable (`local x`, `const x`, or a function declaration name).
    Local,
    /// A function parameter (including the implicit `self` of method functions).
    Parameter,
    /// A loop variable introduced by a generic or numeric `for`.
    Iteration,
    /// An implicit global variable (an identifier that does not resolve to a local).
    Global,
}

/// A symbol declared in the program.
#[derive(Clone, Debug)]
pub struct Symbol {
    /// The identifier of this symbol.
    pub id: SymbolId,
    /// The source name of the symbol.
    pub name: String,
    /// The kind of declaration that introduced this symbol.
    pub kind: SymbolKind,
    /// The scope in which this symbol was declared.
    pub declaring_scope: ScopeId,
    /// Function scopes that capture this symbol as an upvalue. The direct use is
    /// always recorded first, so the first entry is the function that references it.
    pub capturing_scopes: Vec<ScopeId>,
    /// Number of read accesses to this symbol.
    pub read_count: usize,
    /// Number of write accesses to this symbol (including its declaration).
    pub write_count: usize,
    /// When the declaration initializes this symbol directly from another
    /// symbol (for example `local math = math`), the symbol that the
    /// initializer resolves to. Declarations with any other expression (or no
    /// value) leave this `None`.
    pub initializer: Option<SymbolId>,
}

impl Symbol {
    /// Returns whether this symbol is captured by any nested function.
    #[inline]
    pub fn is_captured(&self) -> bool {
        !self.capturing_scopes.is_empty()
    }

    /// Returns whether this symbol is a lexical variable (not an implicit global).
    #[inline]
    pub fn is_local(&self) -> bool {
        self.kind != SymbolKind::Global
    }
}

/// How a symbol is accessed at a program point.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// The symbol is read.
    Read,
    /// The symbol is assigned a new value.
    Write,
    /// The symbol is introduced (declaration or initialization).
    Declaration,
}

/// A single access to a symbol recorded during semantic analysis.
#[derive(Clone, Debug)]
pub struct SymbolUse {
    /// The accessed symbol.
    pub symbol: SymbolId,
    /// The kind of access.
    pub kind: AccessKind,
    /// The scope in which the access occurs.
    pub scope: ScopeId,
    /// The source position of the accessed identifier, when token data is available.
    pub token: Option<TokenKey>,
}

/// A stable key identifying a token by its source position.
///
/// Token keys are only available when the AST was produced with
/// [`Parser::preserve_tokens`](crate::Parser::preserve_tokens).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenKey {
    /// The 1-based line number of the token.
    pub line: usize,
    /// The byte offset at which the token starts.
    pub start: usize,
}

impl TokenKey {
    /// Builds a token key from a token, if it has source position information.
    pub fn of(token: &Token) -> Option<Self> {
        Some(Self {
            line: token.get_line_number()?,
            start: token.get_start()?,
        })
    }
}
