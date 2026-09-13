//! Multi-tree, module-aware semantic model.
//!
//! [`MultiModuleModel`] holds the syntax trees of several modules together
//! with their single-file [`SemanticModel`]s. Each module has an explicit
//! [`ModuleInterface`] describing the values and types it exports, and imports
//! bind a local symbol directly to the target module's interface
//! (`ImportBinding`). Cross-module queries therefore resolve by identity: the
//! caller's import alias selects a target module, and the requested name is
//! looked up in that module's declared interface. No ambient name dictionary
//! is ever consulted.
//!
//! This layer is deliberately free of path resolution, file I/O and code
//! generation; it only analyzes syntax trees and answers semantic queries.

use std::collections::HashMap;
use std::fmt;

use crate::nodes::{
    Block, Expression, LastStatement, Prefix, Statement, TableEntry, TableExpression, Variable,
};

use super::{ScopeId, SemanticModel, SymbolId, TypeId};

/// Identifies a module inside a [`MultiModuleModel`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

impl ModuleId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A value exported by a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportValue {
    /// The public name of the export.
    pub name: String,
    /// The local symbol backing the export, when the exported value is a
    /// simple identifier. Inline expressions have no symbol to rename.
    pub symbol: Option<SymbolId>,
}

/// The explicit public interface of a module.
#[derive(Clone, Debug, Default)]
pub struct ModuleInterface {
    /// Named exports of the module's returned table.
    pub values: HashMap<String, Option<SymbolId>>,
    /// The single returned value, when the module returns a non-table symbol.
    pub value: Option<SymbolId>,
    /// Exported type aliases by name.
    pub types: HashMap<String, TypeId>,
}

impl ModuleInterface {
    /// Returns whether the module exports the named value.
    #[inline]
    pub fn has_value(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// Returns the symbol backing a named export, if the export exists and is
    /// backed by a symbol.
    pub fn value_symbol(&self, name: &str) -> Option<SymbolId> {
        self.values.get(name).copied().flatten()
    }

    /// Returns the exported type with this name, if any.
    #[inline]
    pub fn type_alias(&self, name: &str) -> Option<TypeId> {
        self.types.get(name).copied()
    }

    /// Iterates over the named value exports.
    pub fn iter_values(&self) -> impl Iterator<Item = ExportValue> + '_ {
        self.values.iter().map(|(name, symbol)| ExportValue {
            name: name.clone(),
            symbol: *symbol,
        })
    }
}

/// A local symbol bound to a target module by a `require`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImportBinding {
    /// The local symbol that aliases the required module.
    pub alias: SymbolId,
    /// The imported module.
    pub target: ModuleId,
}

/// An error produced by the multi-tree semantic layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleError {
    /// An exported declaration is not bound in the module's model.
    UnboundExport {
        /// The unbound export name.
        name: String,
    },
    /// A cross-module value access names an export the target module does not
    /// declare.
    UnknownExport {
        /// The target module key.
        module: String,
        /// The requested export name.
        name: String,
    },
    /// A cross-module value access uses a symbol that is not an import alias.
    UnknownImportAlias {
        /// The caller module key.
        module: String,
        /// The name of the symbol that was used as an import.
        name: String,
    },
    /// No imported module exports a referenced type name.
    UnknownImportedType {
        /// The caller module key.
        module: String,
        /// The unresolved type name.
        name: String,
    },
    /// More than one imported module exports the same type name.
    AmbiguousImportedType {
        /// The ambiguous type name.
        name: String,
        /// The keys of the modules that export it.
        modules: Vec<String>,
    },
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleError::UnboundExport { name } => {
                write!(
                    f,
                    "exported name `{}` is not bound in the module model",
                    name
                )
            }
            ModuleError::UnknownExport { module, name } => {
                write!(f, "module `{}` does not export `{}`", module, name)
            }
            ModuleError::UnknownImportAlias { module, name } => write!(
                f,
                "`{}` is not an imported module alias in module `{}`",
                name, module
            ),
            ModuleError::UnknownImportedType { module, name } => write!(
                f,
                "no module imported by `{}` exports the type `{}`",
                module, name
            ),
            ModuleError::AmbiguousImportedType { name, modules } => write!(
                f,
                "the imported type `{}` is exported by several modules: {}",
                name,
                modules.join(", ")
            ),
        }
    }
}

impl std::error::Error for ModuleError {}

/// A module inside the multi-tree container.
pub struct ModuleModel {
    /// The identifier of this module.
    pub id: ModuleId,
    /// The logical module key (for example a normalized path).
    pub key: String,
    /// The module's syntax tree.
    pub block: Block,
    /// The single-file semantic model of `block`.
    pub model: SemanticModel,
    /// The file scope of the module inside `model`.
    pub file_scope: ScopeId,
    /// The module's explicit public interface.
    pub interface: ModuleInterface,
    /// Local symbols that alias imported modules.
    pub imports: Vec<ImportBinding>,
    /// The `require` paths written in this module, mapped to their resolved
    /// target modules.
    pub dependencies: HashMap<String, ModuleId>,
}

/// A container of several analyzed modules with identity-based import
/// bindings.
#[derive(Default)]
pub struct MultiModuleModel {
    modules: Vec<ModuleModel>,
}

impl fmt::Debug for MultiModuleModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiModuleModel")
            .field("module_count", &self.modules.len())
            .finish()
    }
}

impl MultiModuleModel {
    /// Creates an empty container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an analyzed module and extracts its export interface.
    pub fn add_module(
        &mut self,
        key: impl Into<String>,
        block: Block,
        model: SemanticModel,
    ) -> Result<ModuleId, ModuleError> {
        let key = key.into();
        let interface = extract_interface(&block, &model)?;
        let id = ModuleId(self.modules.len() as u32);
        let file_scope = model.chunk_scope();
        self.modules.push(ModuleModel {
            id,
            key,
            block,
            model,
            file_scope,
            interface,
            imports: Vec::new(),
            dependencies: HashMap::new(),
        });
        Ok(id)
    }

    /// Returns the module with the given identifier.
    #[inline]
    pub fn module(&self, id: ModuleId) -> &ModuleModel {
        &self.modules[id.index()]
    }

    /// Returns the mutable module with the given identifier.
    #[inline]
    pub fn module_mut(&mut self, id: ModuleId) -> &mut ModuleModel {
        &mut self.modules[id.index()]
    }

    /// Returns the number of modules.
    #[inline]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Returns whether there are no modules.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Iterates over all module identifiers in insertion order.
    pub fn iter_ids(&self) -> impl Iterator<Item = ModuleId> {
        (0..self.modules.len() as u32).map(ModuleId)
    }

    /// Returns the module with the given key, if any.
    pub fn module_by_key(&self, key: &str) -> Option<ModuleId> {
        self.modules
            .iter()
            .find(|module| module.key == key)
            .map(|module| module.id)
    }

    /// Binds a local alias symbol to an imported module.
    pub fn bind_import(&mut self, module: ModuleId, alias: SymbolId, target: ModuleId) {
        let imports = &mut self.modules[module.index()].imports;
        if let Some(binding) = imports.iter_mut().find(|binding| binding.alias == alias) {
            binding.target = target;
        } else {
            imports.push(ImportBinding { alias, target });
        }
    }

    /// Returns the module imported through `alias`, if any.
    pub fn import_target(&self, module: ModuleId, alias: SymbolId) -> Option<ModuleId> {
        self.modules[module.index()]
            .imports
            .iter()
            .find(|binding| binding.alias == alias)
            .map(|binding| binding.target)
    }

    /// Iterates over `(alias, target)` import bindings of a module.
    pub fn imports(&self, module: ModuleId) -> impl Iterator<Item = ImportBinding> + '_ {
        self.modules[module.index()].imports.iter().copied()
    }

    /// Returns the modules that export a type with this name.
    pub fn modules_exporting_type(&self, name: &str) -> Vec<ModuleId> {
        self.modules
            .iter()
            .filter(|module| module.interface.types.contains_key(name))
            .map(|module| module.id)
            .collect()
    }

    /// Resolves `alias.field` by identity.
    ///
    /// The alias must be bound to an imported module, and `field` must be a
    /// declared export of that module. Returns the exported symbol when the
    /// export is symbol-backed, and `None` when it is an inline value.
    pub fn resolve_imported_value(
        &self,
        module: ModuleId,
        alias: SymbolId,
        field: &str,
    ) -> Result<Option<SymbolId>, ModuleError> {
        let target =
            self.import_target(module, alias)
                .ok_or_else(|| ModuleError::UnknownImportAlias {
                    module: self.module(module).key.clone(),
                    name: self.module(module).model.symbol(alias).name.clone(),
                })?;

        match self.module(target).interface.values.get(field) {
            Some(symbol) => Ok(*symbol),
            None => Err(ModuleError::UnknownExport {
                module: self.module(target).key.clone(),
                name: field.to_owned(),
            }),
        }
    }

    /// Resolves a type name through the caller's imports by identity.
    ///
    /// Exactly one imported module must export the type; zero is
    /// [`ModuleError::UnknownImportedType`] and more than one is
    /// [`ModuleError::AmbiguousImportedType`].
    pub fn resolve_imported_type(
        &self,
        module: ModuleId,
        name: &str,
    ) -> Result<(ModuleId, TypeId), ModuleError> {
        let mut found: Vec<(ModuleId, TypeId)> = Vec::new();
        for binding in self.imports(module) {
            if let Some(&id) = self.module(binding.target).interface.types.get(name) {
                found.push((binding.target, id));
            }
        }

        match found.len() {
            0 => Err(ModuleError::UnknownImportedType {
                module: self.module(module).key.clone(),
                name: name.to_owned(),
            }),
            1 => Ok(found[0]),
            _ => Err(ModuleError::AmbiguousImportedType {
                name: name.to_owned(),
                modules: found
                    .iter()
                    .map(|(id, _)| self.module(*id).key.clone())
                    .collect(),
            }),
        }
    }
}

/// Extracts a module's explicit export interface from its syntax tree.
///
/// Supported value export patterns are:
///
/// * `return { key = value, ["key"] = value }` — the field names are the
///   exports; identifier-valued fields carry the backing symbol;
/// * `local table = ...; table.key = value; ...; return table` — top-level
///   assignments to a returned table are exports;
/// * `return identifier` of any other value — a single unnamed export.
///
/// Exported types are `export type` declarations. A module that returns any
/// other expression has an opaque interface (no named exports); referencing an
/// export of such a module is a typed error, never a silent miss.
pub fn extract_interface(
    block: &Block,
    model: &SemanticModel,
) -> Result<ModuleInterface, ModuleError> {
    let mut interface = ModuleInterface::default();

    for statement in block.iter_statements() {
        if let Statement::TypeDeclaration(declaration) = statement {
            if declaration.is_exported() {
                let name = declaration.get_name().get_name().clone();
                let id = model
                    .resolve_type(declaration.get_name())
                    .ok_or_else(|| ModuleError::UnboundExport { name: name.clone() })?;
                interface.types.insert(name, id);
            }
        }
    }

    let Some(LastStatement::Return(return_statement)) = block.get_last_statement() else {
        return Ok(interface);
    };

    let mut expressions = return_statement.iter_expressions();
    let Some(first) = expressions.next() else {
        return Ok(interface);
    };
    if expressions.next().is_some() {
        // Multiple return values make the named interface ambiguous; expose
        // nothing rather than guessing.
        return Ok(interface);
    }

    match first {
        Expression::Table(table) => {
            for (name, symbol) in table_exports(table, model) {
                interface.values.insert(name, symbol);
            }
        }
        Expression::Identifier(identifier) => {
            let Some(symbol) = model.resolve_identifier(identifier) else {
                return Err(ModuleError::UnboundExport {
                    name: identifier.get_name().clone(),
                });
            };

            let mut table_backed = false;
            for statement in block.iter_statements() {
                match statement {
                    Statement::LocalAssign(assignment) => {
                        if assignment.variables_len() != 1 || assignment.values_len() != 1 {
                            continue;
                        }
                        let variable = assignment
                            .iter_variables()
                            .next()
                            .expect("the assignment was checked to have one variable");
                        if model.resolve_identifier(variable.get_identifier()) != Some(symbol) {
                            continue;
                        }
                        if let Some(Expression::Table(table)) = assignment.iter_values().next() {
                            table_backed = true;
                            for (name, backing) in table_exports(table, model) {
                                interface.values.insert(name, backing);
                            }
                        }
                    }
                    Statement::Assign(assignment) => {
                        let assignments = assignment.iter_variables().zip(assignment.iter_values());
                        for (variable, value) in assignments {
                            if let Some((name, backing)) =
                                mutation_export(variable, value, symbol, model)
                            {
                                table_backed = true;
                                interface.values.insert(name, backing);
                            }
                        }
                    }
                    Statement::Function(function) => {
                        let name = function.get_name();
                        if model.resolve_identifier(name.get_name()) != Some(symbol) {
                            continue;
                        }
                        let field = name
                            .get_field_names()
                            .first()
                            .or_else(|| name.get_method())
                            .cloned();
                        if let Some(field) = field {
                            table_backed = true;
                            interface
                                .values
                                .entry(field.get_name().clone())
                                .or_insert(None);
                        }
                    }
                    _ => {}
                }
            }

            if !table_backed {
                interface.value = Some(symbol);
            }
        }
        _ => {}
    }

    Ok(interface)
}

fn table_exports(
    table: &TableExpression,
    model: &SemanticModel,
) -> Vec<(String, Option<SymbolId>)> {
    let mut exports = Vec::new();

    for entry in table.iter_entries() {
        match entry {
            TableEntry::Field(field) => {
                exports.push((
                    field.get_field().get_name().clone(),
                    symbol_of(field.get_value(), model),
                ));
            }
            TableEntry::Index(index) => {
                if let Some(name) = string_name(index.get_key()) {
                    exports.push((name, symbol_of(index.get_value(), model)));
                }
            }
            TableEntry::Value(_) => {}
        }
    }

    exports
}

fn mutation_export(
    variable: &Variable,
    value: &Expression,
    target: SymbolId,
    model: &SemanticModel,
) -> Option<(String, Option<SymbolId>)> {
    let name = match variable {
        Variable::Field(field) => {
            if !prefix_resolves_to(field.get_prefix(), target, model) {
                return None;
            }
            field.get_field().get_name().clone()
        }
        Variable::Index(index) => {
            if !prefix_resolves_to(index.get_prefix(), target, model) {
                return None;
            }
            string_name(index.get_index())?
        }
        Variable::Identifier(_) => return None,
    };

    Some((name, symbol_of(value, model)))
}

fn prefix_resolves_to(prefix: &Prefix, target: SymbolId, model: &SemanticModel) -> bool {
    match prefix {
        Prefix::Identifier(identifier) => model.resolve_identifier(identifier) == Some(target),
        _ => false,
    }
}

fn symbol_of(expression: &Expression, model: &SemanticModel) -> Option<SymbolId> {
    match expression {
        Expression::Identifier(identifier) => model.resolve_identifier(identifier),
        _ => None,
    }
}

fn string_name(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(string) => std::str::from_utf8(string.get_value())
            .ok()
            .map(str::to_owned),
        _ => None,
    }
}
