use std::collections::HashMap;

use crate::nodes::Identifier;

use super::label::{GotoBinding, GotoLabel, LabelId};
use super::scope::{Scope, ScopeId, ScopeKind};
use super::symbol::{Symbol, SymbolId, SymbolKind, SymbolUse};
use super::type_alias::{TypeId, TypeSymbol};

/// The semantic entity that a bindable identifier occurrence refers to.
///
/// Returned by [`SemanticModel::binding_of`], which resolves value, label and
/// type occurrences uniformly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// A runtime symbol (local, parameter, iteration variable or global).
    Symbol(SymbolId),
    /// A goto label.
    Label(LabelId),
    /// A type alias.
    Type(TypeId),
}

impl Binding {
    /// Returns the runtime symbol, if this binding refers to one.
    #[inline]
    pub const fn as_symbol(self) -> Option<SymbolId> {
        match self {
            Binding::Symbol(symbol) => Some(symbol),
            _ => None,
        }
    }

    /// Returns the label, if this binding refers to one.
    #[inline]
    pub const fn as_label(self) -> Option<LabelId> {
        match self {
            Binding::Label(label) => Some(label),
            _ => None,
        }
    }

    /// Returns the type alias, if this binding refers to one.
    #[inline]
    pub const fn as_type(self) -> Option<TypeId> {
        match self {
            Binding::Type(r#type) => Some(r#type),
            _ => None,
        }
    }
}

/// The result of a semantic analysis pass over a parsed [`Block`](crate::nodes::Block).
///
/// The model owns flat arenas for symbols, scopes and labels, all addressed by
/// compact `u32` identifiers. All query methods are `O(1)` (or `O(depth)` for
/// name resolution) and never guess: an unresolved identifier returns `None`
/// / an error instead of silently falling back to a default.
#[derive(Clone, Debug)]
pub struct SemanticModel {
    /// All declared symbols, indexed by [`SymbolId`].
    pub symbols: Vec<Symbol>,
    /// All lexical scopes, indexed by [`ScopeId`].
    pub scopes: Vec<Scope>,
    /// All goto labels, in source order, indexed by [`LabelId`].
    pub labels: Vec<GotoLabel>,
    /// All resolved goto occurrences, in source order.
    pub goto_bindings: Vec<GotoBinding>,
    /// All declared type aliases, indexed by [`TypeId`].
    pub types: Vec<TypeSymbol>,
    root_scope: ScopeId,
    chunk_scope: ScopeId,
    global_symbols: HashMap<String, SymbolId>,
    uses: Vec<Vec<SymbolUse>>,
}

impl SemanticModel {
    /// Builds a model from the arenas gathered by the analyzer. The arguments
    /// mirror the model fields exactly.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        symbols: Vec<Symbol>,
        scopes: Vec<Scope>,
        labels: Vec<GotoLabel>,
        goto_bindings: Vec<GotoBinding>,
        types: Vec<TypeSymbol>,
        root_scope: ScopeId,
        chunk_scope: ScopeId,
        global_symbols: HashMap<String, SymbolId>,
        uses: Vec<Vec<SymbolUse>>,
    ) -> Self {
        Self {
            symbols,
            scopes,
            labels,
            goto_bindings,
            types,
            root_scope,
            chunk_scope,
            global_symbols,
            uses,
        }
    }

    /// Returns the global environment scope.
    #[inline]
    pub fn root_scope(&self) -> ScopeId {
        self.root_scope
    }

    /// Returns the scope of the main chunk (child of the global scope).
    #[inline]
    pub fn chunk_scope(&self) -> ScopeId {
        self.chunk_scope
    }

    /// Returns the symbol with the given identifier.
    #[inline]
    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.index()]
    }

    /// Returns the scope with the given identifier.
    #[inline]
    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }

    /// Returns the label with the given identifier.
    #[inline]
    pub fn label(&self, id: LabelId) -> &GotoLabel {
        &self.labels[id.index()]
    }

    /// Returns the type symbol with the given identifier.
    #[inline]
    pub fn type_symbol(&self, id: TypeId) -> &TypeSymbol {
        &self.types[id.index()]
    }

    /// Returns the implicit global symbol with the given name, if one was created.
    #[inline]
    pub fn global_symbol(&self, name: &str) -> Option<SymbolId> {
        self.global_symbols.get(name).copied()
    }

    /// Returns the symbol bound to the given identifier.
    ///
    /// The binding is read from the identifier itself (see
    /// [`Identifier::get_binding`]), so no token data is required and
    /// same-scope shadowing resolves exactly: every occurrence is bound to the
    /// declaration that was visible when it was analyzed.
    ///
    /// A binding that points outside the symbol arena, at a label, or at a
    /// symbol with a different name is rejected, which catches identifiers that
    /// were never analyzed or that belong to a stale model.
    pub fn resolve_identifier(&self, identifier: &Identifier) -> Option<SymbolId> {
        let binding = identifier.get_binding()?;
        if !binding.is_symbol() {
            return None;
        }
        let id = SymbolId(binding.index());
        let symbol = self.symbols.get(id.index())?;
        (symbol.id == id && symbol.name == *identifier.get_name()).then_some(id)
    }

    /// Returns the label bound to the given label identifier.
    ///
    /// Like [`Self::resolve_identifier`] this uses the structural binding
    /// recorded on the identifier, validates it against the label arena and
    /// never falls back to source tokens.
    pub fn resolve_label(&self, identifier: &Identifier) -> Option<LabelId> {
        let binding = identifier.get_binding()?;
        if !binding.is_label() {
            return None;
        }
        let id = LabelId(binding.index());
        let label = self.labels.get(id.index())?;
        (label.id == id && label.name == *identifier.get_name()).then_some(id)
    }

    /// Returns the type alias bound to the given type name identifier.
    ///
    /// Like [`Self::resolve_identifier`] this reads and validates the
    /// structural binding, so same-scope type redeclarations resolve exactly
    /// and no token data is required.
    pub fn resolve_type(&self, identifier: &Identifier) -> Option<TypeId> {
        let binding = identifier.get_binding()?;
        if !binding.is_type() {
            return None;
        }
        let id = TypeId(binding.index());
        let r#type = self.types.get(id.index())?;
        (r#type.id == id && r#type.name == *identifier.get_name()).then_some(id)
    }

    /// Returns the value symbol bound to an identifier occurrence.
    ///
    /// This is a descriptive alias of [`Self::resolve_identifier`] for callers
    /// that query symbols uniformly.
    #[inline]
    pub fn symbol_of(&self, identifier: &Identifier) -> Option<SymbolId> {
        self.resolve_identifier(identifier)
    }

    /// Returns the label bound to a goto or label name occurrence.
    #[inline]
    pub fn label_of(&self, identifier: &Identifier) -> Option<LabelId> {
        self.resolve_label(identifier)
    }

    /// Returns the type alias bound to a type name occurrence.
    #[inline]
    pub fn type_of(&self, identifier: &Identifier) -> Option<TypeId> {
        self.resolve_type(identifier)
    }

    /// Resolves any bindable identifier occurrence to the entity it binds to.
    ///
    /// The binding tag recorded on the identifier selects the domain, so the
    /// caller does not need to know whether a name refers to a value, a label
    /// or a type. Stale or foreign bindings return `None`.
    pub fn binding_of(&self, identifier: &Identifier) -> Option<Binding> {
        match identifier.get_binding()? {
            binding if binding.is_label() => self.resolve_label(identifier).map(Binding::Label),
            binding if binding.is_type() => self.resolve_type(identifier).map(Binding::Type),
            _ => self.resolve_identifier(identifier).map(Binding::Symbol),
        }
    }

    /// Resolves a type name lexically starting from `scope`.
    ///
    /// This is the name-level counterpart of [`Self::resolve_type`]: it
    /// returns the most recent type declaration with this name in the nearest
    /// scope that declares it.
    pub fn resolve_type_in_scope(&self, scope: ScopeId, name: &str) -> Option<TypeId> {
        let mut current = Some(scope);
        while let Some(id) = current {
            let scope = &self.scopes[id.index()];
            if let Some(r#type) = scope
                .types
                .iter()
                .rev()
                .find(|&&r#type| self.types[r#type.index()].name == name)
            {
                return Some(*r#type);
            }
            current = scope.parent;
        }
        None
    }

    /// Resolves a name lexically starting from `scope`, without recording any use.
    ///
    /// This is a name-level convenience query: it returns the most recent
    /// declaration of `name` in the nearest scope that declares it. Because
    /// Lua allows redeclaring a name inside the same scope, a name alone does
    /// not identify an occurrence; use [`Self::resolve_identifier`] for exact,
    /// occurrence-level resolution, or [`Self::resolve_in_scope_at`] to query a
    /// name at a specific declaration position.
    pub fn resolve_in_scope(&self, scope: ScopeId, name: &str) -> Option<SymbolId> {
        let mut current = Some(scope);
        while let Some(id) = current {
            let scope = &self.scopes[id.index()];
            if let Some(symbol) = scope
                .symbols
                .iter()
                .rev()
                .find(|&&symbol| self.symbols[symbol.index()].name == name)
            {
                return Some(*symbol);
            }
            current = scope.parent;
        }
        None
    }

    /// Resolves `name` at a declaration position inside `scope`.
    ///
    /// `declared` maps scopes of the active chain (starting at `scope`) to the
    /// number of declarations that precede the query point. Within a scope only
    /// the first `declared` symbols are considered, so a use between two
    /// declarations of the same name resolves to the first one; scopes missing
    /// from `declared` are treated as fully declared.
    ///
    /// # Panics
    ///
    /// Panics when a declaration count exceeds the number of symbols the scope
    /// actually contains: such a count cannot describe this model.
    pub fn resolve_in_scope_at(
        &self,
        scope: ScopeId,
        name: &str,
        declared: &[(ScopeId, usize)],
    ) -> Option<SymbolId> {
        let mut current = Some(scope);
        while let Some(id) = current {
            let scope = &self.scopes[id.index()];
            let limit = declared
                .iter()
                .find_map(|(declared_scope, count)| (*declared_scope == id).then_some(*count))
                .unwrap_or(scope.symbols.len());
            assert!(
                limit <= scope.symbols.len(),
                "cannot resolve with more declarations than the scope contains"
            );
            if let Some(symbol) = scope.symbols[..limit]
                .iter()
                .rev()
                .find(|&&symbol| self.symbols[symbol.index()].name == name)
            {
                return Some(*symbol);
            }
            current = scope.parent;
        }
        None
    }

    /// Returns the function (or chunk) scope that lexically contains `scope`.
    pub fn function_scope_of(&self, scope: ScopeId) -> Option<ScopeId> {
        let mut current = Some(scope);
        while let Some(id) = current {
            let scope = &self.scopes[id.index()];
            match scope.kind {
                ScopeKind::Global => return None,
                ScopeKind::File | ScopeKind::Function => return Some(id),
                ScopeKind::Block => current = scope.parent,
            }
        }
        None
    }

    /// Returns all recorded accesses to a symbol, in source order.
    #[inline]
    pub fn uses_of(&self, symbol: SymbolId) -> &[SymbolUse] {
        &self.uses[symbol.index()]
    }

    /// Returns whether a symbol is never read.
    ///
    /// An unread local can be removed when its initializer has no side effects
    /// (see [`Evaluator::has_side_effects`](crate::process::Evaluator::has_side_effects)).
    #[inline]
    pub fn is_unused(&self, symbol: SymbolId) -> bool {
        self.symbols[symbol.index()].read_count == 0
    }

    /// Returns whether a symbol is only written at its declaration, so its
    /// value never changes after initialization.
    #[inline]
    pub fn is_immutable(&self, symbol: SymbolId) -> bool {
        self.symbols[symbol.index()].write_count <= 1
    }

    /// If `symbol` is a read-only local initialized from a global symbol
    /// (for example `local math = math`), returns that global symbol.
    ///
    /// Rules that rewrite standard-library calls can use this to optimize
    /// through an immutable alias while remaining correct when the name is
    /// shadowed. Returns `None` for globals themselves, mutable locals, and
    /// locals initialized from anything other than a global symbol.
    pub fn immutable_global_alias(&self, symbol: SymbolId) -> Option<SymbolId> {
        let local = &self.symbols[symbol.index()];
        if local.kind == SymbolKind::Global || !self.is_immutable(symbol) {
            return None;
        }
        let target = local.initializer?;
        let global = &self.symbols[target.index()];
        (global.kind == SymbolKind::Global).then_some(target)
    }

    /// Returns whether every non-global symbol with this name is an immutable
    /// alias of the same-named global symbol.
    ///
    /// When this holds, a reference to `name` can be treated as a reference to
    /// the global even if a local with that name exists: the local is only ever
    /// initialized from the global and never written again. Returns `true` when
    /// no non-global symbol has the name.
    ///
    /// This is what lets standard-library rewrites optimize through
    /// `local math = math` instead of conservatively refusing or aliasing.
    pub fn name_is_transparent_global(&self, name: &str) -> bool {
        self.symbols
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Global && symbol.name == name)
            .all(|symbol| self.immutable_global_alias(symbol.id).is_some())
    }

    /// Iterates over all symbols captured as upvalues by nested functions.
    pub fn captured_symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter().filter(|symbol| symbol.is_captured())
    }

    /// Iterates over the labels declared inside a function (or chunk) scope,
    /// in source order.
    pub fn labels_in_function(&self, function: ScopeId) -> impl Iterator<Item = &GotoLabel> + '_ {
        self.labels
            .iter()
            .filter(move |label| label.function_scope == function)
    }

    /// Iterates over the goto bindings inside a function (or chunk) scope,
    /// in source order.
    pub fn goto_bindings_in_function(
        &self,
        function: ScopeId,
    ) -> impl Iterator<Item = &GotoBinding> + '_ {
        self.goto_bindings
            .iter()
            .filter(move |binding| binding.site.function_scope == function)
    }
}
