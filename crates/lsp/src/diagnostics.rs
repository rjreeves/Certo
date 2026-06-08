use lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use certo_ast::span::Span;
use crate::analysis::Analysis;
use crate::pos::offset_to_position;

pub fn collect_diagnostics(analysis: &Analysis) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let src = &analysis.src;

    for (msg, span) in &analysis.parse_errors {
        diags.push(make_diag(src, *span, msg.clone(), DiagnosticSeverity::ERROR));
    }
    for e in &analysis.resolve_errors {
        diags.push(make_diag(src, e.span, e.to_string(), DiagnosticSeverity::ERROR));
    }
    for e in &analysis.type_errors {
        diags.push(make_diag(src, e.span, e.message(), DiagnosticSeverity::ERROR));
    }

    diags
}

fn make_diag(src: &str, span: Span, message: String, severity: DiagnosticSeverity) -> Diagnostic {
    let start = offset_to_position(src, span.start);
    let end   = offset_to_position(src, span.end);
    Diagnostic {
        range:    Range { start, end },
        severity: Some(severity),
        message,
        source:   Some("certo".to_string()),
        ..Default::default()
    }
}
