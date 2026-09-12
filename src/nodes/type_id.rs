use super::BindingId;

/// Identifies a type declaration in a semantic analysis.
///
/// The identifier is an index into a [`SemanticModel`](crate::semantic::SemanticModel)'s
/// flat `types` arena. It lives beside [`BindingId`] so that
/// [`BindingId::type_alias`] can take the typed id directly instead of a raw
/// index.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

impl TypeId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the structural binding corresponding to this type declaration.
    #[inline]
    pub const fn as_binding(self) -> BindingId {
        BindingId::type_alias(self)
    }
}
