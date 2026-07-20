//! Human-readable compiler diagnostics.
//!
//! Renders errors with source context, line/column numbers, and a caret
//! underline — no external dependencies.
//!
//! ```text
//! error[E0100]: undefined identifier `foo`
//!   --> tasks.cto:8:5
//!    |
//!  8 |     foo(x)
//!    |     ^^^
//! ```

use certo_ast::span::Span;

// ------------------------------------------------------------------ //
// Severity
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Error   => "error",
            Severity::Warning => "warning",
            Severity::Note    => "note",
        }
    }

    /// ANSI escape for the severity colour (empty when colour is off).
    fn colour(self, colour: bool) -> &'static str {
        if !colour { return ""; }
        match self {
            Severity::Error   => "\x1b[1;31m",   // bold red
            Severity::Warning => "\x1b[1;33m",   // bold yellow
            Severity::Note    => "\x1b[1;36m",   // bold cyan
        }
    }
}

// ------------------------------------------------------------------ //
// Diagnostic
// ------------------------------------------------------------------ //

/// A single compiler diagnostic — a severity, a message, and an optional
/// source location with an optional secondary label shown under the caret.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity:    Severity,
    /// Error code, e.g. "E0100". Empty string if none.
    pub code:        String,
    /// Primary message shown on the first line.
    pub message:     String,
    /// Source span for the primary underline.
    pub span:        Option<Span>,
    /// Short label printed under the caret (e.g. "expected Int here").
    /// If empty, no secondary label is shown.
    pub label:       String,
    /// Secondary notes appended after the snippet, one per line.
    pub notes:       Vec<String>,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code:     code.into(),
            message:  message.into(),
            span:     None,
            label:    String::new(),
            notes:    Vec::new(),
        }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            code:     code.into(),
            message:  message.into(),
            span:     None,
            label:    String::new(),
            notes:    Vec::new(),
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

// ------------------------------------------------------------------ //
// Renderer
// ------------------------------------------------------------------ //

/// Render a slice of diagnostics to a `String`.
///
/// `filename` is used in the `-->` line.  `src` is the full source text.
/// Pass `colour = false` when writing to a file or non-TTY.
pub fn render_all(
    diags:    &[Diagnostic],
    src:      &str,
    filename: &str,
    colour:   bool,
) -> String {
    let mut out = String::new();
    for d in diags {
        out.push_str(&render_one(d, src, filename, colour));
    }
    out
}

/// Render a single diagnostic.
pub fn render_one(
    d:        &Diagnostic,
    src:      &str,
    filename: &str,
    colour:   bool,
) -> String {
    let reset  = if colour { "\x1b[0m"  } else { "" };
    let bold   = if colour { "\x1b[1m"  } else { "" };
    let dim    = if colour { "\x1b[2m"  } else { "" };
    let sev_col = d.severity.colour(colour);

    let mut out = String::new();

    // ── Header line ───────────────────────────────────────────────────
    // error[E0100]: undefined identifier `foo`
    let code_part = if d.code.is_empty() {
        String::new()
    } else {
        format!("[{}]", d.code)
    };
    out.push_str(&format!(
        "{sev_col}{}{code_part}{reset}{bold}: {}{reset}\n",
        d.severity.label(),
        d.message,
    ));

    // ── Source snippet ────────────────────────────────────────────────
    if let Some(span) = d.span {
        // DUMMY span (0,0) on a non-empty source means end-of-file.
        let eff_start = if span.start == 0 && span.end == 0 && !src.is_empty() {
            src.len().saturating_sub(1)
        } else {
            span.start as usize
        };
        let (line_no, col, line_text) = locate(src, eff_start);
        let end_col = {
            let line_end = line_text.len();
            let line_start_byte = eff_start - col; // byte offset of start of this line
            let eff_end = if span.start == 0 && span.end == 0 {
                eff_start + 1
            } else {
                (span.end as usize).min(src.len())
            };
            let raw_end = eff_end.saturating_sub(line_start_byte);
            raw_end.min(line_end)
        };
        let underline_len = end_col.saturating_sub(col).max(1);
        let line_no_width = format!("{}", line_no).len().max(3);
        let pad = " ".repeat(line_no_width);

        //   --> filename:line:col
        out.push_str(&format!(
            "{dim}{pad}--> {filename}:{line_no}:{}{reset}\n",
            col + 1
        ));
        //    |
        out.push_str(&format!("{dim}{pad} |{reset}\n"));
        // NN | <source line>
        out.push_str(&format!(
            "{dim}{line_no:>line_no_width$} |{reset} {line_text}\n"
        ));
        //    |     ^^^  optional label
        let caret = "^".repeat(underline_len);
        let spaces = " ".repeat(col);
        let label_part = if d.label.is_empty() {
            String::new()
        } else {
            format!(" {}", d.label)
        };
        out.push_str(&format!(
            "{dim}{pad} |{reset} {spaces}{sev_col}{caret}{label_part}{reset}\n"
        ));
        // trailing blank |
        out.push_str(&format!("{dim}{pad} |{reset}\n"));

        // optional `= note:` lines
        for note in &d.notes {
            out.push_str(&format!("{dim}{pad} = {reset}note: {note}\n"));
        }
    } else {
        for note in &d.notes {
            out.push_str(&format!("  = note: {note}\n"));
        }
    }

    out.push('\n');
    out
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

/// Given source text and a byte offset, return `(line_number_1based, col_0based, line_text)`.
fn locate(src: &str, byte_offset: usize) -> (usize, usize, &str) {
    let offset = byte_offset.min(src.len());
    let before = &src[..offset];
    let line_no = before.chars().filter(|&c| c == '\n').count() + 1;
    let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let col = offset - line_start;

    // Extract the line text (strip trailing newline).
    let rest = &src[line_start..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line_text = &rest[..line_end];

    (line_no, col, line_text)
}

// ------------------------------------------------------------------ //
// Convenience builders from each error type
// ------------------------------------------------------------------ //

use certo_ast::span::Span as AstSpan;

/// Convert a `certo_parser::ParseError` into a `Diagnostic`.
pub mod from_parser {
    use super::{Diagnostic, AstSpan};

    // We reference the parser error types by name to avoid a hard dep on
    // certo-parser from this crate. Instead the CLI/test-runner calls this
    // helper directly. We expose a free function that accepts the formatted
    // message string + span so this crate stays decoupled.
    pub fn parse_error(message: String, span: AstSpan) -> Diagnostic {
        Diagnostic::error("", message)
            .with_span(span)
    }
}

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

#[cfg(test)]
mod tests {
    use super::*;
    use certo_ast::span::Span;

    #[test]
    fn locate_first_line() {
        let src = "hello world\nsecond line\n";
        let (line, col, text) = locate(src, 6);
        assert_eq!(line, 1);
        assert_eq!(col,  6);
        assert_eq!(text, "hello world");
    }

    #[test]
    fn locate_second_line() {
        let src = "hello\nsecond line\n";
        let (line, col, text) = locate(src, 9);
        assert_eq!(line, 2);
        assert_eq!(col,  3);
        assert_eq!(text, "second line");
    }

    #[test]
    fn render_basic() {
        let src = "fn foo(): Int = bar + 1\n";
        let span = Span::new(16, 19); // "bar"
        let d = Diagnostic::error("E0100", "undefined identifier `bar`")
            .with_span(span);
        let out = render_one(&d, src, "test.cto", false);
        assert!(out.contains("error[E0100]"));
        assert!(out.contains("test.cto:1:17"));
        assert!(out.contains("bar"));
        assert!(out.contains("^^^"));
    }

    #[test]
    fn render_no_span() {
        let d = Diagnostic::error("E0203", "recursive function requires explicit return type")
            .with_note("add a `: ReturnType` annotation");
        let out = render_one(&d, "", "test.cto", false);
        assert!(out.contains("error[E0203]"));
        assert!(out.contains("note: add a"));
    }
}
