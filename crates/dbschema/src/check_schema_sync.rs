use std::collections::HashMap;

use certo_ast::decl::Decl;
use certo_ast::module::Module;
use certo_ast::span::Span;

use crate::error::{DbError, DbErrorKind};
use crate::schema::{snake_to_camel, snake_to_pascal, LiveTable, Schema};

/// Cross-check every `type X = {...}` that has `impl DbRow for X {}` against a live
/// database schema snapshot, reporting drift as compile errors (E0522-E0525).
///
/// Only `DbRow`-annotated types are checked — that's the existing, established signal
/// (already used by `certo db pull`-generated code and the bound on `dbQueryTyped`/
/// `Query.list<T: DbRow>`) that a type mirrors a real table. Most modules declare plenty
/// of plain data record types that were never meant to correspond to a database table —
/// checking those against the live schema too would just be noise.
///
/// This is a pure function (no I/O) so it's easy to test without a real database: the
/// caller is responsible for actually connecting and introspecting (see `certo`'s
/// `[database] schema-sync = true` support), and passes the result in as `live`.
///
/// Table/column name matching bridges the two naming conventions in play: `certo db pull`
/// generates `PascalCase` types and `camelCase` fields from `snake_case` Postgres
/// identifiers, so a live table `orders` is expected to correspond to `type Orders`, and a
/// live column `customer_id` to a field named `customerId`.
pub fn check_schema_sync(module: &Module, schema: &Schema, live: &[LiveTable]) -> Vec<DbError> {
    let mut errors = Vec::new();
    let dbrow_types = collect_dbrow_types(module);
    if dbrow_types.is_empty() { return errors; }

    let live_by_certo_name: HashMap<String, &LiveTable> = live.iter()
        .map(|t| (snake_to_pascal(&t.name), t))
        .collect();

    for (type_name, span) in &dbrow_types {
        let Some(table) = schema.tables.get(type_name) else { continue };

        let Some(live_table) = live_by_certo_name.get(type_name) else {
            errors.push(DbError {
                kind: DbErrorKind::SchemaSyncTableMissing { table: type_name.clone() },
                span: *span,
            });
            continue;
        };

        let live_cols: HashMap<String, &crate::schema::LiveColumn> = live_table.columns.iter()
            .map(|c| (snake_to_camel(&c.name), c))
            .collect();

        for col in &table.columns {
            let Some(live_col) = live_cols.get(&col.name) else {
                errors.push(DbError {
                    kind: DbErrorKind::SchemaSyncColumnMissing {
                        table: type_name.clone(), column: col.name.clone(),
                    },
                    span: *span,
                });
                continue;
            };

            // `col.ty` may carry a trailing `?` when written as `Type?` sugar rather than
            // via the field's separate `optional` flag — strip it the same defensive way
            // `check_migrations::types_compat` does, since `nullable` is the real signal.
            let declared_ty = col.ty.trim_end_matches('?');
            if declared_ty != live_col.certo_type
                && !crate::schema::decimal_bare_vs_param(declared_ty, &live_col.certo_type) {
                errors.push(DbError {
                    kind: DbErrorKind::SchemaSyncTypeMismatch {
                        table: type_name.clone(), column: col.name.clone(),
                        declared: declared_ty.to_string(), live: live_col.certo_type.clone(),
                    },
                    span: *span,
                });
            }

            if col.nullable != live_col.nullable {
                errors.push(DbError {
                    kind: DbErrorKind::SchemaSyncNullabilityMismatch {
                        table: type_name.clone(), column: col.name.clone(),
                        declared_nullable: col.nullable, live_nullable: live_col.nullable,
                    },
                    span: *span,
                });
            }
        }
    }

    errors
}

/// Every `impl DbRow for X {}` in the module, as `(X, span-of-the-impl)`.
fn collect_dbrow_types(module: &Module) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Impl(imp) = &sdecl.node {
            let is_dbrow = imp.trait_path.as_ref()
                .and_then(|p| p.segments.last())
                .map(|s| s.node == "DbRow")
                .unwrap_or(false);
            if is_dbrow {
                if let Some(seg) = imp.type_path.segments.last() {
                    out.push((seg.node.clone(), imp.span));
                }
            }
        }
    }
    out
}
