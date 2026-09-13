//! Static linking of exported module values.
//!
//! When a module is required only through an immutable local alias and the
//! alias is never mutated or leaked, the fields it reads are stable. If the
//! exporting module returns an inline table (or a table variable that is never
//! mutated), its exported fields that are backed by an immutable local are also
//! stable. In that case a consumer can read the exported value directly instead
//! of going through the module's returned table.
//!
//! [`link_exported_values`] performs that rewrite:
//!
//! * it mints one root-level local per statically linked export;
//! * it inserts `__R4_VALUE_mod_field = field` right before the exporting
//!   module's `return`, so the value is captured when the module is loaded;
//! * it rewrites `alias.field` (and `alias["field"]`) in every module to the
//!   hoisted local.
//!
//! The rewrite is only applied when it is provably equivalent:
//!
//! * the exporting module returns an inline table, or a table variable that is
//!   neither captured, mutated nor leaked;
//! * the exported field is backed by a non-global immutable local;
//! * every symbol that (transitively) aliases the required module is immutable,
//!   uncaptured, never field-written, never dynamically written and never
//!   escapes.
//!
//! The runtime loader and the returned tables are left in place, so method
//! calls, dynamic indexing and the loader's caching keep working exactly as
//! before.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::modules::{classify_require_call, RequireCall};
use crate::nodes::{
    AssignStatement, Expression, Identifier, Prefix, Statement, TypedIdentifier, Variable,
    VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{
    EscapeAnalysis, ModuleId, ModuleModel, MultiModuleModel, SemanticModel, SymbolId, SymbolKind,
};

/// Rewrites statically resolvable exported value reads to root-level locals.
///
/// Returns the `local` declarations that must be prepended to the bundle so the
/// hoisted locals are in scope before the module definitions capture them.
pub(crate) fn link_exported_values(
    graph: &mut MultiModuleModel,
    root: ModuleId,
    order: &[ModuleId],
    names: &mut NameGenerator,
) -> Vec<Statement> {
    let module_ids: Vec<ModuleId> = order.to_vec();

    let analyses: HashMap<ModuleId, EscapeAnalysis> = module_ids
        .iter()
        .map(|&id| {
            let module = graph.module(id);
            (id, EscapeAnalysis::analyze(&module.block, &module.model))
        })
        .collect();

    // Import bindings and the (transitive) local aliases of each module.
    let mut aliases: HashMap<ModuleId, HashMap<SymbolId, ModuleId>> = HashMap::new();
    let mut unsafe_targets: HashSet<ModuleId> = HashSet::new();

    // A `require` that is not the direct initializer of a local alias can leak
    // or mutate the shared module table, so its target cannot be linked.
    for &id in &module_ids {
        unsafe_targets.extend(non_alias_require_targets(graph.module(id)));
    }

    for &caller in &module_ids {
        let module = graph.module(caller);
        let mut import_aliases: HashMap<SymbolId, ModuleId> = HashMap::new();
        for binding in graph.imports(caller) {
            import_aliases.insert(binding.alias, binding.target);
        }
        if import_aliases.is_empty() {
            continue;
        }

        let analysis = &analyses[&caller];
        let mut caller_aliases = HashMap::new();
        for symbol in module.model.symbols.iter().map(|symbol| symbol.id) {
            let Some(target) = resolve_module_alias(&module.model, symbol, &import_aliases) else {
                continue;
            };
            caller_aliases.insert(symbol, target);

            if !is_stable_alias(&module.model, analysis, symbol) {
                unsafe_targets.insert(target);
            }
        }
        aliases.insert(caller, caller_aliases);
    }

    // Exported fields that can be linked.
    let mut candidates: HashMap<(ModuleId, String), SymbolId> = HashMap::new();
    for &target in order.iter().filter(|id| **id != root) {
        if unsafe_targets.contains(&target) {
            continue;
        }
        let module = graph.module(target);
        if module.interface.values.is_empty() {
            continue;
        }
        if module.interface.table_symbol.is_some() {
            // Only inline returned tables are linked: a table variable can be
            // mutated or captured from inside the exporting module.
            continue;
        }
        for (field, symbol) in &module.interface.values {
            let Some(symbol) = symbol else {
                continue;
            };
            let info = module.model.symbol(*symbol);
            if info.kind != SymbolKind::Global && module.model.is_immutable(*symbol) {
                candidates.insert((target, field.clone()), *symbol);
            }
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }

    let module_keys: HashMap<ModuleId, String> = module_ids
        .iter()
        .map(|&id| (id, graph.module(id).key.clone()))
        .collect();

    // Rewrite the references, minting a root local the first time it is used.
    let mut mapping: BTreeMap<(ModuleId, String), String> = BTreeMap::new();
    for &caller in &module_ids {
        let Some(caller_aliases) = aliases.get(&caller) else {
            continue;
        };
        if caller_aliases.is_empty() {
            continue;
        }

        let mut block = std::mem::take(&mut graph.module_mut(caller).block);
        {
            let module = graph.module(caller);
            let mut linker = ValueLinker {
                model: &module.model,
                aliases: caller_aliases,
                candidates: &candidates,
                module_keys: &module_keys,
                mapping: &mut mapping,
                names,
            };
            DefaultVisitor::visit_block(&mut block, &mut linker);
        }
        graph.module_mut(caller).block = block;
    }

    if mapping.is_empty() {
        return Vec::new();
    }

    // Capture the values when each exporting module returns.
    let mut by_target: BTreeMap<ModuleId, Vec<Statement>> = BTreeMap::new();
    for ((target, field), name) in &mapping {
        let symbol = candidates[&(*target, field.clone())];
        let symbol_name = graph.module(*target).model.symbol(symbol).name.clone();
        let assignment = AssignStatement::from_variable(
            Variable::from(Identifier::new(name.clone())),
            Expression::identifier(symbol_name),
        );
        by_target
            .entry(*target)
            .or_default()
            .push(assignment.into());
    }
    for (target, assignments) in by_target {
        let block = &mut graph.module_mut(target).block;
        let mut statements = block.take_statements();
        statements.extend(assignments);
        block.set_statements(statements);
    }

    // Declare the hoisted locals at the root.
    mapping
        .values()
        .map(|name| {
            VariableAssignment::new(vec![TypedIdentifier::new(name.clone())], Vec::new()).into()
        })
        .collect()
}

fn is_stable_alias(model: &SemanticModel, analysis: &EscapeAnalysis, symbol: SymbolId) -> bool {
    let hard_escape_free = analysis
        .info(symbol)
        .map(|info| info.hard_escapes == 0)
        .unwrap_or(true);
    hard_escape_free
        && !analysis.has_dynamic_write(symbol)
        && analysis
            .info(symbol)
            .map(|info| info.field_writes.is_empty())
            .unwrap_or(true)
        && !model.symbol(symbol).is_captured()
        && model.is_immutable(symbol)
}

/// Follows immutable local initializers until they reach a `require` alias.
fn resolve_module_alias(
    model: &SemanticModel,
    symbol: SymbolId,
    import_aliases: &HashMap<SymbolId, ModuleId>,
) -> Option<ModuleId> {
    let mut current = symbol;
    for _ in 0..model.symbols.len() {
        if let Some(&target) = import_aliases.get(&current) {
            return Some(target);
        }
        let info = model.symbol(current);
        if info.kind == SymbolKind::Global {
            return None;
        }
        match info.initializer {
            Some(next) if next != current => current = next,
            _ => return None,
        }
    }
    None
}

/// Returns the modules referenced by a `require` that is not the direct
/// initializer of an import alias.
///
/// Such a reference can mutate or leak the shared module table (for example
/// `require("./a").field = 1`), so statically linking those modules is unsafe.
fn non_alias_require_targets(module: &ModuleModel) -> HashSet<ModuleId> {
    let mut collector = NonAliasRequires {
        model: &module.model,
        dependencies: &module.dependencies,
        pending_alias: false,
        targets: HashSet::new(),
    };
    let mut block = module.block.clone();
    DefaultVisitor::visit_block(&mut block, &mut collector);
    collector.targets
}

struct NonAliasRequires<'a> {
    model: &'a SemanticModel,
    dependencies: &'a HashMap<String, ModuleId>,
    pending_alias: bool,
    targets: HashSet<ModuleId>,
}

impl NonAliasRequires<'_> {
    fn check_call(&mut self, call: &crate::nodes::FunctionCall) {
        if let RequireCall::Path { path } = classify_require_call(call, self.model) {
            if let Some(&target) = self.dependencies.get(&path) {
                self.targets.insert(target);
            }
        }
    }
}

impl NodeProcessor for NonAliasRequires<'_> {
    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        self.pending_alias = is_alias_require(assignment, self.model);
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Call(call) = expression {
            if self.pending_alias {
                self.pending_alias = false;
                return;
            }
            self.check_call(call);
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        if let Prefix::Call(call) = prefix {
            self.pending_alias = false;
            self.check_call(call);
        }
    }
}

fn is_alias_require(assignment: &VariableAssignment, model: &SemanticModel) -> bool {
    if assignment.variables_len() != 1 || assignment.values_len() != 1 {
        return false;
    }
    matches!(
        assignment.iter_values().next(),
        Some(Expression::Call(call))
            if matches!(classify_require_call(call, model), RequireCall::Path { .. })
    )
}

struct ValueLinker<'a> {
    model: &'a SemanticModel,
    aliases: &'a HashMap<SymbolId, ModuleId>,
    candidates: &'a HashMap<(ModuleId, String), SymbolId>,
    module_keys: &'a HashMap<ModuleId, String>,
    mapping: &'a mut BTreeMap<(ModuleId, String), String>,
    names: &'a mut NameGenerator,
}

impl ValueLinker<'_> {
    fn field_key(&self, prefix: &Prefix, field: &str) -> Option<(ModuleId, String)> {
        let Prefix::Identifier(identifier) = prefix else {
            return None;
        };
        let symbol = self.model.resolve_identifier(identifier)?;
        let target = *self.aliases.get(&symbol)?;
        let key = (target, field.to_owned());
        self.candidates.contains_key(&key).then_some(key)
    }

    fn expression_key(&self, expression: &Expression) -> Option<(ModuleId, String)> {
        match expression {
            Expression::Field(field) => {
                self.field_key(field.get_prefix(), field.get_field().get_name())
            }
            Expression::Index(index) => match index.get_index() {
                Expression::String(string) => {
                    let name = std::str::from_utf8(string.get_value()).ok()?;
                    self.field_key(index.get_prefix(), name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn prefix_key(&self, prefix: &Prefix) -> Option<(ModuleId, String)> {
        match prefix {
            Prefix::Field(field) => {
                self.field_key(field.get_prefix(), field.get_field().get_name())
            }
            Prefix::Index(index) => match index.get_index() {
                Expression::String(string) => {
                    let name = std::str::from_utf8(string.get_value()).ok()?;
                    self.field_key(index.get_prefix(), name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn hoisted_name(&mut self, key: &(ModuleId, String)) -> String {
        if let Some(name) = self.mapping.get(key) {
            return name.clone();
        }
        let (target, field) = key;
        let module_key = self
            .module_keys
            .get(target)
            .map(String::as_str)
            .unwrap_or("module");
        let safe_module = sanitize(module_key);
        let safe_field = sanitize(field);
        let name = self
            .names
            .generate_with_prefix(format!("__R4_VALUE_{safe_module}_{safe_field}"));
        self.mapping.insert(key.clone(), name.clone());
        name
    }
}

impl NodeProcessor for ValueLinker<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Some(key) = self.expression_key(expression) {
            let name = self.hoisted_name(&key);
            *expression = Expression::identifier(name);
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        if let Some(key) = self.prefix_key(prefix) {
            let name = self.hoisted_name(&key);
            *prefix = Prefix::from_name(name);
        }
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
