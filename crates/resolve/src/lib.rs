mod scope;
mod error;
mod resolve_module;
mod resolve_decl;
mod resolve_expr;
mod resolve_pattern;

pub use error::{ResolveError, ResolveErrorKind};
pub use resolve_module::resolve;
pub use scope::Res;

#[cfg(test)]
mod tests;
