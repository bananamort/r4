use std::fmt;

/// Options controlling semantic analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemanticOptions {
    /// When enabled, reading or writing an identifier that does not resolve to a
    /// local variable is an error instead of an implicit global access.
    pub strict_globals: bool,
    /// When enabled, a type name that resolves neither to a type declaration,
    /// a generic type parameter, nor a Luau built-in type is an error.
    pub strict_types: bool,
}

/// An error produced by semantic analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticError {
    /// The precise reason of the error.
    pub kind: SemanticErrorKind,
    /// The source line of the offending construct, when token data is available.
    pub line: Option<usize>,
}

/// The different semantic errors that can be reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticErrorKind {
    /// A name was used but was neither declared as a local nor allowed as a global
    /// (only produced with [`SemanticOptions::strict_globals`]).
    UndeclaredSymbol {
        /// The offending name.
        name: String,
    },
    /// A label is declared twice in the same active function scope.
    DuplicateLabel {
        /// The duplicated label name.
        name: String,
        /// The source line of the previous declaration, if known.
        previous_line: Option<usize>,
    },
    /// A goto does not resolve to any visible label.
    UndefinedLabel {
        /// The targeted label name.
        name: String,
    },
    /// A goto jumps into the scope of a local variable.
    GotoIntoLocalScope {
        /// The targeted label name.
        label: String,
        /// The local variable whose scope would be entered.
        symbol: String,
    },
    /// A type name resolves neither to a type declaration, a generic type
    /// parameter, nor a built-in Luau type (only produced with
    /// [`SemanticOptions::strict_types`]).
    UnresolvedType {
        /// The offending type name.
        name: String,
    },
}

impl SemanticError {
    pub(crate) fn undeclared(name: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::UndeclaredSymbol {
                name: name.to_string(),
            },
            line,
        }
    }

    pub(crate) fn duplicate_label(
        name: &str,
        previous_line: Option<usize>,
        line: Option<usize>,
    ) -> Self {
        Self {
            kind: SemanticErrorKind::DuplicateLabel {
                name: name.to_string(),
                previous_line,
            },
            line,
        }
    }

    pub(crate) fn undefined_label(name: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::UndefinedLabel {
                name: name.to_string(),
            },
            line,
        }
    }

    pub(crate) fn goto_into_local_scope(label: &str, symbol: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::GotoIntoLocalScope {
                label: label.to_string(),
                symbol: symbol.to_string(),
            },
            line,
        }
    }

    pub(crate) fn unresolved_type(name: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::UnresolvedType {
                name: name.to_string(),
            },
            line,
        }
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "line {}: ", line)?;
        }
        match &self.kind {
            SemanticErrorKind::UndeclaredSymbol { name } => {
                write!(f, "undeclared symbol `{}`", name)
            }
            SemanticErrorKind::DuplicateLabel {
                name,
                previous_line,
            } => match previous_line {
                Some(previous_line) => write!(
                    f,
                    "label `{}` already defined at line {}",
                    name, previous_line
                ),
                None => write!(f, "label `{}` already defined", name),
            },
            SemanticErrorKind::UndefinedLabel { name } => {
                write!(f, "no visible label `{}` for goto", name)
            }
            SemanticErrorKind::GotoIntoLocalScope { label, symbol } => write!(
                f,
                "goto `{}` jumps into the scope of local `{}`",
                label, symbol
            ),
            SemanticErrorKind::UnresolvedType { name } => {
                write!(f, "unresolved type `{}`", name)
            }
        }
    }
}

impl std::error::Error for SemanticError {}

/// A successful semantic analysis result.
pub type SemanticResult<T> = Result<T, SemanticError>;
