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
///
/// An offset past the end of the source resolves to the position of the end of
/// the source; the caller is responsible for validating spans when that
/// distinction matters (see [`DiagnosticFormatter`]).
pub fn line_position(source: &str, offset: usize) -> LinePosition {
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

/// A catalog entry describing a diagnostic code.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticDescriptor {
    /// The stable machine readable code.
    pub code: &'static str,
    /// The severity every diagnostic with this code has.
    pub severity: Severity,
    /// A short human readable title.
    pub title: &'static str,
}

impl DiagnosticDescriptor {
    /// Creates a descriptor.
    pub const fn new(code: &'static str, severity: Severity, title: &'static str) -> Self {
        Self {
            code,
            severity,
            title,
        }
    }
}

/// Every diagnostic code the engine can emit.
pub static DIAGNOSTIC_CATALOG: &[DiagnosticDescriptor] = &[
    DiagnosticDescriptor::new(
        "semantic/undeclared-symbol",
        Severity::Error,
        "Undeclared symbol",
    ),
    DiagnosticDescriptor::new(
        "semantic/duplicate-label",
        Severity::Error,
        "Duplicate label",
    ),
    DiagnosticDescriptor::new(
        "semantic/undefined-label",
        Severity::Error,
        "Undefined label",
    ),
    DiagnosticDescriptor::new(
        "semantic/goto-into-local-scope",
        Severity::Error,
        "Goto enters a local scope",
    ),
    DiagnosticDescriptor::new(
        "semantic/unresolved-type",
        Severity::Error,
        "Unresolved type",
    ),
    DiagnosticDescriptor::new("parse/syntax", Severity::Error, "Syntax error"),
    DiagnosticDescriptor::new("parse/conversion", Severity::Error, "Conversion error"),
    DiagnosticDescriptor::new("parse/panic", Severity::Error, "Parser panic"),
    DiagnosticDescriptor::new("module/missing", Severity::Error, "Missing module"),
    DiagnosticDescriptor::new("module/cyclic-require", Severity::Error, "Cyclic require"),
    DiagnosticDescriptor::new("module/parse", Severity::Error, "Module parse error"),
    DiagnosticDescriptor::new("module/analysis", Severity::Error, "Module analysis error"),
    DiagnosticDescriptor::new(
        "module/non-literal-require",
        Severity::Error,
        "Non-literal require",
    ),
    DiagnosticDescriptor::new(
        "module/interface",
        Severity::Error,
        "Module interface error",
    ),
    DiagnosticDescriptor::new(
        "module/unbound-export",
        Severity::Error,
        "Unbound exported name",
    ),
    DiagnosticDescriptor::new("module/unknown-export", Severity::Error, "Unknown export"),
    DiagnosticDescriptor::new(
        "module/unknown-import-alias",
        Severity::Error,
        "Unknown import alias",
    ),
    DiagnosticDescriptor::new(
        "module/unknown-imported-type",
        Severity::Error,
        "Unknown imported type",
    ),
    DiagnosticDescriptor::new(
        "module/ambiguous-imported-type",
        Severity::Error,
        "Ambiguous imported type",
    ),
    DiagnosticDescriptor::new(
        "query/unidentified-block",
        Severity::Error,
        "Unidentified block",
    ),
    DiagnosticDescriptor::new("query/unknown-block", Severity::Error, "Unknown block"),
    DiagnosticDescriptor::new("query/foreign-block", Severity::Error, "Foreign block"),
    DiagnosticDescriptor::new(
        "query/no-enclosing-scope",
        Severity::Error,
        "No enclosing scope",
    ),
    DiagnosticDescriptor::new(
        "query/unbound-identifier",
        Severity::Error,
        "Unbound identifier",
    ),
    DiagnosticDescriptor::new("query/unbound-label", Severity::Error, "Unbound label"),
    DiagnosticDescriptor::new("query/unbound-type", Severity::Error, "Unbound type"),
    DiagnosticDescriptor::new(
        "query/unbound-occurrence",
        Severity::Error,
        "Unbound occurrence",
    ),
    DiagnosticDescriptor::new("query/missing-span", Severity::Error, "Missing token span"),
    DiagnosticDescriptor::new("compile/unknown-tree", Severity::Error, "Unknown tree"),
    DiagnosticDescriptor::new(
        "annotation/unknown-block",
        Severity::Error,
        "Unknown annotation block",
    ),
    DiagnosticDescriptor::new(
        "annotation/unknown-statement",
        Severity::Error,
        "Unknown annotation statement",
    ),
    DiagnosticDescriptor::new(
        "annotation/unknown-expression",
        Severity::Error,
        "Unknown annotation expression",
    ),
    DiagnosticDescriptor::new(
        "annotation/missing-condition",
        Severity::Error,
        "Missing annotation condition",
    ),
    DiagnosticDescriptor::new(
        "annotation/unknown-identifier",
        Severity::Error,
        "Unknown annotation identifier",
    ),
];

/// Returns the descriptor for a diagnostic code, if it is cataloged.
pub fn descriptor_for(code: &str) -> Option<&'static DiagnosticDescriptor> {
    DIAGNOSTIC_CATALOG
        .iter()
        .find(|descriptor| descriptor.code == code)
}

impl Diagnostic {
    /// Returns the catalog descriptor for this diagnostic's code, if any.
    pub fn descriptor(&self) -> Option<&'static DiagnosticDescriptor> {
        self.code.and_then(descriptor_for)
    }
}

/// An error produced while formatting a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticFormatError {
    /// The diagnostic code is not part of the catalog.
    UnknownCode {
        /// The unrecognized code.
        code: String,
    },
    /// The byte span does not fit the source or splits a character.
    InvalidSpan {
        /// The offending span.
        span: TextSpan,
        /// The length of the source in bytes.
        source_len: usize,
    },
    /// The line/column location does not fit the source.
    InvalidLocation {
        /// The offending location.
        location: LinePositionSpan,
    },
}

impl fmt::Display for DiagnosticFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticFormatError::UnknownCode { code } => {
                write!(f, "diagnostic code `{}` is not cataloged", code)
            }
            DiagnosticFormatError::InvalidSpan { span, source_len } => write!(
                f,
                "span {}..{} is out of bounds for a {}-byte source",
                span.start, span.end, source_len
            ),
            DiagnosticFormatError::InvalidLocation { location } => {
                write!(f, "location {} is out of bounds for the source", location)
            }
        }
    }
}

impl std::error::Error for DiagnosticFormatError {}

/// An error produced by a diagnostic query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticQueryError {
    /// The node has no source span because token data was not preserved.
    MissingSpan,
}

impl fmt::Display for DiagnosticQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticQueryError::MissingSpan => {
                write!(f, "the node has no source span")
            }
        }
    }
}

impl std::error::Error for DiagnosticQueryError {}

impl DiagnosticQueryError {
    /// Returns a stable machine readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            DiagnosticQueryError::MissingSpan => "query/missing-span",
        }
    }
}

/// Renders diagnostics with their source line and a caret annotation.
///
/// Formatting never guesses: a cataloged code is required when a code is
/// present, and the span/location must fit the source or a typed error is
/// returned.
#[derive(Clone, Debug)]
pub struct DiagnosticFormatter<'a> {
    source: &'a str,
    context_lines: usize,
}

impl<'a> DiagnosticFormatter<'a> {
    /// Creates a formatter over `source`.
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            context_lines: 0,
        }
    }

    /// Sets how many lines of context to print before and after the offending
    /// line (defaults to `0`).
    pub fn with_context_lines(mut self, context_lines: usize) -> Self {
        self.context_lines = context_lines;
        self
    }

    /// Returns the source the formatter was built with.
    #[inline]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Renders a single diagnostic.
    pub fn render(&self, diagnostic: &Diagnostic) -> Result<String, DiagnosticFormatError> {
        if let Some(code) = diagnostic.code {
            if descriptor_for(code).is_none() {
                return Err(DiagnosticFormatError::UnknownCode {
                    code: code.to_owned(),
                });
            }
        }

        let severity = diagnostic
            .descriptor()
            .map(|descriptor| descriptor.severity)
            .unwrap_or(diagnostic.severity);

        let mut header = format!("{}", severity);
        if let Some(code) = diagnostic.code {
            header.push('[');
            header.push_str(code);
            header.push(']');
        }
        header.push_str(": ");
        header.push_str(&diagnostic.message);

        let Some(location) = self.resolve_location(diagnostic)? else {
            return Ok(header);
        };

        let lines: Vec<&str> = self.source.lines().collect();
        let line_index = location
            .start
            .line
            .checked_sub(1)
            .ok_or(DiagnosticFormatError::InvalidLocation { location })?;
        let line = lines
            .get(line_index)
            .ok_or(DiagnosticFormatError::InvalidLocation { location })?;
        let line_length = line.chars().count() + 1;
        if location.start.column < 1
            || location.start.column > line_length
            || location.end.column < 1
            || location.end.column > line_length
        {
            return Err(DiagnosticFormatError::InvalidLocation { location });
        }

        let mut output = header;
        output.push('\n');
        output.push_str(&format!(
            "  --> {}:{}\n",
            location.start.line, location.start.column
        ));

        let gutter_width = (location.start.line + self.context_lines)
            .to_string()
            .len()
            .max(1);
        output.push_str(&format!("{:>width$} |\n", "", width = gutter_width));

        let first = location.start.line.saturating_sub(1 + self.context_lines);
        for (index, text) in lines
            .iter()
            .enumerate()
            .skip(first)
            .take(1 + self.context_lines * 2)
        {
            let number = index + 1;
            let is_target = number == location.start.line;
            output.push_str(&format!(
                "{:>width$} | {}\n",
                number,
                text,
                width = gutter_width
            ));

            if is_target {
                let indent = " ".repeat(location.start.column.saturating_sub(1));
                let caret_count = if location.start.line == location.end.line {
                    location
                        .end
                        .column
                        .saturating_sub(location.start.column)
                        .max(1)
                } else {
                    line.chars()
                        .count()
                        .saturating_sub(location.start.column - 1)
                        .max(1)
                };
                output.push_str(&format!(
                    "{:>width$} | {}{}\n",
                    "",
                    indent,
                    "^".repeat(caret_count),
                    width = gutter_width
                ));
            }
        }

        output.pop();
        Ok(output)
    }

    fn resolve_location(
        &self,
        diagnostic: &Diagnostic,
    ) -> Result<Option<LinePositionSpan>, DiagnosticFormatError> {
        if let Some(location) = diagnostic.location {
            return Ok(Some(location));
        }
        let Some(span) = diagnostic.span else {
            return Ok(None);
        };

        let source_len = self.source.len();
        if span.end > source_len
            || span.start > span.end
            || !self.source.is_char_boundary(span.start)
            || !self.source.is_char_boundary(span.end)
        {
            return Err(DiagnosticFormatError::InvalidSpan { span, source_len });
        }
        Ok(Some(line_position_span(self.source, span)))
    }
}

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
