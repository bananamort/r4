//! Structured `--!` directive comments.
//!
//! Luau source files may begin with `--!` directives such as `--!strict`,
//! `--!nonstrict`, `--!nocheck`, `--!native` or `--!optimize 2`. Unlike an
//! ordinary comment, a directive is structured: a name and an optional value.
//! [`Directive::parse`] recognizes the exact `--!` prefix (no whitespace is
//! allowed between `--`, `!` and the name) and [`Directives`] collects the
//! directives of a tree so an analysis can configure itself.
//!
//! Nothing here guesses: a comment that does not start with `--!` is not a
//! directive, and a malformed directive (`--!` with no name) is a typed error.

use std::fmt;

use crate::nodes::{Block, Trivia, TriviaError, TriviaKind};
use crate::process::collect_trivia;

/// A structured `--!` directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    /// The directive name (for example `strict`).
    pub name: String,
    /// The directive value, when one was written.
    pub value: Option<String>,
}

/// The analysis strictness a directive selects.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Strictness {
    /// `--!strict`: undeclared names and unknown types are errors.
    Strict,
    /// `--!nonstrict`: strict checking is disabled.
    NonStrict,
    /// `--!nocheck`: checking is disabled entirely.
    NoCheck,
}

impl Strictness {
    /// Returns a stable human-readable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Strictness::Strict => "strict",
            Strictness::NonStrict => "nonstrict",
            Strictness::NoCheck => "nocheck",
        }
    }
}

/// An error produced while reading or collecting directives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectiveError {
    /// The trivia's recorded range is not a valid slice of the source.
    InvalidRange {
        /// The start byte offset.
        start: usize,
        /// The end byte offset.
        end: usize,
        /// The length of the source.
        length: usize,
    },
    /// A directive contained no name (`--!`).
    MissingName,
    /// Two directives selected different strictness modes.
    ConflictingStrictness {
        /// The first strictness mode that was seen.
        first: Strictness,
        /// The conflicting strictness mode.
        second: Strictness,
    },
}

impl DirectiveError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            DirectiveError::InvalidRange { .. } => "directive/invalid-range",
            DirectiveError::MissingName => "directive/missing-name",
            DirectiveError::ConflictingStrictness { .. } => "directive/conflicting-strictness",
        }
    }
}

impl fmt::Display for DirectiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirectiveError::InvalidRange { start, end, length } => write!(
                f,
                "directive range {}..{} is not a valid slice of a {}-byte source",
                start, end, length
            ),
            DirectiveError::MissingName => write!(f, "a `--!` directive must have a name"),
            DirectiveError::ConflictingStrictness { first, second } => write!(
                f,
                "conflicting strictness directives `--!{}` and `--!{}`",
                first.as_str(),
                second.as_str()
            ),
        }
    }
}

impl std::error::Error for DirectiveError {}

impl From<TriviaError> for DirectiveError {
    fn from(error: TriviaError) -> Self {
        match error {
            TriviaError::InvalidRange { start, end, length } => {
                DirectiveError::InvalidRange { start, end, length }
            }
        }
    }
}

impl Directive {
    /// Parses a comment's text into a directive.
    ///
    /// Returns `Ok(None)` when the text is not a `--!` directive.
    pub fn parse(text: &str) -> Result<Option<Directive>, DirectiveError> {
        let Some(body) = text.strip_prefix("--!") else {
            return Ok(None);
        };
        let body = body.trim_end_matches(['\r', '\n']);
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = &body[..name_end];
        if name.is_empty() {
            return Err(DirectiveError::MissingName);
        }
        let value = body[name_end..].trim();
        Ok(Some(Directive {
            name: name.to_owned(),
            value: if value.is_empty() {
                None
            } else {
                Some(value.to_owned())
            },
        }))
    }

    /// Returns the strictness this directive selects, if any.
    pub fn strictness(&self) -> Option<Strictness> {
        match self.name.as_str() {
            "strict" => Some(Strictness::Strict),
            "nonstrict" => Some(Strictness::NonStrict),
            "nocheck" => Some(Strictness::NoCheck),
            _ => None,
        }
    }
}

impl Trivia {
    /// Parses this trivia as a `--!` directive.
    ///
    /// Returns `Ok(None)` for non-comment trivia and for comments that are not
    /// directives. A comment that starts with `--!` but has no name is a typed
    /// error; the recorded range is read fallibly against `source`.
    pub fn directive(&self, source: &str) -> Result<Option<Directive>, DirectiveError> {
        if self.kind() != TriviaKind::Comment {
            return Ok(None);
        }
        let text = self.try_read_with(source)?;
        Directive::parse(text)
    }
}

/// The directives of one source file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directives {
    directives: Vec<Directive>,
}

impl Directives {
    /// Creates an empty list of directives.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collects every `--!` directive from `block`.
    ///
    /// The traversal reads every piece of trivia, so a directive is found no
    /// matter where it is attached. Conflicting strictness directives are a
    /// typed error rather than a silent last-one-wins.
    pub fn from_block(block: &Block, source: &str) -> Result<Self, DirectiveError> {
        let mut directives = Vec::new();
        for trivia in collect_trivia(block) {
            if let Some(directive) = trivia.directive(source)? {
                directives.push(directive);
            }
        }

        let mut strictness: Option<Strictness> = None;
        for directive in &directives {
            if let Some(mode) = directive.strictness() {
                match strictness {
                    Some(first) if first != mode => {
                        return Err(DirectiveError::ConflictingStrictness {
                            first,
                            second: mode,
                        });
                    }
                    _ => strictness = Some(mode),
                }
            }
        }

        Ok(Self { directives })
    }

    /// Returns the number of directives.
    #[inline]
    pub fn len(&self) -> usize {
        self.directives.len()
    }

    /// Returns whether there are no directives.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.directives.is_empty()
    }

    /// Iterates over the directives in source order.
    pub fn iter(&self) -> impl Iterator<Item = &Directive> {
        self.directives.iter()
    }

    /// Returns the first directive with the given name.
    pub fn get(&self, name: &str) -> Option<&Directive> {
        self.directives
            .iter()
            .find(|directive| directive.name == name)
    }

    /// Returns the strictness selected by the directives, if any.
    pub fn strictness(&self) -> Option<Strictness> {
        self.directives.iter().find_map(Directive::strictness)
    }
}
