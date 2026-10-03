//! Drift: how the live database differs from what the applied migrations
//! say it should be.
//!
//! Expected = the `ir.json` of the last *applied* migration (not `IR.json`,
//! which may include migrations not applied yet). Live = introspection.
//! The report is `diff(live, expected)`: what would have to change in the
//! database for it to match the migrations, worded as findings.
//!
//! Fidelity notes:
//! - Relationships have no database form; they are copied from expected.
//! - Defaults and CHECK bodies are compared as normalised text (`normalize`):
//!   quotes, parentheses, whitespace, `::casts` and case are ignored. This
//!   catches changed literals, operators, columns and functions, but not a
//!   change that only alters grouping, so treat "no drift" as strong, not
//!   absolute, evidence for expressions.
//! - Objects SDL cannot express are reported as `notes`, not as drift.

use crate::error::RunnerError;
use crate::exec::Executor;
use crate::introspect::LiveSchema;
use crate::migration::list;
use crate::project::Project;
use crate::runner::{connection_err, reconcile};
use certo_mdl::{diff, MigrationPlan, Op};
use certo_sdl::{ColumnIR, ExprIR, SchemaIR};
use certo_sql::{render_expr, Dialect};
use std::collections::HashSet;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    /// Expected by the migrations, absent from the database.
    Missing,
    /// Present in the database, unknown to the migrations.
    Unexpected,
    /// Present in both, but different.
    Different,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DriftItem {
    pub kind: DriftKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Drift {
    /// What the database was compared against.
    pub expected_from: String,
    pub items: Vec<DriftItem>,
    /// Live objects SDL cannot represent (not counted as drift).
    pub notes: Vec<String>,
    /// The plan that would bring the database to the expected schema
    /// (`diff(live, expected)`). Lower it to review a repair script.
    pub plan: MigrationPlan,
    /// The schemas `plan` goes between: the database as read (aligned), and the expected schema.
    /// SQLite lowering needs them.
    pub live: SchemaIR,
    pub expected: SchemaIR,
}

impl Drift {
    pub fn in_sync(&self) -> bool { self.items.is_empty() }

    /// The script that would bring the database back to the migrations (review before running).
    pub fn repair_sql(&self, dialect: Dialect) -> Result<String, certo_sql::LowerError> {
        certo_sql::render_with(&self.plan, dialect, certo_sql::Schemas { old: &self.live, new: &self.expected })
    }
}

/// Words that can continue a multi-word type name after `::`.
const TYPE_WORDS: &[&str] = &["precision", "varying", "with", "without", "time", "zone"];

/// `'-1'::integer` -> `-1`: the server prints a numeric literal it had to cast
/// as a quoted string plus a cast; the meaning is just the number.
fn unquote_numeric_literals(s: &str) -> String {
    const NUMERIC: &[&str] = &["smallint", "integer", "bigint", "numeric", "real", "double precision", "int"];
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\'' {
            // find the end of the quoted literal ('' is an escaped quote)
            let mut j = i + 1;
            let mut body = String::new();
            while j < c.len() {
                if c[j] == '\'' && c.get(j + 1) == Some(&'\'') { body.push('\''); j += 2; } else if c[j] == '\'' { break; } else { body.push(c[j]); j += 1; }
            }
            let numeric = !body.is_empty()
                && body.trim_start_matches('-').chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                && body.chars().any(|ch| ch.is_ascii_digit());
            if numeric && c.get(j + 1) == Some(&':') && c.get(j + 2) == Some(&':') {
                let rest: String = c[j + 3..].iter().collect::<String>().to_lowercase();
                if let Some(t) = NUMERIC.iter().find(|t| rest.starts_with(**t)) {
                    out.push_str(&body);
                    i = j + 3 + t.chars().count();
                    continue;
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// `x = ANY (ARRAY[a, b])` -> `x IN (a, b)` and `x <> ALL (ARRAY[a, b])` ->
/// `x NOT IN (a, b)`: the forms the server stores for `IN` / `NOT IN`.
fn rewrite_any_arrays(s: &str) -> String {
    let mut out = s.to_string();
    for (from, to) in [("= ANY (ARRAY[", " IN ("), ("<> ALL (ARRAY[", " NOT IN (")] {
        loop {
            let lower = out.to_lowercase();
            let Some(at) = lower.find(&from.to_lowercase()) else { break };
            let open = at + from.len();
            // matching `]` (arrays nest) followed by the closing `)` of ANY(...)
            let bytes: Vec<char> = out.chars().collect();
            let mut depth = 1;
            let mut k = open;
            while k < bytes.len() && depth > 0 {
                match bytes[k] { '[' => depth += 1, ']' => depth -= 1, _ => {} }
                if depth > 0 { k += 1; }
            }
            if depth != 0 { break; }
            let head: String = bytes[..at].iter().collect();
            let list: String = bytes[open..k].iter().collect();
            let mut tail_start = k + 1; // past `]`
            if bytes.get(tail_start) == Some(&')') { tail_start += 1; } // past ANY's `)`
            let tail: String = bytes[tail_start..].iter().collect();
            out = format!("{head}{to}{list}){tail}");
        }
    }
    out
}

/// Canonical form for comparing SQL expressions from different sources:
/// `::casts` removed (before whitespace is dropped, so a multi-word type such
/// as `double precision` cannot swallow what follows it), then quotes,
/// parentheses and whitespace removed and case folded.
pub fn normalize(sql: &str) -> String {
    let sql = &rewrite_any_arrays(&unquote_numeric_literals(sql));
    let chars: Vec<char> = sql.chars().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' && chars.get(i + 1) == Some(&':') {
            i += 2;
            // type name: "quoted" or bare identifier, then any continuation words
            loop {
                if chars.get(i) == Some(&'"') {
                    i += 1;
                    while i < chars.len() && chars[i] != '"' { i += 1; }
                    i += 1;
                } else {
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
                }
                while chars.get(i) == Some(&'[') && chars.get(i + 1) == Some(&']') { i += 2; }
                // continue over " word" only if the word is part of a type name
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() { j += 1; }
                let start = j;
                while j < chars.len() && chars[j].is_alphabetic() { j += 1; }
                let word: String = chars[start..j].iter().collect::<String>().to_lowercase();
                if j > start && j > i && TYPE_WORDS.contains(&word.as_str()) {
                    i = j;
                } else {
                    break;
                }
            }
            continue;
        }
        kept.push(chars[i]);
        i += 1;
    }
    kept.chars()
        .filter(|c| !matches!(c, '"' | '(' | ')') && !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
        // PostgreSQL treats these as the same function in a default
        .replace("current_timestamp", "now")
}

/// Make `live` comparable with `expected`: copy what has no database form
/// (relationships) and replace `Raw` expressions that mean the same as the
/// expected ones with the expected ones.
pub fn align(dialect: Dialect, expected: &SchemaIR, live: &mut SchemaIR) {
    let same = |raw: &str, e: &ExprIR| normalize(raw) == normalize(&render_expr(dialect, e));
    if dialect == Dialect::Sqlite {
        align_sqlite(expected, live);
    }
    for lt in &mut live.tables {
        let Some(et) = expected.table(&lt.name) else { continue };
        lt.relationships = et.relationships.clone();
        for lc in &mut lt.columns {
            if let (Some(ExprIR::Raw { sql }), Some(Some(ed))) = (&lc.default, et.column(&lc.name).map(|c| &c.default))
                && same(sql, ed)
            {
                lc.default = Some(ed.clone());
            }
        }
        for lk in &mut lt.constraints {
            if let (ExprIR::Raw { sql }, Some(ek)) = (&lk.expr, et.constraints.iter().find(|k| k.name == lk.name))
                && same(sql, &ek.expr)
            {
                lk.expr = ek.expr.clone();
            }
        }
    }
}

/// What SQLite cannot keep apart or cannot show:
/// * an enum that no column uses leaves no trace in the database;
/// * `serial` and both identity forms are one thing (`INTEGER PRIMARY KEY
///   AUTOINCREMENT`), and it is always an `INTEGER`, whatever width was asked.
fn align_sqlite(expected: &SchemaIR, live: &mut SchemaIR) {
    let used: HashSet<&str> = expected
        .tables
        .iter()
        .flat_map(|t| &t.columns)
        .filter_map(|c| match &c.ty {
            certo_sdl::TypeIR::Enum(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    for e in &expected.enums {
        if !used.contains(e.name.as_str()) && !live.enums.iter().any(|l| l.name == e.name) {
            live.enums.push(e.clone());
        }
    }
    live.enums.sort_by(|a, b| a.name.cmp(&b.name));
    for lt in &mut live.tables {
        let Some(et) = expected.table(&lt.name) else { continue };
        for lc in &mut lt.columns {
            let Some(ec) = et.column(&lc.name) else { continue };
            if lc.generated.is_some() && ec.generated.is_some() {
                lc.generated = ec.generated;
                lc.ty = ec.ty.clone();
            }
        }
    }
}

fn show(dialect: Dialect, e: &Option<ExprIR>) -> String {
    match e {
        Some(e) => render_expr(dialect, e),
        None => "none".into(),
    }
}

/// Human description of how a live column differs from the expected one.
fn column_changes(dialect: Dialect, table: &str, live: &ColumnIR, exp: &ColumnIR) -> String {
    let mut parts = Vec::new();
    let mut add = |what: &str, l: String, e: String| parts.push(format!("{what}: database has {l}, migrations expect {e}"));
    if live.ty != exp.ty {
        add("type", certo_sdl::describe_type(&live.ty), certo_sdl::describe_type(&exp.ty));
    }
    if live.nullable != exp.nullable {
        add("nullability", nn(live.nullable), nn(exp.nullable));
    }
    if live.unique != exp.unique {
        add("unique", live.unique.to_string(), exp.unique.to_string());
    }
    if live.primary_key != exp.primary_key {
        add("primary key", live.primary_key.to_string(), exp.primary_key.to_string());
    }
    if live.default != exp.default {
        add("default", show(dialect, &live.default), show(dialect, &exp.default));
    }
    if live.generated != exp.generated {
        let g = |g: Option<certo_sdl::Generation>| match g {
            None => "none",
            Some(certo_sdl::Generation::Serial) => "serial",
            Some(certo_sdl::Generation::Always) => "identity (always)",
            Some(certo_sdl::Generation::ByDefault) => "identity (by default)",
        };
        add("auto-generation", g(live.generated).into(), g(exp.generated).into());
    }
    format!("column {table}.{}: {}", exp.name, parts.join("; "))
}

fn nn(nullable: bool) -> String { if nullable { "NULL" } else { "NOT NULL" }.into() }

fn strip_marker(op: &Op) -> String { op.describe().chars().skip(2).collect() }

/// Turn `diff(live, expected)` into findings. Drop+add pairs for the same
/// index, constraint or foreign key become one "different" finding.
fn items(dialect: Dialect, plan: &MigrationPlan) -> Vec<DriftItem> {
    let key = |op: &Op| -> Option<(&'static str, String, String)> {
        match op {
            Op::DropIndex { table, name } => Some(("index", table.clone(), name.clone())),
            Op::CreateIndex { table, index } => Some(("index", table.clone(), index.name.clone())),
            Op::DropConstraint { table, name } => Some(("check constraint", table.clone(), name.clone())),
            Op::AddConstraint { table, constraint } => Some(("check constraint", table.clone(), constraint.name.clone())),
            Op::DropForeignKey { table, name } => Some(("foreign key", table.clone(), name.clone())),
            Op::AddForeignKey { table, column, .. } => Some(("foreign key", table.clone(), column.clone())),
            _ => None,
        }
    };
    let dropped: HashSet<_> = plan.ops.iter().filter(|o| matches!(o, Op::DropIndex { .. } | Op::DropConstraint { .. } | Op::DropForeignKey { .. })).filter_map(key).collect();
    let added: HashSet<_> = plan.ops.iter().filter(|o| matches!(o, Op::CreateIndex { .. } | Op::AddConstraint { .. } | Op::AddForeignKey { .. })).filter_map(key).collect();
    let both: HashSet<_> = dropped.intersection(&added).cloned().collect();

    let mut out = Vec::new();
    let mut emitted: HashSet<(&'static str, String, String)> = HashSet::new();
    for op in &plan.ops {
        if matches!(op, Op::AddRelationship { .. } | Op::DropRelationship { .. }) {
            continue; // no database representation
        }
        if let Some(k) = key(op)
            && both.contains(&k)
        {
            if emitted.insert(k.clone()) {
                let what = if k.0 == "foreign key" { format!("{} on {}.{}", k.0, k.1, k.2) } else { format!("{} {} on {}", k.0, k.2, k.1) };
                out.push(DriftItem { kind: DriftKind::Different, text: format!("{what}: definition differs") });
            }
            continue;
        }
        let kind = match op.describe().chars().next() {
            Some('+') => DriftKind::Missing,
            Some('-') => DriftKind::Unexpected,
            _ => DriftKind::Different,
        };
        let text = match op {
            Op::AlterColumn { table, before, after } => column_changes(dialect, table, before, after),
            Op::AlterType { name, .. } => format!("type {name}: fields differ"),
            _ => strip_marker(op),
        };
        out.push(DriftItem { kind, text });
    }
    out
}

/// Compare `expected` with an introspected schema.
pub fn compare(dialect: Dialect, expected: &SchemaIR, live: LiveSchema, expected_from: &str) -> Drift {
    let LiveSchema { ir: mut live_ir, notes } = live;
    align(dialect, expected, &mut live_ir);
    let plan = diff(&live_ir, expected);
    Drift {
        expected_from: expected_from.to_string(),
        items: items(dialect, &plan),
        notes,
        plan,
        live: live_ir,
        expected: expected.clone(),
    }
}

/// Introspect the database and compare it with the last applied migration.
/// Fails first if the migration history itself is inconsistent.
pub fn check(project: &Project, exec: &mut dyn Executor) -> Result<Drift, RunnerError> {
    crate::runner::check_dialect(project, exec)?;
    let files = list(project)?;
    let applied = exec.applied().map_err(connection_err)?;
    let k = reconcile(&files, &applied)?;
    let (expected, from) = if k == 0 {
        (SchemaIR::empty(), "an empty schema (no migrations applied yet)".to_string())
    } else {
        let m = &files[k - 1];
        let path = m.dir.join("ir.json");
        let text = fs::read_to_string(&path).map_err(|e| crate::error::io(&path, e))?;
        let ir = SchemaIR::from_json(&text)
            .map_err(|e| RunnerError::Project(format!("{}: {e}", path.display())))?;
        (ir, format!("the schema after {}", m.label()))
    };
    let live = exec.introspect().map_err(connection_err)?;
    Ok(compare(project.dialect(), &expected, live, &from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_sdl::compile;

    fn ir(src: &str) -> SchemaIR {
        let (ir, d) = compile(src);
        ir.unwrap_or_else(|| panic!("{d:?}"))
    }

    fn drift(expected: &str, live: &str) -> Drift {
        compare(Dialect::Postgres, &ir(expected), LiveSchema { ir: ir(live), notes: vec![] }, "test")
    }

    fn texts(d: &Drift) -> Vec<String> {
        d.items.iter().map(|i| format!("{:?}: {}", i.kind, i.text)).collect()
    }

    const BASE: &str = "table users { id: uuid primary key  email: text not null }";

    #[test]
    fn normalize_ignores_quotes_parens_casts_case_and_space() {
        assert_eq!(normalize("'user'::\"Role\""), normalize("'user'"));
        assert_eq!(normalize("((length(nick) > 0))"), normalize("(length(\"nick\") > 0)"));
        assert_eq!(normalize("(age >= 18)"), normalize("(\"age\" >= 18)"));
        assert_eq!(normalize("now()"), normalize("NOW()"));
        assert_eq!(normalize("CURRENT_TIMESTAMP"), normalize("now()"));
        assert_eq!(normalize("'x'::text"), normalize("'x'"));
        assert_eq!(normalize("'2026-01-01'::timestamp with time zone"), normalize("'2026-01-01'"));
        // a multi-word cast must not swallow what follows it (found on a live server)
        assert_eq!(
            normalize("((score >= (0)::double precision) OR (score IS NULL))"),
            normalize("((\"score\" >= 0) OR ((\"score\" IS NULL)))")
        );
        assert_eq!(normalize("(a)::character varying AND b"), normalize("a AND b"));
        assert_eq!(normalize("x::text[] = y"), normalize("x = y"));
        // numeric literals the server printed as a cast string
        assert_eq!(normalize("'-1'::integer"), normalize("(-1)"));
        assert_eq!(normalize("'1.5'::numeric"), normalize("1.5"));
        assert_ne!(normalize("'5'::text"), normalize("5"), "a text literal is not a number");
        // IN / NOT IN are stored as ANY / ALL over an array
        assert_eq!(
            normalize("(status = ANY (ARRAY['new'::text, 'paid'::text]))"),
            normalize("(\"status\" IN ('new', 'paid'))")
        );
        assert_eq!(
            normalize("(n <> ALL (ARRAY[1, 2, 3]))"),
            normalize("(\"n\" NOT IN (1, 2, 3))")
        );
        assert_ne!(normalize("(n = ANY (ARRAY[1, 2]))"), normalize("(n IN (1, 3))"));
        assert_ne!(normalize("(0)::double precision OR a"), normalize("0 OR b"));
        // but real differences survive
        assert_ne!(normalize("(age >= 18)"), normalize("(age > 18)"));
        assert_ne!(normalize("'a'"), normalize("'b'"));
        assert_ne!(normalize("lower(x)"), normalize("upper(x)"));
    }

    #[test]
    fn identical_schemas_are_in_sync() {
        assert!(drift(BASE, BASE).in_sync());
    }

    #[test]
    fn missing_and_unexpected_objects() {
        let d = drift(BASE, "table users { id: uuid primary key }");
        assert_eq!(texts(&d), ["Missing: column users.email"]);
        let d = drift(BASE, "table users { id: uuid primary key  email: text not null  extra: int }");
        assert_eq!(texts(&d), ["Unexpected: column users.extra"]);
        let d = drift(BASE, "");
        assert!(texts(&d).contains(&"Missing: table users".to_string()));
        let d = drift("", BASE);
        assert_eq!(texts(&d), ["Unexpected: table users"]);
    }

    #[test]
    fn column_differences_are_described() {
        let d = drift(BASE, "table users { id: uuid primary key  email: int not null }");
        assert_eq!(texts(&d), ["Different: column users.email: type: database has int, migrations expect text"]);
        let d = drift(BASE, "table users { id: uuid primary key  email: text unique }");
        assert!(texts(&d)[0].contains("unique: database has true, migrations expect false"), "{:?}", texts(&d));
        let d = drift(BASE, "table users { id: uuid primary key  email: text }");
        assert!(texts(&d)[0].contains("nullability: database has NULL, migrations expect NOT NULL"));
        let d = drift(BASE, "table users { id: uuid primary key  email: text not null default \"x\" }");
        assert!(texts(&d)[0].contains("default: database has 'x', migrations expect none"), "{:?}", texts(&d));
    }

    #[test]
    fn generation_and_sequence_differences() {
        let d = drift("table t { id: int primary key  n: serial }", "table t { id: int primary key  n: int not null }");
        assert_eq!(texts(&d), ["Different: column t.n: auto-generation: database has none, migrations expect serial"]);
        let d = drift("table t { id: int primary key  n: int generated always }", "table t { id: int primary key  n: int generated by default }");
        assert!(texts(&d)[0].contains("database has identity (by default), migrations expect identity (always)"), "{:?}", texts(&d));
        let d = drift("sequence s start 10", "");
        assert_eq!(texts(&d), ["Missing: sequence s"]);
        let d = drift("", "sequence s");
        assert_eq!(texts(&d), ["Unexpected: sequence s"]);
        let d = drift("sequence s start 10 increment 2", "sequence s");
        assert_eq!(texts(&d), ["Different: sequence s"]);
        // a nextval default the server prints with a cast still matches
        let e = ir("sequence s table t { id: bigint primary key default nextval(s) }");
        let mut l = e.clone();
        l.tables[0].columns[0].default = Some(ExprIR::Raw { sql: "nextval('s'::regclass)".into() });
        let d = compare(Dialect::Postgres, &e, LiveSchema { ir: l, notes: vec![] }, "test");
        assert!(d.in_sync(), "{:?}", texts(&d));
    }

    #[test]
    fn enums_types_indexes_constraints_and_keys() {
        let a = "enum E { a, b } type T { x: int } table t { id: int primary key  n: int } index i on t (n) constraint c on t using n > 0";
        // missing variant / changed index / changed check
        let d = drift(a, "enum E { a } type T { x: int } table t { id: int primary key  n: int } index i on t (id) constraint c on t using n > 1");
        let t = texts(&d);
        assert!(t.contains(&"Missing: enum variant E.b".to_string()), "{t:?}");
        assert!(t.contains(&"Different: index i on t: definition differs".to_string()), "{t:?}");
        assert!(t.contains(&"Different: check constraint c on t: definition differs".to_string()), "{t:?}");
        // extra index / constraint, missing type
        let d = drift("table t { id: int primary key  n: int }", a);
        let t = texts(&d);
        assert!(t.contains(&"Unexpected: index i on t".to_string()), "{t:?}");
        assert!(t.contains(&"Unexpected: constraint c on t".to_string()), "{t:?}");
        assert!(t.contains(&"Unexpected: enum E".to_string()), "{t:?}");

        // foreign key present vs absent vs different action
        let u = "table u { id: int primary key } ";
        let with = format!("{u} table t {{ id: int primary key  r: int references u }}");
        let cas = format!("{u} table t {{ id: int primary key  r: int references u on delete cascade }}");
        let without = format!("{u} table t {{ id: int primary key  r: int }}");
        assert_eq!(texts(&drift(&with, &without)), ["Missing: foreign key t.r -> u.id"]);
        assert_eq!(texts(&drift(&without, &with)), ["Unexpected: foreign key t.r"]);
        assert_eq!(texts(&drift(&with, &cas)), ["Different: foreign key on t.r: definition differs"]);
    }

    #[test]
    fn relationships_never_count_as_drift() {
        let a = "table u { id: int primary key } table t { id: int primary key  r: u -> one }";
        let live = "table u { id: int primary key } table t { id: int primary key }";
        assert!(drift(a, live).in_sync());
    }

    fn with_raw(expected: &str, table: &str, col: &str, raw: &str) -> Drift {
        let e = ir(expected);
        let mut l = e.clone();
        let t = l.tables.iter_mut().find(|t| t.name == table).unwrap();
        t.columns.iter_mut().find(|c| c.name == col).unwrap().default = Some(ExprIR::Raw { sql: raw.into() });
        compare(Dialect::Postgres, &e, LiveSchema { ir: l, notes: vec![] }, "test")
    }

    #[test]
    fn raw_defaults_align_when_equivalent() {
        let s = "enum R { a, b } table t { id: uuid primary key default gen_uuid()  r: R default b  n: int default 5  s: text default \"hi\"  ts: timestamp default now() }";
        assert!(with_raw(s, "t", "id", "gen_random_uuid()").in_sync());
        assert!(with_raw(s, "t", "r", "'b'::\"R\"").in_sync());
        assert!(with_raw(s, "t", "n", "5").in_sync());
        assert!(with_raw(s, "t", "s", "'hi'::text").in_sync());
        assert!(with_raw(s, "t", "ts", "now()").in_sync());
        // a genuinely different default is drift
        let d = with_raw(s, "t", "n", "6");
        assert_eq!(d.items.len(), 1);
        assert!(d.items[0].text.contains("default: database has 6"), "{}", d.items[0].text);
        assert!(!with_raw(s, "t", "r", "'a'::\"R\"").in_sync());
    }

    #[test]
    fn raw_checks_align_when_equivalent() {
        let e = ir("table t { id: int primary key  n: int } constraint c on t using n >= 18 and n != 99");
        let mut l = e.clone();
        l.tables[0].constraints[0].expr = ExprIR::Raw { sql: "((n >= 18) AND (n <> 99))".into() };
        let d = compare(Dialect::Postgres, &e, LiveSchema { ir: l.clone(), notes: vec![] }, "t");
        assert!(d.in_sync(), "{:?}", texts(&d));
        l.tables[0].constraints[0].expr = ExprIR::Raw { sql: "((n >= 21) AND (n <> 99))".into() };
        let d = compare(Dialect::Postgres, &e, LiveSchema { ir: l, notes: vec![] }, "t");
        assert_eq!(texts(&d), ["Different: check constraint c on t: definition differs"]);
    }

    #[test]
    fn notes_are_carried_but_are_not_drift() {
        let d = compare(
            Dialect::Postgres,
            &ir(BASE),
            LiveSchema { ir: ir(BASE), notes: vec!["column t.c has type varchar(255)".into()] },
            "test",
        );
        assert!(d.in_sync());
        assert_eq!(d.notes.len(), 1);
    }

    #[test]
    fn the_repair_plan_is_live_to_expected() {
        let d = drift(BASE, "table users { id: uuid primary key }");
        let sql = certo_sql::render(&d.plan, Dialect::Postgres).unwrap();
        assert!(sql.contains("ADD COLUMN \"email\" text NOT NULL"), "{sql}");
    }
}
