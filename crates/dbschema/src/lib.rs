mod schema;
mod check_migrations;
mod error;

#[cfg(feature = "live")]
pub mod pull;

pub use schema::{Schema, SchemaTable, SchemaColumn, DriftItem};
pub use error::{DbError, DbErrorKind};

use std::path::Path;
use certo_ast::module::Module;

/// Check a module's migrations for internal consistency using types declared
/// in the same module.  This is the fast, hermetic path used by the compiler.
pub fn check_module(module: &Module) -> Result<Schema, Vec<DbError>> {
    let schema = Schema::from_module(module);
    let errors = check_migrations::check_migrations(module, &schema);
    if errors.is_empty() { Ok(schema) } else { Err(errors) }
}

/// Check a module's migrations against a committed `schema.json` snapshot.
/// This is the recommended compile-time path: deterministic, no network.
///
/// Returns an error string if the snapshot cannot be loaded.
pub fn check_module_against_snapshot(
    module:        &Module,
    snapshot_path: &Path,
) -> Result<Schema, Result<Vec<DbError>, String>> {
    let snapshot = Schema::load(snapshot_path).map_err(Err)?;
    let errors   = check_migrations::check_migrations(module, &snapshot);
    if errors.is_empty() { Ok(snapshot) } else { Err(Ok(errors)) }
}

#[cfg(test)]
mod tests;
