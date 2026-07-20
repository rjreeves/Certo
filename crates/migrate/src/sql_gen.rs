use certo_ast::decl::{MigrationOp, AlterOp, ColumnDef, FkAction};
use certo_ast::types::TypeExpr;

/// Generate an SQL DDL string for a single `MigrationOp`.
pub fn op_to_sql(op: &MigrationOp) -> String {
    match op {
        MigrationOp::CreateTable { name, columns, .. } => create_table(name, columns),
        MigrationOp::DropTable   { name, .. }          => format!("DROP TABLE {};", name),
        MigrationOp::AlterTable  { name, ops, .. }     => alter_table(name, ops),
        MigrationOp::CreateIndex { name, table, columns, .. } => {
            let cols = columns.join(", ");
            format!("CREATE INDEX {} ON {} ({});", name, table, cols)
        }
        MigrationOp::DropIndex   { name, .. }          => format!("DROP INDEX {};", name),
        MigrationOp::RawSql      { sql, .. }            => sql.clone(),
    }
}

fn create_table(table: &str, columns: &[ColumnDef]) -> String {
    let mut defs: Vec<String> = columns.iter().map(col_def).collect();

    // Collect composite primary key if more than one PK column
    let pk_cols: Vec<&str> = columns.iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    if pk_cols.len() > 1 {
        // Remove individual PK flags from column defs — recreate without PRIMARY KEY inline
        defs = columns.iter().map(|c| col_def_no_pk(c)).collect();
        defs.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    format!("CREATE TABLE {} (\n{}\n);", table, defs.join(",\n"))
}

fn col_def(col: &ColumnDef) -> String {
    let ty = te_to_sql(&col.ty.node);
    let mut s = format!("    {} {}", col.name, ty);
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
    let mut s = format!("    {} {}", col.name, ty);
    if !col.nullable { s.push_str(" NOT NULL"); }
    if col.unique    { s.push_str(" UNIQUE"); }
    if let Some(def) = &col.default {
        s.push_str(&format!(" DEFAULT {}", expr_to_sql_default(&def.node)));
    }
    s
}

fn alter_table(table: &str, ops: &[AlterOp]) -> String {
    let clauses: Vec<String> = ops.iter().map(alter_op_clause).collect();
    format!("ALTER TABLE {}\n    {};", table, clauses.join(",\n    "))
}

fn alter_op_clause(op: &AlterOp) -> String {
    match op {
        AlterOp::AddColumn { def } =>
            format!("ADD COLUMN {}", col_def(def).trim_start()),
        AlterOp::DropColumn { name, .. } =>
            format!("DROP COLUMN {}", name),
        AlterOp::AddForeignKey { column, references, on_delete, .. } => {
            let action = fk_action(on_delete);
            format!("ADD FOREIGN KEY ({}) REFERENCES {} ON DELETE {}", column, references, action)
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

fn te_to_sql(te: &TypeExpr) -> &'static str {
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
                "UUID"     => "UUID",
                "Date"     => "DATE",
                "DateTime" => "TIMESTAMPTZ",
                "Json"     => "JSONB",
                _          => "TEXT",
            }
        }
        Option { .. } => "TEXT",
        _             => "TEXT",
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
