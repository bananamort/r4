//! Identity-based module bundling.
//!
//! The bundler is a pure consumer of the two layers below it. Dependency
//! discovery, path resolution and topological ordering come from
//! [`crate::modules::ModuleResolver`]; the module boundaries, export
//! interfaces and import bindings come from
//! [`crate::semantic::MultiModuleModel`]. This module never resolves a
//! cross-module name by string dictionary: references are retargeted through
//! the target module's interface by identity, and a reference that cannot be
//! resolved that way is a typed error.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::mem;

use crate::modules::{ModuleProvider, ModuleResolver, ResolveError, ResolvedModules};
use crate::nodes::{
    AssignStatement, Block, Expression, FunctionAssignment, FunctionCall, FunctionExpression,
    IfStatement, IndexExpression, Prefix, ReturnStatement, Statement, StringExpression, Type,
    TypeName, TypedIdentifier, Variable, VariableAssignment,
};
use crate::parser::ParserError;
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{
    is_builtin_type_name, ModuleError, ModuleId, MultiModuleModel, SemanticError, TypeId,
};

use super::convert::{convert_require, ConvertRequireError, RequirePathConverter};
use super::link;

/// The key used for the entry chunk in the resolved module graph.
const ROOT_KEY: &str = "<root>";

/// An error produced while bundling modules.
#[derive(Debug)]
pub enum BundleError {
    /// A required module is not provided.
    MissingModule {
        /// The normalized path that could not be loaded.
        path: String,
    },
    /// Modules require each other cyclically.
    CyclicRequire {
        /// The module that closes the cycle.
        path: String,
    },
    /// A module source could not be parsed.
    Parse {
        /// The module path.
        path: String,
        /// The parser error.
        error: ParserError,
    },
    /// A module source could not be analyzed.
    Analysis {
        /// The module path.
        path: String,
        /// The semantic error.
        error: SemanticError,
    },
    /// A resolved `require` call does not use a static string argument.
    NonLiteralRequire {
        /// The module containing the call, or `None` for the root chunk.
        module: Option<String>,
        /// The source line of the call, if known.
        line: Option<usize>,
    },
    /// Two modules export types with the same name.
    DuplicateExportedType {
        /// The conflicting type name.
        name: String,
    },
    /// A referenced type is exported by a module that the caller does not
    /// import.
    UnimportedType {
        /// The referring module key.
        module: String,
        /// The type name.
        name: String,
    },
    /// A provider found a module but could not read its source.
    ModuleLoad {
        /// The load error.
        error: crate::modules::ModuleLoadError,
    },
    /// A cross-module reference could not be resolved by identity.
    Module(ModuleError),
    /// A `require` call could not be converted.
    Convert(ConvertRequireError),
}

impl From<ResolveError> for BundleError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::MissingModule { path } => BundleError::MissingModule { path },
            ResolveError::ModuleLoad { error } => BundleError::ModuleLoad { error },
            ResolveError::CyclicRequire { path } => BundleError::CyclicRequire { path },
            ResolveError::Parse { path, error } => BundleError::Parse { path, error },
            ResolveError::Analysis { path, error } => BundleError::Analysis { path, error },
            ResolveError::NonLiteralRequire { module, line } => {
                let module = (module != ROOT_KEY).then_some(module);
                BundleError::NonLiteralRequire { module, line }
            }
            ResolveError::Interface(error) => BundleError::Module(error),
        }
    }
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::MissingModule { path } => write!(f, "module `{}` was not found", path),
            BundleError::ModuleLoad { error } => write!(f, "{}", error),
            BundleError::CyclicRequire { path } => {
                write!(f, "cyclic require involving module `{}`", path)
            }
            BundleError::Parse { path, error } => {
                write!(f, "failed to parse module `{}`: {}", path, error)
            }
            BundleError::Analysis { path, error } => {
                write!(f, "failed to analyze module `{}`: {}", path, error)
            }
            BundleError::NonLiteralRequire { module, line } => {
                if let Some(module) = module {
                    write!(f, "module `{}` requires a non-literal path", module)?;
                } else {
                    write!(f, "the chunk requires a non-literal path")?;
                }
                if let Some(line) = line {
                    write!(f, " at line {}", line)?;
                }
                Ok(())
            }
            BundleError::DuplicateExportedType { name } => write!(
                f,
                "exported type `{}` is declared by more than one module",
                name
            ),
            BundleError::UnimportedType { module, name } => write!(
                f,
                "module `{}` references the type `{}`, which it does not import",
                module, name
            ),
            BundleError::Module(error) => write!(f, "{}", error),
            BundleError::Convert(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for BundleError {}

/// Bundles every statically resolvable module reachable from `block`.
///
/// Discovery, path resolution and cycle detection are handled by the module
/// resolver; cross-module references are retargeted through each module's
/// declared interface. Returns the number of modules bundled and writes the
/// resulting chunk back into `block`.
pub fn bundle_modules<P: ModuleProvider>(
    block: &mut Block,
    model: &crate::semantic::SemanticModel,
    provider: &P,
) -> Result<usize, BundleError> {
    let root_block = mem::take(block);
    let resolved = ModuleResolver::new(provider)
        .resolve(root_block, model.clone(), ROOT_KEY)
        .map_err(BundleError::from)?;

    let module_count = resolved
        .order
        .iter()
        .filter(|module| **module != resolved.root)
        .count();

    if module_count == 0 {
        let mut resolved = resolved;
        let root = resolved.root;
        *block = mem::take(&mut resolved.graph.module_mut(root).block);
        return Ok(0);
    }

    *block = bundle_graph(resolved)?;
    Ok(module_count)
}

fn bundle_graph(resolved: ResolvedModules) -> Result<Block, BundleError> {
    let ResolvedModules {
        mut graph,
        root,
        order,
    } = resolved;

    let mut names = NameGenerator::new(&graph.module(root).model);
    for module in graph.iter_ids() {
        let module = graph.module(module);
        for symbol in &module.model.symbols {
            names.reserve(symbol.name.clone());
        }
        for type_symbol in &module.model.types {
            names.reserve(type_symbol.name.clone());
        }
    }

    validate_cross_module_types(&graph, &order, root)?;
    let linked_values = link::link_exported_values(&mut graph, root, &order, &mut names);
    let hoisted_types = hoist_exported_types(&mut graph, root, &order, &mut names)?;

    let modules_name = names.generate_with_prefix("__R4_MODULES");
    let cache_name = names.generate_with_prefix("__R4_CACHE");
    let loaded_name = names.generate_with_prefix("__R4_LOADED");
    let load_name = names.generate_with_prefix("__R4_LOAD");
    let key_name = names.generate_with_prefix("__R4_KEY");
    let result_name = names.generate_with_prefix("__R4_RESULT");

    let mut definitions = vec![build_loader(
        &modules_name,
        &cache_name,
        &loaded_name,
        &load_name,
        &key_name,
        &result_name,
    )];

    for module_id in order.iter().copied().filter(|module| *module != root) {
        let path = graph.module(module_id).key.clone();
        let converter = LoaderConverter {
            load: load_name.clone(),
            base_directory: crate::modules::parent_directory(&path),
            module_key: path.clone(),
        };
        {
            let module = graph.module_mut(module_id);
            convert_require(&mut module.block, &module.model, &converter)
                .map_err(BundleError::Convert)?;
        }
        let block = mem::take(&mut graph.module_mut(module_id).block);
        definitions.push(build_module_definition(&modules_name, &path, block));
    }

    {
        let converter = LoaderConverter {
            load: load_name,
            base_directory: String::new(),
            module_key: ROOT_KEY.to_owned(),
        };
        let module = graph.module_mut(root);
        convert_require(&mut module.block, &module.model, &converter)
            .map_err(BundleError::Convert)?;
    }

    let mut root_block = mem::take(&mut graph.module_mut(root).block);
    let mut statements = linked_values;
    statements.extend(hoisted_types);
    statements.extend(definitions);
    statements.extend(root_block.take_statements());
    root_block.set_statements(statements);

    Ok(root_block)
}

// ----------------------------------------------------------------------
// Cross-module type validation and hoisting
// ----------------------------------------------------------------------

struct TypeValidator<'a> {
    graph: &'a MultiModuleModel,
    module: ModuleId,
    error: Option<BundleError>,
}

impl NodeProcessor for TypeValidator<'_> {
    fn process_type_name(&mut self, type_name: &mut TypeName) {
        if self.error.is_some() {
            return;
        }

        let identifier = type_name.get_type_name();
        if self
            .graph
            .module(self.module)
            .model
            .resolve_type(identifier)
            .is_some()
        {
            return;
        }

        let name = identifier.get_name();
        if is_builtin_type_name(name) {
            return;
        }

        match self.graph.resolve_imported_type(self.module, name) {
            Ok(_) => {}
            Err(ModuleError::UnknownImportedType { .. }) => {
                if !self.graph.modules_exporting_type(name).is_empty() {
                    self.error = Some(BundleError::UnimportedType {
                        module: self.graph.module(self.module).key.clone(),
                        name: name.clone(),
                    });
                }
            }
            Err(error) => self.error = Some(BundleError::Module(error)),
        }
    }
}

fn validate_cross_module_types(
    graph: &MultiModuleModel,
    order: &[ModuleId],
    root: ModuleId,
) -> Result<(), BundleError> {
    let mut modules: Vec<ModuleId> = order.to_vec();
    if !modules.contains(&root) {
        modules.push(root);
    }

    for module in modules {
        let mut block = graph.module(module).block.clone();
        let mut validator = TypeValidator {
            graph,
            module,
            error: None,
        };
        DefaultVisitor::visit_block(&mut block, &mut validator);
        if let Some(error) = validator.error {
            return Err(error);
        }
    }

    Ok(())
}

struct TypeRenamer<'a> {
    graph: &'a MultiModuleModel,
    module: ModuleId,
    local: &'a HashMap<TypeId, String>,
    global: &'a HashMap<(ModuleId, TypeId), String>,
}

impl NodeProcessor for TypeRenamer<'_> {
    fn process_type_name(&mut self, type_name: &mut TypeName) {
        let identifier = type_name.get_type_name();

        if let Some(id) = self
            .graph
            .module(self.module)
            .model
            .resolve_type(identifier)
        {
            if let Some(fresh) = self.local.get(&id) {
                type_name.mutate_type_name().set_name(fresh.clone());
            }
            return;
        }

        if let Ok((target, id)) = self
            .graph
            .resolve_imported_type(self.module, identifier.get_name())
        {
            if let Some(fresh) = self.global.get(&(target, id)) {
                type_name.mutate_type_name().set_name(fresh.clone());
            }
        }
    }
}

struct TypeReferenceCollector<'a> {
    model: &'a crate::semantic::SemanticModel,
    ids: Vec<TypeId>,
}

impl NodeProcessor for TypeReferenceCollector<'_> {
    fn process_type_name(&mut self, type_name: &mut TypeName) {
        if let Some(id) = self.model.resolve_type(type_name.get_type_name()) {
            self.ids.push(id);
        }
    }
}

fn referenced_types(model: &crate::semantic::SemanticModel, r#type: &Type) -> Vec<TypeId> {
    let mut collector = TypeReferenceCollector {
        model,
        ids: Vec::new(),
    };
    let mut r#type = r#type.clone();
    DefaultVisitor::visit_type(&mut r#type, &mut collector);
    collector.ids
}

struct ExtractedDeclaration {
    id: TypeId,
    name: String,
    exported: bool,
    statement: Statement,
}

fn hoist_exported_types(
    graph: &mut MultiModuleModel,
    root: ModuleId,
    order: &[ModuleId],
    names: &mut NameGenerator,
) -> Result<Vec<Statement>, BundleError> {
    // Reject duplicate exported type names across all modules (and the root).
    let mut owner: HashMap<String, ModuleId> = HashMap::new();
    for module in graph.iter_ids() {
        let interface = &graph.module(module).interface;
        for name in interface.types.keys() {
            if let Some(previous) = owner.insert(name.clone(), module) {
                if previous != module {
                    return Err(BundleError::DuplicateExportedType { name: name.clone() });
                }
            }
        }
    }

    // Extract each module's top-level type declarations and decide which move
    // to the root: exported types plus the local types they depend on.
    let mut extracted: HashMap<ModuleId, Vec<ExtractedDeclaration>> = HashMap::new();
    let mut hoisted_flags: HashMap<ModuleId, Vec<bool>> = HashMap::new();
    let mut local_mappings: HashMap<ModuleId, HashMap<TypeId, String>> = HashMap::new();
    let mut global_mapping: HashMap<(ModuleId, TypeId), String> = HashMap::new();

    for &module_id in order.iter().filter(|module| **module != root) {
        let module = graph.module_mut(module_id);
        let model = &module.model;

        let mut declarations = Vec::new();
        let mut rest = Vec::new();

        for statement in module.block.take_statements() {
            if let Statement::TypeDeclaration(declaration) = &statement {
                let name = declaration.get_name().get_name().clone();
                let id = model.resolve_type(declaration.get_name()).ok_or_else(|| {
                    BundleError::Module(ModuleError::UnboundExport { name: name.clone() })
                })?;
                declarations.push(ExtractedDeclaration {
                    id,
                    name,
                    exported: declaration.is_exported(),
                    statement,
                });
            } else {
                rest.push(statement);
            }
        }
        module.block.set_statements(rest);

        let index_by_id: HashMap<TypeId, usize> = declarations
            .iter()
            .enumerate()
            .map(|(index, declaration)| (declaration.id, index))
            .collect();

        let mut dependencies: Vec<Vec<usize>> = vec![Vec::new(); declarations.len()];
        for (index, declaration) in declarations.iter().enumerate() {
            let Statement::TypeDeclaration(statement) = &declaration.statement else {
                continue;
            };
            for referenced in referenced_types(model, statement.get_type()) {
                if let Some(&dependency) = index_by_id.get(&referenced) {
                    dependencies[index].push(dependency);
                }
            }
        }

        let mut hoisted: HashSet<usize> = HashSet::new();
        let mut stack: Vec<usize> = declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| declaration.exported)
            .map(|(index, _)| index)
            .collect();
        while let Some(index) = stack.pop() {
            if hoisted.insert(index) {
                stack.extend(dependencies[index].iter().copied());
            }
        }

        let mut mapping = HashMap::new();
        let mut flags = vec![false; declarations.len()];
        for &index in &hoisted {
            let declaration = &declarations[index];
            let name = declaration.name.clone();
            let fresh = names.generate_with_prefix(format!("__R4_TYPE_{name}"));
            global_mapping.insert((module_id, declaration.id), fresh.clone());
            mapping.insert(declaration.id, fresh);
            flags[index] = true;
        }

        extracted.insert(module_id, declarations);
        hoisted_flags.insert(module_id, flags);
        local_mappings.insert(module_id, mapping);
    }

    // Rename references and move the hoisted declarations to the root.
    let mut hoisted_statements: Vec<Statement> = Vec::new();

    for &module_id in order.iter().filter(|module| **module != root) {
        let declarations = extracted
            .remove(&module_id)
            .expect("every module was extracted");
        let flags = hoisted_flags
            .remove(&module_id)
            .expect("every module has hoisted flags");
        let mapping = local_mappings
            .remove(&module_id)
            .expect("every module has a type mapping");

        let mut block = mem::take(&mut graph.module_mut(module_id).block);
        let mut rebuilt: Vec<Statement> = Vec::new();

        for (index, declaration) in declarations.into_iter().enumerate() {
            let ExtractedDeclaration {
                id, mut statement, ..
            } = declaration;
            if flags[index] {
                if let Statement::TypeDeclaration(declaration) = &mut statement {
                    declaration.remove_exported();
                    if let Some(fresh) = mapping.get(&id) {
                        declaration.mutate_name().set_name(fresh.clone());
                    }
                }
                rename_statement(graph, module_id, &mapping, &global_mapping, &mut statement);
                hoisted_statements.push(statement);
            } else {
                rebuilt.push(statement);
            }
        }

        rebuilt.extend(block.take_statements());
        block.set_statements(rebuilt);
        rename_block(graph, module_id, &mapping, &global_mapping, &mut block);
        graph.module_mut(module_id).block = block;
    }

    {
        let mut root_block = mem::take(&mut graph.module_mut(root).block);
        let empty = HashMap::new();
        rename_block(graph, root, &empty, &global_mapping, &mut root_block);
        graph.module_mut(root).block = root_block;
    }

    Ok(hoisted_statements)
}

fn rename_statement(
    graph: &MultiModuleModel,
    module: ModuleId,
    local: &HashMap<TypeId, String>,
    global: &HashMap<(ModuleId, TypeId), String>,
    statement: &mut Statement,
) {
    let mut renamer = TypeRenamer {
        graph,
        module,
        local,
        global,
    };
    DefaultVisitor::visit_statement(statement, &mut renamer);
}

fn rename_block(
    graph: &MultiModuleModel,
    module: ModuleId,
    local: &HashMap<TypeId, String>,
    global: &HashMap<(ModuleId, TypeId), String>,
    block: &mut Block,
) {
    let mut renamer = TypeRenamer {
        graph,
        module,
        local,
        global,
    };
    DefaultVisitor::visit_block(block, &mut renamer);
}

// ----------------------------------------------------------------------
// Loader construction
// ----------------------------------------------------------------------

struct LoaderConverter {
    load: String,
    base_directory: String,
    module_key: String,
}

impl RequirePathConverter for LoaderConverter {
    fn convert(&self, path: &str) -> Option<Expression> {
        let normalized = crate::modules::normalize_require_path(path, &self.base_directory);
        Some(
            FunctionCall::from_name(self.load.clone())
                .with_argument(StringExpression::from_value(normalized))
                .into(),
        )
    }

    fn convert_instance(&self, segments: &[crate::modules::InstanceSegment]) -> Option<Expression> {
        let normalized = crate::modules::resolve_instance_path(&self.module_key, segments)?;
        Some(
            FunctionCall::from_name(self.load.clone())
                .with_argument(StringExpression::from_value(normalized))
                .into(),
        )
    }
}

fn index_expression(base: &str, key_expression: Expression) -> IndexExpression {
    IndexExpression::new(Prefix::from_name(base), key_expression)
}

fn index_identifier(base: &str, key: &str) -> Expression {
    index_expression(base, Expression::identifier(key)).into()
}

fn store_statement(base: &str, key: &str, value: Expression) -> Statement {
    AssignStatement::from_variable(
        Variable::from(index_expression(base, Expression::identifier(key))),
        value,
    )
    .into()
}

fn build_loader(
    modules: &str,
    cache: &str,
    loaded: &str,
    load: &str,
    key: &str,
    result: &str,
) -> Statement {
    let cached_check = IfStatement::create(
        index_identifier(loaded, key),
        Block::from(ReturnStatement::one(index_identifier(cache, key))),
    );

    let module_call =
        FunctionCall::from_prefix(index_expression(modules, Expression::identifier(key)));
    let compute = VariableAssignment::from_variable(result).with_value(module_call);
    let store_cache = store_statement(cache, key, Expression::identifier(result));
    let store_loaded = store_statement(loaded, key, Expression::from(true));
    let return_result = ReturnStatement::one(Expression::identifier(result));

    let mut body = Block::default();
    body.push_statement(cached_check);
    body.push_statement(compute);
    body.push_statement(store_cache);
    body.push_statement(store_loaded);
    body.set_last_statement(return_result);

    let function = FunctionAssignment::new(load, body, vec![TypedIdentifier::new(key)], false);

    function.into()
}

fn build_module_definition(modules: &str, path: &str, block: Block) -> Statement {
    let variable = Variable::from(index_expression(
        modules,
        StringExpression::from_value(path).into(),
    ));
    let function = FunctionExpression::from_block(block);

    AssignStatement::from_variable(variable, Expression::from(function)).into()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::modules::MemoryModuleProvider;
    use crate::rules::test_support::{analyze, generate, parse};
    use crate::semantic::SemanticAnalyzer;

    #[test]
    fn bundles_required_modules() {
        let (mut block, model) = analyze("local math = require('./math')\nreturn math.add(1, 2)");
        let mut provider = MemoryModuleProvider::new();
        provider.insert(
            "math",
            "local function add(a, b) return a + b end\nreturn { add = add }",
        );

        let bundled = bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
        assert_eq!(bundled, 1);

        let output = generate(&block);
        assert!(output.contains("__R4_MODULES"), "unexpected: {output}");
        assert!(!output.contains("require("), "unexpected: {output}");
        assert!(output.contains("['math']"), "unexpected: {output}");

        let reparsed = parse(&output);
        let new_model = SemanticAnalyzer::new()
            .analyze(&reparsed)
            .expect("bundled code must analyze");
        assert!(
            new_model.symbols.iter().any(|symbol| symbol.name == "math"),
            "the module binding must still be declared: {output}"
        );
        assert!(
            new_model
                .symbols
                .iter()
                .any(|symbol| symbol.name == "__R4_VALUE_math_add" && symbol.read_count >= 1),
            "the exported value must be statically linked and read: {output}"
        );
    }

    #[test]
    fn bundles_nested_modules() {
        let (mut block, model) = analyze("return require('./a')");
        let mut provider = MemoryModuleProvider::new();
        provider.insert("a", "return require('./b')");
        provider.insert("b", "return 1");

        let bundled = bundle_modules(&mut block, &model, &provider).expect("bundling must succeed");
        assert_eq!(bundled, 2);

        let output = generate(&block);
        assert!(output.contains("['a']"), "unexpected: {output}");
        assert!(output.contains("['b']"), "unexpected: {output}");
        assert!(!output.contains("require("), "unexpected: {output}");
    }

    #[test]
    fn reports_missing_modules() {
        let (mut block, model) = analyze("return require('./missing')");
        let provider = MemoryModuleProvider::new();
        let error = bundle_modules(&mut block, &model, &provider)
            .expect_err("a missing module must be reported");

        assert!(matches!(
            error,
            BundleError::MissingModule { ref path } if path == "missing"
        ));
    }

    #[test]
    fn reports_cyclic_modules() {
        let (mut block, model) = analyze("return require('./a')");
        let mut provider = MemoryModuleProvider::new();
        provider.insert("a", "return require('./b')");
        provider.insert("b", "return require('./a')");

        let error =
            bundle_modules(&mut block, &model, &provider).expect_err("a cycle must be reported");

        assert!(matches!(error, BundleError::CyclicRequire { .. }));
    }

    #[test]
    fn reports_non_literal_requires() {
        let (mut block, model) = analyze("return require(path)");
        let provider = MemoryModuleProvider::new();
        let error = bundle_modules(&mut block, &model, &provider)
            .expect_err("a dynamic require must be reported");

        assert!(matches!(
            error,
            BundleError::NonLiteralRequire { module: None, .. }
        ));
    }

    #[test]
    fn ignores_shadowed_require() {
        let code = "local require = function(path) return path end\nreturn require('./x')";
        let (mut block, model) = analyze(code);
        let provider = MemoryModuleProvider::new();

        let bundled = bundle_modules(&mut block, &model, &provider).expect("no require to bundle");
        assert_eq!(bundled, 0);
        assert!(generate(&block).contains("require("));
    }
}
