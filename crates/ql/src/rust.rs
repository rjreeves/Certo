//! Rust code generation from compiled QL statements: the typed contract (parameter types, result
//! columns with nullability) becomes Rust types, so a host calls `recent_orders(&mut client, min_total,
//! since)` and gets `Vec<RecentOrdersRow>` instead of binding `$1` and reading columns by hand.
//!
//! Plain driver code, synchronous, one driver per dialect: PostgreSQL uses the `postgres` crate (any
//! `GenericClient`: a `Client` or a `Transaction`), SQLite uses `rusqlite` (a `Connection`, which a
//! `Transaction` derefs to), MySQL uses the `mysql` crate (anything `Queryable`: a `Conn`, a `PooledConn` or a
//! `Transaction`). MySQL has no `RETURNING`, so a statement that returns rows there is a query.
//!
//! * One `pub struct <Name>Row` per statement with result columns and one function per statement
//!   (`snake_case` of its name). A mutation without `returning` returns the affected-row count.
//! * Nullable parameters and columns are `Option<T>`. Text and bytes are taken as `&str` / `&[u8]`.
//! * Enums become Rust enums with `as_db` / `from_db`, and read and bind as their database text.
//! * Every statement's SQL is also exposed as a constant (`<NAME>_SQL`).
//! * `decimal` is `rust_decimal::Decimal`, `uuid` is `uuid::Uuid`, timestamps are `chrono::DateTime<Utc>`
//!   (`timestamp`) and `chrono::NaiveDateTime` (`timestamp naive`), dates `chrono::NaiveDate`, and `json`
//!   is a small generated `Json(String)`. The file's first lines say which crates and features it needs.
//!   Under SQLite these are stored as text (decimals as numbers): compare timestamps in the application.

use crate::{lower_mutation_with, lower_with, LowerOptions, MutationKind, ParamIR, Statement};
use certo_sdl::{Builtin, SchemaIR, TypeIR};
use certo_sql::Dialect;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct RustOptions {
    /// The dialect the statements were compiled for: it decides which driver the code calls.
    pub dialect: Dialect,
}

impl Default for RustOptions {
    fn default() -> Self {
        RustOptions { dialect: Dialect::Postgres }
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait",
    "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield",
];
/// Keywords that cannot be written as raw identifiers.
const NO_RAW: &[&str] = &["self", "Self", "super", "crate"];

/// `customer_id` -> `CustomerId`, `recentOrders` -> `RecentOrders`. Always a valid type name.
fn pascal(s: &str) -> String {
    let mut out = String::new();
    for part in s.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()) {
        let mut cs = part.chars();
        if let Some(f) = cs.next() {
            out.extend(f.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    if out.is_empty() {
        out.push_str("Item");
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if NO_RAW.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// `RecentOrders` / `recentOrders` / `recent-orders` -> `recent_orders`, escaped if it is a keyword.
fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower {
                out.push('_');
            }
            out.extend(c.to_lowercase());
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    let mut out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        out.push_str("item");
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    escape(out)
}

fn escape(name: String) -> String {
    if NO_RAW.contains(&name.as_str()) {
        format!("{name}_")
    } else if KEYWORDS.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}

/// Make `name` unique among `taken`, by appending a number.
fn unique(name: String, taken: &mut HashSet<String>) -> String {
    let mut n = name.clone();
    let mut i = 2;
    while !taken.insert(n.clone()) {
        n = format!("{name}{i}");
        i += 1;
    }
    n
}

/// A Rust type for a column or parameter, before nullability.
struct Rs {
    /// The type as a field of a row (owned).
    owned: String,
    /// The type as a parameter (borrowed for text and bytes).
    param: String,
    kind: Kind,
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Plain,
    Decimal,
    Uuid,
    Timestamp,
    Json,
    Enum,
}

fn rs_type(t: &TypeIR, enums: &BTreeMap<String, String>) -> Rs {
    let plain = |s: &str| Rs { owned: s.into(), param: s.into(), kind: Kind::Plain };
    match t {
        TypeIR::Builtin(b) => match b {
            Builtin::Text | Builtin::Varchar(_) | Builtin::Char(_) => Rs { owned: "String".into(), param: "&str".into(), kind: Kind::Plain },
            Builtin::Json => Rs { owned: "Json".into(), param: "&Json".into(), kind: Kind::Json },
            Builtin::SmallInt => plain("i16"),
            Builtin::Int => plain("i32"),
            Builtin::BigInt => plain("i64"),
            Builtin::Decimal | Builtin::Numeric(..) => Rs { kind: Kind::Decimal, ..plain("rust_decimal::Decimal") },
            Builtin::Real => plain("f32"),
            Builtin::Float => plain("f64"),
            Builtin::Bool => plain("bool"),
            Builtin::Uuid => Rs { kind: Kind::Uuid, ..plain("uuid::Uuid") },
            Builtin::Timestamp => Rs { kind: Kind::Timestamp, ..plain("chrono::DateTime<chrono::Utc>") },
            Builtin::TimestampNaive => plain("chrono::NaiveDateTime"),
            Builtin::Date => plain("chrono::NaiveDate"),
            Builtin::Bytes => Rs { owned: "Vec<u8>".into(), param: "&[u8]".into(), kind: Kind::Plain },
        },
        TypeIR::Enum(n) => {
            let name = enums.get(n).cloned().unwrap_or_else(|| pascal(n));
            Rs { owned: name.clone(), param: name, kind: Kind::Enum }
        }
        // composite types cannot appear in a parameter or result column
        TypeIR::Composite(_) => plain("()"),
    }
}

fn opt(ty: &str, nullable: bool) -> String {
    if nullable { format!("Option<{ty}>") } else { ty.to_string() }
}

/// Every enum the statements mention, by schema name.
fn used_enums(statements: &[Statement]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &TypeIR| {
        if let TypeIR::Enum(n) = t
            && !out.contains(n)
        {
            out.push(n.clone());
        }
    };
    for s in statements {
        s.params().iter().for_each(|p| add(&p.ty));
        s.columns().iter().for_each(|c| add(&c.ty));
    }
    out.sort();
    out
}

/// A raw string literal holding `s` whatever it contains.
fn raw(s: &str) -> String {
    let mut hashes = 1;
    while s.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let h = "#".repeat(hashes);
    format!("r{h}\"{s}\"{h}")
}

/// A plain string literal for a short text (an enum variant's database text).
fn quoted(s: &str) -> String {
    format!("{s:?}")
}

/// Which supporting pieces the generated file needs.
#[derive(Default)]
struct Uses {
    decimal: bool,
    uuid: bool,
    timestamp: bool,
    naive_or_date: bool,
    chrono: bool,
    json: bool,
    enums: bool,
}

fn uses(statements: &[Statement]) -> Uses {
    let mut u = Uses::default();
    let mut add = |t: &TypeIR| match t {
        TypeIR::Builtin(Builtin::Decimal | Builtin::Numeric(..)) => u.decimal = true,
        TypeIR::Builtin(Builtin::Uuid) => u.uuid = true,
        TypeIR::Builtin(Builtin::Timestamp) => {
            u.timestamp = true;
            u.chrono = true;
        }
        TypeIR::Builtin(Builtin::TimestampNaive | Builtin::Date) => {
            u.naive_or_date = true;
            u.chrono = true;
        }
        TypeIR::Builtin(Builtin::Json) => u.json = true,
        TypeIR::Enum(_) => u.enums = true,
        _ => {}
    };
    for s in statements {
        s.params().iter().for_each(|p| add(&p.ty));
        s.columns().iter().for_each(|c| add(&c.ty));
    }
    u
}

/// Generate one Rust source file for `statements` (compiled against `schema`).
pub fn generate_rust(schema: &SchemaIR, statements: &[Statement], opts: &RustOptions) -> String {
    let pg = opts.dialect == Dialect::Postgres;
    let my = opts.dialect == Dialect::Mysql;
    let sqlite = !pg && !my;
    let u = uses(statements);
    let mut o = String::new();
    let w = &mut o;
    let _ = writeln!(w, "// <auto-generated>\n// Generated by `certo ql codegen`. Changes are lost when it runs again.\n// </auto-generated>");
    // what the file needs from Cargo.toml
    let _ = writeln!(w, "//\n// Needs, in Cargo.toml:");
    if pg {
        let mut features = Vec::new();
        if u.chrono { features.push("\"with-chrono-0_4\""); }
        if u.uuid { features.push("\"with-uuid-1\""); }
        if features.is_empty() {
            let _ = writeln!(w, "//   postgres = \"0.19\"");
        } else {
            let _ = writeln!(w, "//   postgres = {{ version = \"0.19\", features = [{}] }}", features.join(", "));
        }
        if u.decimal { let _ = writeln!(w, "//   rust_decimal = {{ version = \"1\", features = [\"db-postgres\"] }}"); }
    } else if my {
        if u.chrono {
            let _ = writeln!(w, "//   mysql = {{ version = \"25\", features = [\"chrono\"] }}");
        } else {
            let _ = writeln!(w, "//   mysql = \"25\"");
        }
        if u.decimal { let _ = writeln!(w, "//   rust_decimal = \"1\""); }
    } else {
        if u.chrono || u.timestamp {
            let _ = writeln!(w, "//   rusqlite = {{ version = \"0.32\", features = [\"chrono\"] }}");
        } else {
            let _ = writeln!(w, "//   rusqlite = \"0.32\"");
        }
        if u.decimal { let _ = writeln!(w, "//   rust_decimal = \"1\""); }
    }
    if u.chrono { let _ = writeln!(w, "//   chrono = \"0.4\""); }
    if u.uuid { let _ = writeln!(w, "//   uuid = \"1\""); }
    let _ = writeln!(w);

    // ---- enums ---------------------------------------------------------------------------
    let mut enum_names: BTreeMap<String, String> = BTreeMap::new();
    let mut taken_types: HashSet<String> = HashSet::new();
    for reserved in ["Json", "Error"] {
        taken_types.insert(reserved.to_string());
    }
    for name in used_enums(statements) {
        let t = unique(pascal(&name), &mut taken_types);
        enum_names.insert(name, t);
    }
    for (db_name, rs_name) in &enum_names {
        let variants: Vec<&String> =
            schema.enums.iter().find(|e| &e.name == db_name).map(|e| e.variants.iter().collect()).unwrap_or_default();
        let mut used = HashSet::new();
        let names: Vec<String> = variants.iter().map(|v| unique(pascal(v), &mut used)).collect();
        let _ = writeln!(w, "/// The enum `{db_name}`; each value reads and binds as its database text.");
        let _ = writeln!(w, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub enum {rs_name} {{");
        for n in &names {
            let _ = writeln!(w, "    {n},");
        }
        let _ = writeln!(w, "}}\n");
        let _ = writeln!(w, "impl {rs_name} {{\n    /// The text the database stores.\n    pub fn as_db(self) -> &'static str {{\n        match self {{");
        for (n, v) in names.iter().zip(&variants) {
            let _ = writeln!(w, "            {rs_name}::{n} => {},", quoted(v));
        }
        let _ = writeln!(w, "        }}\n    }}\n");
        let _ = writeln!(w, "    /// The value for database text, if it is one of this enum's.\n    pub fn from_db(text: &str) -> Option<{rs_name}> {{\n        match text {{");
        for (n, v) in names.iter().zip(&variants) {
            let _ = writeln!(w, "            {} => Some({rs_name}::{n}),", quoted(v));
        }
        let _ = writeln!(w, "            _ => None,\n        }}\n    }}\n}}\n");
        if pg {
            let _ = writeln!(w, "impl postgres::types::ToSql for {rs_name} {{
    fn to_sql(&self, _ty: &postgres::types::Type, out: &mut postgres::types::private::BytesMut) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {{
        out.extend_from_slice(self.as_db().as_bytes());
        Ok(postgres::types::IsNull::No)
    }}
    fn accepts(_ty: &postgres::types::Type) -> bool {{
        true
    }}
    postgres::types::to_sql_checked!();
}}

impl<'a> postgres::types::FromSql<'a> for {rs_name} {{
    fn from_sql(_ty: &postgres::types::Type, raw: &'a [u8]) -> Result<{rs_name}, Box<dyn std::error::Error + Sync + Send>> {{
        let text = std::str::from_utf8(raw)?;
        {rs_name}::from_db(text).ok_or_else(|| format!(\"unexpected {rs_name} value '{{text}}'\").into())
    }}
    fn accepts(_ty: &postgres::types::Type) -> bool {{
        true
    }}
}}
");
        } else if my {
            let _ = writeln!(w, "impl {rs_name} {{
    fn parse(text: String) -> Result<{rs_name}, mysql::Error> {{
        {rs_name}::from_db(&text).ok_or_else(|| mysql::Error::FromValueError(mysql::Value::from(text)))
    }}
}}
");
        } else {
            let _ = writeln!(w, "impl rusqlite::types::ToSql for {rs_name} {{
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {{
        Ok(self.as_db().into())
    }}
}}

impl rusqlite::types::FromSql for {rs_name} {{
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<{rs_name}> {{
        let text = value.as_str()?;
        {rs_name}::from_db(text).ok_or_else(|| rusqlite::types::FromSqlError::Other(format!(\"unexpected {rs_name} value '{{text}}'\").into()))
    }}
}}
");
        }
    }

    // ---- supporting types ----------------------------------------------------------------------
    if u.json {
        let _ = writeln!(w, "/// JSON text.\n#[derive(Debug, Clone, PartialEq, Eq)]\npub struct Json(pub String);\n");
        if pg {
            let _ = writeln!(w, "impl postgres::types::ToSql for Json {{
    fn to_sql(&self, ty: &postgres::types::Type, out: &mut postgres::types::private::BytesMut) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {{
        if *ty == postgres::types::Type::JSONB {{
            out.extend_from_slice(&[1]);
        }}
        out.extend_from_slice(self.0.as_bytes());
        Ok(postgres::types::IsNull::No)
    }}
    fn accepts(ty: &postgres::types::Type) -> bool {{
        *ty == postgres::types::Type::JSON || *ty == postgres::types::Type::JSONB
    }}
    postgres::types::to_sql_checked!();
}}

impl<'a> postgres::types::FromSql<'a> for Json {{
    fn from_sql(ty: &postgres::types::Type, raw: &'a [u8]) -> Result<Json, Box<dyn std::error::Error + Sync + Send>> {{
        let raw = if *ty == postgres::types::Type::JSONB {{ &raw[1..] }} else {{ raw }};
        Ok(Json(std::str::from_utf8(raw)?.to_string()))
    }}
    fn accepts(ty: &postgres::types::Type) -> bool {{
        *ty == postgres::types::Type::JSON || *ty == postgres::types::Type::JSONB
    }}
}}
");
        } else if my {
            // MySQL reads and binds JSON as its text: see the conversions in each function
        } else {
            let _ = writeln!(w, "impl rusqlite::types::ToSql for Json {{
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {{
        Ok(self.0.as_str().into())
    }}
}}

impl rusqlite::types::FromSql for Json {{
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Json> {{
        Ok(Json(value.as_str()?.to_string()))
    }}
}}
");
        }
    }
    if my {
        let _ = writeln!(w, "fn col<T: mysql::prelude::FromValue>(row: &mut mysql::Row, i: usize) -> Result<T, mysql::Error> {{\n    row.take_opt(i)\n        .ok_or_else(|| mysql::Error::FromValueError(mysql::Value::NULL))?\n        .map_err(|e| mysql::Error::FromValueError(e.0))\n}}\n");
        if u.uuid {
            let _ = writeln!(w, "fn parse_uuid(text: String) -> Result<uuid::Uuid, mysql::Error> {{\n    uuid::Uuid::parse_str(&text).map_err(|_| mysql::Error::FromValueError(mysql::Value::from(text)))\n}}\n");
        }
    }
    if sqlite {
        // SQLite keeps these as text (decimals as numbers); rusqlite has no impls for them, or a different one
        if u.decimal {
            let _ = writeln!(w, "struct SqlDecimal(rust_decimal::Decimal);

impl rusqlite::types::ToSql for SqlDecimal {{
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {{
        Ok(self.0.to_string().into())
    }}
}}

impl rusqlite::types::FromSql for SqlDecimal {{
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<SqlDecimal> {{
        use rusqlite::types::{{FromSqlError, ValueRef}};
        let d = match value {{
            ValueRef::Integer(i) => Ok(rust_decimal::Decimal::from(i)),
            ValueRef::Real(f) => rust_decimal::Decimal::try_from(f).map_err(|e| FromSqlError::Other(Box::new(e))),
            ValueRef::Text(t) => std::str::from_utf8(t)
                .map_err(|e| FromSqlError::Other(Box::new(e)))?
                .parse::<rust_decimal::Decimal>()
                .map_err(|e| FromSqlError::Other(Box::new(e))),
            _ => Err(FromSqlError::InvalidType),
        }}?;
        Ok(SqlDecimal(d))
    }}
}}
");
        }
        if u.uuid {
            let _ = writeln!(w, "struct SqlUuid(uuid::Uuid);

impl rusqlite::types::ToSql for SqlUuid {{
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {{
        Ok(self.0.to_string().into())
    }}
}}

impl rusqlite::types::FromSql for SqlUuid {{
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<SqlUuid> {{
        uuid::Uuid::parse_str(value.as_str()?).map(SqlUuid).map_err(|e| rusqlite::types::FromSqlError::Other(Box::new(e)))
    }}
}}
");
        }
        if u.timestamp {
            let _ = writeln!(w, "/// Written the way SQLite's own `CURRENT_TIMESTAMP` is (UTC, no offset).
struct SqlTimestamp(chrono::DateTime<chrono::Utc>);

impl rusqlite::types::ToSql for SqlTimestamp {{
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {{
        Ok(self.0.naive_utc().format(\"%Y-%m-%d %H:%M:%S%.f\").to_string().into())
    }}
}}

impl rusqlite::types::FromSql for SqlTimestamp {{
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<SqlTimestamp> {{
        <chrono::DateTime<chrono::Utc> as rusqlite::types::FromSql>::column_result(value).map(SqlTimestamp)
    }}
}}
");
        }
    }

    // ---- rows ------------------------------------------------------------------------------
    // names first, so two statements that differ only in case or punctuation do not collide
    struct Names {
        func: String,
        konst: String,
        row: String,
    }
    let mut taken_fns: HashSet<String> = HashSet::new();
    let mut names: Vec<Names> = Vec::new();
    for s in statements {
        let func = unique(snake(s.name()), &mut taken_fns);
        let konst = format!("{}_SQL", func.trim_start_matches("r#").to_uppercase());
        let row = unique(format!("{}Row", pascal(s.name())), &mut taken_types);
        names.push(Names { func, konst, row });
    }
    for (s, n) in statements.iter().zip(&names) {
        if s.columns().is_empty() {
            continue;
        }
        let mut taken = HashSet::new();
        let _ = writeln!(w, "/// One row of `{}`.\n#[derive(Debug, Clone, PartialEq)]\npub struct {} {{", s.name(), n.row);
        for c in s.columns() {
            let field = unique(snake(&c.name), &mut taken);
            let _ = writeln!(w, "    pub {field}: {},", opt(&rs_type(&c.ty, &enum_names).owned, c.nullable));
        }
        let _ = writeln!(w, "}}\n");
    }

    // ---- statements --------------------------------------------------------------------------
    for (s, n) in statements.iter().zip(&names) {
        let kind = match s {
            Statement::Query(_) => "query",
            Statement::Mutation(m) => match m.ir.kind {
                MutationKind::Insert => "insert",
                MutationKind::Update => "update",
                MutationKind::Delete => "delete",
            },
        };
        let has_rows = !s.columns().is_empty();
        // PostgreSQL enum result columns are selected as text; harmless for the generated reader and for other drivers
        let lo = LowerOptions { enums_as_text: true };
        let sql = match s {
            Statement::Query(q) => lower_with(opts.dialect, &q.ir, lo).sql,
            Statement::Mutation(m) => lower_mutation_with(opts.dialect, &m.ir, lo).sql,
        };
        let _ = writeln!(w, "/// The SQL of `{kind} {}`.\npub const {}: &str = {};\n", s.name(), n.konst, raw(&sql));

        // parameters, in declaration order; one that is never used keeps its place but not its name
        let mut taken: HashSet<String> = ["client", "conn"].iter().map(|x| x.to_string()).collect();
        let params: Vec<(String, &ParamIR, Rs, bool)> = s
            .params()
            .iter()
            .map(|p| {
                let used = s.param_order().iter().any(|x| x == &p.name);
                let base = snake(&p.name);
                let base = base.trim_start_matches("r#").to_string();
                let name = unique(if used { base } else { format!("_{base}") }, &mut taken);
                let name = if KEYWORDS.contains(&name.as_str()) { format!("r#{name}") } else { name };
                (name, p, rs_type(&p.ty, &enum_names), used)
            })
            .collect();
        let mut sig: Vec<String> = Vec::new();
        sig.push(if pg { "client: &mut C".into() } else if my { "conn: &mut C".into() } else { "conn: &rusqlite::Connection".into() });
        for (name, p, rs, _) in &params {
            sig.push(format!("{name}: {}", opt(&rs.param, p.nullable)));
        }
        let (generics, ret_err) = if pg {
            ("<C: postgres::GenericClient>", "postgres::Error")
        } else if my {
            ("<C: mysql::prelude::Queryable>", "mysql::Error")
        } else {
            ("", "rusqlite::Error")
        };
        let ret = if has_rows { format!("Vec<{}>", n.row) } else { "u64".to_string() };
        let _ = writeln!(w, "/// Runs `{kind} {}`{}.", s.name(),
            if has_rows || kind == "query" { "" } else { "; returns the number of rows affected" });
        if sig.len() > 7 {
            let _ = writeln!(w, "#[allow(clippy::too_many_arguments)]");
        }
        let _ = writeln!(w, "pub fn {}{generics}({}) -> Result<{ret}, {ret_err}> {{", n.func, sig.join(", "));

        // binding, in placeholder order
        let bound: Vec<String> = s
            .param_order()
            .iter()
            .filter_map(|placeholder| params.iter().find(|(_, p, _, _)| &p.name == placeholder))
            .map(|(name, p, rs, _)| bind_expr(name, rs, p.nullable, opts.dialect))
            .collect();
        if pg {
            let _ = writeln!(w, "    let params: &[&(dyn postgres::types::ToSql + Sync)] = &[{}];", bound.join(", "));
            if has_rows {
                let _ = writeln!(w, "    let rows = client.query({}, params)?;", n.konst);
                let _ = writeln!(w, "    rows.iter()\n        .map(|row| {{\n            Ok({} {{", n.row);
                let mut taken = HashSet::new();
                for (i, c) in s.columns().iter().enumerate() {
                    let field = unique(snake(&c.name), &mut taken);
                    let _ = writeln!(w, "                {field}: row.try_get({i})?,");
                }
                let _ = writeln!(w, "            }})\n        }})\n        .collect()");
            } else {
                let _ = writeln!(w, "    client.execute({}, params)", n.konst);
            }
        } else if my {
            let _ = writeln!(w, "    let params = mysql::Params::Positional(vec![{}]);", bound.join(", "));
            if has_rows {
                let _ = writeln!(w, "    let rows: Vec<mysql::Row> = conn.exec({}, params)?;", n.konst);
                let _ = writeln!(w, "    rows.into_iter()\n        .map(|mut row| {{\n            Ok({} {{", n.row);
                let mut taken = HashSet::new();
                for (i, c) in s.columns().iter().enumerate() {
                    let field = unique(snake(&c.name), &mut taken);
                    let _ = writeln!(w, "                {field}: {},", mysql_read_expr(i, &rs_type(&c.ty, &enum_names), c.nullable));
                }
                let _ = writeln!(w, "            }})\n        }})\n        .collect()");
            } else {
                let _ = writeln!(w, "    Ok(conn.exec_iter({}, params)?.affected_rows())", n.konst);
            }
        } else {
            let list = bound.join(", ");
            if has_rows {
                let _ = writeln!(w, "    let mut stmt = conn.prepare({})?;", n.konst);
                let _ = writeln!(w, "    let rows = stmt.query_map(rusqlite::params![{list}], |row| {{\n        Ok({} {{", n.row);
                let mut taken = HashSet::new();
                for (i, c) in s.columns().iter().enumerate() {
                    let field = unique(snake(&c.name), &mut taken);
                    let _ = writeln!(w, "            {field}: {},", read_expr(i, &rs_type(&c.ty, &enum_names), c.nullable));
                }
                let _ = writeln!(w, "        }})\n    }})?;\n    rows.collect()");
            } else {
                let _ = writeln!(w, "    conn.execute({}, rusqlite::params![{list}]).map(|n| n as u64)", n.konst);
            }
        }
        let _ = writeln!(w, "}}\n");
    }
    while o.ends_with("\n\n") {
        o.pop();
    }
    o
}

/// The expression bound as a parameter's value.
fn bind_expr(name: &str, rs: &Rs, nullable: bool, dialect: Dialect) -> String {
    if dialect == Dialect::Postgres {
        return format!("&{name}");
    }
    if dialect == Dialect::Mysql {
        // a `mysql::Value`: text for what MySQL keeps as text (enum values, uuids, json), UTC for timestamps
        return match (rs.kind, nullable) {
            (Kind::Uuid, false) => format!("mysql::Value::from({name}.to_string())"),
            (Kind::Uuid, true) => format!("mysql::Value::from({name}.map(|v| v.to_string()))"),
            (Kind::Timestamp, false) => format!("mysql::Value::from({name}.naive_utc())"),
            (Kind::Timestamp, true) => format!("mysql::Value::from({name}.map(|v| v.naive_utc()))"),
            (Kind::Json, false) => format!("mysql::Value::from({name}.0.as_str())"),
            (Kind::Json, true) => format!("mysql::Value::from({name}.map(|v| v.0.as_str()))"),
            (Kind::Enum, false) => format!("mysql::Value::from({name}.as_db())"),
            (Kind::Enum, true) => format!("mysql::Value::from({name}.map(|v| v.as_db()))"),
            _ => format!("mysql::Value::from({name})"),
        };
    }
    let wrapper = match rs.kind {
        Kind::Decimal => Some("SqlDecimal"),
        Kind::Uuid => Some("SqlUuid"),
        Kind::Timestamp => Some("SqlTimestamp"),
        _ => None,
    };
    match (wrapper, nullable) {
        (Some(wr), false) => format!("&{wr}({name})"),
        (Some(wr), true) => format!("&{name}.map({wr})"),
        (None, _) => format!("&{name}"),
    }
}

/// The expression reading column `i` of the current MySQL row (`row` is a `mysql::Row`, taken from).
fn mysql_read_expr(i: usize, rs: &Rs, nullable: bool) -> String {
    let get = |ty: &str| format!("col::<{ty}>(&mut row, {i})?");
    match (rs.kind, nullable) {
        (Kind::Uuid, false) => format!("parse_uuid({})?", get("String")),
        (Kind::Uuid, true) => format!("{}.map(parse_uuid).transpose()?", get("Option<String>")),
        (Kind::Timestamp, false) => format!("{}.and_utc()", get("chrono::NaiveDateTime")),
        (Kind::Timestamp, true) => format!("{}.map(|v| v.and_utc())", get("Option<chrono::NaiveDateTime>")),
        (Kind::Json, false) => format!("Json({})", get("String")),
        (Kind::Json, true) => format!("{}.map(Json)", get("Option<String>")),
        (Kind::Enum, false) => format!("{}::parse({})?", rs.owned, get("String")),
        (Kind::Enum, true) => format!("{}.map({}::parse).transpose()?", get("Option<String>"), rs.owned),
        _ => format!("col(&mut row, {i})?"),
    }
}

/// The expression reading column `i` of the current SQLite row.
fn read_expr(i: usize, rs: &Rs, nullable: bool) -> String {
    let wrapper = match rs.kind {
        Kind::Decimal => Some("SqlDecimal"),
        Kind::Uuid => Some("SqlUuid"),
        Kind::Timestamp => Some("SqlTimestamp"),
        _ => None,
    };
    match (wrapper, nullable) {
        (Some(wr), false) => format!("row.get::<_, {wr}>({i})?.0"),
        (Some(wr), true) => format!("row.get::<_, Option<{wr}>>({i})?.map(|v| v.0)"),
        (None, _) => format!("row.get({i})?"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile;

    const SCHEMA: &str = "enum Role { admin, user_ }
        table users { id: serial primary key  name: text not null  role: Role  born: date  balance: decimal(10,2) }
        table orders { id: serial primary key  user_id: int not null references users  total: decimal(10,2) not null }";

    fn gen_for(ql: &str, dialect: Dialect) -> String {
        let (ir, d) = certo_sdl::compile(SCHEMA);
        let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
        let (s, d) = compile(&schema, ql, dialect);
        let s = s.unwrap_or_else(|| panic!("{d:?}"));
        generate_rust(&schema, &s, &RustOptions { dialect })
    }

    #[test]
    fn a_query_becomes_a_struct_and_a_function() {
        let code = gen_for(
            "query recent_orders(min_total: decimal(10,2), since: date null) {
                from orders o join users u on o.user_id == u.id
                where o.total >= :min_total select o.id, u.name as customer, u.born, o.total }",
            Dialect::Postgres,
        );
        assert!(code.contains("pub struct RecentOrdersRow {"), "{code}");
        assert!(code.contains("    pub id: i32,\n    pub customer: String,\n    pub born: Option<chrono::NaiveDate>,\n    pub total: rust_decimal::Decimal,"), "{code}");
        assert!(
            code.contains("pub fn recent_orders<C: postgres::GenericClient>(client: &mut C, min_total: rust_decimal::Decimal, _since: Option<chrono::NaiveDate>) -> Result<Vec<RecentOrdersRow>, postgres::Error>"),
            "{code}"
        );
        assert!(code.contains("pub const RECENT_ORDERS_SQL: &str = r#\"SELECT"), "{code}");
        assert!(code.contains("&[&min_total]"), "{code}");
        assert!(code.contains("born: row.try_get(2)?,"), "{code}");
        assert!(code.contains("postgres = { version = \"0.19\", features = [\"with-chrono-0_4\"] }"), "{code}");
        assert!(code.contains("rust_decimal = { version = \"1\", features = [\"db-postgres\"] }"), "{code}");

        let lite = gen_for("query q(n: int) { from users u where u.id == :n select u.id, u.balance }", Dialect::Sqlite);
        assert!(lite.contains("pub fn q(conn: &rusqlite::Connection, n: i32) -> Result<Vec<QRow>, rusqlite::Error>"), "{lite}");
        assert!(lite.contains("rusqlite::params![&n]"), "{lite}");
        assert!(lite.contains("balance: row.get::<_, Option<SqlDecimal>>(1)?.map(|v| v.0),"), "{lite}");
        assert!(lite.contains("//   rusqlite = \"0.32\""), "{lite}");
    }

    #[test]
    fn enums_become_rust_enums_with_their_database_text() {
        let code = gen_for(
            "query by_role(r: Role null) { from users u where :r is null or u.role == :r select u.id, u.role }
             insert add(n: text, r: Role) { into users set name = :n, role = :r returning role }",
            Dialect::Postgres,
        );
        assert!(code.contains("pub enum Role {\n    Admin,\n    User,\n}"), "{code}");
        assert!(code.contains("Role::User => \"user_\",") && code.contains("\"admin\" => Some(Role::Admin),"), "{code}");
        assert!(code.contains("pub struct ByRoleRow {\n    pub id: i32,\n    pub role: Option<Role>,\n}"), "{code}");
        assert!(code.contains("r: Option<Role>"), "{code}");
        assert!(code.contains("impl postgres::types::ToSql for Role"), "{code}");
        let lite = gen_for("query q(r: Role) { from users u where u.role == :r select u.role }", Dialect::Sqlite);
        assert!(lite.contains("impl rusqlite::types::FromSql for Role") && !lite.contains("postgres"), "{lite}");
    }

    #[test]
    fn mutations_return_counts_or_rows() {
        let code = gen_for(
            "insert add(n: text) { into users set name = :n returning id }
             update rename(id: int, n: text) { users u set name = :n where u.id == :id }
             delete purge() { from orders o all rows }",
            Dialect::Postgres,
        );
        assert!(code.contains("-> Result<Vec<AddRow>, postgres::Error>") && code.contains("pub struct AddRow {\n    pub id: i32,\n}"), "{code}");
        assert!(code.contains("pub fn rename<C: postgres::GenericClient>(client: &mut C, id: i32, n: &str) -> Result<u64, postgres::Error>"), "{code}");
        assert!(code.contains("client.execute(RENAME_SQL, params)"), "{code}");
        assert!(code.contains("pub fn purge<C: postgres::GenericClient>(client: &mut C) -> Result<u64, postgres::Error>"), "{code}");
        assert!(!code.contains("RenameRow") && !code.contains("PurgeRow"));
        // parameters are bound in placeholder order, not declaration order: `n` is first in the SQL
        let rename = code.split("pub fn rename").nth(1).unwrap();
        assert!(rename.find("&[&n, &id]").is_some(), "{rename}");
        let lite = gen_for("update rename(id: int, n: text) { users u set name = :n where u.id == :id }", Dialect::Sqlite);
        assert!(lite.contains("conn.execute(RENAME_SQL, rusqlite::params![&n, &id]).map(|n| n as u64)"), "{lite}");
    }

    #[test]
    fn names_are_made_safe() {
        let code = gen_for(
            "query class_list(type: int, string_val: text null) { from users u where u.id == :type and u.name == :string_val select u.id as in_ }
             query class_list2() { from users u select u.id }",
            Dialect::Postgres,
        );
        assert!(code.contains("r#type: i32, string_val: Option<&str>"), "{code}");
        assert!(code.contains("pub r#in: i32,"), "{code}");
        // two statements whose names collapse to the same Rust name stay distinct
        let code = gen_for("query a_b() { from users u select u.id } query aB() { from users u select u.id }", Dialect::Postgres);
        assert!(code.contains("pub fn a_b<") && code.contains("pub fn a_b2<"), "{code}");
        assert!(code.contains("pub struct ABRow") && code.contains("pub struct ABRow2"), "{code}");
    }

    #[test]
    fn raw_strings_survive_quotes() {
        assert_eq!(raw("select 1"), "r#\"select 1\"#");
        assert_eq!(raw("a\"#b"), "r##\"a\"#b\"##");
    }
}
