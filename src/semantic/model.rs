use std::collections::HashMap;

use crate::nodes::{Identifier, Token};

use super::label::{GotoBinding, GotoLabel, LabelId};
use super::scope::{Scope, ScopeId, ScopeKind};
use super::symbol::{Symbol, SymbolId, SymbolUse, TokenKey};

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
    root_scope: ScopeId,
    chunk_scope: ScopeId,
    identifier_bindings: HashMap<TokenKey, SymbolId>,
    label_tokens: HashMap<TokenKey, LabelId>,
    goto_tokens: HashMap<TokenKey, usize>,
    global_symbols: HashMap<String, SymbolId>,
    uses: Vec<Vec<SymbolUse>>,
}

impl SemanticModel {
    /// Builds a model from the arenas and lookup tables gathered by the
    /// analyzer. The arguments mirror the model fields exactly.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        symbols: Vec<Symbol>,
        scopes: Vec<Scope>,
        labels: Vec<GotoLabel>,
        goto_bindings: Vec<GotoBinding>,
        root_scope: ScopeId,
        chunk_scope: ScopeId,
        identifier_bindings: HashMap<TokenKey, SymbolId>,
        label_tokens: HashMap<TokenKey, LabelId>,
        goto_tokens: HashMap<TokenKey, usize>,
        global_symbols: HashMap<String, SymbolId>,
        uses: Vec<Vec<SymbolUse>>,
    ) -> Self {
        Self {
            symbols,
            scopes,
            labels,
            goto_bindings,
            root_scope,
            chunk_scope,
            identifier_bindings,
            label_tokens,
            goto_tokens,
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

    /// Returns the implicit global symbol with the given name, if one was created.
    #[inline]
    pub fn global_symbol(&self, name: &str) -> Option<SymbolId> {
        self.global_symbols.get(name).copied()
    }

    /// Returns the symbol bound to an identifier using its source position.
    ///
    /// Returns `None` when the AST does not carry token data or when the
    /// identifier was never analyzed.
    pub fn symbol_at_token(&self, token: &Token) -> Option<SymbolId> {
        TokenKey::of(token).and_then(|key| self.identifier_bindings.get(&key).copied())
    }

    /// Returns the symbol bound to the given identifier.
    pub fn resolve_identifier(&self, identifier: &Identifier) -> Option<SymbolId> {
        identifier
            .get_token()
            .and_then(|token| self.symbol_at_token(token))
    }

    /// Resolves a name lexically starting from `scope`, without recording any use.
    ///
    /// This is useful for queries without token data. It performs a nearest-scope
    /// lookup and never falls back to a different name.
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

    /// Returns the label bound to a label name token, if token data is available.
    pub fn label_at_token(&self, token: &Token) -> Option<LabelId> {
        TokenKey::of(token).and_then(|key| self.label_tokens.get(&key).copied())
    }

    /// Returns the goto binding associated with a goto label name token, if token
    /// data is available.
    pub fn goto_binding_at_token(&self, token: &Token) -> Option<&GotoBinding> {
        TokenKey::of(token)
            .and_then(|key| self.goto_tokens.get(&key).copied())
            .map(|index| &self.goto_bindings[index])
    }
}
