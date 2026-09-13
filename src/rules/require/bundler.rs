use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::nodes::{
    AssignStatement, Block, Expression, FunctionAssignment, FunctionCall, FunctionExpression,
    IfStatement, IndexExpression, Prefix, ReturnStatement, Statement, StringExpression, Type,
    TypeName, TypedIdentifier, Variable, VariableAssignment,
};
use crate::parser::{Parser, ParserError};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{SemanticAnalyzer, SemanticError, SemanticModel, TypeId};

use super::convert::{convert_require, ConvertRequireError, RequirePathConverter};
use super::{
    classify_require_call, normalize_require_path, parent_directory, require_call_line, RequireCall,
};

/// Provides the source text of a module by its normalized path.
pub trait ModuleProvider {
    /// Returns the module source, or `None` when the module does not exist.
    fn load(&self, path: &str) -> Option<String>;
}

/// A [`ModuleProvider`] backed by an in-memory map.
#[derive(Clone, Debug, Default)]
pub struct MemoryModuleProvider {
    modules: HashMap<String, String>,
}

impl MemoryModuleProvider {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a module source under `path`.
    pub fn insert(&mut self, path: impl Into<String>, source: impl Into<String>) {
        self.modules.insert(path.into(), source.into());
    }
}

impl ModuleProvider for MemoryModuleProvider {
    fn load(&self, path: &str) -> Option<String> {
        self.modules.get(path).cloned()
    }
}

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
    /// An exported type declaration is not bound in its module's model.
    UnresolvedExportedType {
        /// The module declaring the type.
        module: String,
        /// The type name.
        name: String,
    },
    /// A `require` call could not be converted.
    Convert(ConvertRequireError),
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::MissingModule { path } => {
                write!(f, "module `{}` was not found", path)
            }
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
                    write!(f, "module `{}` requires a non-literal path", module)
                } else {
                    write!(f, "the chunk requires a non-literal path")
                }?;
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
            BundleError::UnresolvedExportedType { module, name } => write!(
                f,
                "exported type `{}` of module `{}` is not bound in the semantic model",
                name, module
            ),
            BundleError::Convert(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for BundleError {}

struct BundledModule {
    block: Block,
    model: SemanticModel,
}

struct Discovery<'a, P> {
    provider: &'a P,
    modules: HashMap<String, BundledModule>,
    visiting: HashSet<String>,
    order: Vec<String>,
}

impl<P: ModuleProvider> Discovery<'_, P> {
    fn discover(&mut self, path: &str, base_directory: &str) -> Result<(), BundleError> {
        let normalized = normalize_require_path(path, base_directory);

        if self.visiting.contains(&normalized) {
            return Err(BundleError::CyclicRequire { path: normalized });
        }
        if self.modules.contains_key(&normalized) {
            return Ok(());
        }

        let source = self
            .provider
            .load(&normalized)
            .ok_or_else(|| BundleError::MissingModule {
                path: normalized.clone(),
            })?;

        let mut block = Parser::default()
            .parse(&source)
            .map_err(|error| BundleError::Parse {
                path: normalized.clone(),
                error,
            })?;
        let model =
            SemanticAnalyzer::new()
                .analyze(&block)
                .map_err(|error| BundleError::Analysis {
                    path: normalized.clone(),
                    error,
                })?;

        self.visiting.insert(normalized.clone());
        let dependencies = collect_dependencies(&mut block, &model, Some(normalized.clone()))?;
        for dependency in dependencies {
            self.discover(&dependency, &parent_directory(&normalized))?;
        }
        self.visiting.remove(&normalized);

        self.order.push(normalized.clone());
        self.modules
            .insert(normalized, BundledModule { block, model });

        Ok(())
    }
}

/// Bundles every statically resolvable module reachable from `block`.
///
/// The pass discovers `require("<path>")` calls that resolve to the global
/// `require` (directly or through an immutable alias), loads each module from
/// `provider`, and inlines the modules as lazily evaluated functions behind a
/// caching loader. Every synthesized identifier is allocated by
/// [`NameGenerator`] against the root model and every bundled module, so the
/// generated names cannot collide anywhere in the produced program.
///
/// Returns the number of modules bundled. The caller owns cache invalidation
/// after the rewrite (for example through
/// [`Session`](crate::rules::Session)).
pub fn bundle_modules<P: ModuleProvider>(
    block: &mut Block,
    model: &SemanticModel,
    provider: &P,
) -> Result<usize, BundleError> {
    let mut discovery = Discovery {
        provider,
        modules: HashMap::new(),
        visiting: HashSet::new(),
        order: Vec::new(),
    };

    let root_dependencies = collect_dependencies(block, model, None)?;
    for dependency in root_dependencies {
        discovery.discover(&dependency, "")?;
    }

    let module_count = discovery.order.len();
    if module_count == 0 {
        return Ok(0);
    }

    let mut names = NameGenerator::new(model);
    for module in discovery.modules.values() {
        for symbol in &module.model.symbols {
            names.reserve(symbol.name.clone());
        }
        for type_symbol in &module.model.types {
            names.reserve(type_symbol.name.clone());
        }
    }

    let hoisted_types = hoist_exported_types(
        &mut discovery.modules,
        &discovery.order,
        block,
        model,
        &mut names,
    )?;

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

    for path in discovery.order.clone() {
        let mut module = discovery
            .modules
            .remove(&path)
            .expect("every discovered module is stored");
        let converter = LoaderConverter {
            load: load_name.clone(),
            base_directory: parent_directory(&path),
        };
        convert_require(&mut module.block, &module.model, &converter)
            .map_err(BundleError::Convert)?;
        definitions.push(build_module_definition(&modules_name, &path, module.block));
    }

    let converter = LoaderConverter {
        load: load_name,
        base_directory: String::new(),
    };
    convert_require(block, model, &converter).map_err(BundleError::Convert)?;

    let mut statements = hoisted_types;
    statements.extend(definitions);
    statements.extend(block.take_statements());
    block.set_statements(statements);

    Ok(module_count)
}

fn is_exported_type_declaration(statement: &Statement) -> bool {
    matches!(statement, Statement::TypeDeclaration(declaration) if declaration.is_exported())
}

fn declaration_name(statement: &Statement) -> String {
    match statement {
        Statement::TypeDeclaration(declaration) => declaration.get_name().get_name().clone(),
        _ => unreachable!("only type declarations are hoisted"),
    }
}

/// A top-level type declaration extracted from a module.
struct ModuleTypeDeclaration {
    id: TypeId,
    exported: bool,
    statement: Statement,
}

struct TypeRenamer<'a> {
    model: &'a SemanticModel,
    local: &'a HashMap<TypeId, String>,
    global: &'a HashMap<String, String>,
}

impl NodeProcessor for TypeRenamer<'_> {
    fn process_type_name(&mut self, type_name: &mut TypeName) {
        let identifier = type_name.get_type_name();
        let target = match self.model.resolve_type(identifier) {
            // A resolved name is only renamed when it is one of this module's
            // hoisted types; a shadowing local type keeps its name.
            Some(id) => self.local.get(&id).cloned(),
            // An unresolved name may refer to a type exported by another
            // module; retarget it to that module's hoisted type.
            None => self.global.get(identifier.get_name()).cloned(),
        };
        if let Some(fresh) = target {
            type_name.mutate_type_name().set_name(fresh);
        }
    }
}

struct TypeReferenceCollector<'a> {
    model: &'a SemanticModel,
    ids: Vec<TypeId>,
}

impl NodeProcessor for TypeReferenceCollector<'_> {
    fn process_type_name(&mut self, type_name: &mut TypeName) {
        if let Some(id) = self.model.resolve_type(type_name.get_type_name()) {
            self.ids.push(id);
        }
    }
}

fn referenced_types(model: &SemanticModel, r#type: &Type) -> Vec<TypeId> {
    let mut collector = TypeReferenceCollector {
        model,
        ids: Vec::new(),
    };
    let mut r#type = r#type.clone();
    DefaultVisitor::visit_type(&mut r#type, &mut collector);
    collector.ids
}

/// Hoists exported type declarations out of bundled modules into the root
/// scope.
///
/// Every exported type (and any local type it depends on) is minted a fresh
/// collision-free name by `names`, removed from its module, and re-declared at
/// the top of the root chunk. References are retargeted structurally through
/// each module's model, so a shadowing local type is never renamed. Two
/// modules exporting the same type name is a typed error rather than a silent
/// collision.
fn hoist_exported_types(
    modules: &mut HashMap<String, BundledModule>,
    order: &[String],
    root_block: &mut Block,
    root_model: &SemanticModel,
    names: &mut NameGenerator,
) -> Result<Vec<Statement>, BundleError> {
    let module_exports = modules.values().any(|module| {
        module
            .block
            .iter_statements()
            .any(is_exported_type_declaration)
    });
    let root_exports = root_block
        .iter_statements()
        .any(is_exported_type_declaration);

    if !module_exports && !root_exports {
        return Ok(Vec::new());
    }

    let mut exported_owner: HashMap<String, String> = HashMap::new();
    let mut global_names: HashMap<String, String> = HashMap::new();

    // Root exported types already live in the root scope.
    if root_exports {
        for statement in root_block.iter_statements() {
            if let Statement::TypeDeclaration(declaration) = statement {
                if declaration.is_exported() {
                    let name = declaration.get_name().get_name().clone();
                    exported_owner.entry(name.clone()).or_default();
                    global_names.entry(name.clone()).or_insert(name);
                }
            }
        }
    }

    // Extract every top-level type declaration and reject duplicate exported
    // names across modules.
    let mut extracted: Vec<(String, Vec<ModuleTypeDeclaration>)> = Vec::new();
    for path in order {
        let module = modules.get_mut(path).expect("every ordered module exists");
        let model = &module.model;

        let mut declarations = Vec::new();
        let mut rest = Vec::new();

        for statement in module.block.take_statements() {
            if let Statement::TypeDeclaration(declaration) = &statement {
                let name = declaration.get_name().get_name().clone();
                let id = model.resolve_type(declaration.get_name()).ok_or_else(|| {
                    BundleError::UnresolvedExportedType {
                        module: path.clone(),
                        name: name.clone(),
                    }
                })?;

                if declaration.is_exported() {
                    if let Some(owner) = exported_owner.insert(name.clone(), path.clone()) {
                        if owner != *path {
                            return Err(BundleError::DuplicateExportedType { name });
                        }
                    }
                }

                declarations.push(ModuleTypeDeclaration {
                    id,
                    exported: declaration.is_exported(),
                    statement,
                });
            } else {
                rest.push(statement);
            }
        }

        module.block.set_statements(rest);
        extracted.push((path.clone(), declarations));
    }

    // Decide which declarations to hoist (exported types plus their local
    // dependencies) and mint their fresh names.
    let mut module_names: HashMap<String, HashMap<TypeId, String>> = HashMap::new();
    let mut hoisted_flags: HashMap<String, Vec<bool>> = HashMap::new();

    for (path, declarations) in &extracted {
        let module = modules.get(path).expect("every extracted module exists");

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
            for referenced in referenced_types(&module.model, statement.get_type()) {
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

        let mut mapping: HashMap<TypeId, String> = HashMap::new();
        let mut flags = vec![false; declarations.len()];
        for &index in &hoisted {
            let declaration = &declarations[index];
            let name = declaration_name(&declaration.statement);
            let fresh = names.generate_with_prefix(format!("__R4_TYPE_{name}"));
            if declaration.exported {
                global_names.insert(name, fresh.clone());
            }
            mapping.insert(declaration.id, fresh);
            flags[index] = true;
        }

        module_names.insert(path.clone(), mapping);
        hoisted_flags.insert(path.clone(), flags);
    }

    // Move the hoisted declarations to the root and retarget references.
    let mut hoisted_statements: Vec<Statement> = Vec::new();

    for (path, declarations) in extracted {
        let module = modules
            .get_mut(&path)
            .expect("every extracted module exists");
        let mapping = module_names
            .get(&path)
            .expect("every module has a type mapping");
        let flags = hoisted_flags
            .get(&path)
            .expect("every module has hoisted flags");

        let BundledModule { block, model } = module;

        let mut rebuilt: Vec<Statement> = Vec::new();
        for (index, declaration) in declarations.into_iter().enumerate() {
            if flags[index] {
                let ModuleTypeDeclaration {
                    id,
                    statement: declaration_statement,
                    ..
                } = declaration;
                let mut statement = declaration_statement;
                if let Statement::TypeDeclaration(declaration) = &mut statement {
                    declaration.remove_exported();
                    if let Some(fresh) = mapping.get(&id) {
                        declaration.mutate_name().set_name(fresh.clone());
                    }
                }

                let mut renamer = TypeRenamer {
                    model,
                    local: mapping,
                    global: &global_names,
                };
                DefaultVisitor::visit_statement(&mut statement, &mut renamer);
                hoisted_statements.push(statement);
            } else {
                rebuilt.push(declaration.statement);
            }
        }

        rebuilt.extend(block.take_statements());
        block.set_statements(rebuilt);

        let mut renamer = TypeRenamer {
            model,
            local: mapping,
            global: &global_names,
        };
        DefaultVisitor::visit_block(block, &mut renamer);
    }

    let empty: HashMap<TypeId, String> = HashMap::new();
    let mut root_renamer = TypeRenamer {
        model: root_model,
        local: &empty,
        global: &global_names,
    };
    DefaultVisitor::visit_block(root_block, &mut root_renamer);

    Ok(hoisted_statements)
}

fn collect_dependencies(
    block: &mut Block,
    model: &SemanticModel,
    module: Option<String>,
) -> Result<Vec<String>, BundleError> {
    struct Collector<'a> {
        model: &'a SemanticModel,
        module: Option<String>,
        dependencies: Vec<String>,
        error: Option<BundleError>,
    }

    impl Collector<'_> {
        fn check(&mut self, call: &FunctionCall) {
            if self.error.is_some() {
                return;
            }
            match classify_require_call(call, self.model) {
                RequireCall::NotRequire => {}
                RequireCall::Path { path } => self.dependencies.push(path),
                RequireCall::NonLiteral => {
                    self.error = Some(BundleError::NonLiteralRequire {
                        module: self.module.clone(),
                        line: require_call_line(call),
                    });
                }
            }
        }
    }

    impl NodeProcessor for Collector<'_> {
        fn process_expression(&mut self, expression: &mut Expression) {
            if let Expression::Call(call) = expression {
                self.check(call);
            }
        }

        fn process_statement(&mut self, statement: &mut Statement) {
            if let Statement::Call(call) = statement {
                self.check(call);
            }
        }
    }

    let mut collector = Collector {
        model,
        module,
        dependencies: Vec::new(),
        error: None,
    };
    DefaultVisitor::visit_block(block, &mut collector);

    match collector.error {
        Some(error) => Err(error),
        None => Ok(collector.dependencies),
    }
}

struct LoaderConverter {
    load: String,
    base_directory: String,
}

impl RequirePathConverter for LoaderConverter {
    fn convert(&self, path: &str) -> Option<Expression> {
        let normalized = normalize_require_path(path, &self.base_directory);
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
