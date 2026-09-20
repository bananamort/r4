use super::label::LabelId;
use super::symbol::SymbolId;
use super::type_alias::TypeId;

/// Identifies a scope in the [`SemanticModel`](super::SemanticModel).
///
/// The identifier is an index into the model's flat `scopes` arena.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub u32);

impl ScopeId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The kind of a scope.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// The implicit global environment scope. Contains only global symbols.
    Global,
    /// The scope of a chunk (main function). Locals declared at the top level
    /// belong to this scope.
    File,
    /// The scope of a function body (parameters and body locals share it).
    Function,
    /// The scope of a plain block (`do ... end`, if branches, loop bodies).
    Block,
}

impl ScopeKind {
    /// Returns whether this scope establishes a function boundary for upvalue
    /// capture analysis.
    #[inline]
    pub fn is_function_boundary(self) -> bool {
        matches!(self, ScopeKind::File | ScopeKind::Function)
    }

    /// Returns a stable human-readable name for this scope kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            ScopeKind::Global => "global",
            ScopeKind::File => "file",
            ScopeKind::Function => "function",
            ScopeKind::Block => "block",
        }
    }
}

/// A lexical scope.
#[derive(Clone, Debug)]
pub struct Scope {
    /// The identifier of this scope.
    pub id: ScopeId,
    /// The enclosing scope, if any.
    pub parent: Option<ScopeId>,
    /// The scopes directly nested in this one.
    pub children: Vec<ScopeId>,
    /// The kind of this scope.
    pub kind: ScopeKind,
    /// Symbols declared directly in this scope, in declaration order.
    pub symbols: Vec<SymbolId>,
    /// Labels declared directly in this scope, in declaration order.
    pub labels: Vec<LabelId>,
    /// Type aliases declared directly in this scope, in declaration order.
    pub types: Vec<TypeId>,
}

impl Scope {
    /// Returns whether this scope is the global environment scope.
    #[inline]
    pub fn is_global(&self) -> bool {
        self.kind == ScopeKind::Global
    }
}
