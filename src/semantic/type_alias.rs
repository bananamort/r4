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
    /// The type this alias directly refers to when it was declared as a bare
    /// alias such as `type A = B`.
    ///
    /// Structural aliases (`type A = { x: number }`), generic aliases and
    /// self-referential declarations leave this `None`. Follow the chain with
    /// [`SemanticModel::expand_type_alias`](crate::semantic::SemanticModel::expand_type_alias).
    pub alias_of: Option<TypeId>,
}
