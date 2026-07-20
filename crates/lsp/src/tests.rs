use crate::analysis::Analysis;
use crate::diagnostics::collect_diagnostics;
use crate::hover::handle_hover;
use crate::completion::handle_completion;
use crate::pos::{offset_to_position, position_to_offset};
use lsp_types::Position;

// ------------------------------------------------------------------ //
// pos helpers
// ------------------------------------------------------------------ //

#[test]
fn offset_to_position_single_line() {
    let src = "hello world";
    let pos = offset_to_position(src, 6);
    assert_eq!(pos.line, 0);
    assert_eq!(pos.character, 6);
}

#[test]
fn offset_to_position_multiline() {
    let src = "line one\nline two\nline three";
    let pos = offset_to_position(src, 9); // first char of "line two"
    assert_eq!(pos.line, 1);
    assert_eq!(pos.character, 0);
}

#[test]
fn position_to_offset_roundtrip() {
    let src = "fn add(a: Int, b: Int): Int = a + b";
    for i in [0u32, 3, 7, 10, 20] {
        let pos = offset_to_position(src, i);
        let back = position_to_offset(src, pos);
        assert_eq!(back, i, "roundtrip failed at offset {}", i);
    }
}

// ------------------------------------------------------------------ //
// Analysis / diagnostics
// ------------------------------------------------------------------ //

#[test]
fn valid_module_has_no_diagnostics() {
    let src = "module A\nfn answer(): Int = 42";
    let analysis = Analysis::run(src);
    let diags = collect_diagnostics(&analysis);
    assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
}

#[test]
fn parse_error_produces_diagnostic() {
    let src = "module A\nfn = ";
    let analysis = Analysis::run(src);
    let diags = collect_diagnostics(&analysis);
    assert!(!diags.is_empty(), "expected at least one diagnostic");
}

#[test]
fn symbol_table_contains_function() {
    let src = "module A\nfn greet(): Text = \"hi\"";
    let analysis = Analysis::run(src);
    assert!(analysis.symbols.iter().any(|s| s.name == "greet"),
        "expected 'greet' in symbol table");
}

#[test]
fn symbol_detail_shows_signature() {
    let src = "module A\nfn add(x: Int, y: Int): Int = x + y";
    let analysis = Analysis::run(src);
    let sym = analysis.symbols.iter().find(|s| s.name == "add").unwrap();
    assert!(sym.detail.contains("Int"), "detail: {}", sym.detail);
}

// ------------------------------------------------------------------ //
// Hover
// ------------------------------------------------------------------ //

#[test]
fn hover_on_fn_name_returns_signature() {
    let src = "module A\nfn greet(): Text = \"hi\"";
    let analysis = Analysis::run(src);
    // "greet" starts at offset 12 (after "module A\nfn ")
    let offset = src.find("greet").unwrap() as u32;
    let pos = offset_to_position(src, offset + 1); // inside the name
    let hover = handle_hover(&analysis, pos);
    assert!(hover.is_some(), "expected hover result");
    if let Some(h) = hover {
        use lsp_types::HoverContents;
        if let HoverContents::Markup(mc) = h.contents {
            assert!(mc.value.contains("greet"), "hover text: {}", mc.value);
        }
    }
}

// ------------------------------------------------------------------ //
// Completion
// ------------------------------------------------------------------ //

#[test]
fn completion_includes_keywords() {
    let src = "module A\nfn answer(): Int = 42";
    let analysis = Analysis::run(src);
    let pos = Position { line: 1, character: 0 };
    let resp = handle_completion(&analysis, pos);
    use lsp_types::CompletionResponse;
    let items = match resp {
        CompletionResponse::List(l) => l.items,
        CompletionResponse::Array(a) => a,
    };
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"fn"), "keywords missing from completion");
    assert!(labels.contains(&"if"), "keywords missing from completion");
}

#[test]
fn completion_includes_declared_function() {
    let src = "module A\nfn greet(): Text = \"hi\"";
    let analysis = Analysis::run(src);
    let pos = Position { line: 1, character: 0 };
    let resp = handle_completion(&analysis, pos);
    use lsp_types::CompletionResponse;
    let items = match resp {
        CompletionResponse::List(l) => l.items,
        CompletionResponse::Array(a) => a,
    };
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"greet"), "function not in completion list");
}

#[test]
fn completion_filters_by_prefix() {
    let src = "module A\nfn greet(): Text = \"hi\"\nfn goodbye(): Text = \"bye\"";
    let analysis = Analysis::run(src);
    // Cursor after "gr"
    let offset = src.find("greet").unwrap() as u32 + 2;
    let pos = offset_to_position(src, offset);
    let resp = handle_completion(&analysis, pos);
    use lsp_types::CompletionResponse;
    let items = match resp {
        CompletionResponse::List(l) => l.items,
        CompletionResponse::Array(a) => a,
    };
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"greet"), "expected 'greet'");
    assert!(!labels.contains(&"goodbye"), "should not contain 'goodbye' with prefix 'gr'... wait it starts with 'go'");
}
