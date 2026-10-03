//! Host API for the migration runner. Same conventions as `api.rs`: plain
//! `&str` in, JSON string out, the `extern "C"` wrappers live in `lib.rs`.
//!
//! Every call takes the project directory and an optional options object and
//! is self-contained: database calls open a connection, do their work and close
//! it (the advisory lock is held only for that time). Nothing is cached
//! between calls, so there are no handles to manage or leak.
//!
//! Options are JSON objects; unknown fields are rejected so typos are caught.
//! A NULL / absent options string means `{}`.
//!
//! Success responses (all have `"ok": true`):
//!   init    {root, dialect}
//!   new     {seq, name, dir, summary[], destructive, warnings}
//!   list    {migrations: [{seq, name, label, dir, checksum, batches, statements}]}
//!   status  {applied: [{seq, name, checksum, applied_at}], pending: [{seq, name}]}
//!   apply   {dry_run, migrations[] (labels), scripts: [{label, sql}]}
//!   adopt   {dry_run, schema_sdl, adopted: {tables, columns, enums, types, sequences, indexes, constraints},
//!            omissions[], migration (label|null), known_drift: [{kind, text}]}
//!   drift   {in_sync, expected_from, items: [{kind, text}], notes[], repair_sql, repair_error}
//!           `ok` is true even when there is drift: the call worked and found it.
//!
//! Failures are `{"ok": false, "error": {"code", "message", ...}}`:
//!   invalid_options, missing_url, io {path}, project, connection,
//!   compile {file, diagnostics[], rendered}   (diagnostics as in `certo_sdl_compile`,
//!                                              positioned in that file's text),
//!   no_changes, destructive {operations[]}, unsupported {op, reason},
//!   history_drift, schema_drift {items[]},
//!   database {seq, name, statement, message, applied[]}  (`applied` = labels
//!   that succeeded before this one failed; they stay applied).

use crate::api::diagnostics_json;
use certo_diagnostics::Diagnostic;
use certo_runner::{
    adopt as adopt_database, apply_with_progress, drift, migration, status, AdoptOptions, ApplyOptions,
    DriftKind, Executor, Project, RunnerError,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

fn kind_str(k: DriftKind) -> &'static str {
    match k {
        DriftKind::Missing => "missing",
        DriftKind::Unexpected => "unexpected",
        DriftKind::Different => "different",
    }
}

fn error_json(code: &str, message: impl Into<String>, extra: Value) -> String {
    let mut e = json!({ "code": code, "message": message.into() });
    if let (Some(obj), Some(more)) = (e.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            obj.insert(k.clone(), v.clone());
        }
    }
    json!({ "ok": false, "error": e }).to_string()
}

fn runner_error(e: &RunnerError) -> String {
    let msg = e.to_string();
    match e {
        RunnerError::Io { path, .. } => error_json("io", msg, json!({ "path": path.display().to_string() })),
        RunnerError::Project(_) => error_json("project", msg, json!({})),
        RunnerError::Connection(_) => error_json("connection", msg, json!({})),
        RunnerError::Compile { file, rendered, diagnostics, source } => error_json(
            "compile",
            msg,
            json!({
                "file": file,
                "rendered": rendered,
                "diagnostics": diagnostics_json(diagnostics as &[Diagnostic], source),
            }),
        ),
        RunnerError::NoChanges => error_json("no_changes", msg, json!({})),
        RunnerError::Destructive(ops) => error_json("destructive", msg, json!({ "operations": ops })),
        RunnerError::Unsupported(u) => error_json("unsupported", msg, json!({ "op": u.op, "reason": u.reason })),
        RunnerError::Drift(_) => error_json("history_drift", msg, json!({})),
        RunnerError::SchemaDrift(_) => error_json("schema_drift", msg, json!({})),
        RunnerError::Database { seq, name, statement, message } => error_json(
            "database",
            msg,
            json!({ "seq": seq, "name": name, "statement": statement, "message": message, "applied": [] }),
        ),
    }
}

fn parse_opts<T: DeserializeOwned + Default>(opts: Option<&str>) -> Result<T, String> {
    match opts.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(T::default()),
        Some(s) => serde_json::from_str(s)
            .map_err(|e| error_json("invalid_options", format!("options are invalid: {e}"), json!({}))),
    }
}

fn open(dir: &str) -> Result<Project, String> {
    Project::open(Path::new(dir)).map_err(|e| runner_error(&e))
}

fn connect(project: &Project, url: &Option<String>) -> Result<Box<dyn Executor>, String> {
    let Some(url) = url.as_deref().filter(|u| !u.is_empty()) else {
        return Err(error_json(
            "missing_url",
            "options.url is required (a postgres:// URL, or a database file path for a sqlite project)",
            json!({}),
        ));
    };
    certo_runner::connect(project, url).map_err(|e| runner_error(&e))
}

// ---- init ---------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct InitOpts {
    dialect: Option<String>,
}

pub fn init(dir: &str, opts: Option<&str>) -> String {
    let o: InitOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    match Project::init(Path::new(dir), o.dialect.as_deref().unwrap_or("postgres")) {
        Ok(p) => json!({ "ok": true, "root": p.root.display().to_string(), "dialect": p.config.dialect }).to_string(),
        Err(e) => runner_error(&e),
    }
}

// ---- new ----------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MdlOpt {
    label: Option<String>,
    source: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct NewOpts {
    name: Option<String>,
    mdl: Option<MdlOpt>,
    #[serde(default)]
    allow_destructive: bool,
}

pub fn new_migration(dir: &str, opts: Option<&str>) -> String {
    let o: NewOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let Some(name) = o.name else {
        return error_json("invalid_options", "options.name is required", json!({}));
    };
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    let label = o.mdl.as_ref().and_then(|m| m.label.clone()).unwrap_or_else(|| "migration.mdl".into());
    let mdl = o.mdl.as_ref().map(|m| (label.as_str(), m.source.as_str()));
    match migration::create(&project, &name, mdl, o.allow_destructive) {
        Ok(c) => json!({
            "ok": true,
            "seq": c.seq,
            "name": c.name,
            "dir": c.dir.display().to_string(),
            "summary": c.summary,
            "destructive": c.destructive,
            "warnings": c.warnings,
        })
        .to_string(),
        Err(e) => runner_error(&e),
    }
}

// ---- list ---------------------------------------------------------------

pub fn list(dir: &str) -> String {
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    match migration::list(&project) {
        Ok(ms) => {
            let items: Vec<Value> = ms
                .iter()
                .map(|m| {
                    json!({
                        "seq": m.seq,
                        "name": m.name,
                        "label": m.label(),
                        "dir": m.dir.display().to_string(),
                        "checksum": m.checksum,
                        "batches": m.script.batches.len(),
                        "statements": m.script.batches.iter().map(|b| b.statements.len()).sum::<usize>(),
                    })
                })
                .collect();
            json!({ "ok": true, "migrations": items }).to_string()
        }
        Err(e) => runner_error(&e),
    }
}

// ---- status -------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DbOpts {
    url: Option<String>,
}

pub fn migration_status(dir: &str, opts: Option<&str>) -> String {
    let o: DbOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    let mut db = match connect(&project, &o.url) { Ok(d) => d, Err(e) => return e };
    match status(&project, &mut db) {
        Ok(s) => json!({
            "ok": true,
            "applied": s.applied.iter().map(|a| json!({
                "seq": a.seq, "name": a.name, "checksum": a.checksum, "applied_at": a.applied_at,
            })).collect::<Vec<_>>(),
            "pending": s.pending.iter().map(|(seq, name)| json!({ "seq": seq, "name": name })).collect::<Vec<_>>(),
        })
        .to_string(),
        Err(e) => runner_error(&e),
    }
}

// ---- apply --------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ApplyOpts {
    url: Option<String>,
    #[serde(default)]
    dry_run: bool,
    to: Option<u32>,
    #[serde(default)]
    check_drift: bool,
}

pub fn apply(dir: &str, opts: Option<&str>) -> String {
    let o: ApplyOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    let mut db = match connect(&project, &o.url) { Ok(d) => d, Err(e) => return e };

    if o.check_drift {
        match drift::check(&project, &mut db) {
            Ok(d) if !d.in_sync() => {
                let items: Vec<Value> =
                    d.items.iter().map(|i| json!({ "kind": kind_str(i.kind), "text": i.text })).collect();
                return error_json(
                    "schema_drift",
                    format!("the database has drifted from {}; nothing was applied", d.expected_from),
                    json!({ "items": items }),
                );
            }
            Ok(_) => {}
            Err(e) => return runner_error(&e),
        }
    }

    let mut done: Vec<String> = Vec::new();
    let result = apply_with_progress(
        &project,
        &mut db,
        &ApplyOptions { dry_run: o.dry_run, to: o.to },
        &mut |label| done.push(label.to_string()),
    );
    match result {
        Ok(r) => json!({
            "ok": true,
            "dry_run": r.dry_run,
            "migrations": r.migrations,
            "scripts": r.scripts.iter().map(|(l, s)| json!({ "label": l, "sql": s })).collect::<Vec<_>>(),
        })
        .to_string(),
        Err(e) => {
            let mut v: Value = serde_json::from_str(&runner_error(&e)).expect("valid json");
            if v["error"]["code"] == "database" {
                v["error"]["applied"] = json!(done);
            }
            v.to_string()
        }
    }
}

// ---- drift --------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriftOpts {
    url: Option<String>,
    /// Include the script that would repair the drift (default true).
    #[serde(default = "yes")]
    repair_sql: bool,
}

fn yes() -> bool { true }

impl Default for DriftOpts {
    fn default() -> Self { DriftOpts { url: None, repair_sql: true } }
}

pub fn drift_report(dir: &str, opts: Option<&str>) -> String {
    let o: DriftOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    let mut db = match connect(&project, &o.url) { Ok(d) => d, Err(e) => return e };
    let d = match drift::check(&project, &mut db) { Ok(d) => d, Err(e) => return runner_error(&e) };

    let (mut repair_sql, mut repair_error) = (Value::Null, Value::Null);
    if o.repair_sql && !d.in_sync() {
        match d.repair_sql(project.dialect()) {
            Ok(s) => repair_sql = json!(s),
            Err(e) => repair_error = json!(e.to_string()),
        }
    }
    json!({
        "ok": true,
        "in_sync": d.in_sync(),
        "expected_from": d.expected_from,
        "items": d.items.iter().map(|i| json!({ "kind": kind_str(i.kind), "text": i.text })).collect::<Vec<_>>(),
        "notes": d.notes,
        "repair_sql": repair_sql,
        "repair_error": repair_error,
    })
    .to_string()
}

// ---- adopt ----------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AdoptOpts {
    url: Option<String>,
    /// Report what would be adopted; write and record nothing.
    #[serde(default)]
    dry_run: bool,
    /// Overwrite a schema.sdl that already has declarations.
    #[serde(default)]
    force: bool,
}

/// Turn an existing database into `schema.sdl` plus a baseline migration that
/// is recorded as applied (never run). `omissions` lists everything in the
/// database that SDL cannot express and was therefore left out.
pub fn adopt(dir: &str, opts: Option<&str>) -> String {
    let o: AdoptOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let project = match open(dir) { Ok(p) => p, Err(e) => return e };
    let mut db = match connect(&project, &o.url) { Ok(d) => d, Err(e) => return e };
    match adopt_database(&project, &mut db, &AdoptOptions { dry_run: o.dry_run, force: o.force }) {
        Ok(r) => json!({
            "ok": true,
            "dry_run": r.dry_run,
            "schema_sdl": r.schema_sdl,
            "adopted": {
                "tables": r.adopted.tables,
                "columns": r.adopted.columns,
                "enums": r.adopted.enums,
                "types": r.adopted.types,
                "sequences": r.adopted.sequences,
                "indexes": r.adopted.indexes,
                "constraints": r.adopted.constraints,
            },
            "omissions": r.omissions,
            "migration": r.migration,
            "known_drift": r.known_drift.iter().map(|i| json!({ "kind": kind_str(i.kind), "text": i.text })).collect::<Vec<_>>(),
        })
        .to_string(),
        Err(e) => runner_error(&e),
    }
}

// ---- schema import ------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ImportOpts {
    url: Option<String>,
    /// `postgres` or `sqlite`; by default `postgres` for a `postgres://` URL and `sqlite` otherwise.
    dialect: Option<String>,
}

/// Read a database's schema as SDL, with no project and without changing the database. options:
/// `{"url": "...", "dialect"?: "postgres"|"sqlite"}`. Result: `schema_sdl`, `imported` (counts) and
/// `omissions` (everything SDL cannot express, left out).
pub fn import(opts: Option<&str>) -> String {
    let o: ImportOpts = match parse_opts(opts) { Ok(o) => o, Err(e) => return e };
    let Some(url) = o.url.as_deref().filter(|u| !u.is_empty()) else {
        return error_json("missing_url", "options.url is required (a postgres:// URL, or a database file path for sqlite)", json!({}));
    };
    let dialect = match o.dialect.as_deref() {
        Some(d @ ("postgres" | "sqlite")) => d,
        Some(other) => return error_json("invalid_options", format!("unknown dialect `{other}` (postgres, sqlite)"), json!({})),
        None if url.starts_with("postgres://") || url.starts_with("postgresql://") => "postgres",
        None => "sqlite",
    };
    if dialect == "sqlite" {
        // opening would create an empty database for a mistyped path; reading must not
        let path = url.strip_prefix("sqlite://").or_else(|| url.strip_prefix("sqlite:")).unwrap_or(url);
        if !Path::new(path).is_file() {
            return error_json("connection", format!("the sqlite database `{path}` does not exist"), json!({}));
        }
    }
    let mut db = match certo_runner::connect_to(dialect, url) { Ok(d) => d, Err(e) => return runner_error(&e) };
    match certo_runner::import_schema(&mut db) {
        Ok(p) => json!({
            "ok": true,
            "schema_sdl": p.sdl,
            "imported": {
                "tables": p.counts.tables,
                "columns": p.counts.columns,
                "enums": p.counts.enums,
                "types": p.counts.types,
                "sequences": p.counts.sequences,
                "indexes": p.counts.indexes,
                "constraints": p.counts.constraints,
            },
            "omissions": p.omissions,
        })
        .to_string(),
        Err(e) => runner_error(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value { serde_json::from_str(s).unwrap() }

    fn code(s: &str) -> String { v(s)["error"]["code"].as_str().unwrap_or("<none>").to_string() }

    fn project() -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().to_str().unwrap().to_string();
        assert_eq!(v(&init(&path, None))["ok"], true);
        (d, path)
    }

    fn set_schema(dir: &str, src: &str) { std::fs::write(Path::new(dir).join("schema.sdl"), src).unwrap(); }

    fn new_json(name: &str) -> String { json!({ "name": name }).to_string() }

    #[test]
    fn import_validates_its_options_and_never_creates_a_database() {
        assert_eq!(code(&import(None)), "missing_url");
        assert_eq!(code(&import(Some(r#"{"url":"x","dialect":"oracle"}"#))), "invalid_options");
        assert_eq!(code(&import(Some(r#"{"url":"x","bogus":1}"#))), "invalid_options");
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("nope.db");
        let opts = json!({ "url": missing.to_str().unwrap() }).to_string();
        assert_eq!(code(&import(Some(&opts))), "connection");
        assert!(!missing.exists(), "a mistyped path must not become an empty database");
    }

    #[test]
    fn init_new_list_without_a_database() {
        let (_d, dir) = project();
        assert_eq!(code(&init(&dir, None)), "project"); // already initialised
        assert_eq!(code(&init(&dir, Some(r#"{"dialect":"oracle"}"#))), "project");

        set_schema(&dir, "table users { id: uuid primary key  email: text not null }");
        let r = v(&new_migration(&dir, Some(&new_json("Initial Schema"))));
        assert_eq!(r["ok"], true);
        assert_eq!((r["seq"].clone(), r["name"].clone()), (json!(1), json!("initial_schema")));
        assert_eq!(r["summary"], json!(["+ table users"]));
        assert_eq!(r["destructive"], false);

        let l = v(&list(&dir));
        assert_eq!(l["migrations"][0]["label"], "0001_initial_schema");
        assert_eq!(l["migrations"][0]["checksum"].as_str().unwrap().len(), 64);
        assert_eq!(l["migrations"][0]["statements"], 1);

        // nothing changed
        assert_eq!(code(&new_migration(&dir, Some(&new_json("again")))), "no_changes");
    }

    #[test]
    fn compile_errors_come_back_positioned() {
        let (_d, dir) = project();
        set_schema(&dir, "table t { id: uuid primary key\n  x: nope }");
        let r = v(&new_migration(&dir, Some(&new_json("x"))));
        assert_eq!(r["error"]["code"], "compile");
        assert_eq!(r["error"]["file"], "schema.sdl");
        let d = &r["error"]["diagnostics"][0];
        assert_eq!(d["code"], "SDL202");
        assert_eq!((d["span"]["line"].clone(), d["span"]["column"].clone()), (json!(2), json!(6)));
        assert!(r["error"]["rendered"].as_str().unwrap().contains("unknown type `nope`"));

        // an MDL problem is positioned in the MDL text, under its own label
        set_schema(&dir, "table t { id: uuid primary key }");
        new_migration(&dir, Some(&new_json("one")));
        set_schema(&dir, "table t { id: uuid primary key  y: int }");
        let opts = json!({ "name": "two", "mdl": { "label": "fix.mdl", "source": "// c\nrename table ghost -> t" } });
        let r = v(&new_migration(&dir, Some(&opts.to_string())));
        assert_eq!(r["error"]["file"], "fix.mdl");
        assert_eq!(r["error"]["diagnostics"][0]["code"], "MDL301");
        assert_eq!(r["error"]["diagnostics"][0]["span"]["line"], 2);
    }

    #[test]
    fn destructive_and_unsupported_are_structured() {
        let (_d, dir) = project();
        set_schema(&dir, "enum E { a, b } table t { id: int primary key  x: text }");
        new_migration(&dir, Some(&new_json("init")));

        set_schema(&dir, "enum E { a, b } table t { id: int primary key }");
        let r = v(&new_migration(&dir, Some(&new_json("drop"))));
        assert_eq!(r["error"]["code"], "destructive");
        assert_eq!(r["error"]["operations"], json!(["- column t.x"]));
        let ok = v(&new_migration(&dir, Some(&json!({ "name": "drop", "allow_destructive": true }).to_string())));
        assert_eq!(ok["destructive"], true);

        set_schema(&dir, "enum E { a } table t { id: int primary key }");
        let r = v(&new_migration(&dir, Some(&json!({ "name": "shrink", "allow_destructive": true }).to_string())));
        assert_eq!(r["error"]["code"], "unsupported");
        assert_eq!(r["error"]["op"], "- enum variant E.b");
        assert!(r["error"]["reason"].as_str().unwrap().contains("remap"));
    }

    #[test]
    fn option_and_project_errors() {
        let (_d, dir) = project();
        assert_eq!(code(&new_migration(&dir, Some("{}"))), "invalid_options"); // name required
        assert_eq!(code(&new_migration(&dir, Some("not json"))), "invalid_options");
        assert_eq!(code(&new_migration(&dir, Some(r#"{"name":"x","typo":1}"#))), "invalid_options");
        let nowhere = tempfile::tempdir().unwrap();
        let none = nowhere.path().to_str().unwrap();
        assert_eq!(code(&list(none)), "project");
        assert_eq!(code(&new_migration(none, Some(&new_json("x")))), "project");
    }

    #[test]
    fn database_calls_need_a_url_and_report_connection_failures() {
        let (_d, dir) = project();
        for f in [migration_status, apply, drift_report, adopt] {
            assert_eq!(code(&f(&dir, None)), "missing_url");
            assert_eq!(code(&f(&dir, Some(r#"{"url":""}"#))), "missing_url");
            assert_eq!(code(&f(&dir, Some(r#"{"url":"not a url"}"#))), "connection");
            // nothing is listening here
            assert_eq!(code(&f(&dir, Some(r#"{"url":"postgres://u@127.0.0.1:1/db"}"#))), "connection");
            assert_eq!(code(&f(&dir, Some(r#"{"url":"postgres://u@127.0.0.1:1/db","bogus":true}"#))), "invalid_options");
        }
    }

    #[test]
    fn errors_never_echo_the_connection_password() {
        let (_d, dir) = project();
        let r = apply(&dir, Some(r#"{"url":"postgres://user:hunter2secret@127.0.0.1:1/db"}"#));
        assert!(!r.contains("hunter2secret"), "{r}");
    }
}
