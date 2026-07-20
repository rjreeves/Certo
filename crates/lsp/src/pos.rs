use lsp_types::Position;
use certo_ast::span::Span;

/// Convert a byte offset in `src` to an LSP Position (0-based line + character).
pub fn offset_to_position(src: &str, offset: u32) -> Position {
    let offset = offset.min(src.len() as u32) as usize;
    let prefix = &src[..offset];
    let line = prefix.chars().filter(|&c| c == '\n').count() as u32;
    let col_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let character = (offset - col_start) as u32;
    Position { line, character }
}

/// Convert an LSP Position to a byte offset in `src`.
pub fn position_to_offset(src: &str, pos: Position) -> u32 {
    let mut line = 0u32;
    let mut offset = 0usize;
    for (i, ch) in src.char_indices() {
        if line == pos.line {
            offset = i + pos.character as usize;
            break;
        }
        if ch == '\n' {
            line += 1;
        }
        if i + ch.len_utf8() == src.len() {
            offset = src.len();
        }
    }
    if src.is_empty() {
        return 0;
    }
    offset.min(src.len()) as u32
}

/// Return true if `offset` is inside (inclusive) the span.
pub fn span_contains(span: Span, offset: u32) -> bool {
    offset >= span.start && offset <= span.end
}
