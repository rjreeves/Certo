mod ty;
mod unify;
mod env;
mod infer_expr;
mod exhaustive;
mod infer_decl;
mod error;

pub use ty::{Ty, assign_var_names};
pub use error::{TypeError, TypeErrorKind, Warning, WarningKind};
pub use infer_decl::{check_module, check_module_seeded, check_module_warnings};
pub use env::TypeEnv;
pub use infer_expr::anonymous_record_type;

#[cfg(test)]
mod tests;
