use certo_ast::decl::{MigrationOp, AlterOp, ColumnDef, FkAction};
use certo_ast::types::TypeExpr;
use certo_dbschema::camel_to_snake;
use crate::error::MigrateError;

/// Generate an SQL DDL string for a single `MigrationOp`. Panics on a column default the
/// generator cannot express; `try_op_to_sql` reports that as an error instead.
pub fn op_to_sql(op: &MigrationOp) -> String {
    try_op_to_sql(op).unwrap_or_else(|e| panic!("{e}"))
}

/// Generate the SQL for one `MigrationOp`, or say why it cannot be expressed.
pub fn try_op_to_sql(op: &MigrationOp) -> Result<String, MigrateError> {
    Ok(match op {
        MigrationOp::CreateTable { name, columns, .. } => create_table(name, columns)?,
        MigrationOp::DropTable   { name, .. }          => format!("DROP TABLE {};", camel_to_snake(name)),
        MigrationOp::AlterTable  { name, ops, .. }     => alter_table(name, ops)?,
        MigrationOp::CreateIndex { name, table, columns, .. } => {
            // BACKLOG item 285 — the index's own name is left as written
            // (it's a plain DBA-facing identifier, not part of `certo db
            // pull`'s table/column round-trip), but the table and every
            // column it indexes must be converted like everywhere else.
            let cols = columns.iter().map(|c| camel_to_snake(c)).collect::<Vec<_>>().join(", ");
            format!("CREATE INDEX {} ON {} ({});", name, camel_to_snake(table), cols)
        }
        MigrationOp::DropIndex   { name, .. }          => format!("DROP INDEX {};", name),
        // BACKLOG item 378 - statements are joined by newlines, so each needs its own terminator.
        MigrationOp::RawSql      { sql, .. }            => {
            let t = sql.trim_end();
            if t.ends_with(';') { t.to_string() } else { format!("{t};") }
        }
    })
}

fn create_table(table: &str, columns: &[ColumnDef]) -> Result<String, MigrateError> {
    let mut defs: Vec<String> = columns.iter().map(col_def).collect::<Result<_, _>>()?;

    // Collect composite primary key if more than one PK column
    let pk_cols: Vec<String> = columns.iter()
        .filter(|c| c.primary_key)
        .map(|c| camel_to_snake(&c.name))
        .collect();
    if pk_cols.len() > 1 {
        // Remove individual PK flags from column defs — recreate without PRIMARY KEY inline
        defs = columns.iter().map(|c| col_def_no_pk(c)).collect::<Result<_, _>>()?;
        defs.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    Ok(format!("CREATE TABLE {} (\n{}\n);", camel_to_snake(table), defs.join(",\n")))
}

fn col_def(col: &ColumnDef) -> Result<String, MigrateError> {
    let ty = te_to_sql(&col.ty.node);
    let mut s = format!("    {} {}", camel_to_snake(&col.name), ty);
    if !(col.nullable || is_option(&col.ty.node)) { s.push_str(" NOT NULL"); }
    if col.unique      { s.push_str(" UNIQUE"); }
    if col.primary_key { s.push_str(" PRIMARY KEY"); }
    if let Some(def) = &col.default {
        s.push_str(&format!(" DEFAULT {}", expr_to_sql_default(&def.node, &col.name)?));
    }
    Ok(s)
}

fn col_def_no_pk(col: &ColumnDef) -> Result<String, MigrateError> {
    let ty = te_to_sql(&col.ty.node);
    let mut s = format!("    {} {}", camel_to_snake(&col.name), ty);
    if !(col.nullable || is_option(&col.ty.node)) { s.push_str(" NOT NULL"); }
    if col.unique    { s.push_str(" UNIQUE"); }
    if let Some(def) = &col.default {
        s.push_str(&format!(" DEFAULT {}", expr_to_sql_default(&def.node, &col.name)?));
    }
    Ok(s)
}

/// `T?` columns are nullable (the type already says so).
fn is_option(te: &TypeExpr) -> bool { matches!(te, TypeExpr::Option { .. }) }

fn alter_table(table: &str, ops: &[AlterOp]) -> Result<String, MigrateError> {
    let clauses: Vec<String> = ops.iter().map(alter_op_clause).collect::<Result<_, _>>()?;
    Ok(format!("ALTER TABLE {}\n    {};", camel_to_snake(table), clauses.join(",\n    ")))
}

fn alter_op_clause(op: &AlterOp) -> Result<String, MigrateError> {
    Ok(match op {
        AlterOp::AddColumn { def } =>
            format!("ADD COLUMN {}", col_def(def)?.trim_start()),
        AlterOp::DropColumn { name, .. } =>
            format!("DROP COLUMN {}", camel_to_snake(name)),
        AlterOp::AddForeignKey { column, references, on_delete, .. } => {
            let action = fk_action(on_delete);
            format!("ADD FOREIGN KEY ({}) REFERENCES {} ON DELETE {}", camel_to_snake(column), camel_to_snake(references), action)
        }
    })
}

fn fk_action(a: &FkAction) -> &'static str {
    match a {
        FkAction::Cascade   => "CASCADE",
        FkAction::SetNull   => "SET NULL",
        FkAction::Restrict  => "RESTRICT",
        FkAction::NoAction  => "NO ACTION",
    }
}

fn te_to_sql(te: &TypeExpr) -> String {
    use TypeExpr::*;
    match te {
        Named { path, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("TEXT");
            match name {
                // BACKLOG item 380 - every built-in numeric/time/byte type has a real column type
                // (these used to fall through to TEXT).
                "Int" | "Int64" => "BIGINT",
                "Int32"    => "INTEGER",
                "Int16" | "Int8" => "SMALLINT",
                "UInt" | "UInt64" => "NUMERIC(20, 0)",
                "Float" | "Float64" => "DOUBLE PRECISION",
                "Float32"  => "REAL",
                "Bytes"    => "BYTEA",
                "Timestamp" => "TIMESTAMPTZ",
                "Decimal"  => "NUMERIC",
                "Bool"     => "BOOLEAN",
                "Text"     => "TEXT",
                "BoundedText" => "TEXT",
                "UUID"     => "UUID",
                "Date"     => "DATE",
                "DateTime" => "TIMESTAMPTZ",
                "Json"     => "JSONB",
                _          => "TEXT",
            }.to_string()
        }
        // `Decimal(p, s)`/`BoundedText(n)` (BACKLOG items 132/147) — both
        // previously fell through to the generic `_ => "TEXT"` catch-all
        // below (confirmed: `Decimal(19, 4)` migration columns silently
        // emitted `TEXT` DDL, dropping precision/scale entirely, never
        // actually fixed when item 132 shipped Decimal's own type support).
        // `NUMERIC(p, s)`/`VARCHAR(n)` are real Postgres syntax, so unlike
        // every other case in this function these two carry the parameter
        // through into the generated DDL rather than discarding it.
        DecimalParam { precision, scale, .. } => format!("NUMERIC({}, {})", precision, scale),
        BoundedTextParam { max_len, .. } => format!("VARCHAR({})", max_len),
        // `T?` is a nullable column of `T`'s type, not TEXT.
        Option { inner, .. } => te_to_sql(&inner.node),
        _             => "TEXT".to_string(),
    }
}

/// A column `default` expression as SQL (BACKLOG item 379): literals (including negative and
/// decimal numbers and UUIDs) and calls such as `now()` or `gen_random_uuid()`. Anything else is
/// an error instead of silently becoming `DEFAULT NULL`.
fn expr_to_sql_default(expr: &certo_ast::expr::Expr, column: &str) -> Result<String, MigrateError> {
    use certo_ast::expr::{Expr, Lit, UnOp};
    let unsupported = || MigrateError::Unsupported(format!(
        "the default for column `{column}` is not a literal or a function call the migration can express as SQL"));
    match expr {
        Expr::Lit { value, .. } => Ok(match value {
            Lit::Int(n)     => n.to_string(),
            Lit::Float(f)   => format!("{}", f),
            Lit::Decimal(d) => d.clone(),
            Lit::Bool(b)    => if *b { "TRUE".into() } else { "FALSE".into() },
            Lit::String(s)  => format!("'{}'", s.replace('\'', "''")),
            Lit::Uuid(u)    => format!("'{}'", u.replace('\'', "''")),
            Lit::Unit       => "NULL".into(),
            Lit::FString(_) => return Err(unsupported()),
        }),
        Expr::UnOp { op: UnOp::Neg, expr: inner, .. } => {
            let v = expr_to_sql_default(&inner.node, column)?;
            Ok(format!("-{v}"))
        }
        // `now()`, `gen_random_uuid()`, `uuid_generate_v4()`: a call is passed through by name.
        Expr::App { func, args, .. } => {
            let Expr::Path { path, .. } = &func.node else { return Err(unsupported()) };
            let name = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            let rendered: Vec<String> = args.iter()
                .map(|a| expr_to_sql_default(&a.value.node, column))
                .collect::<Result<_, _>>()?;
            Ok(format!("{}({})", name, rendered.join(", ")))
        }
        _ => Err(unsupported()),
    }
}
