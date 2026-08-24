/// The line-width at which we switch from inline to expanded layout.
pub const LINE_WIDTH: usize = 80;

pub fn ind(level: usize) -> String {
    "    ".repeat(level)
}

/// Try inline first; fall back to multiline if the inline form is too long.
///
/// `open`/`close` are the surrounding delimiters (e.g. `"("` / `")"`).
/// `sep` is the separator between items (e.g. `", "`).
/// `trailing_sep` is appended after the last item in multiline mode only (e.g. `","` for records).
pub fn group(
    items:        &[String],
    open:         &str,
    close:        &str,
    sep:          &str,
    indent:       usize,
    trailing_sep: bool,
) -> String {
    if items.is_empty() {
        return format!("{}{}", open, close);
    }

    let inline = format!("{}{}{}", open, items.join(sep), close);
    if inline.len() <= LINE_WIDTH {
        return inline;
    }

    let inner_ind = ind(indent + 1);
    let outer_ind = ind(indent);
    let sep_nl = format!("{}\n{}", sep.trim_end(), inner_ind);
    let body = items.join(&sep_nl);
    let trail = if trailing_sep { sep.trim() } else { "" };
    format!("{}\n{}{}{}\n{}{}", open, inner_ind, body, trail, outer_ind, close)
}

/// Format an operator with surrounding spaces.
pub fn binop_str(op: &certo_ast::expr::BinOp) -> &'static str {
    use certo_ast::expr::BinOp;
    match op {
        BinOp::Add            => "+",
        BinOp::Sub            => "-",
        BinOp::Mul            => "*",
        BinOp::Div            => "/",
        BinOp::Rem            => "%",
        BinOp::Pow            => "**",
        BinOp::Eq             => "==",
        BinOp::NotEq          => "!=",
        BinOp::Lt             => "<",
        BinOp::LtEq           => "<=",
        BinOp::Gt             => ">",
        BinOp::GtEq           => ">=",
        BinOp::And            => "and",
        BinOp::Or             => "or",
        BinOp::RangeInclusive => "..",
        BinOp::RangeExclusive => "...",
        BinOp::NullCoalesce   => "??",
        BinOp::Concat         => "++",
        BinOp::In             => "in",
    }
}
