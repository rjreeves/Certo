mod hir;
mod lower;
mod error;
mod raw_return;
mod generic_kind;

pub use hir::*;
pub use error::{LowerError, LowerErrorKind};
pub use lower::lower_module;
pub use raw_return::RAW_ELEM_RET_VAR;

#[cfg(test)]
mod tests;
