//! Pure-Rust host API. Every function takes `&str`s and returns a JSON
//! string; the `extern "C"` layer in `lib.rs` only moves strings across the
//! boundary. Keeping the logic here makes it testable without FFI.
//!
//! Result envelope, always a JSON object:
//!   * `"ok": true`  — the operation succeeded (fields depend on the call).
//!   * `"ok": false` with `"diagnostics"` — the *schema* has errors (a normal outcome).
//!   * `"ok": false` with `"error"` — the *call* failed: bad input JSON,
//!     unknown dialect, an op the dialect cannot express, an internal fault.
//!     `error` is `{ "code", "message" }` plus call-specific fields.
//!
//! Error codes: `null_argument`, `invalid_utf8`, `invalid_ir`,
//! `invalid_plan`, `unsupported_version`, `unknown_dialect`, `unsupported`,
//! `panic`.

use certo_diagnostics::{render_all, Diagnostic, Severity};
use certo_mdl::{MigrationPlan, PLAN_VERSION};
use certo_sdl::{SchemaIR, IR_VERSION};
use certo_sql::{Dialect, LowerError};
use serde_json::{json, Value};

/// Bumped on any incompatible change to the C signatures or JSON shapes.
pub const ABI_VERSION: u32 = 1;

pub fn error(code: &str, message: impl Into<String>) -> String {
    json!({ "ok": false, "error": { "code": code, "message": message.into() } }).to_string()
}

/// Compile SDL source. `ir` is present only when `ok` is true; `diagnostics`
/// (errors and warnings) and a human-readable `rendered` text are always present.
pub fn compile(source: &str) -> String {
    let (ir, diags) = certo_sdl::compile(source);
    json!({
        "ok": ir.is_some(),
        "ir": ir,
        "diagnostics": diagnostics_json(&diags, source),
        "rendered": render_all(&diags, source, "schema.sdl", false),
    })
    .to_string()
}

/// Compile QL against a SchemaIR (JSON). On success `statements` holds every
/// query, insert, update and delete (tagged `kind`) and `queries` just the
/// read queries. Per statement it has its parameters, typed result columns (`type` + `nullable`), the SQL
/// and `param_order` (which declared parameter is `$1`, `$2`, ...). QL errors
/// come back as `diagnostics` positioned in the QL text with `ok: false`.
pub fn ql_compile(schema_ir: &str, ql_source: &str, dialect: &str) -> String {
    let Some(dialect) = Dialect::from_name(dialect) else {
        return error("unknown_dialect", format!("unknown SQL dialect `{dialect}` (supported: postgres, sqlite)"));
    };
    let schema = match parse_ir(schema_ir, "schema") { Ok(v) => v, Err(e) => return e };
    let compiled = certo_ql::compile_full(&schema, ql_source, dialect);
    let (statements, fragments, diags) = (compiled.statements, compiled.fragments, compiled.diagnostics);
    // persisted views, in the order they must be created; `table` is what the query language reads (add it to a schema's
    // `tables` to compile other files against these views)
    let views: Vec<_> = compiled
        .views
        .iter()
        .map(|v| json!({ "name": v.name, "columns": v.columns.iter().map(|c| json!({ "name": c.name, "type": c.ty, "nullable": c.nullable })).collect::<Vec<_>>(), "create_sql": v.create_sql, "drop_sql": v.drop_sql, "table": v.table }))
        .collect();
    let all = statements.as_deref().map(certo_ql::to_json);
    // fragments are not statements, but a host may want to show them: name, parameters and result columns
    let fragments: Vec<_> = fragments
        .iter()
        .map(|f| json!({ "name": f.name, "params": f.params, "columns": f.columns.iter().map(|c| json!({ "name": c.name, "type": c.ty, "nullable": c.nullable })).collect::<Vec<_>>() }))
        .collect();
    // `queries` keeps its original meaning (read queries only); `statements`
    // holds everything, each tagged with its `kind`.
    let queries = all.as_ref().and_then(|a| a.as_array()).map(|a| {
        a.iter().filter(|s| s["kind"] == "query").cloned().collect::<Vec<_>>()
    });
    json!({
        "ok": statements.is_some(),
        "queries": queries,
        "statements": all,
        "fragments": fragments,
        "views": views,
        "diagnostics": diagnostics_json(&diags, ql_source),
        "rendered": render_all(&diags, ql_source, "queries.ql", false),
    })
    .to_string()
}

/// Diff two SchemaIR JSON documents into a migration plan.
pub fn diff_ir(old_ir: &str, new_ir: &str) -> String {
    let old = match parse_ir(old_ir, "old") { Ok(v) => v, Err(e) => return e };
    let new = match parse_ir(new_ir, "new") { Ok(v) => v, Err(e) => return e };
    plan_json(certo_mdl::diff(&old, &new))
}

/// Like `diff_ir`, but steered by an MDL migration (renames, enum remaps,
/// backfills, before/after steps). Problems in the MDL come back as
/// `diagnostics` (positions refer to the MDL source) with `ok: false`.
pub fn plan_migration(old_ir: &str, new_ir: &str, mdl_source: &str) -> String {
    let old = match parse_ir(old_ir, "old") { Ok(v) => v, Err(e) => return e };
    let new = match parse_ir(new_ir, "new") { Ok(v) => v, Err(e) => return e };
    let (plan, diags) = certo_mdl::compile_migration(&old, &new, mdl_source);
    let rendered = render_all(&diags, mdl_source, "migration.mdl", false);
    let Some(plan) = plan else {
        return json!({
            "ok": false,
            "diagnostics": diagnostics_json(&diags, mdl_source),
            "rendered": rendered,
        })
        .to_string();
    };
    let mut result: Value = serde_json::from_str(&plan_json(plan)).expect("plan_json is valid JSON");
    result["diagnostics"] = json!(diagnostics_json(&diags, mdl_source));
    result["rendered"] = json!(rendered);
    result.to_string()
}

fn plan_json(plan: MigrationPlan) -> String {
    let summary: Vec<Value> = plan
        .ops
        .iter()
        .map(|op| json!({ "text": op.describe(), "destructive": op.is_destructive() }))
        .collect();
    json!({
        "ok": true,
        "empty": plan.is_empty(),
        "destructive": plan.has_destructive(),
        "summary": summary,
        "plan": plan,
    })
    .to_string()
}

/// Generate typed host code from QL (see `certo_ql::csharp` and `certo_ql::rust`). `options`:
/// `{"language": "csharp" | "rust", "dialect": "postgres" | "sqlite" | "mysql", "namespace"?, "class_name"?, "async"?}`
/// (`dialect` defaults to postgres; `namespace` and `class_name` are for C#; Rust code calls the
/// `postgres` crate, `rusqlite` or `mysql` according to the dialect, or with `"async": true` the async
/// drivers `tokio-postgres` and `mysql_async` (SQLite has none)). On success `code` holds one source file; QL errors come back
/// as `diagnostics` exactly as for `ql_compile`, with `ok: false`.
pub fn ql_codegen(schema_ir: &str, ql_source: &str, options: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    struct Opts {
        language: Option<String>,
        dialect: Option<String>,
        namespace: Option<String>,
        class_name: Option<String>,
        #[serde(rename = "async")]
        asynchronous: Option<bool>,
    }
    let opts: Opts = match serde_json::from_str(if options.trim().is_empty() { "{}" } else { options }) {
        Ok(o) => o,
        Err(e) => return error("invalid_options", format!("options are invalid: {e}")),
    };
    let rust = match opts.language.as_deref() {
        Some("csharp" | "cs") => false,
        Some("rust" | "rs") => true,
        Some(other) => return error("unknown_language", format!("unknown language `{other}` (supported: csharp, rust)")),
        None => return error("invalid_options", "options.language is required (supported: csharp, rust)"),
    };
    let dialect_name = opts.dialect.as_deref().unwrap_or("postgres");
    let Some(dialect) = Dialect::from_name(dialect_name) else {
        return error("unknown_dialect", format!("unknown SQL dialect `{dialect_name}` (supported: postgres, sqlite)"));
    };
    let asynchronous = opts.asynchronous.unwrap_or(false);
    if asynchronous && !rust {
        return error("invalid_options", "options.async is for Rust code (C# methods are already async)");
    }
    if asynchronous && !certo_ql::async_supported(dialect) {
        return error("unsupported_option", "there is no async driver for SQLite (rusqlite is synchronous): generate the sync code, or run it on a blocking thread");
    }
    let schema = match parse_ir(schema_ir, "schema") { Ok(v) => v, Err(e) => return e };
    let (statements, diags) = certo_ql::compile(&schema, ql_source, dialect);
    let mut cs = certo_ql::CSharpOptions { dialect, ..Default::default() };
    if let Some(n) = opts.namespace { cs.namespace = n; }
    if let Some(c) = opts.class_name { cs.class_name = c; }
    json!({
        "ok": statements.is_some(),
        "code": statements.as_deref().map(|s| if rust {
            certo_ql::generate_rust(&schema, s, &certo_ql::RustOptions { dialect, async_: asynchronous })
        } else {
            certo_ql::generate_csharp(&schema, s, &cs)
        }),
        "diagnostics": diagnostics_json(&diags, ql_source),
        "rendered": render_all(&diags, ql_source, "queries.ql", false),
    })
    .to_string()
}

/// Lower a migration plan JSON document to SQL batches. SQLite also needs the
/// schemas the plan was made between: use `lower_sql_with_schemas`.
pub fn lower_sql(plan_json: &str, dialect: &str) -> String {
    lower_sql_impl(plan_json, dialect, None)
}

/// Like `lower_sql`, with the old and new schema IR (needed for SQLite, ignored by PostgreSQL).
pub fn lower_sql_with_schemas(plan_json: &str, dialect: &str, old_ir: &str, new_ir: &str) -> String {
    let old = match parse_ir(old_ir, "old") { Ok(v) => v, Err(e) => return e };
    let new = match parse_ir(new_ir, "new") { Ok(v) => v, Err(e) => return e };
    lower_sql_impl(plan_json, dialect, Some((&old, &new)))
}

fn lower_sql_impl(plan_json: &str, dialect: &str, schemas: Option<(&SchemaIR, &SchemaIR)>) -> String {
    let Some(dialect) = Dialect::from_name(dialect) else {
        return error("unknown_dialect", format!("unknown SQL dialect `{dialect}` (supported: postgres, sqlite, mysql)"));
    };
    let plan: MigrationPlan = match serde_json::from_str(plan_json) {
        Ok(p) => p,
        Err(e) => return error("invalid_plan", format!("plan JSON is invalid: {e}")),
    };
    if plan.version != PLAN_VERSION {
        return error(
            "unsupported_version",
            format!("plan version {} is not supported (expected {PLAN_VERSION})", plan.version),
        );
    }
    let lowered = match schemas {
        Some((old, new)) => certo_sql::lower_batches_with(&plan, dialect, certo_sql::Schemas { old, new }),
        None => certo_sql::lower_batches(&plan, dialect),
    };
    match lowered {
        Ok(batches) => {
            let script = match schemas {
                Some((old, new)) => certo_sql::render_with(&plan, dialect, certo_sql::Schemas { old, new }),
                None => certo_sql::render(&plan, dialect),
            }
            .unwrap_or_default();
            let batches: Vec<Value> = batches
                .iter()
                .map(|b| json!({ "transactional": b.transactional, "statements": b.statements }))
                .collect();
            json!({ "ok": true, "batches": batches, "script": script }).to_string()
        }
        Err(LowerError { op, reason }) if schemas.is_none() && dialect != Dialect::Postgres => json!({
            "ok": false,
            "error": {
                "code": "needs_schemas",
                "message": format!("cannot lower `{op}`: {reason}"),
            }
        })
        .to_string(),
        Err(LowerError { op, reason }) => json!({
            "ok": false,
            "error": {
                "code": "unsupported",
                "message": format!("cannot lower `{op}`: {reason}"),
                "op": op,
                "reason": reason,
            }
        })
        .to_string(),
    }
}

fn parse_ir(s: &str, which: &str) -> Result<SchemaIR, String> {
    let ir: SchemaIR = serde_json::from_str(s)
        .map_err(|e| error("invalid_ir", format!("{which} IR JSON is invalid: {e}")))?;
    if ir.version != IR_VERSION {
        return Err(error(
            "unsupported_version",
            format!("{which} IR version {} is not supported (expected {IR_VERSION})", ir.version),
        ));
    }
    Ok(ir)
}

pub(crate) fn diagnostics_json(diags: &[Diagnostic], src: &str) -> Vec<Value> {
    diags
        .iter()
        .map(|d| {
            let span = d.span.map(|s| {
                let (start, end) = (s.start as usize, s.end as usize);
                let (line, column) = position(src, start);
                let (end_line, end_column) = position(src, end);
                json!({
                    "start": start, "end": end,
                    "line": line, "column": column,
                    "end_line": end_line, "end_column": end_column,
                })
            });
            json!({
                "severity": match d.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Note => "note",
                },
                "code": d.code,
                "message": d.message,
                "span": span,
                "label": d.label,
                "notes": d.notes,
            })
        })
        .collect()
}

/// 1-based line and column of a byte offset. The column counts UTF-16 code
/// units, matching how .NET indexes strings.
fn position(src: &str, mut byte: usize) -> (usize, usize) {
    byte = byte.min(src.len());
    while !src.is_char_boundary(byte) { byte -= 1; }
    let before = &src[..byte];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before[line_start..].encode_utf16().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value { serde_json::from_str(s).unwrap() }

    fn code(s: &str) -> String { v(s)["error"]["code"].as_str().unwrap_or("<none>").to_string() }

    #[test]
    fn codegen_returns_a_source_file_or_diagnostics() {
        let ir = v(&compile("table t { id: serial primary key  n: int }"))["ir"].to_string();
        let opts = r#"{"language":"csharp","namespace":"App.Db","class_name":"Q","dialect":"sqlite"}"#;
        let r = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", opts));
        assert_eq!(r["ok"], true);
        let code = r["code"].as_str().unwrap();
        assert!(code.contains("namespace App.Db;") && code.contains("public static partial class Q"), "{code}");
        assert!(code.contains("ByNRow(int Id, int? N)") && code.contains("ByNAsync(this DbConnection connection, int n"), "{code}");
        assert!(code.contains("p.ParameterName = \"?1\";"), "sqlite binds ?1");
        let bad = v(&ql_codegen(&ir, "query q() { from t select t.ghost }", opts));
        assert_eq!(bad["ok"], false);
        assert!(bad["code"].is_null() && bad["diagnostics"][0]["code"] == "QL206");
        // async Rust: tokio-postgres and mysql_async; SQLite has no async driver, and C# has no such option
        let a = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"rust","async":true}"#));
        assert!(a["code"].as_str().unwrap().contains("pub async fn by_n<C: tokio_postgres::GenericClient>(client: &C, n: i32)"), "{a}");
        let a = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"rust","dialect":"mysql","async":true}"#));
        assert!(a["code"].as_str().unwrap().contains("pub async fn by_n<C: mysql_async::prelude::Queryable>(conn: &mut C, n: i32)"), "{a}");
        assert_eq!(v(&ql_codegen(&ir, "", r#"{"language":"rust","dialect":"sqlite","async":true}"#))["error"]["code"], "unsupported_option");
        assert_eq!(v(&ql_codegen(&ir, "", r#"{"language":"csharp","async":true}"#))["error"]["code"], "invalid_options");
        // MySQL generates too (C# and Rust), over its own drivers
        let my = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"rust","dialect":"mysql"}"#));
        assert!(my["code"].as_str().unwrap().contains("conn: &mut C"), "{my}");
        let my_cs = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"csharp","dialect":"mysql"}"#));
        assert!(my_cs["code"].as_str().unwrap().contains("ByNAsync(this DbConnection connection, int n"), "{my_cs}");
        // Rust: the driver follows the dialect
        let rs = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"rust","dialect":"sqlite"}"#));
        let rs_code = rs["code"].as_str().unwrap();
        assert!(rs_code.contains("pub struct ByNRow") && rs_code.contains("pub fn by_n(conn: &rusqlite::Connection, n: i32)"), "{rs_code}");
        let pg = v(&ql_codegen(&ir, "query by_n(n: int) { from t where t.n == :n select t.id, t.n }", r#"{"language":"rust"}"#));
        assert!(pg["code"].as_str().unwrap().contains("client: &mut C"));
        assert_eq!(v(&ql_codegen(&ir, "", r#"{"language":"cobol"}"#))["error"]["code"], "unknown_language");
        assert_eq!(v(&ql_codegen(&ir, "", "{}"))["error"]["code"], "invalid_options");
        assert_eq!(v(&ql_codegen(&ir, "", r#"{"language":"csharp","bogus":1}"#))["error"]["code"], "invalid_options");
    }

    #[test]
    fn sqlite_lowering_needs_schemas() {
        let (old, new) = (
            compile("table t { id: serial primary key }"),
            compile("table t { id: serial primary key  n: int not null default 0 }"),
        );
        let (old, new) = (v(&old)["ir"].to_string(), v(&new)["ir"].to_string());
        let plan = v(&diff_ir(&old, &new))["plan"].to_string();
        assert_eq!(v(&lower_sql(&plan, "sqlite"))["error"]["code"], "needs_schemas");
        let r = v(&lower_sql_with_schemas(&plan, "sqlite", &old, &new));
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["script"].as_str().unwrap().contains("ADD COLUMN \"n\" INTEGER NOT NULL DEFAULT 0"));
    }

    #[test]
    fn position_counts_utf16_columns() {
        assert_eq!(position("abc\ndef", 5), (2, 2));
        // "é" is 2 bytes / 1 unit; an astral char is 4 bytes / 2 units
        assert_eq!(position("é x", 3), (1, 3));
        assert_eq!(position("😀x", 4), (1, 3));
        // offset inside a multibyte char snaps back to its start
        assert_eq!(position("é", 1), (1, 1));
        assert_eq!(position("ab", 99), (1, 3));
    }

    #[test]
    fn compile_ok_and_errors() {
        let r = v(&compile("table t { id: uuid primary key }"));
        assert_eq!(r["ok"], true);
        assert_eq!(r["ir"]["tables"][0]["name"], "t");
        assert_eq!(r["diagnostics"], json!([]));

        let r = v(&compile("table t { id: uuid primary key\n  x: nope }"));
        assert_eq!(r["ok"], false);
        assert!(r["ir"].is_null());
        let d = &r["diagnostics"][0];
        assert_eq!(d["code"], "SDL202");
        assert_eq!(d["severity"], "error");
        assert_eq!(d["span"]["line"], 2);
        assert_eq!(d["span"]["column"], 6);
        assert!(r["rendered"].as_str().unwrap().contains("unknown type `nope`"));
        assert!(r.get("error").is_none());
    }

    #[test]
    fn warnings_come_with_a_successful_compile() {
        let r = v(&compile("table t { x: int }"));
        assert_eq!(r["ok"], true);
        assert_eq!(r["diagnostics"][0]["severity"], "warning");
    }

    fn ir_of(src: &str) -> String {
        v(&compile(src))["ir"].to_string()
    }

    #[test]
    fn diff_then_lower_end_to_end() {
        let old = ir_of("table t { id: int primary key }");
        let new = ir_of("table t { id: int primary key  n: text not null default \"x\" }");
        let d = v(&diff_ir(&old, &new));
        assert_eq!(d["ok"], true);
        assert_eq!(d["destructive"], false);
        assert_eq!(d["summary"][0]["text"], "+ column t.n");

        let s = v(&lower_sql(&d["plan"].to_string(), "postgres"));
        assert_eq!(s["ok"], true);
        assert_eq!(s["batches"][0]["transactional"], true);
        assert_eq!(
            s["batches"][0]["statements"][0],
            "ALTER TABLE \"t\" ADD COLUMN \"n\" text NOT NULL DEFAULT 'x';"
        );
        assert!(s["script"].as_str().unwrap().starts_with("BEGIN;"));
    }

    #[test]
    fn destructive_and_empty_flags() {
        let a = ir_of("table t { id: int primary key x: int }");
        let b = ir_of("table t { id: int primary key }");
        let d = v(&diff_ir(&a, &b));
        assert_eq!(d["destructive"], true);
        assert_eq!(d["summary"][0]["destructive"], true);
        let same = v(&diff_ir(&a, &a));
        assert_eq!(same["empty"], true);
    }

    #[test]
    fn operational_errors_use_the_error_envelope() {
        let good = ir_of("table t { id: int primary key }");
        let r = v(&diff_ir("not json", &good));
        assert_eq!((r["ok"].clone(), r["error"]["code"].clone()), (json!(false), json!("invalid_ir")));
        assert!(r["error"]["message"].as_str().unwrap().starts_with("old IR"));
        assert_eq!(v(&diff_ir(&good, "{}"))["error"]["code"], "invalid_ir");

        let bad_ver = good.replacen("\"version\":1", "\"version\":99", 1);
        assert_eq!(v(&diff_ir(&bad_ver, &good))["error"]["code"], "unsupported_version");

        assert_eq!(v(&lower_sql("{}", "oracle"))["error"]["code"], "unknown_dialect");
        assert_eq!(v(&lower_sql("nope", "postgres"))["error"]["code"], "invalid_plan");
        assert_eq!(v(&lower_sql("{\"version\":7,\"ops\":[]}", "pg"))["error"]["code"], "unsupported_version");
    }

    #[test]
    fn unsupported_ops_report_op_and_reason() {
        let a = ir_of("enum E { a, b } table t { id: int primary key }");
        let b = ir_of("enum E { a } table t { id: int primary key }");
        let d = v(&diff_ir(&a, &b));
        let s = v(&lower_sql(&d["plan"].to_string(), "postgres"));
        assert_eq!(s["ok"], false);
        assert_eq!(s["error"]["code"], "unsupported");
        assert_eq!(s["error"]["op"], "- enum variant E.b");
        assert!(s["error"]["reason"].as_str().unwrap().contains("enum value"));
    }

    #[test]
    fn mdl_migration_through_the_api() {
        let old = ir_of("table a { id: int primary key  x: text }");
        let new = ir_of("table b { id: int primary key  x: text }");
        // plain diff: destructive drop + add
        assert_eq!(v(&diff_ir(&old, &new))["destructive"], true);
        // with MDL: a single, safe rename
        let r = v(&plan_migration(&old, &new, "rename table a -> b"));
        assert_eq!(r["ok"], true);
        assert_eq!(r["destructive"], false);
        assert_eq!(r["summary"][0]["text"], "~ rename table a -> b");
        let sql = v(&lower_sql(&r["plan"].to_string(), "postgres"));
        assert_eq!(sql["batches"][0]["statements"][0], "ALTER TABLE \"a\" RENAME TO \"b\";");

        // MDL errors come back as diagnostics against the MDL source
        let bad = v(&plan_migration(&old, &new, "rename table a -> ghost"));
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["diagnostics"][0]["code"], "MDL302");
        assert_eq!(bad["diagnostics"][0]["span"]["line"], 1);
        assert!(bad["rendered"].as_str().unwrap().contains("migration.mdl"));
        assert!(bad.get("error").is_none());

        // syntax error
        assert_eq!(v(&plan_migration(&old, &new, "rename"))["ok"], false);
        // operational errors still use the error envelope
        assert_eq!(v(&plan_migration("x", &new, ""))["error"]["code"], "invalid_ir");
    }

    #[test]
    fn ql_compile_returns_a_typed_contract() {
        let ir = ir_of("table orders { id: serial primary key  total: decimal(10,2) not null  note: text }
                        table customers { id: serial primary key  name: text not null }");
        let src = "query big(min: decimal(10,2)) {
            from orders o left join customers c on true
            where o.total >= :min select o.id, c.name as customer, o.note
        }";
        let r = v(&ql_compile(&ir, src, "postgres"));
        assert_eq!(r["ok"], true);
        let q = &r["queries"][0];
        assert_eq!(q["name"], "big");
        assert_eq!(q["params"][0]["name"], "min");
        assert_eq!(q["params"][0]["type"]["name"], json!({"numeric": [10, 2]}));
        assert_eq!(q["columns"][0], json!({"name": "id", "type": {"kind": "builtin", "name": "int"}, "nullable": false}));
        assert_eq!(q["columns"][1]["nullable"], true, "left-joined column is nullable");
        assert_eq!(q["columns"][2]["nullable"], true, "nullable column");
        assert_eq!(q["param_order"], json!(["min"]));
        assert!(q["sql"].as_str().unwrap().contains("(\"o\".\"total\" >= ($1::numeric(10,2)))"));

        // QL errors are diagnostics positioned in the QL text
        let bad = v(&ql_compile(&ir, "// c
query q() { from orders o select o.ghost }", "postgres"));
        assert_eq!(bad["ok"], false);
        assert!(bad["queries"].is_null());
        let d = &bad["diagnostics"][0];
        assert_eq!(d["code"], "QL206");
        assert_eq!(d["span"]["line"], 2);
        assert!(bad["rendered"].as_str().unwrap().contains("unknown column `ghost`"));
        // warnings ride along with a successful compile
        let w = v(&ql_compile(&ir, "query q(unused: int) { from orders o select o.id }", "postgres"));
        assert_eq!(w["ok"], true);
        assert_eq!(w["diagnostics"][0]["code"], "QL290");

        // mutations ride in `statements`, tagged by kind; `queries` stays read-only
        let m = v(&ql_compile(
            &ir,
            "query q() { from orders o select o.id }
             insert add(n: text) { into customers set name = :n returning id }",
            "postgres",
        ));
        assert_eq!(m["ok"], true);
        assert_eq!(m["queries"].as_array().unwrap().len(), 1);
        assert_eq!(m["statements"][1]["kind"], "insert");
        assert_eq!(m["statements"][1]["columns"][0]["name"], "id");
        assert!(m["statements"][1]["sql"].as_str().unwrap().starts_with("INSERT INTO"));
        // operational errors use the error envelope
        assert_eq!(code(&ql_compile("nope", src, "postgres")), "invalid_ir");
        assert_eq!(code(&ql_compile(&ir, src, "oracle")), "unknown_dialect");
    }

    #[test]
    fn hostile_input_does_not_crash() {
        let chain = vec!["1"; 200_000].join(" + ");
        let r = v(&compile(&format!("constraint c on t using {chain}")));
        assert_eq!(r["ok"], false);
        assert_eq!(r["diagnostics"][0]["code"], "SDL101");
        // junk, empty and unicode input
        for s in ["", "\u{0}", "table", "table ☃ {", "}}}}", "table t { a: int default \"\u{1F600}"] {
            let r = v(&compile(s));
            assert!(r["ok"].is_boolean());
        }
    }
}
