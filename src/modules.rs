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

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::nodes::{
    Arguments, Block, Expression, FunctionCall, Prefix, Statement, StringExpression,
};
use crate::parser::ParserError;
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{
    ModuleError, ModuleId, MultiModuleModel, SemanticAnalyzer, SemanticError, SemanticModel,
};
use crate::Parser;

/// The name of the global `require` function.
pub const REQUIRE_NAME: &str = "require";

/// The Roblox global naming the script currently being analyzed.
const SCRIPT_GLOBAL: &str = "script";

/// The Roblox global naming the data-model root.
const GAME_GLOBAL: &str = "game";

/// An error produced while loading module source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleLoadError {
    /// The provider found the module but could not read its source.
    Read {
        /// The normalized key that was requested.
        key: String,
        /// The underlying filesystem message.
        message: String,
    },
}

impl ModuleLoadError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            ModuleLoadError::Read { .. } => "module/read",
        }
    }
}

impl std::fmt::Display for ModuleLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleLoadError::Read { key, message } => {
                write!(f, "could not read module `{key}`: {message}")
            }
        }
    }
}

impl std::error::Error for ModuleLoadError {}

/// Provides the source text of a module by its normalized key.
pub trait ModuleProvider {
    /// Returns the module source.
    ///
    /// Returns `Ok(None)` when the module does not exist and
    /// `Err(ModuleLoadError)` when it exists but its source could not be read
    /// (for example a permission error or non-UTF-8 bytes), so a read failure
    /// is never mistaken for a missing module.
    fn load(&self, key: &str) -> Result<Option<String>, ModuleLoadError>;
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
    fn load(&self, key: &str) -> Result<Option<String>, ModuleLoadError> {
        Ok(self.modules.get(key).cloned())
    }
}

/// A [`ModuleProvider`] backed by a real directory tree.
///
/// A normalized module key such as `pkg/util` maps to `<root>/pkg/util.<ext>`
/// (trying each configured extension, defaulting to `lua` then `luau`), or to a
/// folder module `<root>/pkg/util/<folder>.<ext>` (defaulting to `init`). Keys
/// that would escape `root` through `..` components are rejected, so a
/// malicious `require` cannot read arbitrary files.
#[derive(Clone, Debug)]
pub struct FileSystemModuleProvider {
    root: PathBuf,
    extensions: Vec<String>,
    module_folders: Vec<String>,
}

impl FileSystemModuleProvider {
    /// Creates a provider resolving `<root>/<key>.lua` and `<root>/<key>.luau`,
    /// falling back to `<root>/<key>/init.lua` and `<root>/<key>/init.luau`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            extensions: vec!["lua".to_owned(), "luau".to_owned()],
            module_folders: vec!["init".to_owned()],
        }
    }

    /// Creates a provider restricted to a single extension.
    pub fn with_extension(root: impl Into<PathBuf>, extension: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            extensions: vec![extension.into()],
            module_folders: vec!["init".to_owned()],
        }
    }

    /// Replaces the list of file extensions that are tried, in order.
    pub fn with_extensions(
        mut self,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.extensions = extensions.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the folder module file names that are tried, in order.
    pub fn with_module_folders(
        mut self,
        folders: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.module_folders = folders.into_iter().map(Into::into).collect();
        self
    }

    /// Returns the root directory.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the first configured file extension.
    #[inline]
    pub fn extension(&self) -> &str {
        self.extensions.first().map(String::as_str).unwrap_or("")
    }

    /// Returns the extensions that are tried, in order.
    #[inline]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// Returns every candidate path for a module key, in resolution order.
    ///
    /// Returns an empty list when the key would escape the root.
    pub fn candidate_paths(&self, key: &str) -> Vec<PathBuf> {
        let Some(base) = self.safe_join(key) else {
            return Vec::new();
        };

        let mut candidates = Vec::new();
        if Path::new(key).extension().is_some() {
            candidates.push(base.clone());
        }
        for extension in &self.extensions {
            candidates.push(base.with_extension(extension));
        }
        for folder in &self.module_folders {
            for extension in &self.extensions {
                candidates.push(base.join(folder).with_extension(extension));
            }
        }
        candidates
    }

    /// Resolves a module key to the first existing candidate path.
    pub fn path_for(&self, key: &str) -> Option<PathBuf> {
        self.candidate_paths(key)
            .into_iter()
            .find(|candidate| candidate.is_file())
    }

    fn safe_join(&self, key: &str) -> Option<PathBuf> {
        let mut path = self.root.clone();
        for component in Path::new(key).components() {
            match component {
                Component::Normal(part) => path.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(path)
    }
}

impl ModuleProvider for FileSystemModuleProvider {
    fn load(&self, key: &str) -> Result<Option<String>, ModuleLoadError> {
        let Some(path) = self.path_for(key) else {
            return Ok(None);
        };
        match std::fs::read_to_string(&path) {
            Ok(source) => Ok(Some(source)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ModuleLoadError::Read {
                key: key.to_owned(),
                message: error.to_string(),
            }),
        }
    }
}

/// A parsed and analyzed module source.
#[derive(Clone, Debug)]
pub struct AnalyzedModule {
    /// The source the module was parsed from.
    pub source: String,
    /// The parsed syntax tree.
    pub block: Block,
    /// The semantic model of [`Self::block`].
    pub model: SemanticModel,
}

/// A content-addressed cache of analyzed module sources.
///
/// Resolution parses and analyzes every discovered module. When a graph is
/// resolved repeatedly (for example while a single file is edited), unchanged
/// dependencies hit this cache instead of being re-parsed and re-analyzed. The
/// cache is keyed by a hash of the source and re-validated by comparing the
/// stored source, so a hash collision can never reuse the wrong analysis.
#[derive(Debug, Default)]
pub struct AnalysisCache {
    entries: HashMap<u64, Arc<AnalyzedModule>>,
}

impl AnalysisCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of cached analyses.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the cache is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes every cached analysis.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the analysis of `source`, if it is cached.
    pub fn get(&self, source: &str) -> Option<Arc<AnalyzedModule>> {
        self.entries
            .get(&source_key(source))
            .filter(|module| module.source == source)
            .cloned()
    }

    /// Stores an analysis, returning the shared handle.
    pub fn insert(&mut self, module: AnalyzedModule) -> Arc<AnalyzedModule> {
        let module = Arc::new(module);
        self.entries
            .insert(source_key(&module.source), module.clone());
        module
    }
}

fn source_key(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

/// One step of a Roblox/Luau instance path in a `require` argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstanceSegment {
    /// `.Parent` — move to the containing folder.
    Parent,
    /// `.Child` / `:FindFirstChild("Child")` / `:WaitForChild("Child")`.
    Child(String),
    /// `game:GetService("Name")`.
    Service(String),
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
    /// The call is a `require` call with a Roblox/Luau instance path argument,
    /// such as `require(script.Parent.Util)`.
    Instance {
        /// The instance path, relative to the requiring script.
        segments: Vec<InstanceSegment>,
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
                (Some(expression), None) => match instance_segments(expression, model) {
                    Some(segments) => RequireCall::Instance { segments },
                    None => RequireCall::NonLiteral,
                },
                _ => RequireCall::NonLiteral,
            }
        }
        Arguments::Table(_) => RequireCall::NonLiteral,
    }
}

/// Extracts the instance path of a Roblox/Luau `require` argument.
///
/// Only paths rooted at the global `script` are recognized; `game:GetService`
/// and any other data-model root depend on a project mapping and are reported
/// as non-literal (fail fast) instead of being guessed. Every root is resolved
/// through the semantic model, so a local that shadows `script` or `game` is
/// never mistaken for the Roblox global.
fn instance_segments(
    expression: &Expression,
    model: &SemanticModel,
) -> Option<Vec<InstanceSegment>> {
    match expression {
        Expression::Identifier(identifier)
            if model.identifier_resolves_to_global(identifier, SCRIPT_GLOBAL) =>
        {
            Some(Vec::new())
        }
        Expression::Field(field) => {
            let mut segments = prefix_instance_segments(field.get_prefix(), model)?;
            segments.push(instance_field_segment(field.get_field().get_name()));
            Some(segments)
        }
        Expression::Call(call) => instance_call_segments(call, model),
        Expression::Parenthese(parenthese) => {
            instance_segments(parenthese.inner_expression(), model)
        }
        Expression::Index(index) => {
            let name = match index.get_index() {
                Expression::String(string) => path_of_string(string)?,
                _ => return None,
            };
            let mut segments = prefix_instance_segments(index.get_prefix(), model)?;
            segments.push(InstanceSegment::Child(name));
            Some(segments)
        }
        _ => None,
    }
}

fn prefix_instance_segments(
    prefix: &Prefix,
    model: &SemanticModel,
) -> Option<Vec<InstanceSegment>> {
    match prefix {
        Prefix::Identifier(identifier)
            if model.identifier_resolves_to_global(identifier, SCRIPT_GLOBAL) =>
        {
            Some(Vec::new())
        }
        Prefix::Field(field) => {
            let mut segments = prefix_instance_segments(field.get_prefix(), model)?;
            segments.push(instance_field_segment(field.get_field().get_name()));
            Some(segments)
        }
        Prefix::Call(call) => instance_call_segments(call, model),
        Prefix::Index(index) => {
            let name = match index.get_index() {
                Expression::String(string) => path_of_string(string)?,
                _ => return None,
            };
            let mut segments = prefix_instance_segments(index.get_prefix(), model)?;
            segments.push(InstanceSegment::Child(name));
            Some(segments)
        }
        Prefix::Parenthese(parenthese) => instance_segments(parenthese.inner_expression(), model),
        _ => None,
    }
}

fn instance_call_segments(
    call: &FunctionCall,
    model: &SemanticModel,
) -> Option<Vec<InstanceSegment>> {
    match call.get_method() {
        Some(method)
            if method.get_name() == "FindFirstChild" || method.get_name() == "WaitForChild" =>
        {
            let name = single_string_argument(call)?;
            let mut segments = prefix_instance_segments(call.get_prefix(), model)?;
            segments.push(InstanceSegment::Child(name));
            Some(segments)
        }
        Some(method) if method.get_name() == "GetService" => {
            let name = single_string_argument(call)?;
            let Prefix::Identifier(identifier) = call.get_prefix() else {
                return None;
            };
            if !model.identifier_resolves_to_global(identifier, GAME_GLOBAL) {
                return None;
            }
            Some(vec![InstanceSegment::Service(name)])
        }
        _ => None,
    }
}

fn instance_field_segment(name: &str) -> InstanceSegment {
    if name == "Parent" {
        InstanceSegment::Parent
    } else {
        InstanceSegment::Child(name.to_owned())
    }
}

fn single_string_argument(call: &FunctionCall) -> Option<String> {
    let Arguments::Tuple(tuple) = call.get_arguments() else {
        return None;
    };
    let mut values = tuple.iter_values();
    match (values.next(), values.next()) {
        (Some(Expression::String(string)), None) => path_of_string(string),
        _ => None,
    }
}

/// Resolves an instance path relative to the key of the requiring module.
///
/// A module key is the Rojo file layout: a `Child` extends the path by one
/// segment, and `Parent` removes the last segment. `game:GetService` cannot be
/// resolved without a project mapping, so it returns `None`.
pub fn resolve_instance_path(key: &str, segments: &[InstanceSegment]) -> Option<String> {
    let mut current = key.to_owned();
    for segment in segments {
        match segment {
            InstanceSegment::Parent => {
                current = parent_directory(&current);
            }
            InstanceSegment::Child(name) => {
                current = if current.is_empty() {
                    name.clone()
                } else {
                    format!("{current}/{name}")
                };
            }
            InstanceSegment::Service(_) => return None,
        }
    }
    Some(current)
}

fn resolves_to_require(model: &SemanticModel, identifier: &crate::nodes::Identifier) -> bool {
    model.identifier_resolves_to_global(identifier, REQUIRE_NAME)
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
///
/// Supports the Luau RFC `@self` prefix, which is relative to the requiring
/// module's own directory, alongside the classic `./` and `../` prefixes.
pub(crate) fn normalize_require_path(path: &str, base_directory: &str) -> String {
    if let Some(rest) = path.strip_prefix("@self") {
        if rest.is_empty() {
            return base_directory.to_owned();
        }
        if let Some(rest) = rest.strip_prefix('/') {
            return join_relative(rest, base_directory);
        }
        return path.to_owned();
    }

    if !path.starts_with("./") && !path.starts_with("../") {
        return path.to_owned();
    }

    join_relative(path, base_directory)
}

fn join_relative(path: &str, base_directory: &str) -> String {
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
    /// A provider found a module but could not read its source.
    ModuleLoad {
        /// The load error.
        error: ModuleLoadError,
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
            ResolveError::ModuleLoad { error } => write!(f, "{}", error),
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
            ResolveError::ModuleLoad { .. } => "module/read",
            ResolveError::CyclicRequire { .. } => "module/cyclic-require",
            ResolveError::Parse { .. } => "module/parse",
            ResolveError::Analysis { .. } => "module/analysis",
            ResolveError::NonLiteralRequire { .. } => "module/non-literal-require",
            ResolveError::Interface(_) => "module/interface",
        }
    }

    /// Converts this error into a structured diagnostic.
    pub fn to_diagnostic(&self) -> crate::diagnostic::Diagnostic {
        match self {
            ResolveError::Interface(error) => error.to_diagnostic(),
            _ => crate::diagnostic::Diagnostic::error(self.to_string()).with_code(self.code()),
        }
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
    ///
    /// Each resolved dependency is parsed and analyzed with a throwaway cache.
    /// Use [`Self::resolve_with_cache`] to reuse analyses across resolutions.
    pub fn resolve(
        &self,
        root_block: Block,
        root_model: SemanticModel,
        root_key: &str,
    ) -> Result<ResolvedModules, ResolveError> {
        let mut cache = AnalysisCache::default();
        self.resolve_with_cache(root_block, root_model, root_key, &mut cache)
    }

    /// Resolves the graph, reusing cached analyses for unchanged sources.
    pub fn resolve_with_cache(
        &self,
        mut root_block: Block,
        root_model: SemanticModel,
        root_key: &str,
        cache: &mut AnalysisCache,
    ) -> Result<ResolvedModules, ResolveError> {
        let mut state = ResolveState {
            provider: self.provider,
            graph: MultiModuleModel::new(),
            by_key: HashMap::new(),
            visiting: Vec::new(),
            order: Vec::new(),
            cache,
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
    cache: &'a mut AnalysisCache,
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
                let source = match self.provider.load(&normalized) {
                    Ok(Some(source)) => source,
                    Ok(None) => {
                        return Err(ResolveError::MissingModule {
                            path: normalized.clone(),
                        })
                    }
                    Err(error) => return Err(ResolveError::ModuleLoad { error }),
                };

                // Reuse a cached analysis of the same source when possible.
                let analyzed = match self.cache.get(&source) {
                    Some(cached) => cached,
                    None => {
                        let block = Parser::default().parse(&source).map_err(|error| {
                            ResolveError::Parse {
                                path: normalized.clone(),
                                error,
                            }
                        })?;
                        let model = SemanticAnalyzer::new().analyze(&block).map_err(|error| {
                            ResolveError::Analysis {
                                path: normalized.clone(),
                                error,
                            }
                        })?;
                        self.cache.insert(AnalyzedModule {
                            source,
                            block,
                            model,
                        })
                    }
                };

                let mut block = analyzed.block.clone();
                let model = analyzed.model.clone();

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
            for (alias, target) in alias_imports(
                &module.block,
                &module.model,
                &module.key,
                &module.dependencies,
            ) {
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
                RequireCall::Instance { segments } => {
                    match resolve_instance_path(self.module_key, &segments) {
                        Some(resolved) => self.dependencies.push((resolved.clone(), resolved)),
                        None => {
                            self.error = Some(ResolveError::NonLiteralRequire {
                                module: self.module_key.to_owned(),
                                line: require_call_line(call),
                            });
                        }
                    }
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

/// Finds `local alias = require(...)` bindings and their target modules.
fn alias_imports(
    block: &Block,
    model: &SemanticModel,
    module_key: &str,
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
        let key = match classify_require_call(call, model) {
            RequireCall::Path { path } => path,
            RequireCall::Instance { segments } => {
                match resolve_instance_path(module_key, &segments) {
                    Some(resolved) => resolved,
                    None => continue,
                }
            }
            RequireCall::NotRequire | RequireCall::NonLiteral => continue,
        };
        let Some(&target) = dependencies.get(&key) else {
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
