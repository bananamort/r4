//! Coordinator of the semantic analysis passes.
//!
//! [`SemanticAnalyzer`] is the public entry point. Internally it drives an
//! [`AnalyzerState`], which owns the symbol/scope arenas and the resolution
//! tables. The AST traversal lives in [`super::walker`], and the Lua 5.2+
//! goto/label pass lives in [`super::goto`].

use std::collections::HashMap;

use crate::nodes::{Block, BlockIdentity, Identifier};

use super::error::{SemanticError, SemanticOptions, SemanticResult};
use super::label::{GotoLabel, GotoSite, LabelId};
use super::model::SemanticModel;
use super::scope::{Scope, ScopeId, ScopeKind};
use super::symbol::{AccessKind, Symbol, SymbolId, SymbolKind, SymbolUse, TokenKey};
use super::type_alias::{TypeId, TypeSymbol};

/// Runs the semantic analysis passes and builds a [`SemanticModel`].
///
/// The analyzer walks the AST once to build the scope tree, declare symbols and
/// record reads/writes (pass 1), then resolves every goto against the collected
/// labels following the Lua 5.2 scoping rules (pass 2).
#[derive(Clone, Debug, Default)]
pub struct SemanticAnalyzer {
    options: SemanticOptions,
}

impl SemanticAnalyzer {
    /// Creates a new analyzer with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new analyzer with the given options.
    pub fn with_options(options: SemanticOptions) -> Self {
        Self { options }
    }

    /// Enables strict global checking: undeclared names produce an error instead
    /// of being treated as implicit globals.
    pub fn strict_globals(mut self) -> Self {
        self.options.strict_globals = true;
        self
    }

    /// Enables strict type checking: a type name that resolves neither to a
    /// type declaration, a generic type parameter, nor a built-in Luau type
    /// produces an error instead of being left unresolved.
    pub fn strict_types(mut self) -> Self {
        self.options.strict_types = true;
        self
    }

    /// Returns the options used by this analyzer.
    pub fn options(&self) -> &SemanticOptions {
        &self.options
    }

    /// Analyzes a block (typically the root chunk of a file) and produces a
    /// [`SemanticModel`].
    pub fn analyze(&self, block: &Block) -> SemanticResult<SemanticModel> {
        AnalyzerState::new(self.options.clone()).analyze(block)
    }
}

/// Mutable state of a single semantic analysis run.
///
/// The fields are visible inside the `semantic` module so that the traversal
/// ([`super::walker`]) and goto resolution ([`super::goto`]) extensions can
/// operate on the same state.
pub(super) struct AnalyzerState {
    pub(super) options: SemanticOptions,
    pub(super) symbols: Vec<Symbol>,
    pub(super) scopes: Vec<Scope>,
    pub(super) labels: Vec<GotoLabel>,
    pub(super) goto_sites: Vec<GotoSite>,
    pub(super) scope_symbols: Vec<HashMap<String, SymbolId>>,
    pub(super) scope_labels: Vec<HashMap<String, LabelId>>,
    pub(super) scope_stack: Vec<ScopeId>,
    pub(super) function_stack: Vec<ScopeId>,
    pub(super) global_symbols: HashMap<String, SymbolId>,
    pub(super) uses: Vec<Vec<SymbolUse>>,
    pub(super) types: Vec<TypeSymbol>,
    pub(super) scope_type_declarations: Vec<Vec<(String, TypeId)>>,
    pub(super) type_declaration_cursors: Vec<usize>,
    pub(super) type_parameter_scopes: Vec<Vec<String>>,
    pub(super) next_block_identity: u32,
    pub(super) root_scope: ScopeId,
    pub(super) chunk_scope: ScopeId,
}

impl AnalyzerState {
    pub(super) fn new(options: SemanticOptions) -> Self {
        Self {
            options,
            symbols: Vec::new(),
            scopes: Vec::new(),
            labels: Vec::new(),
            goto_sites: Vec::new(),
            scope_symbols: Vec::new(),
            scope_labels: Vec::new(),
            scope_stack: Vec::new(),
            function_stack: Vec::new(),
            global_symbols: HashMap::new(),
            uses: Vec::new(),
            types: Vec::new(),
            scope_type_declarations: Vec::new(),
            type_declaration_cursors: Vec::new(),
            type_parameter_scopes: Vec::new(),
            next_block_identity: 0,
            root_scope: ScopeId(0),
            chunk_scope: ScopeId(0),
        }
    }

    pub(super) fn analyze(mut self, block: &Block) -> SemanticResult<SemanticModel> {
        self.root_scope = self.push_scope(ScopeKind::Global);
        self.chunk_scope = self.push_scope(ScopeKind::File);
        self.function_stack.push(self.chunk_scope);

        self.walk_block_statements(block)?;

        let goto_bindings = self.resolve_gotos()?;

        self.function_stack.pop();
        self.pop_scope();

        Ok(SemanticModel::new(
            self.symbols,
            self.scopes,
            self.labels,
            goto_bindings,
            self.types,
            self.root_scope,
            self.chunk_scope,
            self.global_symbols,
            self.uses,
        ))
    }

    // ------------------------------------------------------------------
    // Scope stack management
    // ------------------------------------------------------------------

    pub(super) fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let parent = self.scope_stack.last().copied();
        let id = ScopeId(self.scopes.len() as u32);

        self.scopes.push(Scope {
            id,
            parent,
            children: Vec::new(),
            kind,
            symbols: Vec::new(),
            labels: Vec::new(),
            types: Vec::new(),
        });
        self.scope_symbols.push(HashMap::new());
        self.scope_labels.push(HashMap::new());
        self.scope_type_declarations.push(Vec::new());

        if let Some(parent) = parent {
            self.scopes[parent.index()].children.push(id);
        }

        self.scope_stack.push(id);
        self.type_declaration_cursors.push(0);
        id
    }

    pub(super) fn pop_scope(&mut self) {
        self.scope_stack.pop();
        self.type_declaration_cursors.pop();
    }

    #[inline]
    pub(super) fn current_scope(&self) -> ScopeId {
        *self
            .scope_stack
            .last()
            .expect("a scope is always active while analyzing")
    }

    #[inline]
    pub(super) fn current_function(&self) -> ScopeId {
        *self
            .function_stack
            .last()
            .expect("a function scope is always active while analyzing")
    }

    /// Returns the function (or chunk) scope lexically containing `scope`, or
    /// `None` when the scope belongs to the global environment.
    fn function_of(&self, scope: ScopeId) -> Option<ScopeId> {
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

    // ------------------------------------------------------------------
    // Symbol declaration and resolution
    // ------------------------------------------------------------------

    pub(super) fn declare_symbol(
        &mut self,
        name: &str,
        kind: SymbolKind,
        identifier: Option<&Identifier>,
    ) -> SymbolId {
        let scope = self.current_scope();
        let id = SymbolId(self.symbols.len() as u32);

        self.symbols.push(Symbol {
            id,
            name: name.to_string(),
            kind,
            declaring_scope: scope,
            capturing_scopes: Vec::new(),
            read_count: 0,
            write_count: 0,
            initializer: None,
        });
        self.uses.push(Vec::new());
        self.scopes[scope.index()].symbols.push(id);
        self.scope_symbols[scope.index()].insert(name.to_string(), id);

        if let Some(identifier) = identifier {
            self.bind_identifier(identifier, id);
        }
        self.record_access(id, AccessKind::Declaration, identifier);

        id
    }

    /// Records that `symbol` is initialized from `target` (for example
    /// `local math = math`).
    pub(super) fn set_symbol_initializer(&mut self, symbol: SymbolId, target: SymbolId) {
        self.symbols[symbol.index()].initializer = Some(target);
    }

    fn declare_global(&mut self, name: &str) -> SymbolId {
        let scope = self.root_scope;
        let id = SymbolId(self.symbols.len() as u32);

        self.symbols.push(Symbol {
            id,
            name: name.to_string(),
            kind: SymbolKind::Global,
            declaring_scope: scope,
            capturing_scopes: Vec::new(),
            read_count: 0,
            write_count: 0,
            initializer: None,
        });
        self.uses.push(Vec::new());
        self.scopes[scope.index()].symbols.push(id);
        self.scope_symbols[scope.index()].insert(name.to_string(), id);
        self.global_symbols.insert(name.to_string(), id);

        id
    }

    fn bind_identifier(&mut self, identifier: &Identifier, symbol: SymbolId) {
        identifier.set_binding(symbol.as_binding());
    }

    // ------------------------------------------------------------------
    // Type declaration and resolution
    // ------------------------------------------------------------------

    /// Declares a type alias in the current scope and binds its name.
    pub(super) fn declare_type_symbol(
        &mut self,
        name: &str,
        is_exported: bool,
        identifier: Option<&Identifier>,
    ) -> TypeId {
        let scope = self.current_scope();
        let id = TypeId(self.types.len() as u32);

        self.types.push(TypeSymbol {
            id,
            name: name.to_string(),
            declaring_scope: scope,
            is_exported,
            alias_of: None,
        });
        self.scopes[scope.index()].types.push(id);
        self.scope_type_declarations[scope.index()].push((name.to_string(), id));

        if let Some(identifier) = identifier {
            identifier.set_binding(id.as_binding());
        }

        id
    }

    /// Looks up a type name in the active scope chain without recording a use.
    ///
    /// Resolution is position aware. Within one scope, an occurrence binds to
    /// the most recent declaration that precedes it; if the name is only
    /// declared later in the scope, the occurrence forward-references that
    /// declaration (Luau type aliases are not order sensitive). A name declared
    /// in an inner scope shadows any outer declaration.
    pub(super) fn lookup_type_name(&self, name: &str) -> Option<TypeId> {
        for index in (0..self.scope_stack.len()).rev() {
            let scope_id = self.scope_stack[index];
            let declarations = &self.scope_type_declarations[scope_id.index()];
            let seen = self.type_declaration_cursors[index].min(declarations.len());

            if let Some((_, id)) = declarations[..seen]
                .iter()
                .rev()
                .find(|(declared, _)| declared == name)
            {
                return Some(*id);
            }

            if let Some((_, id)) = declarations[seen..]
                .iter()
                .find(|(declared, _)| declared == name)
            {
                return Some(*id);
            }
        }
        None
    }

    /// Pushes a frame of generic type parameter names.
    pub(super) fn push_type_parameters(&mut self, names: Vec<String>) {
        self.type_parameter_scopes.push(names);
    }

    /// Pops the innermost generic type parameter frame.
    pub(super) fn pop_type_parameters(&mut self) {
        self.type_parameter_scopes.pop();
    }

    /// Returns whether `name` is an active generic type parameter.
    pub(super) fn is_generic_type_parameter(&self, name: &str) -> bool {
        self.type_parameter_scopes
            .iter()
            .rev()
            .any(|scope| scope.iter().any(|parameter| parameter == name))
    }

    /// Resolves a type name occurrence and records its binding.
    ///
    /// Returns `Ok(())` after binding when the name resolves to a declared
    /// type alias. Names that are not declared are accepted when they are
    /// generic type parameters or built-in Luau types; under
    /// [`SemanticOptions::strict_types`] anything else is an
    /// [`SemanticErrorKind::UnresolvedType`] error. No other fallback exists.
    pub(super) fn resolve_type_occurrence(
        &mut self,
        identifier: &Identifier,
    ) -> SemanticResult<()> {
        let name = identifier.get_name();

        if let Some(id) = self.lookup_type_name(name) {
            identifier.set_binding(id.as_binding());
            return Ok(());
        }

        self.check_unresolved_type(identifier)
    }

    /// Validates a type name occurrence that has no structural binding.
    ///
    /// Used for occurrences without a `TypeId` (generic type packs).
    pub(super) fn check_type_occurrence(&self, identifier: &Identifier) -> SemanticResult<()> {
        if self.lookup_type_name(identifier.get_name()).is_some()
            || self.is_generic_type_parameter(identifier.get_name())
            || is_builtin_type_name(identifier.get_name())
        {
            return Ok(());
        }

        if self.options.strict_types {
            return Err(
                SemanticError::unresolved_type(identifier.get_name(), line_of(identifier))
                    .with_span(span_of(identifier)),
            );
        }

        Ok(())
    }

    fn check_unresolved_type(&self, identifier: &Identifier) -> SemanticResult<()> {
        if self.is_generic_type_parameter(identifier.get_name())
            || is_builtin_type_name(identifier.get_name())
        {
            return Ok(());
        }

        if self.options.strict_types {
            return Err(
                SemanticError::unresolved_type(identifier.get_name(), line_of(identifier))
                    .with_span(span_of(identifier)),
            );
        }

        Ok(())
    }

    /// Assigns a fresh structural identity to `block`.
    ///
    /// Every block the analyzer reaches gets a unique identity so passes can
    /// correlate statements between the AST and the control flow graph without
    /// relying on source tokens. Re-analyzing a tree overwrites the identities
    /// with a fresh deterministic sequence.
    pub(super) fn assign_block_identity(&mut self, block: &Block) {
        let identity = BlockIdentity(self.next_block_identity);
        self.next_block_identity = self
            .next_block_identity
            .checked_add(1)
            .expect("the block identity arena is exhausted");
        block.set_identity(identity);
    }

    pub(super) fn record_access(
        &mut self,
        symbol: SymbolId,
        kind: AccessKind,
        identifier: Option<&Identifier>,
    ) {
        let token = identifier.and_then(token_key);
        let scope = self.current_scope();
        match kind {
            AccessKind::Read => self.symbols[symbol.index()].read_count += 1,
            AccessKind::Write | AccessKind::Declaration => {
                self.symbols[symbol.index()].write_count += 1
            }
        }
        self.uses[symbol.index()].push(SymbolUse {
            symbol,
            kind,
            scope,
            token,
        });
    }

    /// Marks the symbol as captured by every function boundary crossed between
    /// its declaration and the current function.
    fn mark_capture(&mut self, symbol: SymbolId) {
        let declaring_scope = self.symbols[symbol.index()].declaring_scope;
        if self.symbols[symbol.index()].kind == SymbolKind::Global {
            return;
        }
        let declaring_function = match self.function_of(declaring_scope) {
            Some(function) => function,
            None => return,
        };
        let current_function = self.current_function();
        if declaring_function == current_function {
            return;
        }

        for &function in self.function_stack.iter().rev() {
            if function == declaring_function {
                break;
            }
            let capturing = &mut self.symbols[symbol.index()].capturing_scopes;
            if !capturing.contains(&function) {
                capturing.push(function);
            }
        }
    }

    pub(super) fn resolve_name(
        &mut self,
        name: &str,
        identifier: Option<&Identifier>,
    ) -> SemanticResult<SymbolId> {
        for index in (0..self.scope_stack.len()).rev() {
            let scope_id = self.scope_stack[index];
            if let Some(&symbol) = self.scope_symbols[scope_id.index()].get(name) {
                self.mark_capture(symbol);
                if let Some(identifier) = identifier {
                    self.bind_identifier(identifier, symbol);
                }
                return Ok(symbol);
            }
        }

        if self.options.strict_globals {
            return Err(
                SemanticError::undeclared(name, identifier.and_then(line_of))
                    .with_span(identifier.and_then(span_of)),
            );
        }

        let symbol = self.declare_global(name);
        if let Some(identifier) = identifier {
            self.bind_identifier(identifier, symbol);
        }
        Ok(symbol)
    }

    pub(super) fn read_identifier(&mut self, identifier: &Identifier) -> SemanticResult<SymbolId> {
        let symbol = self.resolve_name(identifier.get_name(), Some(identifier))?;
        self.record_access(symbol, AccessKind::Read, Some(identifier));
        Ok(symbol)
    }

    pub(super) fn write_identifier(&mut self, identifier: &Identifier) -> SemanticResult<SymbolId> {
        let symbol = self.resolve_name(identifier.get_name(), Some(identifier))?;
        self.record_access(symbol, AccessKind::Write, Some(identifier));
        Ok(symbol)
    }
}

/// Returns the stable source key of an identifier, when token data is available.
pub(super) fn token_key(identifier: &Identifier) -> Option<TokenKey> {
    identifier.get_token().and_then(TokenKey::of)
}

/// Returns the source line of an identifier, when token data is available.
pub(super) fn line_of(identifier: &Identifier) -> Option<usize> {
    identifier
        .get_token()
        .and_then(|token| token.get_line_number())
}

/// Returns the byte range of an identifier, when token data is available.
pub(super) fn span_of(identifier: &Identifier) -> Option<crate::diagnostic::TextSpan> {
    identifier.get_token().and_then(|token| token.get_span())
}

/// Returns whether `name` is a built-in Luau type.
///
/// These names are always resolvable without a declaration; anything else must
/// be declared, be a generic type parameter, or be reported under strict
/// typing.
pub fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "any"
            | "boolean"
            | "buffer"
            | "never"
            | "number"
            | "self"
            | "string"
            | "thread"
            | "unknown"
            | "vector"
    )
}
