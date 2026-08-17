use std::collections::{HashMap, HashSet};
use certo_ast::decl::{Decl, MigrationDecl, MigrationOp, AlterOp};
use certo_ast::module::Module;
use crate::schema::{Schema, te_to_str};
use crate::error::{DbError, DbErrorKind};

/// Validate all migration declarations in a module against the schema.
pub fn check_migrations(module: &Module, schema: &Schema) -> Vec<DbError> {
    let mut errors  = Vec::new();
    let mut seen:    HashMap<String, certo_ast::span::Span> = HashMap::new();
    // Tables created by migrations processed so far (starts empty — tables
    // must be explicitly created by a CreateTable op before being altered).
    let mut created: HashSet<String> = HashSet::new();

    for sdecl in &module.decls {
        if let Decl::Migration(m) = &sdecl.node {
            check_migration(m, schema, &mut created, &mut seen, &mut errors);
        }
    }
    errors
}

fn check_migration(
    m:       &MigrationDecl,
    schema:  &Schema,
    created: &mut HashSet<String>,
    seen:    &mut HashMap<String, certo_ast::span::Span>,
    errors:  &mut Vec<DbError>,
) {
    // E0503 — duplicate name
    if let Some(&first) = seen.get(&m.name) {
        errors.push(DbError {
            kind: DbErrorKind::DuplicateMigration { name: m.name.clone(), first },
            span: m.span,
        });
        return;
    }
    seen.insert(m.name.clone(), m.span);

    // E0504 — non-destructive migration should have a down block
    // (We only warn when up is non-empty and down is completely absent)
    if !m.up.is_empty() && m.down.is_empty() {
        errors.push(DbError {
            kind: DbErrorKind::MissingDownMigration { name: m.name.clone() },
            span: m.span,
        });
    }

    for op in &m.up {
        check_op(op, &m.name, schema, created, errors);
    }
    // Process down in reverse to simulate rollback (just check references)
    for op in &m.down {
        check_op_down(op, &m.name, schema, errors);
    }
}

fn check_op(
    op:      &MigrationOp,
    mig:     &str,
    schema:  &Schema,
    created: &mut HashSet<String>,
    errors:  &mut Vec<DbError>,
) {
    match op {
        MigrationOp::CreateTable { name, columns, span } => {
            // E0505 — table must be declared as a type
            if !schema.has_table(name) {
                errors.push(DbError {
                    kind: DbErrorKind::TableNotDeclaredAsType {
                        migration: mig.to_string(),
                        table:     name.clone(),
                    },
                    span: *span,
                });
                created.insert(name.clone()); // keep going for further checks
                return;
            }
            created.insert(name.clone());

            // E0501 — column types must match the type declaration
            // E0506 — columns must exist on the type
            for col in columns {
                let declared = schema.column_type(name, &col.name);
                match declared {
                    None => errors.push(DbError {
                        kind: DbErrorKind::UnknownColumn {
                            migration: mig.to_string(),
                            table:     name.clone(),
                            column:    col.name.clone(),
                        },
                        span: col.span,
                    }),
                    Some(dt) => {
                        let mt = te_to_str(&col.ty.node);
                        if !types_compat(dt, &mt) {
                            errors.push(DbError {
                                kind: DbErrorKind::ColumnTypeMismatch {
                                    table:      name.clone(),
                                    column:     col.name.clone(),
                                    declared:   dt.to_string(),
                                    migration:  mt,
                                },
                                span: col.span,
                            });
                        }
                    }
                }
            }
        }

        MigrationOp::AlterTable { name, ops, span } => {
            // E0507 — table must exist
            if !created.contains(name.as_str()) {
                errors.push(DbError {
                    kind: DbErrorKind::TableNotCreated {
                        migration: mig.to_string(),
                        table:     name.clone(),
                    },
                    span: *span,
                });
                return;
            }
            for aop in ops {
                match aop {
                    AlterOp::AddColumn { def } => {
                        // E0506 — new column must exist in the type
                        if schema.has_table(name) && schema.column_type(name, &def.name).is_none() {
                            errors.push(DbError {
                                kind: DbErrorKind::UnknownColumn {
                                    migration: mig.to_string(),
                                    table:     name.clone(),
                                    column:    def.name.clone(),
                                },
                                span: def.span,
                            });
                        }
                    }
                    AlterOp::DropColumn { .. } => {} // dropping is always valid structurally
                    AlterOp::AddForeignKey { column, references, span, .. } => {
                        // E0502 — referenced table must be known
                        let ref_table = references.split('.').next().unwrap_or(references);
                        if !schema.has_table(ref_table) && !created.contains(ref_table) {
                            errors.push(DbError {
                                kind: DbErrorKind::UnknownForeignKeyTarget {
                                    table:      name.clone(),
                                    column:     column.clone(),
                                    references: references.clone(),
                                },
                                span: *span,
                            });
                        }
                    }
                }
            }
        }

        MigrationOp::DropTable { name, span } => {
            // E0507
            if !created.contains(name.as_str()) {
                errors.push(DbError {
                    kind: DbErrorKind::TableNotCreated {
                        migration: mig.to_string(),
                        table:     name.clone(),
                    },
                    span: *span,
                });
            } else {
                created.remove(name.as_str());
            }
        }

        MigrationOp::CreateIndex { table, span, .. } => {
            if !created.contains(table.as_str()) {
                errors.push(DbError {
                    kind: DbErrorKind::UnknownTable {
                        migration: mig.to_string(),
                        table:     table.clone(),
                    },
                    span: *span,
                });
            }
        }

        MigrationOp::DropIndex { .. } | MigrationOp::RawSql { .. } => {}
    }
}

fn check_op_down(op: &MigrationOp, _mig: &str, schema: &Schema, errors: &mut Vec<DbError>) {
    // Down operations just need referential validity (no state simulation).
    match op {
        MigrationOp::AlterTable { name, ops, span: _ } => {
            for aop in ops {
                if let AlterOp::AddForeignKey { column, references, span, .. } = aop {
                    let ref_table = references.split('.').next().unwrap_or(references);
                    if !schema.has_table(ref_table) {
                        errors.push(DbError {
                            kind: DbErrorKind::UnknownForeignKeyTarget {
                                table:      name.clone(),
                                column:     column.clone(),
                                references: references.clone(),
                            },
                            span: *span,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Allow `UUID` ↔ `UUID` and also accept `id?` == `UUID?` for nullable pk columns.
fn types_compat(declared: &str, migration: &str) -> bool {
    if declared == migration { return true; }
    // Strip trailing `?` for nullable comparison
    let a = declared.trim_end_matches('?');
    let b = migration.trim_end_matches('?');
    a == b
        || crate::schema::decimal_bare_vs_param(a, b)
        || crate::schema::bounded_text_bare_vs_param(a, b)
}
