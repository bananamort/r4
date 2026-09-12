use super::scope::ScopeId;

pub use crate::nodes::TypeId;

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
