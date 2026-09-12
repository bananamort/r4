use crate::nodes::BindingId;

use super::scope::ScopeId;

/// Identifies a type declaration in the [`SemanticModel`](super::SemanticModel).
///
/// The identifier is an index into the model's flat `types` arena.
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
        BindingId::type_alias(self.0)
    }
}

/// A type alias declared by `type Name = ...` or `export type Name = ...`.
#[derive(Clone, Debug)]
pub struct TypeSymbol {
    /// The identifier of this type declaration.
    pub id: TypeId,
    /// The source name of the type declaration.
    pub name: String,
    /// The scope in which this type was declared.
    pub declaring_scope: ScopeId,
    /// Whether the declaration used the `export` modifier.
    pub is_exported: bool,
}
