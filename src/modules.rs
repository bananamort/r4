//! Module discovery and dependency resolution.
//!
//! This layer owns everything path-related: locating module sources, resolving
//! `require` paths and aliases, detecting cycles and computing the topological
//! order. It parses each discovered file and delegates analysis to the semantic
//! layer (through [`SemanticAnalyzer`] and [`MultiModuleModel`]); it never
//! builds scopes or performs symbol resolution itself.
//!
//! The result is a [`MultiModuleModel`] whose imports are bound to the target
//! modules by identity, plus the topological order the bundler inlines in.

use std::collections::HashMap;
use std::fmt;

use crate::nodes::{
    Arguments, Block, Expression, FunctionCall, Prefix, Statement, StringExpression,
};
use crate::parser::ParserError;
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{
    ModuleError, ModuleId, MultiModuleModel, SemanticAnalyzer, SemanticError, SemanticModel,
    SymbolKind,
};
use crate::Parser;

/// The name of the global `require` function.
pub const REQUIRE_NAME: &str = "require";

/// Provides the source text of a module by its normalized key.
pub trait ModuleProvider {
    /// Returns the module source, or `None` when the module does not exist.
    fn load(&self, key: &str) -> Option<String>;
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

    /// Registers a module source under `key`.
    pub fn insert(&mut self, key: impl Into<String>, source: impl Into<String>) {
        self.modules.insert(key.into(), source.into());
    }
}

impl ModuleProvider for MemoryModuleProvider {
    fn load(&self, key: &str) -> Option<String> {
        self.modules.get(key).cloned()
    }
}

/// Classifies a call as a statically resolvable `require` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequireCall {
    /// The call is not a `require` call, or `require` is shadowed.
    NotRequire,
    /// The call is a `require` call with a single string-literal argument.
    Path {
        /// The literal path.
        path: String,
    },
    /// The call is a `require` call whose argument is not a single string
    /// literal (or is not valid UTF-8).
    NonLiteral,
}

/// Classifies `call`, resolving the `require` prefix through `model`.
pub fn classify_require_call(call: &FunctionCall, model: &SemanticModel) -> RequireCall {
    if call.get_method().is_some() {
        return RequireCall::NotRequire;
    }

    let Prefix::Identifier(identifier) = call.get_prefix() else {
        return RequireCall::NotRequire;
    };

    if !resolves_to_require(model, identifier) {
        return RequireCall::NotRequire;
    }

    match call.get_arguments() {
        Arguments::String(string) => match path_of_string(string) {
            Some(path) => RequireCall::Path { path },
            None => RequireCall::NonLiteral,
        },
        Arguments::Tuple(tuple) => {
            let mut values = tuple.iter_values();
            match (values.next(), values.next()) {
                (Some(Expression::String(string)), None) => match path_of_string(string) {
                    Some(path) => RequireCall::Path { path },
                    None => RequireCall::NonLiteral,
                },
                _ => RequireCall::NonLiteral,
            }
        }
        Arguments::Table(_) => RequireCall::NonLiteral,
    }
}

fn resolves_to_require(model: &SemanticModel, identifier: &crate::nodes::Identifier) -> bool {
    let Some(symbol) = model.resolve_identifier(identifier) else {
        return false;
    };
    let info = model.symbol(symbol);
    if info.kind == SymbolKind::Global {
        return info.name == REQUIRE_NAME;
    }
    match model.immutable_global_alias(symbol) {
        Some(global) => model.symbol(global).name == REQUIRE_NAME,
        None => false,
    }
}

fn path_of_string(string: &StringExpression) -> Option<String> {
    std::str::from_utf8(string.get_value())
        .ok()
        .map(str::to_owned)
}

/// Returns the source line of a `require` call's prefix, when token data is
/// available.
pub(crate) fn require_call_line(call: &FunctionCall) -> Option<usize> {
    match call.get_prefix() {
        Prefix::Identifier(identifier) => identifier
            .get_token()
            .and_then(|token| token.get_line_number()),
        _ => None,
    }
}

/// Returns the directory prefix of a normalized module key.
pub(crate) fn parent_directory(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_owned(),
        None => String::new(),
    }
}

/// Normalizes a relative `require` path against the directory of the module
/// that contains it.
pub(crate) fn normalize_require_path(path: &str, base_directory: &str) -> String {
    if !path.starts_with("./") && !path.starts_with("../") {
        return path.to_owned();
    }

    let mut segments: Vec<&str> = if base_directory.is_empty() {
        Vec::new()
    } else {
        base_directory
            .split('/')
            .filter(|s| !s.is_empty())
            .collect()
    };

    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }

    segments.join("/")
}

/// An error produced while resolving a module graph.
#[derive(Debug)]
pub enum ResolveError {
    /// A required module is not provided.
    MissingModule {
        /// The normalized key that could not be loaded.
        path: String,
    },
    /// Modules require each other cyclically.
    CyclicRequire {
        /// The module that closes the cycle.
        path: String,
    },
    /// A module source could not be parsed.
    Parse {
        /// The module key.
        path: String,
        /// The parser error.
        error: ParserError,
    },
    /// A module source could not be analyzed.
    Analysis {
        /// The module key.
        path: String,
        /// The semantic error.
        error: SemanticError,
    },
    /// A resolved `require` call does not use a static string argument.
    NonLiteralRequire {
        /// The module containing the call.
        module: String,
        /// The source line of the call, if known.
        line: Option<usize>,
    },
    /// The module interface could not be extracted.
    Interface(ModuleError),
}

impl From<ModuleError> for ResolveError {
    fn from(error: ModuleError) -> Self {
        Self::Interface(error)
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::MissingModule { path } => write!(f, "module `{}` was not found", path),
            ResolveError::CyclicRequire { path } => {
                write!(f, "cyclic require involving module `{}`", path)
            }
            ResolveError::Parse { path, error } => {
                write!(f, "failed to parse module `{}`: {}", path, error)
            }
            ResolveError::Analysis { path, error } => {
                write!(f, "failed to analyze module `{}`: {}", path, error)
            }
            ResolveError::NonLiteralRequire { module, line } => {
                write!(f, "module `{}` requires a non-literal path", module)?;
                if let Some(line) = line {
                    write!(f, " at line {}", line)?;
                }
                Ok(())
            }
            ResolveError::Interface(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for ResolveError {}

impl ResolveError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            ResolveError::MissingModule { .. } => "module/missing",
            ResolveError::CyclicRequire { .. } => "module/cyclic-require",
            ResolveError::Parse { .. } => "module/parse",
            ResolveError::Analysis { .. } => "module/analysis",
            ResolveError::NonLiteralRequire { .. } => "module/non-literal-require",
            ResolveError::Interface(_) => "module/interface",
        }
    }

    /// Converts this error into a structured diagnostic.
    pub fn to_diagnostic(&self) -> crate::diagnostic::Diagnostic {
        crate::diagnostic::Diagnostic::error(self.to_string()).with_code(self.code())
    }
}

/// The analyzed modules and their topological order.
#[derive(Debug)]
pub struct ResolvedModules {
    /// The multi-tree container holding every module.
    pub graph: MultiModuleModel,
    /// The entry module (the chunk being bundled).
    pub root: ModuleId,
    /// Module identifiers in topological order: dependencies before dependents,
    /// with the root last.
    pub order: Vec<ModuleId>,
}

/// Discovers and analyzes a module graph.
pub struct ModuleResolver<'a, P> {
    provider: &'a P,
}

impl<'a, P: ModuleProvider> ModuleResolver<'a, P> {
    /// Creates a resolver over `provider`.
    pub fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    /// Resolves the graph starting from an already-analyzed root block.
    pub fn resolve(
        &self,
        mut root_block: Block,
        root_model: SemanticModel,
        root_key: &str,
    ) -> Result<ResolvedModules, ResolveError> {
        let mut state = ResolveState {
            provider: self.provider,
            graph: MultiModuleModel::new(),
            by_key: HashMap::new(),
            visiting: Vec::new(),
            order: Vec::new(),
        };

        let base_directory = parent_directory(root_key);
        let root_dependencies =
            module_dependencies(&mut root_block, &root_model, root_key, &base_directory)?;

        let root = state
            .graph
            .add_module(root_key.to_owned(), root_block, root_model)?;
        state.by_key.insert(root_key.to_owned(), root);
        state.visiting.push(root_key.to_owned());
        state.visit_dependencies(root, root_dependencies)?;
        state.visiting.pop();
        state.order.push(root);

        // Bind alias imports now that every module has an interface.
        state.bind_imports()?;

        Ok(ResolvedModules {
            graph: state.graph,
            root,
            order: state.order,
        })
    }
}

struct ResolveState<'a, P> {
    provider: &'a P,
    graph: MultiModuleModel,
    by_key: HashMap<String, ModuleId>,
    visiting: Vec<String>,
    order: Vec<ModuleId>,
}

impl<P: ModuleProvider> ResolveState<'_, P> {
    fn visit_dependencies(
        &mut self,
        module: ModuleId,
        dependencies: Vec<(String, String)>,
    ) -> Result<(), ResolveError> {
        let mut resolved = HashMap::new();
        for (path, normalized) in dependencies {
            if self.visiting.iter().any(|visiting| visiting == &normalized) {
                return Err(ResolveError::CyclicRequire { path: normalized });
            }

            let target = if let Some(&target) = self.by_key.get(&normalized) {
                target
            } else {
                let source =
                    self.provider
                        .load(&normalized)
                        .ok_or_else(|| ResolveError::MissingModule {
                            path: normalized.clone(),
                        })?;
                let mut block =
                    Parser::default()
                        .parse(&source)
                        .map_err(|error| ResolveError::Parse {
                            path: normalized.clone(),
                            error,
                        })?;
                let model = SemanticAnalyzer::new().analyze(&block).map_err(|error| {
                    ResolveError::Analysis {
                        path: normalized.clone(),
                        error,
                    }
                })?;

                let base_directory = parent_directory(&normalized);
                let child_dependencies =
                    module_dependencies(&mut block, &model, &normalized, &base_directory)?;

                let target = self.graph.add_module(normalized.clone(), block, model)?;
                self.by_key.insert(normalized.clone(), target);

                self.visiting.push(normalized.clone());
                self.visit_dependencies(target, child_dependencies)?;
                self.visiting.pop();
                self.order.push(target);
                target
            };

            resolved.insert(path, target);
        }

        self.graph.module_mut(module).dependencies = resolved;
        Ok(())
    }

    fn bind_imports(&mut self) -> Result<(), ResolveError> {
        let module_ids: Vec<ModuleId> = self.graph.iter_ids().collect();
        let mut bindings: Vec<(ModuleId, crate::semantic::SymbolId, ModuleId)> = Vec::new();

        for module_id in module_ids {
            let module = self.graph.module(module_id);
            for (alias, target) in alias_imports(&module.block, &module.model, &module.dependencies)
            {
                bindings.push((module_id, alias, target));
            }
        }

        for (module, alias, target) in bindings {
            self.graph.bind_import(module, alias, target);
        }

        Ok(())
    }
}

fn module_dependencies(
    block: &mut Block,
    model: &SemanticModel,
    module_key: &str,
    base_directory: &str,
) -> Result<Vec<(String, String)>, ResolveError> {
    struct Collector<'a> {
        model: &'a SemanticModel,
        base_directory: &'a str,
        module_key: &'a str,
        dependencies: Vec<(String, String)>,
        error: Option<ResolveError>,
    }

    impl Collector<'_> {
        fn check(&mut self, call: &FunctionCall) {
            if self.error.is_some() {
                return;
            }
            match classify_require_call(call, self.model) {
                RequireCall::NotRequire => {}
                RequireCall::Path { path } => {
                    let normalized = normalize_require_path(&path, self.base_directory);
                    self.dependencies.push((path, normalized));
                }
                RequireCall::NonLiteral => {
                    self.error = Some(ResolveError::NonLiteralRequire {
                        module: self.module_key.to_owned(),
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
        base_directory,
        module_key,
        dependencies: Vec::new(),
        error: None,
    };
    DefaultVisitor::visit_block(block, &mut collector);

    match collector.error {
        Some(error) => Err(error),
        None => Ok(collector.dependencies),
    }
}

/// Finds `local alias = require("path")` bindings and their target modules.
fn alias_imports(
    block: &Block,
    model: &SemanticModel,
    dependencies: &HashMap<String, ModuleId>,
) -> Vec<(crate::semantic::SymbolId, ModuleId)> {
    let mut bindings = Vec::new();

    for statement in block.iter_statements() {
        let Statement::LocalAssign(assignment) = statement else {
            continue;
        };
        if assignment.variables_len() != 1 || assignment.values_len() != 1 {
            continue;
        }
        let Some(Expression::Call(call)) = assignment.iter_values().next() else {
            continue;
        };
        let RequireCall::Path { path } = classify_require_call(call, model) else {
            continue;
        };
        let Some(&target) = dependencies.get(&path) else {
            continue;
        };
        let variable = assignment
            .iter_variables()
            .next()
            .expect("the assignment was checked to have one variable");
        if let Some(alias) = model.resolve_identifier(variable.get_identifier()) {
            bindings.push((alias, target));
        }
    }

    bindings
}
