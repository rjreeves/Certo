//! Standalone name-resolution pass over a parsed module.
//!
//! **Not part of the `certo build`/`certo check` pipeline.** Undefined-name
//! detection (E0100 here) is redundant with `certo_typeck`'s own check
//! (`TypeErrorKind::UnboundName`, E0206), which the real CLI already runs —
//! wiring this crate in too would just duplicate that diagnostic, one pass
//! later, with no seeded knowledge of the wider stdlib (see `resolve_seeded`
//! below). What this crate *does* provide that typeck doesn't is duplicate-
//! binding detection (E0101/E0102) and validator rule-graph checks
//! (E0700-E0702); those alone don't currently justify a second full pass
//! over every compile, so this crate's only real consumer today is
//! `certo-lsp`, which uses it for live editor diagnostics.
mod scope;
mod error;
mod resolve_module;
mod resolve_decl;
mod resolve_expr;
mod resolve_pattern;

pub use error::{ResolveError, ResolveErrorKind};
pub use resolve_module::{resolve, resolve_seeded};
pub use scope::Res;

#[cfg(test)]
mod tests;
