//! C ABI for the Certo database compiler.
//!
//! Conventions (see `include/certo.h`):
//!   * Strings are UTF-8, NUL-terminated.
//!   * Every function returns a newly allocated JSON string that the caller
//!     must release with `certo_string_free`. Functions never return NULL
//!     for a valid call; a NULL argument yields a `null_argument` error JSON.
//!   * Panics are caught at the boundary and reported as a `panic` error, so
//!     a compiler bug cannot take down the host process.
//!   * All functions are pure and thread-safe.
//!   * Work runs on a dedicated 64 MB-stack thread, so a host thread with a
//!     small stack cannot overflow inside the compiler.
//!
//! The JSON shapes are documented in `api.rs`.

pub mod api;
pub mod runner_api;

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Read a C string argument, or the error JSON to return instead.
///
/// # Safety
/// `p` must be NULL or point to a NUL-terminated string valid for the call.
unsafe fn arg<'a>(p: *const c_char, name: &str) -> Result<&'a str, String> {
    if p.is_null() {
        return Err(api::error("null_argument", format!("argument `{name}` is NULL")));
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_err(|_| api::error("invalid_utf8", format!("argument `{name}` is not valid UTF-8")))
}

/// Stack for the worker thread. Address space is reserved, not committed.
const WORKER_STACK: usize = 64 * 1024 * 1024;

fn into_c(json: String) -> *mut c_char {
    // serde_json escapes control characters, so interior NULs cannot occur.
    CString::new(json).expect("JSON output contains no NUL bytes").into_raw()
}

/// Run `f` on a large-stack worker thread, converting a panic (or a failure
/// to start the thread) into an error JSON, and hand the result to C.
/// Arguments must already be read from C on the calling thread.
fn respond(f: impl FnOnce() -> String + Send) -> *mut c_char {
    let json = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("certo-capi".into())
            .stack_size(WORKER_STACK)
            .spawn_scoped(scope, || catch_unwind(AssertUnwindSafe(f)));
        match worker {
            Ok(handle) => match handle.join() {
                Ok(Ok(s)) => s,
                _ => api::error("panic", "internal compiler error (panic caught at the FFI boundary)"),
            },
            Err(e) => api::error("thread_spawn", format!("could not start compiler thread: {e}")),
        }
    });
    into_c(json)
}

/// ABI version; the host should check it equals the value it was built against.
#[unsafe(no_mangle)]
pub extern "C" fn certo_abi_version() -> u32 {
    api::ABI_VERSION
}

/// Compiler version string, e.g. "0.1.0". Free with `certo_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn certo_version() -> *mut c_char {
    respond(|| env!("CARGO_PKG_VERSION").to_string())
}

/// Compile SDL source to SchemaIR + diagnostics.
///
/// # Safety
/// `source` must be NULL or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_sdl_compile(source: *const c_char) -> *mut c_char {
    match unsafe { arg(source, "source") } {
        Ok(src) => respond(|| api::compile(src)),
        Err(e) => into_c(e),
    }
}

/// Diff two SchemaIR JSON documents into a migration plan.
///
/// # Safety
/// Both arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_diff_ir(old_ir: *const c_char, new_ir: *const c_char) -> *mut c_char {
    let args = unsafe { arg(old_ir, "old_ir").and_then(|o| arg(new_ir, "new_ir").map(|n| (o, n))) };
    match args {
        Ok((old, new)) => respond(|| api::diff_ir(old, new)),
        Err(e) => into_c(e),
    }
}

/// Like `certo_diff_ir`, steered by an MDL migration source (renames, enum
/// remaps, backfills, before/after steps). MDL problems come back as
/// `diagnostics` positioned in the MDL text.
///
/// # Safety
/// All arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_plan_migration(
    old_ir: *const c_char,
    new_ir: *const c_char,
    mdl_source: *const c_char,
) -> *mut c_char {
    let args = unsafe {
        arg(old_ir, "old_ir").and_then(|o| {
            arg(new_ir, "new_ir").and_then(|n| arg(mdl_source, "mdl_source").map(|m| (o, n, m)))
        })
    };
    match args {
        Ok((old, new, mdl)) => respond(|| api::plan_migration(old, new, mdl)),
        Err(e) => into_c(e),
    }
}

/// Compile QL queries (source text) against a SchemaIR JSON document.
/// `dialect` is `"postgres"`. See `api::ql_compile` for the result.
///
/// # Safety
/// All arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_ql_compile(
    schema_ir: *const c_char,
    ql_source: *const c_char,
    dialect: *const c_char,
) -> *mut c_char {
    let args = unsafe {
        arg(schema_ir, "schema_ir").and_then(|s| {
            arg(ql_source, "ql_source").and_then(|q| arg(dialect, "dialect").map(|d| (s, q, d)))
        })
    };
    match args {
        Ok((schema, ql, dialect)) => respond(|| api::ql_compile(schema, ql, dialect)),
        Err(e) => into_c(e),
    }
}

/// Generate typed host code from QL. `options` is JSON: `{"language":"csharp"|"rust",
/// "dialect"?, "namespace"?, "class_name"?}`. See `api::ql_codegen` for the result.
///
/// # Safety
/// All arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_ql_codegen(
    schema_ir: *const c_char,
    ql_source: *const c_char,
    options: *const c_char,
) -> *mut c_char {
    let args = unsafe {
        arg(schema_ir, "schema_ir").and_then(|s| {
            arg(ql_source, "ql_source").and_then(|q| arg(options, "options").map(|o| (s, q, o)))
        })
    };
    match args {
        Ok((schema, ql, options)) => respond(|| api::ql_codegen(schema, ql, options)),
        Err(e) => into_c(e),
    }
}

/// Lower a migration plan JSON document to SQL batches for `dialect`
/// (currently `"postgres"`).
///
/// # Safety
/// Both arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_lower_sql(plan: *const c_char, dialect: *const c_char) -> *mut c_char {
    let args = unsafe { arg(plan, "plan").and_then(|p| arg(dialect, "dialect").map(|d| (p, d))) };
    match args {
        Ok((plan, dialect)) => respond(|| api::lower_sql(plan, dialect)),
        Err(e) => into_c(e),
    }
}

/// Like `certo_lower_sql`, with the old and new schema IR the plan was made between
/// (required for the "sqlite" dialect, which rebuilds tables; ignored by "postgres").
///
/// # Safety
/// All arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_lower_sql_with_schemas(
    plan: *const c_char,
    dialect: *const c_char,
    old_ir: *const c_char,
    new_ir: *const c_char,
) -> *mut c_char {
    let args = unsafe {
        arg(plan, "plan").and_then(|p| {
            arg(dialect, "dialect").and_then(|d| arg(old_ir, "old_ir").and_then(|o| arg(new_ir, "new_ir").map(|n| (p, d, o, n))))
        })
    };
    match args {
        Ok((plan, dialect, old, new)) => respond(|| api::lower_sql_with_schemas(plan, dialect, old, new)),
        Err(e) => into_c(e),
    }
}

// ---- migration runner ----------------------------------------------------
//
// Each call takes the project directory and an optional options JSON object
// (NULL means `{}`), opens whatever it needs, and returns a JSON result. See
// `runner_api.rs` for the options and result of every call.

/// Read an optional C string argument: NULL is `None`, invalid UTF-8 is an error.
///
/// # Safety
/// `p` must be NULL or point to a NUL-terminated string valid for the call.
unsafe fn opt_arg<'a>(p: *const c_char, name: &str) -> Result<Option<&'a str>, String> {
    if p.is_null() { Ok(None) } else { unsafe { arg(p, name) }.map(Some) }
}

/// Shared shape of every runner call.
///
/// # Safety
/// Both pointers must be NULL or valid NUL-terminated strings.
unsafe fn runner_call(
    project_dir: *const c_char,
    options: *const c_char,
    f: fn(&str, Option<&str>) -> String,
) -> *mut c_char {
    let args = unsafe {
        arg(project_dir, "project_dir").and_then(|d| opt_arg(options, "options").map(|o| (d, o)))
    };
    match args {
        Ok((dir, opts)) => respond(move || f(dir, opts)),
        Err(e) => into_c(e),
    }
}

/// Create a migration project (certo-db.toml, schema.sdl, IR.json, migrations/).
/// options: `{"dialect": "postgres"}` (optional).
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_init(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::init) }
}

/// Freeze the difference between schema.sdl and IR.json as the next migration.
/// options: `{"name": "...", "mdl": {"label": "x.mdl", "source": "..."}, "allow_destructive": false}`.
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_new(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::new_migration) }
}

/// List the migrations on disk (no database). `options` is reserved; pass NULL.
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_list(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, |d, _| runner_api::list(d)) }
}

/// Applied vs pending migrations. options: `{"url": "postgres://..."}`.
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_status(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::migration_status) }
}

/// Apply pending migrations. options:
/// `{"url": "...", "dry_run": false, "to": 3, "check_drift": false}`.
/// On a failed migration the error carries `applied[]`: the ones that succeeded first.
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_apply(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::apply) }
}

/// Compare the live database with the last applied migration.
/// options: `{"url": "...", "repair_sql": true}`.
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_drift(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::drift_report) }
}

/// Adopt an existing database: write schema.sdl from its live schema and record
/// a baseline migration as applied (it is not run). options:
/// `{"url": "...", "dry_run": false, "force": false}`. The result lists
/// `omissions` (what SDL cannot express) and `known_drift` (what still differs).
///
/// # Safety
/// Arguments must be NULL or valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_migrate_adopt(project_dir: *const c_char, options: *const c_char) -> *mut c_char {
    unsafe { runner_call(project_dir, options, runner_api::adopt) }
}

/// Read a database's schema as SDL without a project and without changing it. `options` is JSON:
/// `{"url": "...", "dialect"?: "postgres"|"sqlite"}`. Result: `schema_sdl`, `imported` (counts),
/// `omissions` (what SDL cannot express, left out).
///
/// # Safety
/// `options` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_schema_import(options: *const c_char) -> *mut c_char {
    match unsafe { arg(options, "options") } {
        Ok(o) => respond(|| runner_api::import(Some(o))),
        Err(e) => into_c(e),
    }
}

/// Release a string returned by any `certo_*` function. NULL is ignored.
///
/// # Safety
/// `s` must be NULL or a pointer previously returned by this library and
/// not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn certo_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn take(p: *mut c_char) -> Value {
        assert!(!p.is_null());
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        unsafe { certo_string_free(p) };
        serde_json::from_str(&s).unwrap_or(Value::String(s))
    }

    fn c(s: &str) -> CString { CString::new(s).unwrap() }

    #[test]
    fn round_trip_through_the_c_abi() {
        assert_eq!(certo_abi_version(), 1);
        assert_eq!(take(certo_version()), Value::String(env!("CARGO_PKG_VERSION").into()));

        let src = c("table t { id: uuid primary key }");
        let r = take(unsafe { certo_sdl_compile(src.as_ptr()) });
        assert_eq!(r["ok"], true);
        let ir = c(&r["ir"].to_string());

        let empty = c(&api::compile("")).into_string().unwrap();
        let empty_ir = c(&serde_json::from_str::<Value>(&empty).unwrap()["ir"].to_string());
        let d = take(unsafe { certo_diff_ir(empty_ir.as_ptr(), ir.as_ptr()) });
        assert_eq!(d["summary"][0]["text"], "+ table t");

        let plan = c(&d["plan"].to_string());
        let pg = c("postgres");
        let s = take(unsafe { certo_lower_sql(plan.as_ptr(), pg.as_ptr()) });
        assert!(s["batches"][0]["statements"][0].as_str().unwrap().starts_with("CREATE TABLE \"t\""));
    }

    #[test]
    fn ql_through_the_c_abi() {
        let schema = c(&api::compile("table t { id: serial primary key  n: int }"));
        let ir = serde_json::from_str::<Value>(schema.to_str().unwrap()).unwrap()["ir"].to_string();
        let (ir, src, pg) = (c(&ir), c("query q(min: int) { from t select t.id, t.n where t.n > :min }"), c("postgres"));
        // clause order is fixed, so this is a syntax error, reported as a diagnostic
        let r = take(unsafe { certo_ql_compile(ir.as_ptr(), src.as_ptr(), pg.as_ptr()) });
        assert_eq!(r["ok"], false);
        let src = c("query q(min: int) { from t where t.n > :min select t.id, t.n }");
        let r = take(unsafe { certo_ql_compile(ir.as_ptr(), src.as_ptr(), pg.as_ptr()) });
        assert_eq!(r["ok"], true);
        assert_eq!(r["queries"][0]["columns"][1]["nullable"], true);
        let r = take(unsafe { certo_ql_compile(std::ptr::null(), src.as_ptr(), pg.as_ptr()) });
        assert_eq!(r["error"]["code"], "null_argument");
    }

    #[test]
    fn null_and_bad_utf8_arguments() {
        let r = take(unsafe { certo_sdl_compile(std::ptr::null()) });
        assert_eq!(r["error"]["code"], "null_argument");
        let bad = [0xffu8, 0xfe, 0];
        let r = take(unsafe { certo_sdl_compile(bad.as_ptr().cast()) });
        assert_eq!(r["error"]["code"], "invalid_utf8");
        let ok = c("x");
        let r = take(unsafe { certo_diff_ir(ok.as_ptr(), std::ptr::null()) });
        assert_eq!(r["error"]["code"], "null_argument");
        unsafe { certo_string_free(std::ptr::null_mut()) }; // no-op
    }

    #[test]
    fn runner_calls_through_the_c_abi() {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().to_str().unwrap());
        let opts = |j: &str| c(j);

        // NULL options means {}
        let r = take(unsafe { certo_migrate_init(path.as_ptr(), std::ptr::null()) });
        assert_eq!(r["ok"], true);
        std::fs::write(dir.path().join("schema.sdl"), "table t { id: int primary key }").unwrap();
        let o = opts(r#"{"name":"init"}"#);
        let r = take(unsafe { certo_migrate_new(path.as_ptr(), o.as_ptr()) });
        assert_eq!(r["summary"][0], "+ table t");
        let r = take(unsafe { certo_migrate_list(path.as_ptr(), std::ptr::null()) });
        assert_eq!(r["migrations"][0]["label"], "0001_init");

        // NULL project_dir is a null_argument error, not a crash
        for f in [certo_migrate_init, certo_migrate_new, certo_migrate_list, certo_migrate_status, certo_migrate_apply, certo_migrate_drift, certo_migrate_adopt] {
            let r = take(unsafe { f(std::ptr::null(), std::ptr::null()) });
            assert_eq!(r["error"]["code"], "null_argument");
        }
        // bad UTF-8 in the options
        let bad = [b'{', 0xff, 0];
        let r = take(unsafe { certo_migrate_status(path.as_ptr(), bad.as_ptr().cast()) });
        assert_eq!(r["error"]["code"], "invalid_utf8");
        // and no database URL
        let r = take(unsafe { certo_migrate_apply(path.as_ptr(), std::ptr::null()) });
        assert_eq!(r["error"]["code"], "missing_url");
    }

    #[test]
    fn panics_are_contained() {
        let r = take(respond(|| panic!("boom")));
        assert_eq!(r["error"]["code"], "panic");
        // and the library keeps working afterwards
        assert_eq!(take(respond(|| api::compile(""))) ["ok"], true);
    }
}

#[cfg(test)]
mod stack_tests {
    use super::*;

    /// The worker thread, not the caller's, provides the stack: this must
    /// not overflow even when invoked from a deliberately tiny thread.
    #[test]
    fn hostile_input_on_a_tiny_caller_stack() {
        let t = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let nested = format!("constraint c on t using {}1{}", "(".repeat(100_000), ")".repeat(100_000));
                let src = CString::new(nested).unwrap();
                let p = unsafe { certo_sdl_compile(src.as_ptr()) };
                let out = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
                unsafe { certo_string_free(p) };
                out
            })
            .unwrap();
        let out = t.join().unwrap();
        assert!(out.contains("SDL101"), "{out}");
    }
}
