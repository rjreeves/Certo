mod trait_db;
mod check_impl;
mod bounds;
mod error;

pub use trait_db::{TraitDb, TraitDef, ImplRecord, MethodSig};
pub use error::{TraitError, TraitErrorKind};
pub use bounds::{check_bounds, check_call_bounds, check_dbquery_typed_bounds};

use certo_ast::module::Module;

/// Run the full trait-system check on a module:
/// 1. Build the trait/impl registry.
/// 2. Verify each impl satisfies its trait.
/// 3. Verify type-param bounds in function signatures.
pub fn check_module(module: &Module) -> Result<TraitDb, Vec<TraitError>> {
    let db = TraitDb::build(module);

    let mut errors = Vec::new();
    errors.extend(check_impl::check_impls(module, &db));
    errors.extend(check_bounds(module, &db));
    errors.extend(check_dbquery_typed_bounds(module, &db));

    if errors.is_empty() { Ok(db) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
