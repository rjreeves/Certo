use certo_ast::span::Span;

#[derive(Debug, Clone)]
pub struct DbError {
    pub kind: DbErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum DbErrorKind {
    /// E0500 — migration references a table that doesn't exist in the schema.
    UnknownTable { migration: String, table: String },

    /// E0501 — migration column type doesn't match the type declaration.
    ColumnTypeMismatch { table: String, column: String, declared: String, migration: String },

    /// E0502 — foreign key references a table not defined in the schema.
    UnknownForeignKeyTarget { table: String, column: String, references: String },

    /// E0503 — duplicate migration name.
    DuplicateMigration { name: String, first: Span },

    /// E0504 — migration `down` block is empty (non-destructive ops must be reversible).
    MissingDownMigration { name: String },

    /// E0505 — `CreateTable` in migration for a table not declared as a `type`.
    TableNotDeclaredAsType { migration: String, table: String },

    /// E0506 — column in migration not present in the record type.
    UnknownColumn { migration: String, table: String, column: String },

    /// E0507 — `AlterTable` / `DropTable` on a table that was never created.
    TableNotCreated { migration: String, table: String },
}

impl DbError {
    pub fn message(&self) -> String {
        match &self.kind {
            DbErrorKind::UnknownTable { migration, table } =>
                format!("E0500: migration `{}` references unknown table `{}`", migration, table),
            DbErrorKind::ColumnTypeMismatch { table, column, declared, migration } =>
                format!("E0501: column `{}.{}` — declared as `{}` but migration uses `{}`", table, column, declared, migration),
            DbErrorKind::UnknownForeignKeyTarget { table, column, references } =>
                format!("E0502: foreign key `{}.{}` references unknown table `{}`", table, column, references),
            DbErrorKind::DuplicateMigration { name, .. } =>
                format!("E0503: duplicate migration name `{}`", name),
            DbErrorKind::MissingDownMigration { name } =>
                format!("E0504: migration `{}` has no `down` block", name),
            DbErrorKind::TableNotDeclaredAsType { migration, table } =>
                format!("E0505: migration `{}` creates table `{}` which is not declared as a `type`", migration, table),
            DbErrorKind::UnknownColumn { migration, table, column } =>
                format!("E0506: migration `{}` references unknown column `{}` on `{}`", migration, column, table),
            DbErrorKind::TableNotCreated { migration, table } =>
                format!("E0507: migration `{}` alters/drops table `{}` which was never created", migration, table),
        }
    }
}
