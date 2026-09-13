use std::collections::HashSet;
use std::fmt;

use crate::nodes::{Block, Expression, FunctionCall, StringExpression};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

use super::convert::{convert_require, ConvertRequireError, RequirePathConverter};
use super::{classify_require_call, RequireCall};

/// An error produced while applying path aliases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasError {
    /// An alias prefix or target is empty or malformed.
    MalformedAlias {
        /// The alias prefix.
        alias: String,
    },
    /// The same alias prefix was configured twice.
    DuplicateAlias {
        /// The duplicated alias prefix.
        alias: String,
    },
    /// An alias target points at another alias, which can be recursive.
    NestedAlias {
        /// The alias prefix.
        alias: String,
        /// The offending target.
        target: String,
    },
    /// An aliased require path has no mapping.
    UnresolvedAlias {
        /// The unresolved path.
        path: String,
    },
    /// A `require` call could not be converted.
    Convert(ConvertRequireError),
}

impl fmt::Display for AliasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AliasError::MalformedAlias { alias } => {
                write!(f, "malformed path alias `{}`", alias)
            }
            AliasError::DuplicateAlias { alias } => {
                write!(f, "path alias `{}` is configured more than once", alias)
            }
            AliasError::NestedAlias { alias, target } => write!(
                f,
                "path alias `{}` targets `{}`, which is itself an alias",
                alias, target
            ),
            AliasError::UnresolvedAlias { path } => {
                write!(f, "no path alias matches `{}`", path)
            }
            AliasError::Convert(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for AliasError {}

#[derive(Debug)]
struct AliasEntry {
    prefix: String,
    target: String,
}

/// A [`RequirePathConverter`] that rewrites aliased require paths.
///
/// An alias maps a prefix such as `@pkg` to a normalized path such as
/// `./vendor/pkg`; `require("@pkg/foo")` becomes `require("./vendor/pkg/foo")`.
/// Prefixes are validated when the converter is built: empty or malformed
/// entries, duplicate prefixes, and targets that point at another alias (a
/// recursive mapping) are rejected.
#[derive(Debug)]
pub struct AliasRequireConverter {
    aliases: Vec<AliasEntry>,
}

impl AliasRequireConverter {
    /// Builds a converter from `(prefix, target)` pairs.
    pub fn new<I, S, T>(aliases: I) -> Result<Self, AliasError>
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: Into<String>,
    {
        let mut entries: Vec<AliasEntry> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (prefix, target) in aliases {
            let prefix = prefix.into();
            let target = target.into();

            if !prefix.starts_with('@') || prefix.len() < 2 || prefix.ends_with('/') {
                return Err(AliasError::MalformedAlias { alias: prefix });
            }
            if target.is_empty() {
                return Err(AliasError::MalformedAlias { alias: prefix });
            }
            if target.starts_with('@') {
                return Err(AliasError::NestedAlias {
                    alias: prefix,
                    target,
                });
            }
            if !seen.insert(prefix.clone()) {
                return Err(AliasError::DuplicateAlias { alias: prefix });
            }

            entries.push(AliasEntry { prefix, target });
        }

        // Longest prefix first so `@pkg/extra` wins over `@pkg`.
        entries.sort_by(|a, b| {
            b.prefix
                .len()
                .cmp(&a.prefix.len())
                .then_with(|| a.prefix.cmp(&b.prefix))
        });

        Ok(Self { aliases: entries })
    }

    /// Returns whether `path` uses the alias namespace.
    #[inline]
    pub fn is_aliased(path: &str) -> bool {
        path.starts_with('@')
    }

    /// Resolves `path` to a normalized require path, or `None` when no alias
    /// matches.
    pub fn resolve(&self, path: &str) -> Option<String> {
        let entry = self.aliases.iter().find(|entry| {
            path == entry.prefix
                || path
                    .strip_prefix(&entry.prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })?;

        let rest = &path[entry.prefix.len()..];
        Some(join_paths(&entry.target, rest))
    }
}

impl RequirePathConverter for AliasRequireConverter {
    fn convert(&self, path: &str) -> Option<Expression> {
        let resolved = self.resolve(path)?;
        Some(
            FunctionCall::from_name("require")
                .with_argument(StringExpression::from_value(resolved))
                .into(),
        )
    }
}

/// Applies `aliases` to every statically resolved `require` call.
///
/// Aliased paths (`@...`) that no mapping handles are a typed
/// [`AliasError::UnresolvedAlias`]; they are never silently skipped. Shadowed
/// `require` calls are not aliased at all because conversion resolves the
/// prefix through the model.
pub fn apply_path_aliases<I, S, T>(
    block: &mut Block,
    model: &SemanticModel,
    aliases: I,
) -> Result<usize, AliasError>
where
    I: IntoIterator<Item = (S, T)>,
    S: Into<String>,
    T: Into<String>,
{
    let converter = AliasRequireConverter::new(aliases)?;
    validate_aliased_paths(block, model, &converter)?;
    convert_require(block, model, &converter).map_err(AliasError::Convert)
}

fn validate_aliased_paths(
    block: &mut Block,
    model: &SemanticModel,
    converter: &AliasRequireConverter,
) -> Result<(), AliasError> {
    struct Validator<'a> {
        model: &'a SemanticModel,
        converter: &'a AliasRequireConverter,
        error: Option<AliasError>,
    }

    impl Validator<'_> {
        fn check(&mut self, call: &FunctionCall) {
            if self.error.is_some() {
                return;
            }
            if let RequireCall::Path { path } = classify_require_call(call, self.model) {
                if AliasRequireConverter::is_aliased(&path)
                    && self.converter.resolve(&path).is_none()
                {
                    self.error = Some(AliasError::UnresolvedAlias { path });
                }
            }
        }
    }

    impl NodeProcessor for Validator<'_> {
        fn process_expression(&mut self, expression: &mut Expression) {
            if let Expression::Call(call) = expression {
                self.check(call);
            }
        }

        fn process_statement(&mut self, statement: &mut crate::nodes::Statement) {
            if let crate::nodes::Statement::Call(call) = statement {
                self.check(call);
            }
        }
    }

    let mut validator = Validator {
        model,
        converter,
        error: None,
    };
    DefaultVisitor::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn join_paths(target: &str, rest: &str) -> String {
    let relative = target.starts_with("./") || target.starts_with("../");

    let mut segments: Vec<&str> = Vec::new();
    for segment in target.split('/').chain(rest.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }

    let joined = segments.join("/");
    if relative {
        format!("./{joined}")
    } else {
        joined
    }
}
