use crate::diagnostic::TextSpan;
use crate::nodes::{BindingId, BlockIdentity};

use super::scope::ScopeId;
use super::symbol::TokenKey;

/// The structural position of a standalone statement in the analyzed AST.
///
/// Label and goto sites use this position instead of source spans, so passes
/// that reason about control flow keep working when the tree was parsed without
/// token data.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatementPosition {
    /// The block that contains the statement.
    pub block: BlockIdentity,
    /// The index of the statement inside that block.
    pub index: u32,
}
/// A compact snapshot of the lexical symbols visible at a program point.
///
/// Instead of listing every visible symbol (which is `O(gotos × locals)` for a
/// goto-heavy file), a cursor records how many symbols are visible in each
/// active scope. Visibility is then a per-scope prefix comparison, so recording
/// and checking are `O(active scopes)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisibleScopes {
    counts: Vec<(ScopeId, usize)>,
}

impl VisibleScopes {
    /// Creates a cursor from `(scope, visible symbol count)` pairs.
    #[inline]
    pub fn new(counts: Vec<(ScopeId, usize)>) -> Self {
        Self { counts }
    }

    /// Returns how many symbols are visible in `scope`.
    pub fn count(&self, scope: ScopeId) -> usize {
        self.counts
            .iter()
            .find(|(candidate, _)| *candidate == scope)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    /// Returns whether every symbol visible in `self` is also visible in
    /// `other`.
    pub fn is_subset_of(&self, other: &VisibleScopes) -> bool {
        self.counts
            .iter()
            .all(|(scope, count)| other.count(*scope) >= *count)
    }

    /// Iterates over the `(scope, visible symbol count)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (ScopeId, usize)> + '_ {
        self.counts.iter().copied()
    }

    /// Returns whether no symbols are visible.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// Identifies a label in the [`SemanticModel`](super::SemanticModel).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LabelId(pub u32);
impl LabelId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the structural binding corresponding to this label.
    #[inline]
    pub const fn as_binding(self) -> BindingId {
        BindingId::label(self.0)
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
    /// The structural position of the label statement.
    pub position: StatementPosition,
    /// Whether the label is the last non-void statement of its block. Such labels
    /// are considered outside the scope of the local variables declared in the
    /// block (Lua 5.2+ rule), which allows `goto continue` idioms.
    pub is_block_end: bool,
    /// The source position of the label name, when token data is available.
    pub token: Option<TokenKey>,
    /// The byte range of the label name, when token data is available.
    pub span: Option<TextSpan>,
    /// The scopes (and visible symbol counts) at the label.
    pub visible_symbols: VisibleScopes,
}

impl GotoLabel {
    /// Returns whether this label can be targeted from a goto with the given
    /// visible scopes.
    pub fn is_visible_from(&self, visible_symbols: &VisibleScopes) -> bool {
        self.visible_symbols.is_subset_of(visible_symbols)
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
    /// The structural position of the goto statement.
    pub position: StatementPosition,
    /// The source position of the goto label name, when token data is available.
    pub token: Option<TokenKey>,
    /// The byte range of the goto label name, when token data is available.
    pub span: Option<TextSpan>,
    /// The scopes (and visible symbol counts) at the goto.
    pub visible_symbols: VisibleScopes,
}

/// The resolved binding between a goto occurrence and a label.
#[derive(Clone, Debug)]
pub struct GotoBinding {
    /// The label targeted by the goto.
    pub label: LabelId,
    /// The goto occurrence.
    pub site: GotoSite,
}
