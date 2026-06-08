mod schema;
mod check_migrations;
mod error;

pub use schema::{Schema, SchemaTable, SchemaColumn};
pub use error::{DbError, DbErrorKind};

use certo_ast::module::Module;

/// Build the schema and validate all migration declarations.
pub fn check_module(module: &Module) -> Result<Schema, Vec<DbError>> {
    let schema = Schema::build(module);
    let errors = check_migrations::check_migrations(module, &schema);
    if errors.is_empty() { Ok(schema) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
