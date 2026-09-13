//! Structured diagnostics and source locations.
//!
//! Errors produced by the parser, the semantic analyzer, the control flow
//! builder and the module layer can be converted into [`Diagnostic`] values.
//! A diagnostic carries a severity, a human readable message, an optional
//! stable error code and, when source positions are available, both a byte
//! [`TextSpan`] and a [`LinePositionSpan`].
//!
//! This module is dependency free so that every other layer (including
//! [`crate::nodes`]) can attach locations without creating cycles.

use std::fmt;

/// The severity of a [`Diagnostic`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// An error that prevents the operation from succeeding.
    Error,
    /// A warning about suspicious but valid input.
    Warning,
    /// Additional information.
    Info,
}

impl Severity {
    /// Returns the lowercase name of this severity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A half-open byte range into a source string.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct TextSpan {
    /// The inclusive start byte offset.
    pub start: usize,
    /// The exclusive end byte offset.
    pub end: usize,
}

impl TextSpan {
    /// Creates a span from a start and an end byte offset.
    #[inline]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the length of this span in bytes.
    #[inline]
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether this span is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Returns whether `offset` falls inside this span.
    #[inline]
    pub const fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }
}

/// A one-based line and column position.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct LinePosition {
    /// The one-based line number.
    pub line: usize,
    /// The one-based column, counted in Unicode scalar values.
    pub column: usize,
}

impl LinePosition {
    /// Creates a line position.
    #[inline]
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for LinePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A span between two [`LinePosition`]s.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct LinePositionSpan {
    /// The start position.
    pub start: LinePosition,
    /// The end position.
    pub end: LinePosition,
}

impl LinePositionSpan {
    /// Creates a line position span.
    #[inline]
    pub const fn new(start: LinePosition, end: LinePosition) -> Self {
        Self { start, end }
    }
}

impl fmt::Display for LinePositionSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

/// Computes the one-based line position of a byte offset in `source`.
pub fn line_position(source: &str, offset: usize) -> LinePosition {
    let offset = offset.min(source.len());
    let mut line = 1;
    let mut column = 1;

    for (index, character) in source.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    LinePosition::new(line, column)
}

/// Computes the line position span of a byte range in `source`.
pub fn line_position_span(source: &str, span: TextSpan) -> LinePositionSpan {
    LinePositionSpan::new(
        line_position(source, span.start),
        line_position(source, span.end),
    )
}

/// A structured compiler message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The severity of the message.
    pub severity: Severity,
    /// The human readable message.
    pub message: String,
    /// A stable machine readable error code, when available.
    pub code: Option<&'static str>,
    /// The offending byte range, when source positions are available.
    pub span: Option<TextSpan>,
    /// The offending line and column range, when available.
    pub location: Option<LinePositionSpan>,
}

impl Diagnostic {
    /// Creates an error diagnostic with the given message.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Creates a warning diagnostic with the given message.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// Creates an information diagnostic with the given message.
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(Severity::Info, message)
    }

    /// Creates a diagnostic with an explicit severity.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
            code: None,
            span: None,
            location: None,
        }
    }

    /// Attaches a stable error code.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Attaches a byte span.
    pub fn with_span(mut self, span: TextSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Attaches a byte span when `span` is `Some`.
    pub fn with_optional_span(mut self, span: Option<TextSpan>) -> Self {
        self.span = span;
        self
    }

    /// Attaches a line position span.
    pub fn with_location(mut self, location: LinePositionSpan) -> Self {
        self.location = Some(location);
        self
    }

    /// Attaches a line position span when `location` is `Some`.
    pub fn with_optional_location(mut self, location: Option<LinePositionSpan>) -> Self {
        self.location = location;
        self
    }

    /// Resolves the line position of this diagnostic from its source, when it
    /// has a byte span but no line position yet.
    pub fn resolve_location(mut self, source: &str) -> Self {
        if self.location.is_none() {
            if let Some(span) = self.span {
                self.location = Some(line_position_span(source, span));
            }
        }
        self
    }

    /// Replaces the message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.severity)?;
        if let Some(code) = self.code {
            write!(f, "[{}] ", code)?;
        }
        write!(f, "{}", self.message)?;
        if let Some(location) = self.location {
            write!(f, " at {}", location)?;
        } else if let Some(span) = self.span {
            write!(f, " at bytes {}..{}", span.start, span.end)?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn computes_line_positions_across_lines() {
        let source = "local x = 1\nprint(x)\n";
        let span = TextSpan::new(12, 17);
        let location = line_position_span(source, span);
        assert_eq!(location.start, LinePosition::new(2, 1));
        assert_eq!(location.end, LinePosition::new(2, 6));
    }

    #[test]
    fn counts_columns_in_unicode_scalar_values() {
        let source = "é = 1";
        let location = line_position(source, "é".len());
        assert_eq!(location, LinePosition::new(1, 2));
    }
}
