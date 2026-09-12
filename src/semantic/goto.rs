//! Resolution and validation of Lua 5.2+ `goto`/`::label::` statements.
//!
//! Labels follow the same visibility rules as local variables: a label declared
//! in a block is visible in that block and all nested blocks, but never inside a
//! nested function. A goto may target any visible label as long as it does not
//! enter the scope of a local variable declared between the goto and the label.
//!
//! This module is an extension of [`AnalyzerState`]: it owns the label maps, the
//! goto sites, the visibility snapshots used by those checks, and the second
//! analysis pass that binds every goto to its label.

use crate::nodes::{Block, GotoStatement, LabelStatement, Statement};

use super::analyzer::{line_of, token_key, AnalyzerState};
use super::error::{SemanticError, SemanticResult};
use super::label::{GotoBinding, GotoLabel, GotoSite, LabelId};
use super::symbol::SymbolId;

/// Returns whether the statement at `index` is a label at the end of its block.
///
/// Lua treats a label at the end of a block as being outside the scope of the
/// locals declared in that block, which allows the common `goto continue` idiom.
/// Labels are only followed by other labels (and nothing else), and the block
/// must not end with a `return`/`break`/`continue`.
pub(super) fn is_block_end_label(block: &Block, index: usize) -> bool {
    let statements: Vec<&Statement> = block.iter_statements().collect();

    let followed_only_by_labels = statements[index + 1..]
        .iter()
        .all(|next| matches!(next, Statement::Label(_)));

    followed_only_by_labels && block.get_last_statement().is_none()
}

impl AnalyzerState {
    /// Records a goto occurrence together with the locals visible at its
    /// position, for later resolution.
    pub(super) fn record_goto(&mut self, statement: &GotoStatement) {
        let token = token_key(statement.get_label());
        let site = GotoSite {
            name: statement.get_label_name().to_string(),
            scope: self.current_scope(),
            function_scope: self.current_function(),
            token,
            visible_symbols: self.visible_symbols(),
        };

        if let Some(token) = site.token {
            self.goto_tokens.insert(token, self.goto_sites.len());
        }
        self.goto_sites.push(site);
    }

    /// Records a label declaration, applying the trailing-label relaxation when
    /// `is_block_end` is set.
    ///
    /// A label may not shadow another active label in the same function, mirroring
    /// Lua's `checkrepeated`.
    pub(super) fn record_label(
        &mut self,
        label: &LabelStatement,
        is_block_end: bool,
    ) -> SemanticResult<()> {
        let name = label.get_label_name().to_string();
        let line = line_of(label.get_label());

        if let Some(previous) = self.find_active_label(&name) {
            let previous_line = self.labels[previous.index()].token.map(|token| token.line);
            return Err(SemanticError::duplicate_label(&name, previous_line, line));
        }

        let scope = self.current_scope();
        let visible_symbols = if is_block_end {
            self.visible_symbols_outside_current_scope()
        } else {
            self.visible_symbols()
        };

        let token = token_key(label.get_label());
        let id = LabelId(self.labels.len() as u32);
        self.labels.push(GotoLabel {
            id,
            name: name.clone(),
            scope,
            function_scope: self.current_function(),
            is_block_end,
            token,
            visible_symbols,
        });
        self.scope_labels[scope.index()].insert(name, id);
        self.scopes[scope.index()].labels.push(id);
        if let Some(token) = token {
            self.label_tokens.insert(token, id);
        }

        Ok(())
    }

    /// Resolves every recorded goto against the collected labels and returns the
    /// bindings in source order.
    ///
    /// This is the second semantic pass: labels are only known after the whole
    /// function has been walked, which is what makes forward jumps resolvable.
    pub(super) fn resolve_gotos(&self) -> SemanticResult<Vec<GotoBinding>> {
        let mut bindings = Vec::with_capacity(self.goto_sites.len());

        for site in &self.goto_sites {
            let label_id = match self.find_visible_label(site) {
                Some(label) => label,
                None => {
                    return Err(SemanticError::undefined_label(
                        &site.name,
                        site.token.map(|token| token.line),
                    ))
                }
            };

            let label = &self.labels[label_id.index()];
            self.check_forward_goto_locals(site, label)?;

            bindings.push(GotoBinding {
                label: label_id,
                site: site.clone(),
            });
        }

        Ok(bindings)
    }

    /// Searches the scope chain of a goto for a matching label, stopping at the
    /// function boundary: labels declared inside another function are invisible.
    fn find_visible_label(&self, site: &GotoSite) -> Option<LabelId> {
        let mut current = Some(site.scope);

        while let Some(scope_id) = current {
            if let Some(&label) = self.scope_labels[scope_id.index()].get(&site.name) {
                return Some(label);
            }
            if scope_id == site.function_scope {
                break;
            }
            current = self.scopes[scope_id.index()].parent;
        }

        None
    }

    /// Rejects forward jumps that would enter the scope of a local variable.
    ///
    /// For backward jumps the label-side visibility set is a prefix of the
    /// goto-side set, so the difference is empty and the check is trivially
    /// satisfied; this single test therefore implements the Lua rule for both
    /// directions.
    fn check_forward_goto_locals(&self, site: &GotoSite, label: &GotoLabel) -> SemanticResult<()> {
        let offending = label
            .visible_symbols
            .iter()
            .find(|symbol| !site.visible_symbols.contains(symbol));

        match offending {
            Some(symbol) => Err(self.check_goto_into_local_scope(site, *symbol)),
            None => Ok(()),
        }
    }

    /// Builds the explicit rejection error for a goto entering the scope of the
    /// given local symbol.
    fn check_goto_into_local_scope(&self, site: &GotoSite, symbol: SymbolId) -> SemanticError {
        SemanticError::goto_into_local_scope(
            &site.name,
            &self.symbols[symbol.index()].name,
            site.token.map(|token| token.line),
        )
    }

    /// Returns the label with the given name active in the current function, if
    /// any. Labels declared in sibling or already-closed blocks are not active.
    fn find_active_label(&self, name: &str) -> Option<LabelId> {
        for scope_id in self.scope_stack.iter().rev() {
            if let Some(&label) = self.scope_labels[scope_id.index()].get(name) {
                return Some(label);
            }
            if self.scopes[scope_id.index()].kind.is_function_boundary() {
                break;
            }
        }
        None
    }

    /// Returns every lexical symbol visible at the current point (excluding
    /// implicit globals).
    fn visible_symbols(&self) -> Vec<SymbolId> {
        self.visible_symbols_until(self.scope_stack.len())
    }

    /// Returns every lexical symbol visible at the entry of the current scope,
    /// excluding symbols declared inside it.
    fn visible_symbols_outside_current_scope(&self) -> Vec<SymbolId> {
        let len = self.scope_stack.len().saturating_sub(1);
        self.visible_symbols_until(len)
    }

    fn visible_symbols_until(&self, depth: usize) -> Vec<SymbolId> {
        let mut visible = Vec::new();
        for scope_id in self.scope_stack.iter().take(depth) {
            let scope = &self.scopes[scope_id.index()];
            if scope.kind == super::scope::ScopeKind::Global {
                continue;
            }
            visible.extend(scope.symbols.iter().copied());
        }
        visible
    }
}
