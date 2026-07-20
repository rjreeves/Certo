use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Range, Position};
use crate::analysis::Analysis;
use crate::pos::{position_to_offset, offset_to_position};

pub fn handle_hover(analysis: &Analysis, pos: Position) -> Option<Hover> {
    let offset = position_to_offset(&analysis.src, pos);
    let sym = analysis.symbol_at(offset)?;
    let start = offset_to_position(&analysis.src, sym.def_span.start);
    let end   = offset_to_position(&analysis.src, sym.def_span.end);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind:  MarkupKind::Markdown,
            value: format!("```certo\n{}\n```", sym.detail),
        }),
        range: Some(Range { start, end }),
    })
}
