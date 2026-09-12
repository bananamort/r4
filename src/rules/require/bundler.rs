use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::nodes::{
    AssignStatement, Block, Expression, FunctionAssignment, FunctionCall, FunctionExpression,
    IfStatement, IndexExpression, Prefix, ReturnStatement, Statement, StringExpression,
    TypedIdentifier, Variable, VariableAssignment,
};
use crate::parser::{Parser, ParserError};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{SemanticAnalyzer, SemanticError, SemanticModel};

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
    }

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

    let mut statements = definitions;
    statements.extend(block.take_statements());
    block.set_statements(statements);

    Ok(module_count)
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
