//! Structural identities attached to AST nodes by semantic analysis.
//!
//! The semantic analyzer records the meaning of an AST in the AST itself:
//! every identifier occurrence is stamped with the [`BindingId`] it resolves
//! to, and every block is stamped with a unique [`BlockIdentity`]. The
//! identities are metadata: they never take part in structural equality or in
//! the debug representation of a node, so parsed and analyzed trees compare
//! equal and snapshots stay stable.
//!
//! Storing identities on the nodes (instead of looking them up by source token)
//! makes semantic information survive cloning and synthesis: transforming a
//! subtree copies the bindings it already had, and a pass that re-analyzes the
//! tree refreshes every identity in place.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

/// Identifies the semantic entity bound to an identifier occurrence.
///
/// A binding refers either to a symbol (a local, a parameter, an iteration
/// variable or an implicit global) or to a goto label. The two domains are
/// tagged so a label can never be mistaken for a symbol.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingId(u32);

impl BindingId {
    const LABEL_TAG: u32 = 1 << 31;

    /// Creates a binding to the symbol at `index`.
    ///
    /// # Panics
    ///
    /// Panics when `index` does not fit in the symbol domain. Real programs
    /// cannot exhaust the domain (2^31 symbols).
    pub const fn symbol(index: u32) -> Self {
        assert!(
            index < Self::LABEL_TAG,
            "symbol index exceeds the binding domain"
        );
        Self(index)
    }

    /// Creates a binding to the label at `index`.
    ///
    /// # Panics
    ///
    /// Panics when `index` does not fit in the label domain.
    pub const fn label(index: u32) -> Self {
        assert!(
            index < Self::LABEL_TAG,
            "label index exceeds the binding domain"
        );
        Self(index | Self::LABEL_TAG)
    }

    /// Returns whether this binding refers to a label.
    #[inline]
    pub const fn is_label(self) -> bool {
        self.0 & Self::LABEL_TAG != 0
    }

    /// Returns the symbol or label index of this binding.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0 & !Self::LABEL_TAG
    }

    /// Rebuilds a binding from its raw representation.
    #[inline]
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw representation of this binding.
    #[inline]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for BindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_label() {
            write!(f, "Label({})", self.index())
        } else {
            write!(f, "Symbol({})", self.index())
        }
    }
}

/// Identifies a [`Block`](super::Block) node structurally.
///
/// Identities are assigned by semantic analysis and are unique within one
/// analysis of one tree. They let the control flow graph point back at the AST
/// statements it was built from without relying on source tokens.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockIdentity(pub u32);

impl BlockIdentity {
    /// Returns this identity as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A cell holding an optional structural identity.
///
/// The cell is written through shared references while semantic analysis walks
/// the tree, and stays thread safe so the AST keeps its `Send + Sync`
/// guarantees.
#[derive(Debug)]
pub(crate) struct IdentityCell(AtomicU32);

impl IdentityCell {
    const UNBOUND: u32 = u32::MAX;

    /// Creates an empty identity cell.
    pub(crate) const fn empty() -> Self {
        Self(AtomicU32::new(Self::UNBOUND))
    }

    /// Returns the stored identity, if any.
    #[inline]
    pub(crate) fn get(&self) -> Option<u32> {
        match self.0.load(Ordering::Relaxed) {
            Self::UNBOUND => None,
            value => Some(value),
        }
    }

    /// Stores an identity.
    #[inline]
    pub(crate) fn set(&self, value: u32) {
        self.0.store(value, Ordering::Relaxed);
    }

    /// Removes the stored identity.
    #[inline]
    pub(crate) fn clear(&self) {
        self.0.store(Self::UNBOUND, Ordering::Relaxed);
    }
}

impl Clone for IdentityCell {
    fn clone(&self) -> Self {
        Self(AtomicU32::new(self.0.load(Ordering::Relaxed)))
    }
}

impl Default for IdentityCell {
    fn default() -> Self {
        Self::empty()
    }
}
