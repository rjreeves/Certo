use lsp_types::{CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, Position};
use crate::analysis::{Analysis, Symbol, SymbolKind};
use crate::pos::position_to_offset;

/// Keywords and built-in names always offered.
const KEYWORDS: &[&str] = &[
    "fn", "val", "var", "type", "module", "import", "pub", "priv",
    "if", "then", "else", "when",
    "match", "in", "true", "false",
    "and", "or", "not",
    "guard", "require", "ensure", "defer",
    "async", "await", "parallel", "spawn",
    "trait", "impl", "for", "where",
    "migration", "up", "down",
    "statemachine", "validator", "view", "form",
    "test", "property", "dbTest",
    "unsafe", "use", "with",
    "db",
];

pub fn handle_completion(analysis: &Analysis, pos: Position) -> CompletionResponse {
    let offset = position_to_offset(&analysis.src, pos);
    let prefix = word_before(analysis.src.as_str(), offset);

    let mut items: Vec<CompletionItem> = Vec::new();

    // Keywords
    for kw in KEYWORDS {
        if kw.starts_with(prefix) {
            items.push(CompletionItem {
                label:  kw.to_string(),
                kind:   Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }
    }

    // Symbols from the current document
    // Top-level decls, params (valid_from == 0), and locals in-scope at cursor.
    for sym in &analysis.symbols {
        if sym.valid_from > offset { continue; }
        if sym.name.starts_with(prefix) {
            items.push(completion_item_for(sym));
        }
    }

    CompletionResponse::List(CompletionList { is_incomplete: false, items })
}

fn completion_item_for(sym: &Symbol) -> CompletionItem {
    let kind = match sym.kind {
        SymbolKind::Function => CompletionItemKind::FUNCTION,
        SymbolKind::Const    => CompletionItemKind::CONSTANT,
        SymbolKind::Type     => CompletionItemKind::CLASS,
        SymbolKind::Param    => CompletionItemKind::VARIABLE,
        SymbolKind::Local    => CompletionItemKind::VARIABLE,
    };
    CompletionItem {
        label:  sym.name.clone(),
        kind:   Some(kind),
        detail: Some(sym.detail.clone()),
        ..Default::default()
    }
}

fn word_before(src: &str, offset: u32) -> &str {
    let offset = offset.min(src.len() as u32) as usize;
    let bytes  = src.as_bytes();
    let start  = (0..offset).rev()
        .find(|&i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_')
        .map(|i| i + 1)
        .unwrap_or(0);
    &src[start..offset]
}
