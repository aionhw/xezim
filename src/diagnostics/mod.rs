//! Diagnostics: errors and warnings for the SystemVerilog parser.

use crate::ast::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self { severity: Severity::Error, message: message.into(), span }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self { severity: Severity::Warning, message: message.into(), span }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(f, "{}: {} (at byte {}..{})", sev, self.message, self.span.start, self.span.end)
    }
}

/// Format a diagnostic with source context.
pub fn format_diagnostic(source: &str, diag: &Diagnostic) -> String {
    let (line, col) = byte_to_line_col(source, diag.span.start);
    let sev = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    format!("{}:{}:{}: {}: {}", "<source>", line, col, sev, diag.message)
}

fn byte_to_line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= byte_offset { break; }
        if ch == '\n' { line += 1; col = 1; }
        else { col += 1; }
    }
    (line, col)
}
