//! Read a live SQLite database back into a `SchemaIR`.
//!
//! SQLite keeps the declared type name exactly as written, which is how the
//! SQLite adapter's spellings (`UUID TEXT`, `DATE TEXT`, `ENUM_Role TEXT`, ...)
//! map back to SDL types. Enums come back from the inline `CHECK (col IN (...))`
//! on the columns that use them, so an enum no column uses cannot be seen
//! (`drift::align` accounts for that). Like the PostgreSQL reader, anything SDL
//! cannot express (multi-column unique or foreign keys, unique / partial /
//! expression indexes, unnamed CHECKs, columns of unknown type) is left out of
//! the IR and reported in `notes`, never guessed. Defaults and CHECK bodies come
//! back as `ExprIR::Raw` text.

use crate::introspect::{LiveSchema, HISTORY_TABLE};
use certo_sdl::{
    Builtin, ColumnIR, ConstraintIR, EnumIR, ExprIR, ForeignKeyIR, Generation, IndexIR, ReferentialAction, SchemaIR,
    TableIR, TypeIR,
};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};

/// One column of a foreign key: referenced table, local column, referenced column, on update, on delete.
type FkPart = (String, String, Option<String>, String, String);

fn q(s: &str) -> String { format!("'{}'", s.replace('\'', "''")) }

/// Declared type name -> SDL type (enums are handled by the caller).
fn map_type(declared: &str) -> Option<TypeIR> {
    let t = declared.trim();
    let up = t.to_uppercase();
    let b = match up.as_str() {
        "TEXT" => Builtin::Text,
        "UUID TEXT" => Builtin::Uuid,
        "JSON TEXT" => Builtin::Json,
        "DATE TEXT" => Builtin::Date,
        "TIMESTAMPTZ TEXT" => Builtin::Timestamp,
        "TIMESTAMP TEXT" => Builtin::TimestampNaive,
        "SMALLINT" => Builtin::SmallInt,
        "INTEGER" => Builtin::Int,
        "BIGINT" => Builtin::BigInt,
        "NUMERIC" => Builtin::Decimal,
        "REAL" => Builtin::Real,
        "DOUBLE" => Builtin::Float,
        "BOOLEAN" => Builtin::Bool,
        "BLOB" => Builtin::Bytes,
        _ => {
            let (name, args) = up.split_once('(').and_then(|(n, r)| Some((n.trim(), r.strip_suffix(')')?)))?;
            let nums: Vec<u32> = args.split(',').map(|a| a.trim().parse().ok()).collect::<Option<_>>()?;
            return match (name, nums.as_slice()) {
                ("VARCHAR", [n]) if *n >= 1 => Some(TypeIR::Builtin(Builtin::Varchar(*n))),
                ("CHAR", [n]) if *n >= 1 => Some(TypeIR::Builtin(Builtin::Char(*n))),
                ("NUMERIC", [p, s]) if *p >= 1 && s <= p => Some(TypeIR::Builtin(Builtin::Numeric(*p as u16, *s as u16))),
                _ => None,
            };
        }
    };
    Some(TypeIR::Builtin(b))
}

/// `ENUM_Role TEXT` -> `Role`.
fn enum_name(declared: &str) -> Option<String> {
    let t = declared.trim();
    let body = t.get(..t.len().checked_sub(5)?).filter(|_| t.to_uppercase().ends_with(" TEXT"))?;
    let name = body.get(5..).filter(|_| body.to_uppercase().starts_with("ENUM_"))?;
    (!name.is_empty()).then(|| name.to_string())
}

fn action(s: &str) -> Option<ReferentialAction> {
    Some(match s {
        "NO ACTION" => ReferentialAction::NoAction,
        "RESTRICT" => ReferentialAction::Restrict,
        "CASCADE" => ReferentialAction::Cascade,
        "SET NULL" => ReferentialAction::SetNull,
        _ => return None,
    })
}

// ---- reading the CREATE TABLE text ---------------------------------------- //

#[derive(Default, Debug)]
struct TableSql {
    /// Inline `CHECK (...)` bodies per column.
    column_checks: HashMap<String, Vec<String>>,
    /// `CONSTRAINT name CHECK (...)` (name `None` for an unnamed CHECK).
    table_checks: Vec<(Option<String>, String)>,
    autoincrement: bool,
}

/// Split `s` at commas that are outside quotes and parentheses.
fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    let mut quote: Option<char> = None;
    for (i, ch) in s.char_indices() {
        match (quote, ch) {
            (Some(qc), c) if c == qc => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(ch),
            (None, '[') => quote = Some(']'),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Text between the parentheses that start at byte `open` (which must be `(`).
fn group_at(s: &str, open: usize) -> Option<(&str, usize)> {
    let (mut depth, mut quote) = (0i32, None::<char>);
    for (i, ch) in s[open..].char_indices() {
        match (quote, ch) {
            (Some(qc), c) if c == qc => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[open + 1..open + i], open + i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `CHECK (...)` body in `item`, found outside quotes.
fn checks_in(item: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut quote = None::<char>;
    let bytes: Vec<(usize, char)> = item.char_indices().collect();
    let mut k = 0;
    while k < bytes.len() {
        let (i, ch) = bytes[k];
        match (quote, ch) {
            (Some(qc), c) if c == qc => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            _ => {
                let word_start = i == 0 || !item[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
                if word_start && item[i..].len() >= 5 && item[i..i + 5].eq_ignore_ascii_case("check") {
                    let rest = item[i + 5..].trim_start();
                    if rest.starts_with('(') {
                        let open = item.len() - rest.len();
                        if let Some((body, end)) = group_at(item, open) {
                            out.push(body.trim().to_string());
                            while k < bytes.len() && bytes[k].0 < end {
                                k += 1;
                            }
                            continue;
                        }
                    }
                }
            }
        }
        k += 1;
    }
    out
}

/// First identifier of `s` (quoted or bare) and the rest.
fn first_ident(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let first = s.chars().next()?;
    if matches!(first, '"' | '`' | '[' | '\'') {
        let close = if first == '[' { ']' } else { first };
        let end = s[1..].find(close)? + 1;
        return Some((s[1..end].to_string(), &s[end + 1..]));
    }
    let end = s.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(s.len());
    (end > 0).then(|| (s[..end].to_string(), &s[end..]))
}

fn parse_table_sql(sql: &str) -> TableSql {
    let mut out = TableSql::default();
    let Some(open) = sql.find('(') else { return out };
    let Some((body, _)) = group_at(sql, open) else { return out };
    for item in split_top(body) {
        let Some((first, rest)) = first_ident(item) else { continue };
        let kw = first.to_uppercase();
        if kw == "CONSTRAINT" {
            // CONSTRAINT name CHECK (...)
            if let Some((name, after)) = first_ident(rest) {
                for c in checks_in(after) {
                    out.table_checks.push((Some(name.clone()), c));
                }
            }
        } else if kw == "CHECK" {
            for c in checks_in(item) {
                out.table_checks.push((None, c));
            }
        } else if matches!(kw.as_str(), "PRIMARY" | "UNIQUE" | "FOREIGN") {
            // table-level key constraints: read through pragmas instead
        } else {
            let checks = checks_in(rest);
            if !checks.is_empty() {
                out.column_checks.entry(first.clone()).or_default().extend(checks);
            }
            if rest.to_uppercase().contains("AUTOINCREMENT") {
                out.autoincrement = true;
            }
        }
    }
    out
}

/// Values of `"col" IN ('a', 'b')`.
fn enum_values(check: &str, col: &str) -> Option<Vec<String>> {
    let (name, rest) = first_ident(check)?;
    if name != col {
        return None;
    }
    let rest = rest.trim_start();
    let rest = rest.get(..2).filter(|w| w.eq_ignore_ascii_case("in")).map(|_| rest[2..].trim_start())?;
    if !rest.starts_with('(') {
        return None;
    }
    let (list, end) = group_at(rest, 0)?;
    if !rest[end..].trim().is_empty() {
        return None;
    }
    split_top(list)
        .into_iter()
        .map(|v| {
            let v = v.trim();
            v.strip_prefix('\'')?.strip_suffix('\'').map(|s| s.replace("''", "'"))
        })
        .collect()
}

// ---- the reader ------------------------------------------------------------ //

pub fn introspect(conn: &Connection) -> rusqlite::Result<LiveSchema> {
    let mut notes: Vec<String> = Vec::new();

    let mut names: Vec<(String, String)> = Vec::new();
    {
        let mut st = conn.prepare(
            "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))? {
            let (n, sql) = r?;
            if n == HISTORY_TABLE || n == crate::journal::LOG_TABLE {
                continue;
            }
            names.push((n, sql));
        }
    }
    {
        let mut st = conn.prepare("SELECT type, name FROM sqlite_master WHERE type IN ('view', 'trigger') ORDER BY name")?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (t, n) = r?;
            notes.push(format!("{t} {n} is not represented in the schema"));
        }
    }

    let mut enums: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut tables = Vec::new();

    for (tname, sql) in &names {
        let parsed = parse_table_sql(sql);

        struct Info {
            name: String,
            ty: String,
            notnull: bool,
            default: Option<String>,
            pk: i64,
        }
        let mut infos = Vec::new();
        {
            let mut st = conn.prepare(&format!(
                "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo({}) WHERE hidden = 0 ORDER BY cid",
                q(tname)
            ))?;
            for r in st.query_map([], |r| {
                Ok(Info {
                    name: r.get(0)?,
                    ty: r.get(1)?,
                    notnull: r.get::<_, i64>(2)? != 0,
                    default: r.get(3)?,
                    pk: r.get(4)?,
                })
            })? {
                infos.push(r?);
            }
        }
        let pk_count = infos.iter().filter(|i| i.pk > 0).count();

        // unique constraints and indexes
        let mut unique_cols: Vec<String> = Vec::new();
        let mut indexes = Vec::new();
        {
            let mut st = conn.prepare(&format!(
                "SELECT name, \"unique\", origin, partial FROM pragma_index_list({})",
                q(tname)
            ))?;
            let list: Vec<(String, bool, String, bool)> = st
                .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?, r.get::<_, i64>(3)? != 0)))?
                .collect::<rusqlite::Result<_>>()?;
            for (iname, unique, origin, partial) in list {
                if origin == "pk" {
                    continue;
                }
                let mut cols: Vec<Option<String>> = Vec::new();
                let mut st2 = conn.prepare(&format!(
                    "SELECT name FROM pragma_index_xinfo({}) WHERE key = 1 ORDER BY seqno",
                    q(&iname)
                ))?;
                for c in st2.query_map([], |r| r.get::<_, Option<String>>(0))? {
                    cols.push(c?);
                }
                if origin == "u" {
                    match cols.as_slice() {
                        [Some(c)] => unique_cols.push(c.clone()),
                        _ => notes.push(format!("multi-column unique constraint on {tname} is not represented")),
                    }
                    continue;
                }
                // origin "c": CREATE INDEX
                if unique {
                    notes.push(format!("unique index {iname} on {tname} is not represented"));
                } else if partial {
                    notes.push(format!("partial index {iname} on {tname} is not represented"));
                } else if cols.iter().any(Option::is_none) {
                    notes.push(format!("expression index {iname} on {tname} is not represented"));
                } else {
                    indexes.push(IndexIR { name: iname, columns: cols.into_iter().flatten().collect() });
                }
            }
        }

        // foreign keys
        let mut fks: BTreeMap<i64, Vec<FkPart>> = BTreeMap::new();
        {
            let mut st = conn.prepare(&format!(
                "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list({}) ORDER BY id, seq",
                q(tname)
            ))?;
            for r in st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?))
            })? {
                let (id, t, from, to, on_update, on_delete) = r?;
                fks.entry(id).or_default().push((t, from, to, on_update, on_delete));
            }
        }
        let mut references: HashMap<String, ForeignKeyIR> = HashMap::new();
        for (_, parts) in fks {
            if parts.len() != 1 {
                notes.push(format!("multi-column foreign key on {tname} is not represented"));
                continue;
            }
            let (rt, from, to, on_update, on_delete) = parts.into_iter().next().unwrap();
            let to = match to {
                Some(t) => t,
                None => {
                    // REFERENCES other: the other table's primary key
                    let mut st = conn.prepare(&format!("SELECT name FROM pragma_table_info({}) WHERE pk = 1", q(&rt)))?;
                    let pks: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    match pks.as_slice() {
                        [one] => one.clone(),
                        _ => {
                            notes.push(format!("foreign key {tname}.{from} references a table without a single-column primary key"));
                            continue;
                        }
                    }
                }
            };
            let (Some(u), Some(d)) = (action(&on_update), action(&on_delete)) else {
                notes.push(format!("foreign key {tname}.{from} uses a referential action SDL does not have"));
                continue;
            };
            references.insert(from, ForeignKeyIR { table: rt, column: to, on_delete: d, on_update: u });
        }

        // columns
        let mut columns = Vec::new();
        for i in &infos {
            let mut ty = map_type(&i.ty);
            if ty.is_none()
                && let Some(en) = enum_name(&i.ty)
            {
                let values = parsed
                    .column_checks
                    .get(&i.name)
                    .and_then(|cs| cs.iter().find_map(|c| enum_values(c, &i.name)));
                match values {
                    Some(v) => {
                        match enums.get(&en) {
                            Some(existing) if *existing != v => {
                                notes.push(format!("enum {en} has different values in different columns; using the first"));
                            }
                            Some(_) => {}
                            None => {
                                enums.insert(en.clone(), v);
                            }
                        }
                        ty = Some(TypeIR::Enum(en));
                    }
                    None => notes.push(format!("column {tname}.{} is declared as enum {en} but has no readable CHECK", i.name)),
                }
            }
            let Some(ty) = ty else {
                if enum_name(&i.ty).is_none() {
                    notes.push(format!("column {tname}.{} has type `{}`, which SDL cannot express; left out", i.name, i.ty));
                }
                continue;
            };
            let is_pk = i.pk > 0;
            let auto = is_pk && pk_count == 1 && parsed.autoincrement && matches!(ty, TypeIR::Builtin(Builtin::Int | Builtin::SmallInt | Builtin::BigInt));
            columns.push(ColumnIR {
                name: i.name.clone(),
                ty,
                primary_key: is_pk,
                unique: !is_pk && unique_cols.contains(&i.name),
                nullable: !i.notnull && !is_pk,
                default: i.default.clone().map(|sql| ExprIR::Raw { sql }),
                references: references.remove(&i.name),
                generated: auto.then_some(Generation::Serial),
            });
        }
        for (col, _) in references {
            notes.push(format!("foreign key on {tname}.{col} is on a column that was left out"));
        }

        // CHECK constraints (enum membership checks are the enum, not constraints)
        let mut constraints = Vec::new();
        for (name, body) in &parsed.table_checks {
            match name {
                Some(n) => constraints.push(ConstraintIR { name: n.clone(), expr: ExprIR::Raw { sql: body.clone() } }),
                None => notes.push(format!("unnamed CHECK ({body}) on {tname} is not represented")),
            }
        }
        for (col, checks) in &parsed.column_checks {
            let is_enum_col = infos.iter().any(|i| &i.name == col && enum_name(&i.ty).is_some());
            for c in checks {
                if !(is_enum_col && enum_values(c, col).is_some()) {
                    notes.push(format!("CHECK ({c}) on column {tname}.{col} is not represented"));
                }
            }
        }

        indexes.sort_by(|a: &IndexIR, b| a.name.cmp(&b.name));
        constraints.sort_by(|a: &ConstraintIR, b| a.name.cmp(&b.name));
        tables.push(TableIR { name: tname.clone(), columns, relationships: vec![], indexes, constraints });
    }

    let enums: Vec<EnumIR> = enums.into_iter().map(|(name, variants)| EnumIR { name, variants }).collect();
    Ok(LiveSchema {
        ir: SchemaIR { version: certo_sdl::IR_VERSION, tables, enums, types: vec![], sequences: vec![] },
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_types_map_back() {
        assert_eq!(map_type("uuid text"), Some(TypeIR::Builtin(Builtin::Uuid)));
        assert_eq!(map_type("VARCHAR(20)"), Some(TypeIR::Builtin(Builtin::Varchar(20))));
        assert_eq!(map_type("NUMERIC(10,2)"), Some(TypeIR::Builtin(Builtin::Numeric(10, 2))));
        assert_eq!(map_type("DATETIME"), None);
        assert_eq!(enum_name("ENUM_Role TEXT").as_deref(), Some("Role"));
        assert_eq!(enum_name("TEXT"), None);
    }

    #[test]
    fn create_table_text_is_read_for_checks() {
        let sql = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,\n    \"r\" ENUM_Role TEXT NOT NULL DEFAULT 'a, b' CHECK (\"r\" IN ('a, b', 'c')),\n    CONSTRAINT \"pos\" CHECK ((\"n\" > 0)),\n    CONSTRAINT \"t_x_key\" UNIQUE (\"x\")\n)";
        let p = parse_table_sql(sql);
        assert!(p.autoincrement);
        assert_eq!(p.column_checks["r"], ["\"r\" IN ('a, b', 'c')"]);
        assert_eq!(p.table_checks, [(Some("pos".to_string()), "(\"n\" > 0)".to_string())]);
        assert_eq!(enum_values(&p.column_checks["r"][0], "r"), Some(vec!["a, b".to_string(), "c".to_string()]));
    }
}
