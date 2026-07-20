use lsp_types::{GotoDefinitionResponse, Location, Range, Position, Url};
use crate::analysis::Analysis;
use crate::pos::{position_to_offset, offset_to_position};

pub fn handle_goto_definition(
    analysis: &Analysis,
    uri: &Url,
    pos: Position,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(&analysis.src, pos);

    // Find which identifier name is at this offset by scanning path expressions in the AST.
    // Fallback: try every symbol whose def_span contains the offset (clicked on a definition itself).
    let name = ident_at_offset(&analysis.src, offset)?;
    let sym  = analysis.definition_of(&name)?;

    let start = offset_to_position(&analysis.src, sym.def_span.start);
    let end   = offset_to_position(&analysis.src, sym.def_span.end);
    Some(GotoDefinitionResponse::Scalar(Location {
        uri:   uri.clone(),
        range: Range { start, end },
    }))
}

/// Extract the identifier token that covers `offset` in `src`.
/// Scans left and right from offset to find the word boundary.
fn ident_at_offset(src: &str, offset: u32) -> Option<String> {
    let offset = offset as usize;
    if offset > src.len() { return None; }

    let bytes = src.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    // Find start of identifier
    let start = (0..=offset).rev()
        .take_while(|&i| i < src.len() && is_ident(bytes[i]))
        .last()
        .unwrap_or(offset);

    // Find end
    let end = (offset..src.len())
        .take_while(|&i| is_ident(bytes[i]))
        .last()
        .map(|i| i + 1)
        .unwrap_or(offset);

    if start >= end { return None; }
    let word = &src[start..end];
    if word.is_empty() { None } else { Some(word.to_string()) }
}
