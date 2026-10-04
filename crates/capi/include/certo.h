/*
 * certo.h - C ABI for the Certo database compiler (SDL / migrations / SQL).
 *
 * Library: certo_capi.dll (Windows), libcerto_capi.so (Linux),
 *          libcerto_capi.dylib (macOS).
 *
 * Conventions
 *   - All strings are UTF-8 and NUL-terminated.
 *   - Every function below (except certo_abi_version) returns a newly
 *     allocated string that the caller MUST release with certo_string_free.
 *     Never free it with free() / CoTaskMemFree: it belongs to Rust's allocator.
 *   - Results are JSON objects with an "ok" boolean. "ok": false comes in two
 *     flavours: schema errors (a "diagnostics" array) and failed calls (an
 *     "error": {"code","message"} object). See crates/capi/src/api.rs for
 *     every field and error code.
 *   - Functions are pure and thread-safe. A panic inside the compiler is
 *     caught and returned as {"ok":false,"error":{"code":"panic",...}}.
 *   - Passing NULL yields a "null_argument" error result, not a crash.
 *
 * Typical flow
 *   compile(schema.sdl)              -> ir           (persist as IR.json)
 *   diff_ir(previous ir, new ir)     -> plan, summary, destructive flag
 *     or plan_migration(prev, new, mdl text) when the change needs intent
 *     (renames, enum value removal, backfills, hand-written steps)
 *   lower_sql(plan, "postgres")      -> batches / script to execute
 *                                       (sqlite: lower_sql_with_schemas(plan, "sqlite", prev, new))
 */
#ifndef CERTO_H
#define CERTO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Must equal CERTO_ABI_VERSION below, or the host and library are mismatched. */
#define CERTO_ABI_VERSION 1u
uint32_t certo_abi_version(void);

/* Library version, e.g. "0.8.0". */
char *certo_version(void);

/* Compile SDL source.
 * -> {"ok", "ir": SchemaIR|null, "diagnostics":[...], "rendered": "text"} */
char *certo_sdl_compile(const char *source);

/* Diff two SchemaIR JSON documents.
 * -> {"ok", "empty", "destructive", "summary":[{"text","destructive"}], "plan"} */
char *certo_diff_ir(const char *old_ir, const char *new_ir);

/* Like certo_diff_ir, steered by an MDL migration (rename / remap / backfill /
 * before{} / after{}). MDL errors -> {"ok":false, "diagnostics":[...]}
 * positioned in the MDL text; success also carries "diagnostics" (warnings).
 * An additive extension within ABI 1: no earlier signature changed. */
char *certo_plan_migration(const char *old_ir, const char *new_ir, const char *mdl_source);

/* Compile QL queries against a schema (the "ir" from certo_sdl_compile).
 * -> {"ok", "statements":[{kind, name, params:[{name,type,nullable}], columns:[{name,type,nullable}],
 *                          sql, param_order:[...], ir}] | null,
 *     "queries": the same list restricted to kind "query" (kept for older hosts),
 *     "diagnostics":[...], "rendered"}
 * kind is query | insert | update | delete; a mutation's columns are its `returning` list
 * (empty: the host gets a row count). Each statement is a typed contract: bind parameters as $1, $2, ... in param_order; every
 * result column's type and nullability is known before the query runs. QL errors are
 * diagnostics positioned in the QL text (codes QL2xx). Additive within ABI 1. */
char *certo_ql_compile(const char *schema_ir, const char *ql_source, const char *dialect);

/* Generate typed host code from QL: one source file with a record per result and a method per
 * statement. options: {"language":"csharp", "dialect"?:"postgres"|"sqlite", "namespace"?, "class_name"?}
 * -> {"ok", "code": "<C# source>" | null, "diagnostics":[...], "rendered"} (QL errors are
 * diagnostics, as for certo_ql_compile). Additive within ABI 1. */
char *certo_ql_codegen(const char *schema_ir, const char *ql_source, const char *options);

/* Lower a plan (the "plan" value from certo_diff_ir) to SQL. dialect: "postgres" or
 * "sqlite". -> {"ok", "batches":[{"transactional","statements":[...]}], "script"}
 * SQLite cannot be lowered from the plan alone (it rebuilds tables and writes enums as
 * CHECK constraints): certo_lower_sql answers error code "needs_schemas" for it. */
char *certo_lower_sql(const char *plan, const char *dialect);

/* certo_lower_sql plus the old and new schema IR the plan was made between. Required for
 * "sqlite" (rebuilds run as: PRAGMA foreign_keys = OFF; the transaction; PRAGMA foreign_keys
 * = ON, in separate batches); ignored for "postgres". Additive within ABI 1. */
char *certo_lower_sql_with_schemas(const char *plan, const char *dialect,
                                   const char *old_ir, const char *new_ir);

/* ---- Migration runner -------------------------------------------------------
 * Every call takes the project directory and an optional JSON options object
 * (NULL means {}), and returns a JSON result. Each call is self-contained:
 * database calls open a connection, do their work and close it, so there are no
 * handles to manage. Options reject unknown fields. Results and error codes are
 * documented in crates/capi/src/runner_api.rs. Calls may take as long as the
 * database does (apply); run them off your UI thread.
 *
 * Do not run two calls that modify the same project directory at the same time
 * (init / new); concurrent apply calls are serialised by a database lock.
 * Additive extension within ABI 1; no earlier signature changed. */

/* options: {"dialect":"postgres"}                     -> {root, dialect} */
char *certo_migrate_init(const char *project_dir, const char *options);

/* options: {"name", "mdl":{"label","source"}?, "allow_destructive"?}
 *   -> {seq, name, dir, summary[], destructive, warnings}
 * Errors: compile (positioned diagnostics), no_changes, destructive,
 *         unsupported {op, reason}, project, io. */
char *certo_migrate_new(const char *project_dir, const char *options);

/* options: reserved (pass NULL). No database needed.
 *   -> {migrations:[{seq,name,label,dir,checksum,batches,statements}]} */
char *certo_migrate_list(const char *project_dir, const char *options);

/* options: {"url":"postgres://..." or, for a sqlite project, a database file path}                   -> {applied[], pending[]}
 * Errors include history_drift (a migration was edited/removed/renamed). */
char *certo_migrate_status(const char *project_dir, const char *options);

/* options: {"url", "dry_run"?, "to"?, "check_drift"?, "journal"?: {"actor", "environment"?, "tool"}}
 *   -> {dry_run, migrations[], scripts[]}
 * With "journal", each applied migration also adds a row to the append-only table _certo_log (action, subject,
 * actor, environment, tool, detail, at), INSIDE the transaction that makes the change: the journal holds exactly the
 * changes that committed. The table is created on first use and is not part of the schema (no drift, not imported).
 * Failure: error.code "database" with error.applied[] = migrations that had
 * succeeded before the failing one (they stay applied). "schema_drift" if
 * check_drift found the database changed by hand (error.items[]). */
char *certo_migrate_apply(const char *project_dir, const char *options);

/* options: {"url", "repair_sql"?}
 *   -> {in_sync, expected_from, items:[{kind:"missing|unexpected|different",text}],
 *       notes[], repair_sql|null, repair_error|null}
 * "ok" is true even when drift is found: the call worked. */
char *certo_migrate_drift(const char *project_dir, const char *options);

/* options: {"url", "dry_run"?, "force"?, "journal"?: {"actor", "environment"?, "tool"}}
 *   -> {dry_run, schema_sdl, adopted:{tables,columns,enums,types,sequences,indexes,constraints},
 *       omissions[], migration|null, recovered|null, known_drift:[{kind,text}]}
 * Turns an existing database into schema.sdl plus a baseline migration that is
 * RECORDED as applied but not run (the baseline holds real CREATE statements, so
 * an empty database can be rebuilt from the history). `omissions` lists what SDL
 * cannot express and was left out; `known_drift` what therefore still differs.
 * Only for a fresh project and an unmanaged database. While it works the project holds a `.certo-adopt`
 * marker; an adopt that was killed part-way is found by the next one, which undoes the partial work and redoes
 * it (or, if the baseline had already been recorded, only clears the marker); `recovered` says which. */
char *certo_migrate_adopt(const char *project_dir, const char *options);

/* options: {"url", "dialect"?: "postgres"|"sqlite"}  (dialect defaults from the url)
 *   -> {schema_sdl, imported:{tables,columns,enums,types,sequences,indexes,constraints}, omissions[]}
 * Reads a database's schema as SDL with no project, changing nothing (a missing SQLite file is an error,
 * not an empty database). `omissions` lists what SDL cannot express and was left out. */
char *certo_schema_import(const char *options);

/* Release a string returned by this library. NULL is ignored. */
void certo_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* CERTO_H */
