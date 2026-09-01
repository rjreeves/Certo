use certo_ast::decl::{MigrationOp, AlterOp, ColumnDef, FkAction};
use certo_ast::types::TypeExpr;
use certo_dbschema::camel_to_snake;

/// Generate an SQL DDL string for a single `MigrationOp`.
pub fn op_to_sql(op: &MigrationOp) -> String {
    match op {
        MigrationOp::CreateTable { name, columns, .. } => create_table(name, columns),
        MigrationOp::DropTable   { name, .. }          => format!("DROP TABLE {};", camel_to_snake(name)),
        MigrationOp::AlterTable  { name, ops, .. }     => alter_table(name, ops),
        MigrationOp::CreateIndex { name, table, columns, .. } => {
            // BACKLOG item 285 — the index's own name is left as written
            // (it's a plain DBA-facing identifier, not part of `certo db
            // pull`'s table/column round-trip), but the table and every
            // column it indexes must be converted like everywhere else.
            let cols = columns.iter().map(|c| camel_to_snake(c)).collect::<Vec<_>>().join(", ");
            format!("CREATE INDEX {} ON {} ({});", name, camel_to_snake(table), cols)
        }
        MigrationOp::DropIndex   { name, .. }          => format!("DROP INDEX {};", name),
        MigrationOp::RawSql      { sql, .. }            => sql.clone(),
    }
}

fn create_table(table: &str, columns: &[ColumnDef]) -> String {
    let mut defs: Vec<String> = columns.iter().map(col_def).collect();

    // Collect composite primary key if more than one PK column
    let pk_cols: Vec<String> = columns.iter()
        .filter(|c| c.primary_key)
        .map(|c| camel_to_snake(&c.name))
        .collect();
    if pk_cols.len() > 1 {
        // Remove individual PK flags from column defs — recreate without PRIMARY KEY inline
        defs = columns.iter().map(|c| col_def_no_pk(c)).collect();
        defs.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    format!("CREATE TABLE {} (\n{}\n);", camel_to_snake(table), defs.join(",\n"))
}

fn col_def(col: &ColumnDef) -> String {
    let ty = te_to_sql(&col.ty.node);
    let mut s = format!("    {} {}", camel_to_snake(&col.name), ty);
    if !col.nullable   { s.push_str(" NOT NULL"); }
    if col.unique      { s.push_str(" UNIQUE"); }
    if col.primary_key { s.push_str(" PRIMARY KEY"); }
    if let Some(def) = &col.default {
        s.push_str(&format!(" DEFAULT {}", expr_to_sql_default(&def.node)));
    }
    s
}

fn col_def_no_pk(col: &ColumnDef) -> String {
    let ty = te_to_sql(&col.ty.node);
    let mut s = format!("    {} {}", camel_to_snake(&col.name), ty);
    if !col.nullable { s.push_str(" NOT NULL"); }
    if col.unique    { s.push_str(" UNIQUE"); }
    if let Some(def) = &col.default {
        s.push_str(&format!(" DEFAULT {}", expr_to_sql_default(&def.node)));
    }
    s
}

fn alter_table(table: &str, ops: &[AlterOp]) -> String {
    let clauses: Vec<String> = ops.iter().map(alter_op_clause).collect();
    format!("ALTER TABLE {}\n    {};", camel_to_snake(table), clauses.join(",\n    "))
}

fn alter_op_clause(op: &AlterOp) -> String {
    match op {
        AlterOp::AddColumn { def } =>
            format!("ADD COLUMN {}", col_def(def).trim_start()),
        AlterOp::DropColumn { name, .. } =>
            format!("DROP COLUMN {}", camel_to_snake(name)),
        AlterOp::AddForeignKey { column, references, on_delete, .. } => {
            let action = fk_action(on_delete);
            format!("ADD FOREIGN KEY ({}) REFERENCES {} ON DELETE {}", camel_to_snake(column), camel_to_snake(references), action)
        }
    }
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
                "Int"      => "BIGINT",
                "Float"    => "DOUBLE PRECISION",
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
        Option { .. } => "TEXT".to_string(),
        _             => "TEXT".to_string(),
    }
}

fn expr_to_sql_default(expr: &certo_ast::expr::Expr) -> String {
    use certo_ast::expr::{Expr, Lit};
    match expr {
        Expr::Lit { value, .. } => match value {
            Lit::Int(n)    => n.to_string(),
            Lit::Float(f)  => format!("{}", f),
            Lit::Bool(b)   => if *b { "TRUE".into() } else { "FALSE".into() },
            Lit::String(s) => format!("'{}'", s.replace('\'', "''")),
            _              => "NULL".into(),
        },
        _ => "NULL".into(),
    }
}
