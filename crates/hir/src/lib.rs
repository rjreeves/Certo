mod hir;
mod lower;
mod error;

pub use hir::*;
pub use error::{LowerError, LowerErrorKind};
pub use lower::lower_module;

#[cfg(test)]
mod tests;
