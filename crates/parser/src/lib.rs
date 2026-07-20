mod cursor;
mod error;
mod parse_type;
mod parse_pattern;
mod parse_expr;
mod parse_decl;
mod parse_module;

pub use error::{ParseError, ParseErrorKind};
pub use parse_module::parse;

#[cfg(test)]
mod tests;
