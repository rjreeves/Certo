mod printer;
mod fmt_type;
mod fmt_expr;
mod fmt_decl;
mod fmt_module;

pub use fmt_module::fmt_module;
pub use fmt_decl::fmt_decl;
pub use fmt_expr::{fmt_expr, fmt_stmt, fmt_pat};
pub use fmt_type::fmt_type;

/// Parse `src` and reformat it.  Returns the formatted source, or the
/// original source unchanged if parsing fails (so `certo fmt` never
/// corrupts a file it can't parse).
pub fn format_source(src: &str) -> Result<String, Vec<certo_parser::ParseError>> {
    let module = certo_parser::parse_verbatim(src)?;
    // BACKLOG item 445 - the AST has no comments; they are put back from the source.
    Ok(comments::restore(src, fmt_module(&module)))
}

mod comments;

#[cfg(test)]
mod tests;
