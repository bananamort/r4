//! A whole-program index derived from the per-module semantic models.
//!
//! The multi-module container keeps each module's symbols, scopes, types and
//! labels in that module's own [`SemanticModel`](super::SemanticModel), so a
//! module stays self-contained, cacheable and independently analyzable. When a
//! whole-program analysis needs one flat view, it builds a
//! [`WholeProgramIndex`] instead of merging the arenas.
//!
//! The index is a read-only derivation: it assigns a dense rank to every
//! implicit global (shared, one rank per canonical [`GlobalId`]) and to every
//! module-local symbol (addressed by `(ModuleId, SymbolId)`), and it records
//! which module-local symbols denote each global. Nothing is moved, no
//! `BindingId`/`ScopeId`/`TypeId` is remapped, and the index can be dropped at
//! any time. Bitset-based whole-program dataflow can therefore run over
//! `globals().len() + locals().len()` ranks without changing how the AST binds
//! its symbols.

use std::collections::HashMap;

use super::{
    GlobalId, GlobalSymbol, GotoLabel, LabelId, ModuleId, MultiModuleModel, Symbol, SymbolId,
    TypeId, TypeSymbol,
};

/// A module-local symbol addressed from the whole program.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QualifiedSymbol {
    /// The module that owns the symbol.
    pub module: ModuleId,
    /// The symbol inside that module's model.
    pub symbol: SymbolId,
}

/// A module-local type alias addressed from the whole program.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QualifiedType {
    /// The module that owns the type.
    pub module: ModuleId,
    /// The type inside that module's model.
    pub id: TypeId,
}

/// A module-local goto label addressed from the whole program.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QualifiedLabel {
    /// The module that owns the label.
    pub module: ModuleId,
    /// The label inside that module's model.
    pub label: LabelId,
}

/// A flat, densely ranked view over every symbol in a module graph.
///
/// Build it with [`WholeProgramIndex::build`] or
/// [`MultiModuleModel::whole_program_index`].
#[derive(Clone, Debug, Default)]
pub struct WholeProgramIndex {
    globals: Vec<GlobalSymbol>,
    locals: Vec<QualifiedSymbol>,
    local_rank: HashMap<QualifiedSymbol, u32>,
    global_of: HashMap<QualifiedSymbol, GlobalId>,
    global_uses: Vec<Vec<QualifiedSymbol>>,
    types: Vec<QualifiedType>,
    type_rank: HashMap<QualifiedType, u32>,
    labels: Vec<QualifiedLabel>,
    label_rank: HashMap<QualifiedLabel, u32>,
}

impl WholeProgramIndex {
    /// Builds the index from `graph`.
    ///
    /// Globals are ranked by their canonical [`GlobalId`]; locals are ranked in
    /// `(module, declaration order)` order.
    pub fn build(graph: &MultiModuleModel) -> Self {
        let globals: Vec<GlobalSymbol> = graph.globals().cloned().collect();
        let mut global_uses: Vec<Vec<QualifiedSymbol>> = vec![Vec::new(); globals.len()];

        let mut locals = Vec::new();
        let mut local_rank = HashMap::new();
        let mut global_of = HashMap::new();

        let mut types = Vec::new();
        let mut type_rank = HashMap::new();

        let mut labels = Vec::new();
        let mut label_rank = HashMap::new();

        for module in graph.iter_ids() {
            let model = &graph.module(module).model;

            for symbol in &model.symbols {
                let qualified = QualifiedSymbol {
                    module,
                    symbol: symbol.id,
                };
                match graph.global_of(module, symbol.id) {
                    Some(global) => {
                        global_of.insert(qualified, global);
                        global_uses[global.index()].push(qualified);
                    }
                    None => {
                        let rank = locals.len() as u32;
                        local_rank.insert(qualified, rank);
                        locals.push(qualified);
                    }
                }
            }

            for type_symbol in &model.types {
                let qualified = QualifiedType {
                    module,
                    id: type_symbol.id,
                };
                type_rank.insert(qualified, types.len() as u32);
                types.push(qualified);
            }

            for label in &model.labels {
                let qualified = QualifiedLabel {
                    module,
                    label: label.id,
                };
                label_rank.insert(qualified, labels.len() as u32);
                labels.push(qualified);
            }
        }

        Self {
            globals,
            locals,
            local_rank,
            global_of,
            global_uses,
            types,
            type_rank,
            labels,
            label_rank,
        }
    }

    /// Returns the canonical globals, densely ranked by [`GlobalId`].
    #[inline]
    pub fn globals(&self) -> &[GlobalSymbol] {
        &self.globals
    }

    /// Returns the number of canonical globals.
    #[inline]
    pub fn global_count(&self) -> usize {
        self.globals.len()
    }

    /// Returns the global with the given canonical identity.
    #[inline]
    pub fn global(&self, id: GlobalId) -> Option<&GlobalSymbol> {
        self.globals.get(id.index())
    }

    /// Returns every module-local symbol that denotes the given global.
    #[inline]
    pub fn global_uses(&self, id: GlobalId) -> &[QualifiedSymbol] {
        self.global_uses
            .get(id.index())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the canonical global denoted by a module-local symbol, if it is
    /// a global.
    #[inline]
    pub fn global_of(&self, symbol: QualifiedSymbol) -> Option<GlobalId> {
        self.global_of.get(&symbol).copied()
    }

    /// Returns whether the module-local symbol denotes a global.
    #[inline]
    pub fn is_global(&self, symbol: QualifiedSymbol) -> bool {
        self.global_of.contains_key(&symbol)
    }

    /// Returns the module-local symbols, densely ranked from `0`.
    #[inline]
    pub fn locals(&self) -> &[QualifiedSymbol] {
        &self.locals
    }

    /// Returns the number of module-local symbols.
    #[inline]
    pub fn local_count(&self) -> usize {
        self.locals.len()
    }

    /// Returns the module-local symbol with the given dense rank.
    #[inline]
    pub fn local(&self, rank: u32) -> Option<QualifiedSymbol> {
        self.locals.get(rank as usize).copied()
    }

    /// Returns the dense rank of a module-local symbol.
    #[inline]
    pub fn local_rank(&self, symbol: QualifiedSymbol) -> Option<u32> {
        self.local_rank.get(&symbol).copied()
    }

    /// Returns the number of ranks a whole-program bitset needs to cover every
    /// observable symbol (canonical globals followed by module-local symbols).
    #[inline]
    pub fn dense_len(&self) -> usize {
        self.globals.len() + self.locals.len()
    }

    /// Returns the dense rank of a canonical global.
    #[inline]
    pub fn global_rank(&self, id: GlobalId) -> u32 {
        id.0
    }

    /// Returns the dense rank of a symbol in the shared rank space: canonical
    /// globals occupy `0..global_count`, module-local symbols follow.
    #[inline]
    pub fn dense_rank(&self, symbol: QualifiedSymbol) -> Option<u32> {
        if let Some(global) = self.global_of(symbol) {
            return Some(self.global_rank(global));
        }
        self.local_rank(symbol)
            .map(|rank| rank + self.globals.len() as u32)
    }

    /// Returns every type alias, densely ranked from `0`.
    #[inline]
    pub fn types(&self) -> &[QualifiedType] {
        &self.types
    }

    /// Returns the number of type aliases.
    #[inline]
    pub fn type_count(&self) -> usize {
        self.types.len()
    }

    /// Returns the type alias with the given dense rank.
    #[inline]
    pub fn type_at(&self, rank: u32) -> Option<QualifiedType> {
        self.types.get(rank as usize).copied()
    }

    /// Returns the dense rank of a type alias.
    #[inline]
    pub fn type_rank(&self, r#type: QualifiedType) -> Option<u32> {
        self.type_rank.get(&r#type).copied()
    }

    /// Returns every goto label, densely ranked from `0`.
    #[inline]
    pub fn labels(&self) -> &[QualifiedLabel] {
        &self.labels
    }

    /// Returns the number of goto labels.
    #[inline]
    pub fn label_count(&self) -> usize {
        self.labels.len()
    }

    /// Returns the goto label with the given dense rank.
    #[inline]
    pub fn label_at(&self, rank: u32) -> Option<QualifiedLabel> {
        self.labels.get(rank as usize).copied()
    }

    /// Returns the dense rank of a goto label.
    #[inline]
    pub fn label_rank(&self, label: QualifiedLabel) -> Option<u32> {
        self.label_rank.get(&label).copied()
    }

    /// Returns the symbol behind a qualified reference.
    ///
    /// # Panics
    ///
    /// Panics when the qualified reference does not belong to `graph`; the
    /// index is built from one graph and must be queried with the same graph.
    #[inline]
    pub fn symbol<'g>(&self, graph: &'g MultiModuleModel, symbol: QualifiedSymbol) -> &'g Symbol {
        graph.module(symbol.module).model.symbol(symbol.symbol)
    }

    /// Returns the type symbol behind a qualified reference.
    #[inline]
    pub fn type_symbol<'g>(
        &self,
        graph: &'g MultiModuleModel,
        r#type: QualifiedType,
    ) -> &'g TypeSymbol {
        graph.module(r#type.module).model.type_symbol(r#type.id)
    }

    /// Returns the label behind a qualified reference.
    #[inline]
    pub fn goto_label<'g>(
        &self,
        graph: &'g MultiModuleModel,
        label: QualifiedLabel,
    ) -> &'g GotoLabel {
        graph.module(label.module).model.label(label.label)
    }
}

#[cfg(test)]
mod test {
    use crate::semantic::MultiModuleModel;
    use crate::Parser;

    use super::*;

    fn add_module(graph: &mut MultiModuleModel, key: &str, code: &str) {
        let (block, model) = Parser::default()
            .parse_analyzed(code)
            .expect("the test module analyzes");
        graph
            .add_module(key.to_owned(), block, model)
            .expect("the interface is valid");
    }

    #[test]
    fn ranks_globals_and_locals_densely() {
        let mut graph = MultiModuleModel::new();
        add_module(&mut graph, "a", "local x = 1\nprint(x)\nreturn x");
        add_module(&mut graph, "b", "local y = 2\nprint(y)\nreturn y");

        let index = WholeProgramIndex::build(&graph);

        assert_eq!(index.global_count(), 1);
        let print = graph.global_symbol("print").expect("print is global");
        assert_eq!(index.global(print).unwrap().name, "print");
        assert_eq!(index.global_uses(print).len(), 2);

        assert_eq!(index.local_count(), 2);
        assert_eq!(index.local(0).map(|s| s.module.0), Some(0));
        assert_eq!(index.local(1).map(|s| s.module.0), Some(1));
        for rank in 0..index.local_count() as u32 {
            let symbol = index.local(rank).expect("rank is dense");
            assert_eq!(index.local_rank(symbol), Some(rank));
        }

        // Globals and locals share one rank space.
        assert_eq!(index.dense_len(), 3);
        assert_eq!(index.global_rank(print), 0);
        let first_local = index.local(0).unwrap();
        assert_eq!(index.dense_rank(first_local), Some(1));
    }

    #[test]
    fn ranks_types_and_labels() {
        let mut graph = MultiModuleModel::new();
        let (block, model) = Parser::default()
            .lua52()
            .parse_analyzed("::top::\ngoto top\nreturn 0")
            .expect("the Lua 5.2 module analyzes");
        graph
            .add_module("a".to_owned(), block, model)
            .expect("module a");
        add_module(&mut graph, "b", "type B = string\nreturn 0");

        let index = WholeProgramIndex::build(&graph);
        assert_eq!(index.type_count(), 1);
        assert_eq!(index.type_at(0).map(|t| t.module.0), Some(1));
        assert_eq!(index.type_rank(index.type_at(0).unwrap()), Some(0));

        assert_eq!(index.label_count(), 1);
        assert_eq!(index.label_at(0).map(|l| l.module.0), Some(0));
        assert_eq!(index.label_rank(index.label_at(0).unwrap()), Some(0));
    }

    #[test]
    fn distinguishes_globals_from_locals() {
        let mut graph = MultiModuleModel::new();
        add_module(
            &mut graph,
            "a",
            "local shadow = 1\nlocal other = shadow\nreturn other",
        );

        let index = WholeProgramIndex::build(&graph);
        let shadow = index
            .locals()
            .iter()
            .copied()
            .find(|qualified| index.symbol(&graph, *qualified).name == "shadow")
            .expect("shadow is a local");
        assert!(!index.is_global(shadow));
        assert_eq!(index.global_of(shadow), None);
    }
}
