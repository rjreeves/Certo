//! Read a live MySQL 8 schema back into a `SchemaIR` (the current database).
//!
//! The mapping inverts `certo_sql`'s MySQL lowering. Where a MySQL type alone does not say what the column is, the column
//! comment does (`certo:enum Name`, `certo:uuid`, `certo:timestamptz`): without those a `CHAR(36)` is a char and a
//! `DATETIME(6)` is a naive timestamp. Anything SDL cannot express is left out and reported in `notes`, never guessed.
//! Defaults and CHECK bodies come back as `ExprIR::Raw` in the PostgreSQL-flavoured text of `mysqlexpr`, which
//! `drift::align` reconciles with the expected expressions.

use crate::introspect::{LiveSchema, HISTORY_TABLE};
use crate::mysqlexpr;
use certo_sdl::{
    Builtin, ColumnIR, ConstraintIR, EnumIR, ExprIR, ForeignKeyIR, Generation, IndexIR, ReferentialAction, SchemaIR, TableIR, TypeIR,
};
use mysql::prelude::Queryable;
use mysql::Conn;
use std::collections::{BTreeMap, HashMap, HashSet};

/// `enum('a','b','it''s')` -> its values.
fn enum_values(column_type: &str) -> Option<Vec<String>> {
    let body = column_type.strip_prefix("enum(")?.strip_suffix(')')?;
    let c: Vec<char> = body.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < c.len() {
        if c[i] != '\'' {
            return None;
        }
        i += 1;
        let mut v = String::new();
        loop {
            match c.get(i)? {
                '\'' if c.get(i + 1) == Some(&'\'') => {
                    v.push('\'');
                    i += 2;
                }
                '\'' => {
                    i += 1;
                    break;
                }
                '\\' if c.get(i + 1).is_some() => {
                    v.push(c[i + 1]);
                    i += 2;
                }
                &ch => {
                    v.push(ch);
                    i += 1;
                }
            }
        }
        out.push(v);
        match c.get(i) {
            None => break,
            Some(',') => i += 1,
            Some(_) => return None,
        }
    }
    Some(out)
}

/// `varchar(100)` -> 100.
fn length_of(column_type: &str) -> Option<u32> {
    column_type.split_once('(')?.1.split(')').next()?.split(',').next()?.trim().parse().ok()
}

/// The schema type of a column, or why SDL has none.
fn map_type(column_type: &str, data_type: &str, comment: &str) -> Result<TypeIR, String> {
    let unsigned = column_type.contains("unsigned");
    let b = |b: Builtin| Ok(TypeIR::Builtin(b));
    if let Some(name) = comment.strip_prefix("certo:enum ") {
        return match enum_values(column_type) {
            Some(_) => Ok(TypeIR::Enum(name.to_string())),
            None => Err(format!("{column_type} is not an enum")),
        };
    }
    match data_type {
        "enum" => Err("an enum column that certo did not create (its name is unknown)".into()),
        "varchar" => length_of(column_type).map(Builtin::Varchar).map(TypeIR::Builtin).ok_or_else(|| column_type.to_string()),
        "char" if comment == "certo:uuid" => b(Builtin::Uuid),
        "char" => length_of(column_type).map(Builtin::Char).map(TypeIR::Builtin).ok_or_else(|| column_type.to_string()),
        "text" | "tinytext" | "mediumtext" | "longtext" => b(Builtin::Text),
        "tinyint" if column_type == "tinyint(1)" => b(Builtin::Bool),
        "smallint" if !unsigned => b(Builtin::SmallInt),
        "int" if !unsigned => b(Builtin::Int),
        "bigint" if !unsigned => b(Builtin::BigInt),
        "decimal" => {
            let args = column_type.split_once('(').and_then(|(_, r)| r.split(')').next()).unwrap_or("");
            let mut it = args.split(',').map(|x| x.trim().parse::<u16>());
            match (it.next(), it.next()) {
                (Some(Ok(65)), Some(Ok(30))) => b(Builtin::Decimal),
                (Some(Ok(p)), Some(Ok(s))) if s <= p => b(Builtin::Numeric(p, s)),
                (Some(Ok(p)), None) => b(Builtin::Numeric(p, 0)),
                _ => Err(column_type.to_string()),
            }
        }
        "float" => b(Builtin::Real),
        "double" => b(Builtin::Float),
        "date" => b(Builtin::Date),
        "datetime" if comment == "certo:timestamptz" => b(Builtin::Timestamp),
        "datetime" => b(Builtin::TimestampNaive),
        "timestamp" => b(Builtin::Timestamp),
        "json" => b(Builtin::Json),
        "tinyblob" | "blob" | "mediumblob" | "longblob" => b(Builtin::Bytes),
        _ => Err(column_type.to_string()),
    }
}

fn action(rule: &str) -> ReferentialAction {
    match rule {
        "CASCADE" => ReferentialAction::Cascade,
        "SET NULL" => ReferentialAction::SetNull,
        "RESTRICT" => ReferentialAction::Restrict,
        _ => ReferentialAction::NoAction,
    }
}

/// A default that is not an expression: the value as text, written as SQL for the column's type.
fn literal_default(value: &str, ty: &TypeIR) -> String {
    match ty {
        TypeIR::Builtin(Builtin::Bool) => (if value == "0" { "false" } else { "true" }).to_string(),
        TypeIR::Builtin(
            Builtin::SmallInt | Builtin::Int | Builtin::BigInt | Builtin::Decimal | Builtin::Numeric(..) | Builtin::Real | Builtin::Float,
        ) => value.to_string(),
        _ => format!("'{}'", value.replace('\'', "''")),
    }
}

pub fn introspect(conn: &mut Conn) -> Result<LiveSchema, mysql::Error> {
    let mut notes: Vec<String> = Vec::new();
    let reserved: HashSet<&str> =
        [HISTORY_TABLE, crate::journal::LOG_TABLE, crate::views::VIEWS_TABLE, crate::mysql_exec::PROGRESS_TABLE].into_iter().collect();

    // ---- tables and views ------------------------------------------------------------------
    let objects: Vec<(String, String)> = conn.query(
        "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name",
    )?;
    let mut table_names: Vec<String> = Vec::new();
    let mut views: Vec<String> = Vec::new();
    for (name, kind) in objects {
        if reserved.contains(name.as_str()) {
            continue;
        }
        if kind == "VIEW" {
            views.push(name.clone());
            notes.push(format!("view {name} is not represented in the schema"));
        } else {
            table_names.push(name);
        }
    }
    let known: HashSet<&str> = table_names.iter().map(String::as_str).collect();

    // ---- columns ---------------------------------------------------------------------------
    type Row = (String, String, String, String, String, Option<String>, String, String, String);
    let rows: Vec<Row> = conn.query(
        "SELECT table_name, column_name, column_type, data_type, is_nullable, column_default, extra, column_key, column_comment
         FROM information_schema.columns WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position",
    )?;
    let mut tables: BTreeMap<String, Vec<ColumnIR>> = BTreeMap::new();
    let mut enums: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (table, col, column_type, data_type, nullable, default, extra, key, comment) in rows {
        if !known.contains(table.as_str()) {
            continue;
        }
        let ty = match map_type(&column_type, &data_type, &comment) {
            Ok(t) => t,
            Err(why) => {
                notes.push(format!("column {table}.{col} has type {column_type}, which is not represented ({why})"));
                continue;
            }
        };
        if let (TypeIR::Enum(name), Some(values)) = (&ty, enum_values(&column_type)) {
            match enums.get(name) {
                Some(have) if *have != values => notes.push(format!("enum {name} has different values on different columns (kept those of the first)")),
                Some(_) => {}
                None => {
                    enums.insert(name.clone(), values);
                }
            }
        }
        let auto = extra.contains("auto_increment");
        let default = match default {
            Some(text) if !auto => {
                let sql = if extra.contains("DEFAULT_GENERATED") { mysqlexpr::from_server(&text) } else { literal_default(&text, &ty) };
                Some(ExprIR::Raw { sql })
            }
            _ => None,
        };
        tables.entry(table).or_default().push(ColumnIR {
            name: col,
            ty,
            primary_key: key == "PRI",
            unique: false,
            nullable: nullable == "YES" && key != "PRI",
            default,
            references: None,
            generated: auto.then_some(Generation::Serial),
        });
    }

    // ---- foreign keys ----------------------------------------------------------------------
    type Fk = (String, String, String, String, String, String, String);
    /// column, referenced table, referenced column, update rule, delete rule
    type FkColumn = (String, String, String, String, String);
    let fks: Vec<Fk> = conn.query(
        "SELECT k.table_name, k.constraint_name, k.column_name, k.referenced_table_name, k.referenced_column_name, r.update_rule, r.delete_rule
         FROM information_schema.key_column_usage k
         JOIN information_schema.referential_constraints r
           ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name AND r.table_name = k.table_name
         WHERE k.table_schema = DATABASE() AND k.referenced_table_name IS NOT NULL
         ORDER BY k.table_name, k.constraint_name, k.ordinal_position",
    )?;
    let mut fk_names: HashMap<String, HashSet<String>> = HashMap::new();
    let mut fk_parts: BTreeMap<(String, String), Vec<FkColumn>> = BTreeMap::new();
    for (table, name, col, rtable, rcol, on_update, on_delete) in fks {
        fk_names.entry(table.clone()).or_default().insert(name.clone());
        fk_parts.entry((table, name)).or_default().push((col, rtable, rcol, on_update, on_delete));
    }
    for ((table, name), parts) in fk_parts {
        if parts.len() != 1 {
            notes.push(format!("foreign key {name} on {table} has several columns, which is not represented"));
            continue;
        }
        let (col, rtable, rcol, on_update, on_delete) = parts.into_iter().next().unwrap();
        if let Some(c) = tables.get_mut(&table).and_then(|cs| cs.iter_mut().find(|c| c.name == col)) {
            c.references = Some(ForeignKeyIR { table: rtable, column: rcol, on_delete: action(&on_delete), on_update: action(&on_update) });
        }
    }

    // ---- indexes ---------------------------------------------------------------------------
    type Idx = (String, String, u32, String, i64, String);
    let idx: Vec<Idx> = conn.query(
        "SELECT table_name, index_name, seq_in_index, column_name, non_unique, index_type FROM information_schema.statistics
         WHERE table_schema = DATABASE() AND index_name <> 'PRIMARY' ORDER BY table_name, index_name, seq_in_index",
    )?;
    let mut groups: BTreeMap<(String, String), (bool, String, Vec<String>)> = BTreeMap::new();
    for (table, name, _, col, non_unique, kind) in idx {
        let g = groups.entry((table, name)).or_insert((non_unique == 0, kind, Vec::new()));
        g.2.push(col);
    }
    let mut indexes: BTreeMap<String, Vec<IndexIR>> = BTreeMap::new();
    for ((table, name), (unique, kind, cols)) in groups {
        if !known.contains(table.as_str()) {
            continue;
        }
        if kind != "BTREE" {
            notes.push(format!("{kind} index {name} on {table} is not represented"));
        } else if unique && cols.len() == 1 {
            if let Some(c) = tables.get_mut(&table).and_then(|cs| cs.iter_mut().find(|c| c.name == cols[0])) {
                c.unique = true;
            }
        } else if unique {
            notes.push(format!("unique index {name} on {table} covers several columns, which is not represented"));
        } else if fk_names.get(&table).is_some_and(|n| n.contains(&name)) {
            // the index MySQL made to back a foreign key: part of the key, not a schema object
        } else {
            indexes.entry(table).or_default().push(IndexIR { name, columns: cols });
        }
    }

    // ---- CHECK constraints -------------------------------------------------------------------
    let checks: Vec<(String, String, String)> = conn.query(
        "SELECT t.table_name, t.constraint_name, c.check_clause FROM information_schema.table_constraints t
         JOIN information_schema.check_constraints c ON c.constraint_schema = t.constraint_schema AND c.constraint_name = t.constraint_name
         WHERE t.table_schema = DATABASE() AND t.constraint_type = 'CHECK' ORDER BY t.table_name, t.constraint_name",
    )?;
    let mut constraints: BTreeMap<String, Vec<ConstraintIR>> = BTreeMap::new();
    for (table, name, clause) in checks {
        if known.contains(table.as_str()) {
            constraints.entry(table).or_default().push(ConstraintIR { name, expr: ExprIR::Raw { sql: mysqlexpr::from_server(&clause) } });
        }
    }

    // ---- what SDL has no place for -------------------------------------------------------------
    let triggers: Vec<(String, String)> = conn
        .query("SELECT trigger_name, event_object_table FROM information_schema.triggers WHERE trigger_schema = DATABASE() ORDER BY trigger_name")?;
    for (name, table) in triggers {
        notes.push(format!("trigger {name} on {table} is not represented in the schema"));
    }

    // ---- assemble ----------------------------------------------------------------------------
    let ir = SchemaIR {
        version: certo_sdl::IR_VERSION,
        tables: tables
            .into_iter()
            .map(|(name, columns)| TableIR {
                relationships: Vec::new(),
                indexes: indexes.remove(&name).unwrap_or_default(),
                constraints: constraints.remove(&name).unwrap_or_default(),
                columns,
                name,
                view: false,
            })
            .collect(),
        enums: enums.into_iter().map(|(name, variants)| EnumIR { name, variants }).collect(),
        types: Vec::new(),
        sequences: Vec::new(),
        views: Vec::new(),
    };
    Ok(LiveSchema { ir, notes, views })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_values_are_read_with_their_quotes() {
        assert_eq!(enum_values("enum('a','b')"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(enum_values("enum('it''s','x,y')"), Some(vec!["it's".into(), "x,y".into()]));
        assert_eq!(enum_values("varchar(5)"), None);
    }

    #[test]
    fn types_map_back_by_comment_and_shape() {
        use Builtin::*;
        let t = |ct, dt, cm| map_type(ct, dt, cm);
        assert_eq!(t("varchar(100)", "varchar", ""), Ok(TypeIR::Builtin(Varchar(100))));
        assert_eq!(t("char(36)", "char", "certo:uuid"), Ok(TypeIR::Builtin(Uuid)));
        assert_eq!(t("char(36)", "char", ""), Ok(TypeIR::Builtin(Char(36))));
        assert_eq!(t("datetime(6)", "datetime", "certo:timestamptz"), Ok(TypeIR::Builtin(Timestamp)));
        assert_eq!(t("datetime(6)", "datetime", ""), Ok(TypeIR::Builtin(TimestampNaive)));
        assert_eq!(t("decimal(65,30)", "decimal", ""), Ok(TypeIR::Builtin(Decimal)));
        assert_eq!(t("decimal(10,2)", "decimal", ""), Ok(TypeIR::Builtin(Numeric(10, 2))));
        assert_eq!(t("tinyint(1)", "tinyint", ""), Ok(TypeIR::Builtin(Bool)));
        assert_eq!(t("enum('a','b')", "enum", "certo:enum Role"), Ok(TypeIR::Enum("Role".into())));
        assert!(t("tinyint", "tinyint", "").is_err() && t("int unsigned", "int", "").is_err() && t("enum('a')", "enum", "").is_err());
    }
}
