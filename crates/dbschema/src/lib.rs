mod schema;
mod check_migrations;
mod check_query;
mod check_mutation;
mod check_schema_sync;
mod error;

pub use schema::{Schema, SchemaTable, SchemaColumn, LiveTable, LiveColumn, snake_to_pascal, snake_to_camel};
pub use error::{DbError, DbErrorKind};
pub use check_schema_sync::check_schema_sync;

use certo_ast::module::Module;

/// Build the schema and validate all migration declarations and `Query`/`Mutation` builder
/// call sites.
pub fn check_module(module: &Module) -> Result<Schema, Vec<DbError>> {
    let schema = Schema::build(module);
    let mut errors = check_migrations::check_migrations(module, &schema);
    errors.extend(check_query::check_queries(module, &schema));
    errors.extend(check_mutation::check_mutations(module, &schema));
    if errors.is_empty() { Ok(schema) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod check_query_tests;
#[cfg(test)]
mod check_mutation_tests;
#[cfg(test)]
mod check_schema_sync_tests;
