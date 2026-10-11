mod hir;
mod lower;
mod error;
mod raw_return;

pub use hir::*;
pub use error::{LowerError, LowerErrorKind};
pub use lower::lower_module;
pub use raw_return::RAW_ELEM_RET_VAR;

#[cfg(test)]
mod tests;
