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

    /// E0508 — `Query.from("Table")` references a table not declared as a `type`.
    QueryUnknownTable { table: String },

    /// E0509 — `.filter`/`.orderBy` references a column that doesn't exist on the table.
    QueryUnknownColumn { table: String, column: String },

    /// E0510 — a `Query` builder call's table/column/op/dir argument isn't a string literal,
    /// so it cannot be verified against the schema at compile time.
    QueryNonLiteralArg { function: String, position: String },

    /// E0511 — `.filter`'s operator isn't one of the recognized literals. Shared between
    /// `Query`/`Mutation` — both use the same operator set for `.filter`.
    QueryInvalidOperator { op: String },

    /// E0512 — `.orderBy`'s direction isn't `"asc"` or `"desc"`.
    QueryInvalidSortDir { dir: String },

    /// E0513 — `.aggregate`/`.having`'s function isn't `count`/`sum`/`avg`/`min`/`max`.
    QueryInvalidAggFn { agg: String },

    /// E0514 — `.aggregate`'s alias isn't a valid identifier.
    QueryInvalidAlias { alias: String },

    /// E0515 — an unqualified column name matches more than one table in a joined query.
    QueryAmbiguousColumn { column: String, tables: Vec<String> },

    /// E0516 — `.list`/`.first`/`.count`/`.sum`/`.avg`/`.min`/`.max` called on a query that
    /// already has `.groupBy`/`.aggregate` applied — those change the query's shape away
    /// from `SELECT *`/a single scalar, so use `.groupedList` instead.
    QueryGroupedTerminalMisuse { function: String },

    /// E0517 — a `"Table.column"` qualifier names a table that isn't part of this query
    /// (not the base table and not joined).
    QueryColumnTableNotJoined { table: String },

    /// E0518 — `.join`/`.leftJoin`'s ON columns must be written `"Table.column"`, not bare.
    QueryJoinColumnNotQualified { column: String },

    /// E0519 — `Mutation.insertInto`/`.updateTable`/`.deleteFrom`/`.insertMany` references a
    /// table not declared as a `type`.
    MutationUnknownTable { function: String, table: String },

    /// E0520 — a `Mutation` method was called on the wrong kind of mutation, e.g. `.filter`
    /// on an `insert`, or `.set` on a `delete`.
    MutationInvalidStage { function: String, kind: String },

    /// E0521 — `.addRow`'s value count doesn't match `.insertMany`'s declared column count
    /// (only checked when both are literal lists).
    MutationRowArityMismatch { expected: usize, found: usize },

    /// E0522 — a `type` with `impl DbRow for X {}` has no matching table in the live database
    /// (opt-in via `[database] schema-sync = true` in `certo.toml`).
    SchemaSyncTableMissing { table: String },

    /// E0523 — a field on a `DbRow` type has no matching column in the live database.
    SchemaSyncColumnMissing { table: String, column: String },

    /// E0524 — a `DbRow` type's field type doesn't match the live column's type.
    SchemaSyncTypeMismatch { table: String, column: String, declared: String, live: String },

    /// E0525 — a `DbRow` type's field nullability doesn't match the live column's.
    SchemaSyncNullabilityMismatch { table: String, column: String, declared_nullable: bool, live_nullable: bool },

    /// E0526 — `.joinAs`/`.leftJoinAs`/`.fromAs` reuses an alias already in scope for this
    /// query. The classic case is a self-join: joining the same table to itself requires
    /// two distinct aliases (e.g. `"e"`/`"m"`), since a bare alias equal to the table name
    /// can only be used once.
    QueryDuplicateAlias { alias: String },
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
            DbErrorKind::QueryUnknownTable { table } =>
                format!("E0508: `Query.from(\"{}\")` — no `type {}` declared in this module", table, table),
            DbErrorKind::QueryUnknownColumn { table, column } =>
                format!("E0509: column `{}` does not exist on `{}`", column, table),
            DbErrorKind::QueryNonLiteralArg { function, position } =>
                format!("E0510: `{}`'s {} argument must be a string literal", function, position),
            DbErrorKind::QueryInvalidOperator { op } =>
                format!("E0511: `\"{}\"` is not a recognized comparison operator", op),
            DbErrorKind::QueryInvalidSortDir { dir } =>
                format!("E0512: `\"{}\"` is not a valid sort direction — use \"asc\" or \"desc\"", dir),
            DbErrorKind::QueryInvalidAggFn { agg } =>
                format!("E0513: `\"{}\"` is not a recognized aggregate function", agg),
            DbErrorKind::QueryInvalidAlias { alias } =>
                format!("E0514: `\"{}\"` is not a valid alias", alias),
            DbErrorKind::QueryAmbiguousColumn { column, tables } =>
                format!("E0515: column `{}` is ambiguous — present on {}", column,
                    tables.iter().map(|t| format!("`{}`", t)).collect::<Vec<_>>().join(", ")),
            DbErrorKind::QueryGroupedTerminalMisuse { function } =>
                format!("E0516: `{}` cannot be used on a grouped/aggregated query", function),
            DbErrorKind::QueryColumnTableNotJoined { table } =>
                format!("E0517: `{}` is not the base table or a joined table in this query", table),
            DbErrorKind::QueryJoinColumnNotQualified { column } =>
                format!("E0518: join column `\"{}\"` must be qualified as \"Table.column\"", column),
            DbErrorKind::MutationUnknownTable { function, table } =>
                format!("E0519: `{}(\"{}\")` — no `type {}` declared in this module", function, table, table),
            DbErrorKind::MutationInvalidStage { function, kind } =>
                format!("E0520: `{}` cannot be used on {}", function, kind),
            DbErrorKind::MutationRowArityMismatch { expected, found } =>
                format!("E0521: `.addRow` has {} value(s), but `.insertMany` declared {} column(s)", found, expected),
            DbErrorKind::SchemaSyncTableMissing { table } =>
                format!("E0522: `type {}` has `impl DbRow`, but no matching table exists in the live database", table),
            DbErrorKind::SchemaSyncColumnMissing { table, column } =>
                format!("E0523: `{}.{}` has no matching column in the live database", table, column),
            DbErrorKind::SchemaSyncTypeMismatch { table, column, declared, live } =>
                format!("E0524: `{}.{}` is declared as `{}`, but the live database column is `{}`", table, column, declared, live),
            DbErrorKind::SchemaSyncNullabilityMismatch { table, column, declared_nullable, live_nullable } =>
                format!("E0525: `{}.{}` is declared as {}, but the live database column is {}",
                    table, column,
                    if *declared_nullable { "nullable" } else { "not nullable" },
                    if *live_nullable { "nullable" } else { "not nullable" }),
            DbErrorKind::QueryDuplicateAlias { alias } =>
                format!("E0526: alias `\"{}\"` is already used in this query", alias),
        }
    }
}
