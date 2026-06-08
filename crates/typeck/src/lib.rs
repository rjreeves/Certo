mod ty;
mod unify;
mod env;
mod infer_expr;
mod infer_decl;
mod error;

pub use ty::Ty;
pub use error::{TypeError, TypeErrorKind};
pub use infer_decl::check_module;
pub use env::TypeEnv;

#[cfg(test)]
mod tests;
