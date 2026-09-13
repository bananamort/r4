use std::fmt;

use crate::diagnostic::{Diagnostic, TextSpan};
use crate::nodes::BlockIdentity;

use super::scope::ScopeKind;

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
    /// The byte range of the offending construct, when token data is available.
    pub span: Option<TextSpan>,
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
            span: None,
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
            span: None,
        }
    }

    pub(crate) fn undefined_label(name: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::UndefinedLabel {
                name: name.to_string(),
            },
            line,
            span: None,
        }
    }

    pub(crate) fn goto_into_local_scope(label: &str, symbol: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::GotoIntoLocalScope {
                label: label.to_string(),
                symbol: symbol.to_string(),
            },
            line,
            span: None,
        }
    }

    pub(crate) fn unresolved_type(name: &str, line: Option<usize>) -> Self {
        Self {
            kind: SemanticErrorKind::UnresolvedType {
                name: name.to_string(),
            },
            line,
            span: None,
        }
    }

    /// Attaches the byte range of the offending construct.
    pub(crate) fn with_span(mut self, span: Option<TextSpan>) -> Self {
        self.span = span;
        self
    }

    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match &self.kind {
            SemanticErrorKind::UndeclaredSymbol { .. } => "semantic/undeclared-symbol",
            SemanticErrorKind::DuplicateLabel { .. } => "semantic/duplicate-label",
            SemanticErrorKind::UndefinedLabel { .. } => "semantic/undefined-label",
            SemanticErrorKind::GotoIntoLocalScope { .. } => "semantic/goto-into-local-scope",
            SemanticErrorKind::UnresolvedType { .. } => "semantic/unresolved-type",
        }
    }

    /// Converts this error into a structured [`Diagnostic`].
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.to_string())
            .with_code(self.code())
            .with_optional_span(self.span)
    }
}

impl From<SemanticError> for Diagnostic {
    fn from(error: SemanticError) -> Self {
        error.to_diagnostic()
    }
}

/// An error produced by a node-keyed semantic query.
///
/// Node-keyed queries never guess: a node that was not analyzed, or that does
/// not belong to this model, is reported instead of returning a silent
/// fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticQueryError {
    /// The block has no structural identity, so it was never analyzed.
    UnidentifiedBlock,
    /// The block identity is not present in this model.
    UnknownBlock {
        /// The block's identity.
        identity: BlockIdentity,
    },
    /// The block belongs to a different analysis run.
    ForeignBlock {
        /// The block's identity.
        identity: BlockIdentity,
    },
    /// No enclosing scope of the requested kind exists for the block.
    NoEnclosingScope {
        /// The block whose scopes were searched.
        identity: BlockIdentity,
        /// The scope kind that was requested.
        kind: ScopeKind,
    },
    /// The identifier does not resolve to a runtime symbol.
    UnboundIdentifier {
        /// The identifier's name.
        name: String,
    },
    /// The identifier does not resolve to a goto label.
    UnboundLabel {
        /// The identifier's name.
        name: String,
    },
    /// The identifier does not resolve to a type alias.
    UnboundType {
        /// The identifier's name.
        name: String,
    },
    /// The identifier carries no binding from this model.
    UnboundOccurrence {
        /// The identifier's name.
        name: String,
    },
}

impl fmt::Display for SemanticQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticQueryError::UnidentifiedBlock => {
                write!(f, "the block has no structural identity")
            }
            SemanticQueryError::UnknownBlock { identity } => {
                write!(f, "block {:?} does not belong to this model", identity)
            }
            SemanticQueryError::ForeignBlock { identity } => write!(
                f,
                "block {:?} belongs to a different analysis run",
                identity
            ),
            SemanticQueryError::NoEnclosingScope { identity, kind } => {
                write!(f, "block {:?} has no enclosing {:?} scope", identity, kind)
            }
            SemanticQueryError::UnboundIdentifier { name } => {
                write!(f, "identifier `{}` is not bound to a symbol", name)
            }
            SemanticQueryError::UnboundLabel { name } => {
                write!(f, "identifier `{}` is not bound to a label", name)
            }
            SemanticQueryError::UnboundType { name } => {
                write!(f, "identifier `{}` is not bound to a type", name)
            }
            SemanticQueryError::UnboundOccurrence { name } => {
                write!(
                    f,
                    "identifier `{}` carries no binding from this model",
                    name
                )
            }
        }
    }
}

impl std::error::Error for SemanticQueryError {}

impl SemanticQueryError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            SemanticQueryError::UnidentifiedBlock => "query/unidentified-block",
            SemanticQueryError::UnknownBlock { .. } => "query/unknown-block",
            SemanticQueryError::ForeignBlock { .. } => "query/foreign-block",
            SemanticQueryError::NoEnclosingScope { .. } => "query/no-enclosing-scope",
            SemanticQueryError::UnboundIdentifier { .. } => "query/unbound-identifier",
            SemanticQueryError::UnboundLabel { .. } => "query/unbound-label",
            SemanticQueryError::UnboundType { .. } => "query/unbound-type",
            SemanticQueryError::UnboundOccurrence { .. } => "query/unbound-occurrence",
        }
    }

    /// Converts this error into a structured diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.to_string()).with_code(self.code())
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
