use super::scope::ScopeId;
use super::symbol::{SymbolId, TokenKey};

/// Identifies a label in the [`SemanticModel`](super::SemanticModel).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LabelId(pub u32);

impl LabelId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A `::label::` declaration.
#[derive(Clone, Debug)]
pub struct GotoLabel {
    /// The identifier of this label.
    pub id: LabelId,
    /// The name of the label.
    pub name: String,
    /// The scope in which the label is declared.
    pub scope: ScopeId,
    /// The function (or chunk) scope containing the label.
    pub function_scope: ScopeId,
    /// Whether the label is the last non-void statement of its block. Such labels
    /// are considered outside the scope of the local variables declared in the
    /// block (Lua 5.2+ rule), which allows `goto continue` idioms.
    pub is_block_end: bool,
    /// The source position of the label name, when token data is available.
    pub token: Option<TokenKey>,
    /// The symbols visible at the label (adjusted by `is_block_end`).
    pub visible_symbols: Vec<SymbolId>,
}

impl GotoLabel {
    /// Returns whether this label can be targeted from a goto that is inside the
    /// scope of any of the given symbols.
    pub fn is_visible_from(&self, visible_symbols: &[SymbolId]) -> bool {
        self.visible_symbols
            .iter()
            .all(|symbol| visible_symbols.contains(symbol))
    }
}

/// A `goto label` occurrence.
#[derive(Clone, Debug)]
pub struct GotoSite {
    /// The targeted label name.
    pub name: String,
    /// The scope in which the goto appears.
    pub scope: ScopeId,
    /// The function (or chunk) scope containing the goto.
    pub function_scope: ScopeId,
    /// The source position of the goto label name, when token data is available.
    pub token: Option<TokenKey>,
    /// The symbols visible at the goto.
    pub visible_symbols: Vec<SymbolId>,
}

/// The resolved binding between a goto occurrence and a label.
#[derive(Clone, Debug)]
pub struct GotoBinding {
    /// The label targeted by the goto.
    pub label: LabelId,
    /// The goto occurrence.
    pub site: GotoSite,
}
