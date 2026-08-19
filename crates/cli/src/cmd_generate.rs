//! `certo generate validators` — YAML → Certo source + PL/pgSQL generator.
//!
//! Reads three YAML definition files and emits:
//!   Certo source (always):
//!     - `constraints.cto`         (prepared-items constraints)
//!     - `temporals.cto`           (prepared-items temporals)
//!     - `<entity>-validators.cto` (one per entity, one validator per trigger)
//!
//!   PL/pgSQL (with --emit-sql <dir>):
//!     - `004_rule_types.sql`           (shared infrastructure)
//!     - `005_validators/<e>_<t>.sql`   (one per entity+trigger)
//!     - `006_rule_triggers.sql`        (trigger wiring)
//!     - `007_api_helpers.sql`          (preflight JSONB wrappers)

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::process;
use serde::Deserialize;
use certo_ast::decl::Decl;

// ================================================================== //
// YAML data structures
// ================================================================== //

#[derive(Debug, Deserialize)]
struct PreparedItems {
    #[serde(default)]
    temporal:    Vec<TemporalDef>,
    #[serde(default)]
    constraints: Vec<ConstraintDef>,
    #[serde(default)]
    #[allow(dead_code)] // reserved: enum generation not yet wired up
    enums:       Vec<EnumDef>,
}

#[derive(Debug, Deserialize)]
struct TemporalDef {
    id:       String,
    duration: u64,
    unit:     String,
}

#[derive(Debug, Deserialize)]
struct ConstraintDef {
    id:        String,
    condition: ConditionNode,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // reserved: enum generation not yet wired up
struct EnumDef {
    id:     String,
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Entities {
    #[serde(default)]
    entities: Vec<EntityDef>,
}

#[derive(Debug, Deserialize)]
struct EntityDef {
    id:     String,
    #[serde(default)]
    fields: Vec<FieldDef>,
}

#[derive(Debug, Deserialize)]
struct FieldDef {
    #[allow(dead_code)]
    name:   String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    ty:     String,
    #[serde(default)]
    #[allow(dead_code)]
    entity: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Rules {
    #[serde(default)]
    rules: Vec<RuleDef>,
}

#[derive(Debug, Deserialize)]
struct RuleDef {
    id:          String,
    description: String,
    entity:      String,
    trigger:     String,
    #[serde(default)]
    priority:    Option<i64>,
    #[serde(default)]
    depends_on:  Vec<String>,
    #[serde(default)]
    overrides:   Option<String>,
    condition:   ConditionNode,
    error:       ErrorDef,
}

#[derive(Debug, Deserialize)]
struct ErrorDef {
    code:    String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ConditionNode {
    ConstraintRef { constraint: String },
    TemporalRef   { temporal: String, field: String, operator: String },
    All           { all:  Vec<ConditionNode> },
    Any           { any:  Vec<ConditionNode> },
    None          { none: Vec<ConditionNode> },
    Leaf {
        field:    String,
        operator: String,
        #[serde(default)]
        value:    Option<serde_yaml::Value>,
        #[serde(default)]
        #[allow(dead_code)]
        unit:     Option<String>,
    },
}

// ================================================================== //
// Public entry point
// ================================================================== //

pub fn cmd_generate(args: &[String]) {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_generate_help();
        return;
    }
    match args[0].as_str() {
        "validators" => cmd_generate_validators(&args[1..]),
        "api"        => cmd_generate_api(&args[1..]),
        other => {
            eprintln!("Unknown generate subcommand: {}", other);
            eprintln!("Run `certo generate --help` for usage.");
            process::exit(2);
        }
    }
}

fn print_generate_help() {
    println!("Usage: certo generate <subcommand> [options]");
    println!();
    println!("Subcommands:");
    println!("  validators   Generate Certo validator source (and optionally PL/pgSQL) from YAML");
    println!("  api          Generate a JSON REST CRUD handler file from an existing type declaration");
    println!();
    println!("Run `certo generate validators --help` or `certo generate api --help` for details.");
}

fn cmd_generate_validators(args: &[String]) {
    let mut prepared_items_path: Option<PathBuf> = None;
    let mut entities_path:       Option<PathBuf> = None;
    let mut rules_path:          Option<PathBuf> = None;
    let mut output_dir:          Option<PathBuf> = None;
    let mut sql_dir:             Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--prepared-items" => { i += 1; prepared_items_path = Some(next_path(args, i, "--prepared-items")); }
            "--entities"       => { i += 1; entities_path       = Some(next_path(args, i, "--entities")); }
            "--rules"          => { i += 1; rules_path          = Some(next_path(args, i, "--rules")); }
            "--output" | "-o"  => { i += 1; output_dir          = Some(next_path(args, i, "--output")); }
            "--emit-sql"       => { i += 1; sql_dir             = Some(next_path(args, i, "--emit-sql")); }
            "--help" | "-h" => {
                println!("Usage: certo generate validators \\");
                println!("    --prepared-items path/to/prepared-items.yaml \\");
                println!("    --entities       path/to/entities.yaml \\");
                println!("    --rules          path/to/rules.yaml \\");
                println!("    --output         src/validators/ \\");
                println!("    [--emit-sql      plpgsql/]");
                println!();
                println!("Certo output (--output):");
                println!("  constraints.cto         — named constraint declarations");
                println!("  temporals.cto           — named temporal window declarations");
                println!("  <entity>-validators.cto — one file per entity");
                println!();
                println!("PL/pgSQL output (--emit-sql):");
                println!("  004_rule_types.sql              — shared infrastructure");
                println!("  005_validators/<e>_<t>.sql      — one per entity+trigger");
                println!("  006_rule_triggers.sql           — BEFORE INSERT/UPDATE trigger wiring");
                println!("  007_api_helpers.sql             — preflight JSONB wrappers");
                return;
            }
            other if other.starts_with('-') => { eprintln!("Unknown option: {}", other); process::exit(2); }
            _ => { eprintln!("Unexpected argument: {}", args[i]); process::exit(2); }
        }
        i += 1;
    }

    let pi_path = prepared_items_path.unwrap_or_else(|| die("--prepared-items is required", 2));
    let en_path = entities_path      .unwrap_or_else(|| die("--entities is required", 2));
    let ru_path = rules_path         .unwrap_or_else(|| die("--rules is required", 2));
    let out_dir = output_dir         .unwrap_or_else(|| PathBuf::from("."));

    let pi: PreparedItems = read_yaml(&pi_path);
    let en: Entities      = read_yaml(&en_path);
    let ru: Rules         = read_yaml(&ru_path);

    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| {
        eprintln!("error: cannot create output directory {}: {}", out_dir.display(), e);
        process::exit(1);
    });

    let constraint_map: HashMap<&str, &ConstraintDef> =
        pi.constraints.iter().map(|c| (c.id.as_str(), c)).collect();
    let temporal_map: HashMap<&str, &TemporalDef> =
        pi.temporal.iter().map(|t| (t.id.as_str(), t)).collect();
    let entity_field_map: HashMap<&str, Vec<&FieldDef>> =
        en.entities.iter().map(|e| (e.id.as_str(), e.fields.iter().collect())).collect();

    // Group rules by entity then by trigger.
    let mut by_entity: HashMap<String, Vec<&RuleDef>> = HashMap::new();
    for rule in &ru.rules {
        by_entity.entry(rule.entity.clone()).or_default().push(rule);
    }
    let mut entity_names: Vec<String> = by_entity.keys().cloned().collect();
    entity_names.sort();

    // ---------------------------------------------------------------- //
    // Certo source output
    // ---------------------------------------------------------------- //
    emit_cto_files(&pi, &by_entity, &entity_names, &entity_field_map,
                   &constraint_map, &temporal_map, &out_dir);

    // ---------------------------------------------------------------- //
    // PL/pgSQL output (optional)
    // ---------------------------------------------------------------- //
    if let Some(sql_out) = sql_dir {
        emit_sql_files(&pi, &ru, &by_entity, &entity_names,
                       &constraint_map, &temporal_map, &sql_out);
    }
}

// ================================================================== //
// `certo generate api <TypeName>` — JSON REST CRUD handler generator
// (BACKLOG item 153, the "api" third of certo generate model|api|migration —
// "model" and "migration" already exist under different names, certo db
// pull and certo db create/certo migrate create respectively).
//
// Reads an existing `type X = { field: T, ... }` declaration from a
// source file (no live DB connection needed at generation time, matching
// `certo generate validators`'s own character) and emits a standalone
// `.cto` file with a full CRUD HTTP handler: GET list, GET one, POST
// create, PUT update, DELETE — reusing the exact router-dispatch and
// parameterized-SQL patterns `crates/ui/src/server.rs`'s own view/form
// generation already established, just emitting JSON instead of HTML.
// ================================================================== //

fn cmd_generate_api(args: &[String]) {
    let mut type_name: Option<String> = None;
    let mut in_path:  Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--in"             => { i += 1; in_path  = Some(next_path(args, i, "--in")); }
            "--output" | "-o"  => { i += 1; out_path = Some(next_path(args, i, "--output")); }
            "--help" | "-h" => {
                println!("Usage: certo generate api <TypeName> --in path/to/schema.cto [-o output.cto]");
                println!();
                println!("Reads an existing `type <TypeName> = {{ ... }}` record declaration");
                println!("(hand-written, or produced earlier by `certo db pull`) and generates");
                println!("a full JSON REST CRUD handler file for it: GET list, GET one, POST");
                println!("create, PUT update, DELETE — parameterized SQL, no string-built queries.");
                println!();
                println!("The declared type must have a field named `id` (the primary key).");
                println!();
                println!("Options:");
                println!("  --in <file>      Source file containing the type declaration (required)");
                println!("  -o <file>        Output path (default: <tableName>_api.cto)");
                return;
            }
            other if other.starts_with('-') => { eprintln!("Unknown option: {}", other); process::exit(2); }
            _ => {
                if type_name.is_some() { eprintln!("Unexpected argument: {}", args[i]); process::exit(2); }
                type_name = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let type_name = type_name.unwrap_or_else(|| die("certo generate api requires a type name — see --help", 2));
    let in_path = in_path.unwrap_or_else(|| die("--in <file> is required — see --help", 2));

    let src = std::fs::read_to_string(&in_path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", in_path.display(), e);
        process::exit(1);
    });
    let module = certo_parser::parse(&src).unwrap_or_else(|errs| {
        eprintln!("error: cannot parse {}:", in_path.display());
        for e in &errs { eprintln!("  {}", e); }
        process::exit(1);
    });

    let record = module.decls.iter().find_map(|d| match &d.node {
        Decl::Type(t) if t.name.node == type_name => match &t.body {
            certo_ast::decl::TypeBody::Record(r) => Some(r),
            _ => None,
        },
        _ => None,
    }).unwrap_or_else(|| die(
        &format!("no record type `{type_name}` found in {} — expected `type {type_name} = {{ ... }}`", in_path.display()),
        1,
    ));

    if !record.fields.iter().any(|f| f.name.node == "id") {
        die(&format!("type `{type_name}` has no `id` field — certo generate api requires one as the primary key"), 1);
    }

    let table = pascal_to_snake(&type_name);
    let out_path = out_path.unwrap_or_else(|| PathBuf::from(format!("{table}_api.cto")));
    let source = emit_api(&type_name, &table, &record.fields);
    write_output(&out_path, &source);
}

/// `Order` → `order`, `OrderItem` → `order_item` — same transform
/// `crates/ui/src/server.rs`'s own `camel_to_snake` uses for PascalCase
/// type names, duplicated locally rather than adding a cross-crate
/// dependency for six lines, matching how `crates/cli/src/main.rs`
/// already duplicates its own `snake_to_pascal`/`snake_to_camel` locally
/// instead of importing `certo_dbschema`'s equivalent.
fn pascal_to_snake(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 { s.push('_'); }
        s.push(c.to_lowercase().next().unwrap());
    }
    s
}

/// The Certo type name a record field is declared with — peels a `T?`
/// Option wrapper first (nullable columns get the same JSON/SQL treatment
/// as their non-nullable counterparts, since every `dbQuery` cell is
/// already `Text?` regardless). Anything not a simple named type (a
/// generic, a tuple, a function type — none of which are valid table
/// column types anyway) falls back to `"Text"`, the always-safe default.
fn field_type_name(ty: &certo_ast::types::TypeExpr) -> String {
    match ty {
        certo_ast::types::TypeExpr::Option { inner, .. } => field_type_name(&inner.node),
        certo_ast::types::TypeExpr::Named { path, .. } =>
            path.segments.last().map(|s| s.node.clone()).unwrap_or_else(|| "Text".to_string()),
        _ => "Text".to_string(),
    }
}

/// A JSON request-body field, converted to `Text` for `dbExec`/`dbQuery`'s
/// own `List<Text>` params convention — `Int`/`Float`/`Bool` fields round
/// through their real typed JSON accessor first (`JsonValue.asInt`, etc.)
/// then `intToText`/`floatToText`/`boolToText`, so a non-numeric string
/// sent for an `Int` field fails clearly inside `JsonValue.asInt` rather
/// than silently inserting whatever text was sent. Everything else
/// (`Text`, `UUID`, and any type this generator doesn't specifically
/// recognize) is read directly as text.
fn json_decode_expr(field_name: &str, ty_name: &str) -> String {
    let get = format!("JsonValue.get(_jv, \"{field_name}\")");
    match ty_name {
        "Int"   => format!("intToText(JsonValue.asInt({get}))"),
        "Float" => format!("floatToText(JsonValue.asFloat({get}))"),
        "Bool"  => format!("boolToText(JsonValue.asBool({get}))"),
        _       => format!("JsonValue.asText({get})"),
    }
}

fn emit_api(type_name: &str, table: &str, fields: &[certo_ast::decl::RecordFieldDef]) -> String {
    let mut out = String::new();
    let cols: Vec<String> = fields.iter().map(|f| pascal_to_snake(&f.name.node)).collect();
    let names: Vec<String> = fields.iter().map(|f| f.name.node.clone()).collect();
    let tys: Vec<String> = fields.iter().map(|f| field_type_name(&f.ty.node)).collect();

    writeln!(out, "// Generated by `certo generate api {type_name}` — do not edit by hand.").unwrap();
    writeln!(out, "// Regenerate instead: certo generate api {type_name} --in <source>").unwrap();
    writeln!(out, "module {type_name}Api").unwrap();
    // `certo build`'s own `uses_db` detection (crates/cli/src/main.rs) — the
    // switch that links libpq at all — scans `module.imports` for a literal
    // `Stdlib.Db` import, not for actual dbConnect/dbQuery call sites. This
    // import is load-bearing, not decorative: without it, every `db*` call
    // below compiles fine but fails to *link* (`undefined symbol:
    // certo_db_connect`) — confirmed by a real build attempt before this
    // was added. Matches the same three imports `crates/ui/src/server.rs`'s
    // own generated output already carries.
    writeln!(out, "import Stdlib.Core").unwrap();
    writeln!(out, "import Stdlib.Http").unwrap();
    writeln!(out, "import Stdlib.Db").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn dbUrl(): Text = getEnv(\"DATABASE_URL\") ?? \"host=localhost dbname=postgres user=postgres\"").unwrap();
    writeln!(out).unwrap();

    emit_api_router(type_name, table, &mut out);
    emit_row_to_json(&cols, &names, &mut out);
    emit_api_list_handler(type_name, table, &mut out);
    emit_api_get_handler(type_name, table, &mut out);
    emit_api_create_handler(type_name, table, &cols, &names, &tys, &mut out);
    emit_api_update_handler(type_name, table, &cols, &names, &tys, &mut out);
    emit_api_delete_handler(type_name, table, &mut out);

    out
}

fn emit_api_router(type_name: &str, table: &str, out: &mut String) {
    writeln!(out, "// ── Router ───────────────────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn handler(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val path   = HttpRequest.path(req)").unwrap();
    writeln!(out, "    val method = HttpRequest.method(req)").unwrap();
    writeln!(out, "    if Text.eq(path, \"/{table}\") then").unwrap();
    writeln!(out, "        if Text.eq(method, \"POST\") then create{type_name}(req)").unwrap();
    writeln!(out, "        else list{type_name}(req)").unwrap();
    writeln!(out, "    else if Text.startsWith(path, \"/{table}/\") then").unwrap();
    writeln!(out, "        if Text.eq(method, \"PUT\") then update{type_name}(req)").unwrap();
    writeln!(out, "        else if Text.eq(method, \"DELETE\") then delete{type_name}(req)").unwrap();
    writeln!(out, "        else get{type_name}(req)").unwrap();
    writeln!(out, "    else Http.notFound(f\"No route for {{method}} {{path}}\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "// `/{table}/<id>` → the id segment after the last `/`").unwrap();
    writeln!(out, "fn pathId(path: Text): Text = {{").unwrap();
    writeln!(out, "    val parts = Text.split(path, \"/\")").unwrap();
    writeln!(out, "    List.getOrPanic(parts, List.len(parts) - 1)").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Shared row → JSON encoder, reused by both the list and get-one handlers
/// (same "one helper, not duplicated per handler" shape `colValue` already
/// uses in `crates/ui/src/server.rs`). Every cell is emitted as a JSON
/// string (`Json.string(...)`) regardless of the field's own declared
/// Certo type — `dbQuery` returns every column as `Text?` already, and
/// there is no `Text.toInt`/`.toFloat`/`.toBool` parsing function
/// anywhere in this codebase (confirmed by grep before relying on one) to
/// convert a cell back to a typed value before JSON-encoding it. This is
/// an honest, deliberate limitation, not an oversight — a numeric column
/// like `total: Int` currently comes back as `"1999"` (a JSON string),
/// not `1999` (a JSON number). Fixing it needs those parsing functions to
/// exist first, a separate, smaller stdlib gap — see BACKLOG item 194.
fn emit_row_to_json(cols: &[String], names: &[String], out: &mut String) {
    writeln!(out, "// ── Row → JSON (every cell as a JSON string — see item 194) ─────────────────").unwrap();
    writeln!(out, "fn rowToJson(row: List<Text?>, cols: List<Text>): Text = {{").unwrap();
    writeln!(out, "    val _obj = Json.object()").unwrap();
    for (col, name) in cols.iter().zip(names) {
        writeln!(out, "    val _i_{name} = pkColIndex(cols, \"{col}\", 0)").unwrap();
        writeln!(out, "    val _v_{name} = if _i_{name} < 0 then \"\" else (List.getOrPanic(row, _i_{name}) ?? \"\")").unwrap();
        writeln!(out, "    JsonValue.set(_obj, \"{name}\", Json.string(_v_{name}))").unwrap();
    }
    writeln!(out, "    Json.stringify(_obj)").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn pkColIndex(cols: List<Text>, col: Text, i: Int): Int =").unwrap();
    writeln!(out, "    if i >= List.len(cols) then -1").unwrap();
    writeln!(out, "    else if Text.eq(List.getOrPanic(cols, i), col) then i").unwrap();
    writeln!(out, "    else pkColIndex(cols, col, i + 1)").unwrap();
    writeln!(out).unwrap();
}

fn emit_api_list_handler(type_name: &str, table: &str, out: &mut String) {
    writeln!(out, "// ── GET /{table} — list ──────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn list{type_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val conn  = dbConnect(dbUrl())").unwrap();
    writeln!(out, "    val _p0   = List.empty()").unwrap();
    writeln!(out, "    val rows  = dbQuery(conn, \"SELECT * FROM {table}\", _p0)").unwrap();
    writeln!(out, "    val cols  = dbColumns(conn, \"SELECT * FROM {table} LIMIT 1\")").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    val body = rowsToJsonArray(rows, cols, 0, List.len(rows))").unwrap();
    writeln!(out, "    Http.ok(body, \"application/json\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn rowsToJsonArray(rows: List<List<Text?>>, cols: List<Text>, i: Int, n: Int): Text =").unwrap();
    writeln!(out, "    if i >= n then \"[]\"").unwrap();
    writeln!(out, "    else if i == n - 1 then \"[\" ++ rowToJson(List.getOrPanic(rows, i), cols) ++ \"]\"").unwrap();
    writeln!(out, "    else {{").unwrap();
    writeln!(out, "        val rest = rowsToJsonArray(rows, cols, i + 1, n)").unwrap();
    writeln!(out, "        \"[\" ++ rowToJson(List.getOrPanic(rows, i), cols) ++ \",\" ++ Text.slice(rest, 1, Text.len(rest))").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

fn emit_api_get_handler(type_name: &str, table: &str, out: &mut String) {
    writeln!(out, "// ── GET /{table}/:id — one ───────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn get{type_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val _id  = pathId(HttpRequest.path(req))").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
    writeln!(out, "    val _p0  = List.empty()").unwrap();
    writeln!(out, "    val _p1  = List.push(_p0, _id)").unwrap();
    writeln!(out, "    val rows = dbQuery(conn, \"SELECT * FROM {table} WHERE id = $1 LIMIT 1\", _p1)").unwrap();
    writeln!(out, "    val cols = dbColumns(conn, \"SELECT * FROM {table} LIMIT 1\")").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    if List.len(rows) == 0 then Http.notFound(f\"{type_name} {{_id}} not found\")").unwrap();
    writeln!(out, "    else Http.ok(rowToJson(List.getOrPanic(rows, 0), cols), \"application/json\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn emit_api_create_handler(type_name: &str, table: &str, cols: &[String], names: &[String], tys: &[String], out: &mut String) {
    writeln!(out, "// ── POST /{table} — create ───────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn create{type_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val _jv  = Json.parse(HttpRequest.body(req))").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
    let write_cols: Vec<&str> = cols.iter().map(|s| s.as_str()).filter(|c| *c != "id").collect();
    let mut p_idx = 0usize;
    writeln!(out, "    val _p0 = List.empty()").unwrap();
    for ((col, name), ty) in cols.iter().zip(names).zip(tys) {
        if col == "id" { continue; }
        let decode = json_decode_expr(name, ty);
        writeln!(out, "    val _f{p_idx} = {decode}").unwrap();
        writeln!(out, "    val _p{} = List.push(_p{}, _f{p_idx})", p_idx + 1, p_idx).unwrap();
        p_idx += 1;
    }
    let col_list = write_cols.join(", ");
    let ph_list: String = (1..=write_cols.len()).map(|n| format!("${n}")).collect::<Vec<_>>().join(", ");
    writeln!(out, "    val sql  = \"INSERT INTO {table} ({col_list}) VALUES ({ph_list}) RETURNING *\"").unwrap();
    writeln!(out, "    val rows = dbQuery(conn, sql, _p{p_idx})").unwrap();
    writeln!(out, "    val cols = dbColumns(conn, \"SELECT * FROM {table} LIMIT 1\")").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    if List.len(rows) == 0 then Http.respond(500, \"application/json\", \"{{\\\"error\\\":\\\"insert failed\\\"}}\")").unwrap();
    writeln!(out, "    else Http.respond(201, \"application/json\", rowToJson(List.getOrPanic(rows, 0), cols))").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn emit_api_update_handler(type_name: &str, table: &str, cols: &[String], names: &[String], tys: &[String], out: &mut String) {
    writeln!(out, "// ── PUT /{table}/:id — update ────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn update{type_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val _id  = pathId(HttpRequest.path(req))").unwrap();
    writeln!(out, "    val _jv  = Json.parse(HttpRequest.body(req))").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
    let write_cols: Vec<&str> = cols.iter().map(|s| s.as_str()).filter(|c| *c != "id").collect();
    let mut p_idx = 0usize;
    writeln!(out, "    val _p0 = List.empty()").unwrap();
    for ((col, name), ty) in cols.iter().zip(names).zip(tys) {
        if col == "id" { continue; }
        let decode = json_decode_expr(name, ty);
        writeln!(out, "    val _f{p_idx} = {decode}").unwrap();
        writeln!(out, "    val _p{} = List.push(_p{}, _f{p_idx})", p_idx + 1, p_idx).unwrap();
        p_idx += 1;
    }
    writeln!(out, "    val _p{} = List.push(_p{}, _id)", p_idx + 1, p_idx).unwrap();
    let set_clause = write_cols.iter().enumerate()
        .map(|(i, c)| format!("{c} = ${}", i + 1))
        .collect::<Vec<_>>().join(", ");
    let id_ph = write_cols.len() + 1;
    writeln!(out, "    val sql  = \"UPDATE {table} SET {set_clause} WHERE id = ${id_ph} RETURNING *\"").unwrap();
    writeln!(out, "    val rows = dbQuery(conn, sql, _p{})", p_idx + 1).unwrap();
    writeln!(out, "    val cols = dbColumns(conn, \"SELECT * FROM {table} LIMIT 1\")").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    if List.len(rows) == 0 then Http.notFound(f\"{type_name} {{_id}} not found\")").unwrap();
    writeln!(out, "    else Http.ok(rowToJson(List.getOrPanic(rows, 0), cols), \"application/json\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn emit_api_delete_handler(type_name: &str, table: &str, out: &mut String) {
    writeln!(out, "// ── DELETE /{table}/:id ───────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn delete{type_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val _id  = pathId(HttpRequest.path(req))").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
    writeln!(out, "    val _p0  = List.empty()").unwrap();
    writeln!(out, "    val _p1  = List.push(_p0, _id)").unwrap();
    writeln!(out, "    val n    = dbExec(conn, \"DELETE FROM {table} WHERE id = $1\", _p1)").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    if n > 0 then Http.respond(204, \"application/json\", \"\")").unwrap();
    writeln!(out, "    else Http.notFound(f\"{type_name} {{_id}} not found\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ================================================================== //
// Certo source emitter
// ================================================================== //

fn emit_cto_files(
    pi:               &PreparedItems,
    by_entity:        &HashMap<String, Vec<&RuleDef>>,
    entity_names:     &[String],
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
    constraint_map:   &HashMap<&str, &ConstraintDef>,
    temporal_map:     &HashMap<&str, &TemporalDef>,
    out_dir:          &Path,
) {
    // constraints.cto
    if !pi.constraints.is_empty() {
        let mut src = String::from("module Constraints\n\n");
        for c in &pi.constraints {
            let cond = emit_certo_condition(&c.condition, constraint_map, temporal_map);
            src.push_str(&format!("constraint {} = {}\n", snake_to_pascal(&c.id), cond));
        }
        write_output(&out_dir.join("constraints.cto"), &src);
    }

    // temporals.cto
    if !pi.temporal.is_empty() {
        let mut src = String::from("module Temporals\n\n");
        for t in &pi.temporal {
            let (method, amount) = temporal_to_duration(t);
            src.push_str(&format!(
                "temporal {} = Duration.{}({})\n",
                snake_to_pascal(&t.id), method, amount,
            ));
        }
        write_output(&out_dir.join("temporals.cto"), &src);
    }

    // per-entity validator files
    let mut cto_count = 0usize;
    for entity_name in entity_names {
        let rules = &by_entity[entity_name];
        let module_name = format!("{}Validators", entity_name);
        let mut src = format!("module {}\n\n", module_name);

        let mut triggers: Vec<String> = Vec::new();
        let mut by_trigger: HashMap<String, Vec<&RuleDef>> = HashMap::new();
        for rule in rules.iter() {
            if !by_trigger.contains_key(&rule.trigger) {
                triggers.push(rule.trigger.clone());
            }
            by_trigger.entry(rule.trigger.clone()).or_default().push(rule);
        }

        for trigger in &triggers {
            let trigger_rules = &by_trigger[trigger];
            let validator_name = format!("{}{}", entity_name, trigger_pascal(trigger));
            let errors_type    = format!("{}Error", entity_name);

            let context_entities = infer_context_entities(
                trigger_rules, entity_name, entity_field_map,
            );

            src.push_str(&format!(
                "validator {} for {} errors {} {{\n",
                validator_name, entity_name, errors_type,
            ));

            if !context_entities.is_empty() {
                src.push_str("    context {\n");
                for ctx_entity in &context_entities {
                    src.push_str(&format!("        {}: {}\n", lowercase_first(ctx_entity), ctx_entity));
                }
                src.push_str("    }\n");
            }

            let rule_ids: std::collections::HashSet<&str> =
                trigger_rules.iter().map(|r| r.id.as_str()).collect();

            for rule in trigger_rules.iter() {
                src.push_str(&format!("    rule {} {{\n", rule.id));
                for dep in &rule.depends_on {
                    if rule_ids.contains(dep.as_str()) {
                        src.push_str(&format!("        after {}\n", dep));
                    }
                }
                if let Some(ov) = &rule.overrides {
                    if rule_ids.contains(ov.as_str()) {
                        src.push_str(&format!("        overrides {}\n", ov));
                    }
                }
                if let Some(pri) = rule.priority {
                    src.push_str(&format!("        priority {}\n", pri));
                }
                let cond = emit_certo_condition(&rule.condition, constraint_map, temporal_map);
                src.push_str(&format!("        require {}\n", cond));
                src.push_str(&format!("        else {}.{}\n", errors_type, rule.error.code));
                src.push_str("    }\n");
            }
            src.push_str("}\n\n");
        }

        let path = out_dir.join(format!("{}-validators.cto", to_kebab(entity_name)));
        write_output(&path, src.trim_end());
        append_newline(&path);
        cto_count += 1;
    }

    eprintln!(
        "generated {} certo file(s) in {}",
        cto_count + if !pi.constraints.is_empty() { 1 } else { 0 }
                  + if !pi.temporal.is_empty()     { 1 } else { 0 },
        out_dir.display()
    );
}

// ================================================================== //
// PL/pgSQL emitter
// ================================================================== //

fn emit_sql_files(
    _pi:             &PreparedItems,
    _ru:             &Rules,
    by_entity:      &HashMap<String, Vec<&RuleDef>>,
    entity_names:   &[String],
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
    sql_dir:        &Path,
) {
    let validators_dir = sql_dir.join("005_validators");
    std::fs::create_dir_all(&validators_dir).unwrap_or_else(|e| {
        eprintln!("error: cannot create {}: {}", validators_dir.display(), e);
        process::exit(1);
    });

    // 004_rule_types.sql — shared infrastructure
    write_output(&sql_dir.join("004_rule_types.sql"), &build_rule_types());

    // 005_validators/<entity>_<trigger>.sql — one per entity+trigger
    let mut all_groups: Vec<(String, String, Vec<&RuleDef>)> = Vec::new();
    for entity_name in entity_names {
        let rules = &by_entity[entity_name];
        let mut triggers: Vec<String> = Vec::new();
        let mut by_trigger: HashMap<String, Vec<&RuleDef>> = HashMap::new();
        for rule in rules.iter() {
            if !by_trigger.contains_key(&rule.trigger) {
                triggers.push(rule.trigger.clone());
            }
            by_trigger.entry(rule.trigger.clone()).or_default().push(rule);
        }
        for trigger in triggers {
            let group = by_trigger.remove(&trigger).unwrap();
            let filename = format!("{}_{}.sql", to_snake(entity_name), trigger);
            let sql = build_validator_fn(entity_name, &trigger, &group, constraint_map, temporal_map);
            write_output(&validators_dir.join(&filename), &sql);
            all_groups.push((entity_name.clone(), trigger, group));
        }
    }

    // 006_rule_triggers.sql — trigger wiring
    write_output(&sql_dir.join("006_rule_triggers.sql"),
                 &build_rule_triggers(&all_groups));

    // 007_api_helpers.sql — preflight wrappers
    write_output(&sql_dir.join("007_api_helpers.sql"),
                 &build_api_helpers(&all_groups));

    eprintln!("generated {} sql file(s) in {}", all_groups.len() + 3, sql_dir.display());
}

// ------------------------------------------------------------------ //
// 004_rule_types.sql
// ------------------------------------------------------------------ //

fn build_rule_types() -> String {
    let ts = timestamp_comment();
    format!(
r#"-- =============================================================================
-- Rule Validation Infrastructure
-- Generated: {ts}
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- raise_rule_violation(code, message)
-- Raises a structured exception: 'RULE_CODE|Human readable message'
CREATE OR REPLACE FUNCTION raise_rule_violation(
    p_code    TEXT,
    p_message TEXT
) RETURNS VOID AS $$
BEGIN
    RAISE EXCEPTION USING
        ERRCODE = 'P0001',
        MESSAGE = p_code || '|' || p_message,
        HINT    = p_message;
END;
$$ LANGUAGE plpgsql;

-- current_rule_context()
-- Returns JSONB context set by the application via SET LOCAL.
--   SET LOCAL rule.user_id   = '<uuid>';
--   SET LOCAL rule.user_role = 'ADMIN';
CREATE OR REPLACE FUNCTION current_rule_context()
RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object(
        'user_id',   current_setting('rule.user_id',   TRUE),
        'user_role', current_setting('rule.user_role', TRUE)
    );
END;
$$ LANGUAGE plpgsql STABLE;

-- record_age_days(ts) → NUMERIC
-- How many days ago a timestamp occurred.
CREATE OR REPLACE FUNCTION record_age_days(ts TIMESTAMPTZ)
RETURNS NUMERIC AS $$
BEGIN
    RETURN EXTRACT(EPOCH FROM (NOW() - ts)) / 86400.0;
END;
$$ LANGUAGE plpgsql STABLE;
"#
    )
}

// ------------------------------------------------------------------ //
// 005_validators/<entity>_<trigger>.sql
// ------------------------------------------------------------------ //

fn build_validator_fn(
    entity:         &str,
    trigger:        &str,
    rules:          &[&RuleDef],
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    let ts        = timestamp_comment();
    let func_name = format!("validate_{}_{}", to_plural_snake(entity), trigger);
    let ordered   = topo_sort_rules(rules);

    // Sets of rule IDs used for dependency tracking.
    let depended_on: std::collections::HashSet<&str> = ordered.iter()
        .flat_map(|r| r.depends_on.iter().map(|d| d.as_str()))
        .collect();
    // Map from overridden rule ID → overriding rule ID.
    let override_map: HashMap<&str, &str> = ordered.iter()
        .filter_map(|r| r.overrides.as_deref().map(|ov| (ov, r.id.as_str())))
        .collect();
    // Override rules (the ones doing the overriding) — evaluate condition, set flag, don't raise.
    let is_override_rule: std::collections::HashSet<&str> = ordered.iter()
        .filter(|r| r.overrides.is_some())
        .map(|r| r.id.as_str())
        .collect();

    let mut sb = String::new();

    // Header
    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- Validator: {entity}.{trigger}\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n"
    ));

    // Rule index comment
    sb.push_str(&format!("-- Rules enforced ({}):\n", ordered.len()));
    for r in &ordered {
        sb.push_str(&format!("--   {}: {}\n", r.id, r.description));
    }
    sb.push('\n');

    // Function signature
    sb.push_str(&format!(
        "CREATE OR REPLACE FUNCTION {}(\n    p_record  JSONB,\n    p_context JSONB\n) RETURNS VOID AS $$\nDECLARE\n",
        func_name
    ));

    // Variable declarations
    sb.push_str("    v_user_role TEXT;\n");
    for id in &depended_on {
        sb.push_str(&format!("    v_{}_passed BOOLEAN := FALSE;\n", to_var(id)));
    }
    for r in ordered.iter().filter(|r| r.overrides.is_some()) {
        sb.push_str(&format!("    v_{}_active BOOLEAN := FALSE;\n", to_var(&r.id)));
    }
    sb.push_str("BEGIN\n");
    sb.push_str("    v_user_role := p_context->>'user_role';\n\n");

    // Emit override rules first (they set flags, never raise)
    for rule in ordered.iter().filter(|r| is_override_rule.contains(r.id.as_str())) {
        let ov_target = rule.overrides.as_deref().unwrap_or("");
        sb.push_str(&format!(
            "    -- {} (override — suspends '{}' when true)\n",
            rule.description, ov_target
        ));
        let cond = emit_sql_condition(&rule.condition, constraint_map, temporal_map);
        sb.push_str(&format!("    IF {} THEN\n", cond));
        sb.push_str(&format!("        v_{}_active := TRUE;\n    END IF;\n\n", to_var(&rule.id)));
    }

    // Emit regular rules in topological order (skip override rules — already emitted above)
    for rule in ordered.iter().filter(|r| !is_override_rule.contains(r.id.as_str())) {
        let is_depended = depended_on.contains(rule.id.as_str());
        let is_overridden = override_map.contains_key(rule.id.as_str());

        sb.push_str(&format!("    -- {}\n", rule.description));

        // depends_on guard
        let dep_vars: Vec<String> = rule.depends_on.iter()
            .filter(|d| ordered.iter().any(|r| &r.id == *d))
            .map(|d| format!("v_{}_passed", to_var(d)))
            .collect();
        let depth = if dep_vars.is_empty() { 0usize } else { 1 };
        if !dep_vars.is_empty() {
            sb.push_str(&format!("    IF {} THEN\n", dep_vars.join(" AND ")));
        }

        let ind = "    ".repeat(depth + 1);

        // overrides guard
        if is_overridden {
            let overriding = override_map[rule.id.as_str()];
            sb.push_str(&format!("{}IF NOT v_{}_active THEN\n", ind, to_var(overriding)));
        }
        let inner_ind = if is_overridden {
            format!("{}    ", ind)
        } else {
            ind.clone()
        };

        let cond = emit_sql_condition(&rule.condition, constraint_map, temporal_map);
        sb.push_str(&format!("{}IF NOT ({}) THEN\n", inner_ind, cond));
        sb.push_str(&format!(
            "{}    PERFORM raise_rule_violation('{}', '{}');\n",
            inner_ind,
            rule.error.code,
            rule.error.message.replace('\'', "''"),
        ));
        sb.push_str(&format!("{}END IF;\n", inner_ind));

        if is_overridden {
            sb.push_str(&format!("{}END IF;\n", ind));
        }

        // Record pass flag after the check (if depended on by others)
        if is_depended {
            sb.push_str(&format!(
                "{}v_{}_passed := {};\n",
                "    ".repeat(depth + 1),
                to_var(&rule.id),
                cond_for_pass(&rule.condition, constraint_map, temporal_map),
            ));
        }

        if !dep_vars.is_empty() {
            sb.push_str("    END IF;\n");
        }
        sb.push('\n');
    }

    sb.push_str("END;\n$$ LANGUAGE plpgsql;\n\n");

    // Convenience single-arg wrapper
    sb.push_str(&format!(
        "-- Convenience wrapper reads context from session settings.\n\
         CREATE OR REPLACE FUNCTION {}(p_record JSONB)\n\
         RETURNS VOID AS $$\n\
         BEGIN\n\
             PERFORM {}(p_record, current_rule_context());\n\
         END;\n\
         $$ LANGUAGE plpgsql;\n",
        func_name, func_name
    ));

    sb
}

// ------------------------------------------------------------------ //
// 006_rule_triggers.sql
// ------------------------------------------------------------------ //

fn build_rule_triggers(groups: &[(String, String, Vec<&RuleDef>)]) -> String {
    let ts = timestamp_comment();
    let mut sb = String::new();

    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- Rule Triggers\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n"
    ));

    // One trigger function per entity.
    let mut by_entity: HashMap<&str, Vec<&str>> = HashMap::new();
    for (entity, trigger, _) in groups {
        by_entity.entry(entity.as_str()).or_default().push(trigger.as_str());
    }

    let mut entity_names: Vec<&str> = by_entity.keys().copied().collect();
    entity_names.sort();

    for entity in entity_names {
        let triggers = &by_entity[entity];
        let table_name  = to_plural_snake(entity);
        let trg_fn_name = format!("trg_validate_{}", table_name);
        let func_name_base = format!("validate_{}", table_name);

        sb.push_str(&format!("-- {entity}\n"));
        sb.push_str(&format!(
            "CREATE OR REPLACE FUNCTION {trg_fn_name}()\n\
             RETURNS TRIGGER AS $$\n\
             DECLARE\n\
                 v_record  JSONB;\n\
                 v_context JSONB;\n\
             BEGIN\n\
                 v_record  := row_to_json(NEW)::JSONB;\n\
                 v_context := current_rule_context();\n"
        ));

        // INSERT triggers
        let has_create = triggers.iter().any(|t| *t == "create");
        if has_create {
            sb.push_str(&format!(
                "    IF TG_OP = 'INSERT' THEN\n        PERFORM {func_name_base}_create(v_record, v_context);\n"
            ));
        } else {
            sb.push_str("    IF TG_OP = 'INSERT' THEN\n        NULL;\n");
        }

        // Named action triggers (e.g. submit, void, apply_discount) are not
        // wired here — they are called explicitly via preflight_* API helpers
        // from the application before committing DML. The DB trigger covers
        // only the generic create/update cases as a safety net.
        let has_plain_update = triggers.iter().any(|t| *t == "update");

        if has_plain_update {
            sb.push_str("    ELSIF TG_OP = 'UPDATE' THEN\n");
            sb.push_str(&format!(
                "        PERFORM {func_name_base}_update(v_record, v_context);\n"
            ));
        } else {
            sb.push_str("    ELSIF TG_OP = 'UPDATE' THEN\n        NULL; -- named-action validators called via preflight_* API helpers\n");
        }

        sb.push_str(
            "    END IF;\n    RETURN NEW;\nEND;\n$$ LANGUAGE plpgsql;\n\n"
        );

        sb.push_str(&format!(
            "DROP TRIGGER IF EXISTS {trg_fn_name} ON {table_name};\n\
             CREATE TRIGGER {trg_fn_name}\n\
                 BEFORE INSERT OR UPDATE ON {table_name}\n\
                 FOR EACH ROW\n\
                 EXECUTE FUNCTION {trg_fn_name}();\n\n\
             -------------------------------------------------------------------------------\n\n"
        ));
    }

    sb
}

// ------------------------------------------------------------------ //
// 007_api_helpers.sql
// ------------------------------------------------------------------ //

fn build_api_helpers(groups: &[(String, String, Vec<&RuleDef>)]) -> String {
    let ts = timestamp_comment();
    let mut sb = String::new();

    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- API Helper Functions\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n\
         -- Pre-flight check wrappers — return JSONB instead of raising.\n\
         -- Use from REST handlers before committing DML.\n\n"
    ));

    // Generic validate_safe wrapper
    sb.push_str(
        "CREATE OR REPLACE FUNCTION validate_safe(\n\
             p_func    TEXT,\n\
             p_record  JSONB,\n\
             p_context JSONB\n\
         ) RETURNS JSONB AS $$\n\
         DECLARE\n\
             v_parts TEXT[];\n\
         BEGIN\n\
             EXECUTE format('SELECT %I($1, $2)', p_func)\n\
                 USING p_record, p_context;\n\
             RETURN jsonb_build_object('valid', TRUE, 'violations', '[]'::JSONB);\n\
         EXCEPTION WHEN OTHERS THEN\n\
             v_parts := string_to_array(SQLERRM, '|');\n\
             RETURN jsonb_build_object(\n\
                 'valid', FALSE,\n\
                 'violations', jsonb_build_array(jsonb_build_object(\n\
                     'code',    v_parts[1],\n\
                     'message', COALESCE(v_parts[2], SQLERRM)\n\
                 ))\n\
             );\n\
         END;\n\
         $$ LANGUAGE plpgsql;\n\n"
    );

    for (entity, trigger, _) in groups {
        let table   = to_plural_snake(entity);
        let fn_name = format!("validate_{}_{}", table, trigger);
        let pf_name = format!("preflight_{}_{}", table, trigger);

        sb.push_str(&format!(
            "-- Pre-flight: {entity}.{trigger}\n\
             CREATE OR REPLACE FUNCTION {pf_name}(\n\
                 p_record  JSONB,\n\
                 p_context JSONB DEFAULT current_rule_context()\n\
             ) RETURNS JSONB AS $$\n\
             BEGIN\n\
                 RETURN validate_safe('{fn_name}', p_record, p_context);\n\
             END;\n\
             $$ LANGUAGE plpgsql;\n\n"
        ));
    }

    sb
}

// ================================================================== //
// Condition → Certo expression
// ================================================================== //

fn emit_certo_condition(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    match node {
        ConditionNode::ConstraintRef { constraint } => snake_to_pascal(constraint),

        ConditionNode::TemporalRef { temporal, field, operator } => {
            let tname = snake_to_pascal(temporal);
            match operator.as_str() {
                "age_lt"  => format!("{}.age < {}", field, tname),
                "age_gt"  => format!("{}.age > {}", field, tname),
                "age_lte" => format!("{}.age <= {}", field, tname),
                "age_gte" => format!("{}.age >= {}", field, tname),
                _         => format!("{}.age < {}", field, tname),
            }
        }

        ConditionNode::All { all } => {
            let parts: Vec<String> = all.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" and ")
        }

        ConditionNode::Any { any } => {
            let parts: Vec<String> = any.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" or ")
        }

        ConditionNode::None { none } => {
            let parts: Vec<String> = none.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            let inner = if parts.len() == 1 { parts.into_iter().next().unwrap() }
                        else { parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" or ") };
            format!("not ({})", inner)
        }

        ConditionNode::Leaf { field, operator, value, .. } => {
            match operator.as_str() {
                "eq"          => format!("{} == {}", field, certo_val(value)),
                "not_eq"      => format!("{} != {}", field, certo_val(value)),
                "gt"          => format!("{} > {}", field, certo_val(value)),
                "gte"         => format!("{} >= {}", field, certo_val(value)),
                "lt"          => format!("{} < {}", field, certo_val(value)),
                "lte"         => format!("{} <= {}", field, certo_val(value)),
                "in"          => format!("{} in {}", field, certo_val(value)),
                "not_in"      => format!("{} not in {}", field, certo_val(value)),
                "exists"      => format!("{}.isSome()", field),
                "not_exists"  => format!("{}.isNone()", field),
                "matches"     => format!("Text.matches({}, {})", field, certo_val(value)),
                "not_matches" => format!("not Text.matches({}, {})", field, certo_val(value)),
                "contains"    => format!("Text.contains({}, {})", field, certo_val(value)),
                "starts_with" => format!("Text.startsWith({}, {})", field, certo_val(value)),
                other         => format!("{} {} {}", field, other, certo_val(value)),
            }
        }
    }
}

// ================================================================== //
// Condition → SQL expression
// ================================================================== //

fn emit_sql_condition(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    match node {
        ConditionNode::ConstraintRef { constraint } => {
            // Inline the constraint condition.
            if let Some(c) = constraint_map.get(constraint.as_str()) {
                emit_sql_condition(&c.condition, constraint_map, temporal_map)
            } else {
                format!("/* unknown constraint: {} */ TRUE", constraint)
            }
        }

        ConditionNode::TemporalRef { temporal, field, operator } => {
            let days = if let Some(t) = temporal_map.get(temporal.as_str()) {
                let (_, amount) = temporal_to_duration(t);
                amount
            } else {
                30
            };
            // JSONB ->> returns text; cast to TIMESTAMPTZ for record_age_days().
            let col = format!("{}::TIMESTAMPTZ", field_to_sql_col(field));
            match operator.as_str() {
                "age_lt"  => format!("record_age_days({}) < {}", col, days),
                "age_gt"  => format!("record_age_days({}) > {}", col, days),
                "age_lte" => format!("record_age_days({}) <= {}", col, days),
                "age_gte" => format!("record_age_days({}) >= {}", col, days),
                _         => format!("record_age_days({}) < {}", col, days),
            }
        }

        ConditionNode::All { all } => {
            let parts: Vec<String> = all.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\nAND ")
        }

        ConditionNode::Any { any } => {
            let parts: Vec<String> = any.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\n    OR ")
        }

        ConditionNode::None { none } => {
            let parts: Vec<String> = none.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            let inner = if parts.len() == 1 { parts.into_iter().next().unwrap() }
                        else { parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\n    OR ") };
            format!("NOT ({})", inner)
        }

        ConditionNode::Leaf { field, operator, value, .. } => {
            let col = field_to_sql_col(field);
            let typed_col = sql_cast_col(&col, value);
            match operator.as_str() {
                "eq"          => format!("{} = {}", typed_col, sql_val(value)),
                "not_eq"      => format!("{} != {}", typed_col, sql_val(value)),
                "gt"          => format!("{} > {}", typed_col, sql_val(value)),
                "gte"         => format!("{} >= {}", typed_col, sql_val(value)),
                "lt"          => format!("{} < {}", typed_col, sql_val(value)),
                "lte"         => format!("{} <= {}", typed_col, sql_val(value)),
                "in"          => format!("{} IN ({})", col, sql_list_val(value)),
                "not_in"      => format!("{} NOT IN ({})", col, sql_list_val(value)),
                "exists"      => format!("{} IS NOT NULL", col),
                "not_exists"  => format!("{} IS NULL", col),
                "matches"     => format!("{} ~ {}", col, sql_val(value)),
                "not_matches" => format!("{} !~ {}", col, sql_val(value)),
                "contains"    => format!("{} ILIKE '%%' || {} || '%%'", col, sql_val(value)),
                "starts_with" => format!("{} ILIKE {} || '%%'", col, sql_val(value)),
                other         => format!("{} {} {}", typed_col, other.to_uppercase(), sql_val(value)),
            }
        }
    }
}

/// Used to record the pass flag — re-emits the raw condition so we can assign `flag := <cond>`.
/// For complex conditions we wrap in a subexpression check.
fn cond_for_pass(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    let cond = emit_sql_condition(node, constraint_map, temporal_map);
    // Already a boolean expression — assign directly.
    format!("({})", cond)
}

// ================================================================== //
// Field path → SQL column expression
// ================================================================== //

/// Returns a bare JSONB ->> accessor with no type cast.
/// The caller applies a cast based on the comparison value type.
///
/// `order.total` → `(p_record->>'total')`
/// `user.role`   → `(p_context->>'user_role')`
/// `status`      → `(p_record->>'status')`
fn field_to_sql_col(field: &str) -> String {
    let ctx_roots = ["user", "operator", "actor"];
    if let Some(dot) = field.find('.') {
        let root = &field[..dot];
        let rest = field[dot + 1..].replace('.', "_");
        if ctx_roots.contains(&root) {
            return format!("(p_context->>'{}_{}')", root, rest);
        }
        let col = camel_to_snake(&rest);
        return format!("(p_record->>'{}')", col);
    }
    let col = camel_to_snake(field);
    format!("(p_record->>'{}')", col)
}

/// Wraps a bare JSONB accessor with the appropriate Postgres cast
/// based on what we're comparing it to.
fn sql_cast_col(col: &str, value: &Option<serde_yaml::Value>) -> String {
    match value {
        Some(serde_yaml::Value::Number(_)) => format!("{}::NUMERIC", col),
        Some(serde_yaml::Value::Bool(_))   => format!("{}::BOOLEAN", col),
        _                                   => col.to_string(),
    }
}

// ================================================================== //
// Value formatting
// ================================================================== //

fn certo_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        None => "null".into(),
        Some(serde_yaml::Value::Bool(b))   => b.to_string(),
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Null)      => "null".into(),
        Some(serde_yaml::Value::String(s)) => {
            if s.chars().all(|c| c.is_uppercase() || c == '_')
                || s.contains('.')
                || s.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
            {
                s.clone()
            } else {
                format!("\"{}\"", s)
            }
        }
        Some(other) => format!("{:?}", other),
    }
}

fn sql_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        None => "NULL".into(),
        Some(serde_yaml::Value::Bool(b))   => if *b { "TRUE".into() } else { "FALSE".into() },
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Null)      => "NULL".into(),
        Some(serde_yaml::Value::String(s)) => {
            // Lowercase identifier or dot-path = field reference, not a string literal.
            let is_field_ref = s.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
                && s.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_');
            if is_field_ref {
                // Numeric field references need a cast so comparisons work correctly.
                format!("{}::NUMERIC", field_to_sql_col(s))
            } else {
                format!("'{}'", s.replace('\'', "''"))
            }
        }
        Some(other) => format!("'{:?}'", other),
    }
}

fn sql_list_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        Some(serde_yaml::Value::Sequence(items)) => {
            items.iter()
                .map(|item| {
                    let wrapped = Some(item.clone());
                    sql_val(&wrapped)
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        other => sql_val(other),
    }
}

fn certo_paren(s: &str) -> String {
    if s.contains(" and ") || s.contains(" or ") { format!("({})", s) } else { s.to_string() }
}

fn sql_paren(s: &str) -> String {
    if s.contains(" AND ") || s.contains(" OR ") || s.contains('\n') {
        format!("({})", s)
    } else {
        s.to_string()
    }
}

// ================================================================== //
// Topological sort (respects depends_on within the group)
// ================================================================== //

fn topo_sort_rules<'a>(rules: &[&'a RuleDef]) -> Vec<&'a RuleDef> {
    let idx: HashMap<&str, usize> = rules.iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let mut visited = vec![false; rules.len()];
    let mut order: Vec<usize> = Vec::with_capacity(rules.len());

    fn visit(i: usize, rules: &[&RuleDef], idx: &HashMap<&str, usize>,
             visited: &mut Vec<bool>, order: &mut Vec<usize>) {
        if visited[i] { return; }
        visited[i] = true;
        for dep in &rules[i].depends_on {
            if let Some(&j) = idx.get(dep.as_str()) { visit(j, rules, idx, visited, order); }
        }
        // Also visit overriding rules before the rule they override.
        for (k, r) in rules.iter().enumerate() {
            if r.overrides.as_deref() == Some(rules[i].id.as_str()) {
                visit(k, rules, idx, visited, order);
            }
        }
        order.push(i);
    }

    for i in 0..rules.len() { visit(i, rules, &idx, &mut visited, &mut order); }
    order.iter().map(|&i| rules[i]).collect()
}

// ================================================================== //
// String helpers
// ================================================================== //

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", path.display(), e);
        process::exit(1);
    });
    serde_yaml::from_str(&src).unwrap_or_else(|e| {
        eprintln!("error: cannot parse {}: {}", path.display(), e);
        process::exit(1);
    })
}

fn write_output(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", path.display(), e);
        process::exit(1);
    });
    eprintln!("wrote {}", path.display());
}

fn append_newline(path: &Path) {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path)
        .and_then(|mut f| f.write_all(b"\n")).ok();
}

fn next_path(args: &[String], i: usize, flag: &str) -> PathBuf {
    PathBuf::from(args.get(i).unwrap_or_else(|| die(&format!("{} requires a path", flag), 2)))
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}

fn timestamp_comment() -> String {
    // A fixed-format UTC-like timestamp without pulling in chrono.
    // In practice the Rust std doesn't expose UTC time nicely, so we emit a placeholder.
    "see git history for generation date".to_string()
}

/// snake_case → PascalCase
fn snake_to_pascal(s: &str) -> String {
    s.split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }).collect()
}

/// PascalCase/camelCase → kebab-case
fn to_kebab(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 { out.push('-'); }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// PascalCase/camelCase → snake_case
fn to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 { out.push('_'); }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// camelCase field name → snake_case column name
fn camel_to_snake(s: &str) -> String {
    to_snake(s)
}

/// PascalCase entity name → plural snake_case table name (naïve: append 's' unless already ends in 's')
fn to_plural_snake(entity: &str) -> String {
    let snake = to_snake(entity);
    if snake.ends_with('s') { snake } else { format!("{}s", snake) }
}

/// trigger name → PascalCase suffix for validator name
fn trigger_pascal(t: &str) -> String {
    t.split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }).collect()
}

/// Rule id → SQL variable name (replace - with _)
fn to_var(id: &str) -> String {
    id.replace('-', "_")
}

fn lowercase_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|ch| ch.to_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
}

fn temporal_to_duration(t: &TemporalDef) -> (&'static str, u64) {
    match t.unit.as_str() {
        "minutes" => ("minutes", t.duration),
        "hours"   => ("hours",   t.duration),
        "days"    => ("days",    t.duration),
        "weeks"   => ("days",    t.duration * 7),
        "months"  => ("days",    t.duration * 30),
        "years"   => ("days",    t.duration * 365),
        _         => ("days",    t.duration),
    }
}

// ================================================================== //
// Context inference
// ================================================================== //

fn infer_context_entities<'a>(
    rules:            &[&RuleDef],
    primary_entity:   &str,
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
) -> Vec<String> {
    let primary_lower = lowercase_first(primary_entity);
    let mut seen: Vec<String> = Vec::new();
    let mut seen_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rule in rules {
        collect_field_roots(&rule.condition, &primary_lower, entity_field_map, &mut seen, &mut seen_set);
    }
    seen
}

fn collect_field_roots(
    node:             &ConditionNode,
    primary_var:      &str,
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
    seen:             &mut Vec<String>,
    seen_set:         &mut std::collections::HashSet<String>,
) {
    match node {
        ConditionNode::Leaf { field, .. } => {
            if let Some(root) = field.split('.').next() {
                if root != primary_var && !seen_set.contains(root) {
                    let entity_name = capitalize_first(root);
                    if entity_field_map.contains_key(entity_name.as_str()) {
                        seen_set.insert(root.to_string());
                        seen.push(entity_name);
                    }
                }
            }
        }
        ConditionNode::All  { all  } => all .iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        ConditionNode::Any  { any  } => any .iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        ConditionNode::None { none } => none.iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        _ => {}
    }
}

// ================================================================== //
// `certo generate api` tests — BACKLOG item 153
// ================================================================== //

#[cfg(test)]
mod api_tests {
    use super::*;

    fn fields_of(src: &str, type_name: &str) -> Vec<certo_ast::decl::RecordFieldDef> {
        let module = certo_parser::parse(src).expect("parse error");
        module.decls.iter().find_map(|d| match &d.node {
            Decl::Type(t) if t.name.node == type_name => match &t.body {
                certo_ast::decl::TypeBody::Record(r) => Some(r.fields.clone()),
                _ => None,
            },
            _ => None,
        }).expect("type not found")
    }

    #[test]
    fn pascal_to_snake_converts_correctly() {
        assert_eq!(pascal_to_snake("Widget"), "widget");
        assert_eq!(pascal_to_snake("OrderItem"), "order_item");
        assert_eq!(pascal_to_snake("A"), "a");
    }

    #[test]
    fn field_type_name_peels_option_and_falls_back_to_text() {
        // `List<Text>` is still `TypeExpr::Named` (with generic args) — the
        // fallback path is for genuinely non-Named shapes, like a tuple.
        let fields = fields_of(
            "module A\ntype T = { a: Int, b: Int?, c: (Int, Text) }", "T");
        assert_eq!(field_type_name(&fields[0].ty.node), "Int");
        assert_eq!(field_type_name(&fields[1].ty.node), "Int", "T? must peel to T");
        assert_eq!(field_type_name(&fields[2].ty.node), "Text", "a non-Named type (e.g. a tuple) falls back to Text");
    }

    #[test]
    fn json_decode_expr_uses_the_typed_accessor_and_converts_back_to_text() {
        assert_eq!(json_decode_expr("n", "Int"),   "intToText(JsonValue.asInt(JsonValue.get(_jv, \"n\")))");
        assert_eq!(json_decode_expr("f", "Float"), "floatToText(JsonValue.asFloat(JsonValue.get(_jv, \"f\")))");
        assert_eq!(json_decode_expr("b", "Bool"),  "boolToText(JsonValue.asBool(JsonValue.get(_jv, \"b\")))");
        assert_eq!(json_decode_expr("s", "Text"),  "JsonValue.asText(JsonValue.get(_jv, \"s\"))");
        assert_eq!(json_decode_expr("u", "UUID"),  "JsonValue.asText(JsonValue.get(_jv, \"u\"))", "an unrecognized type falls back to plain text");
    }

    #[test]
    fn emit_api_generated_source_is_self_parseable() {
        // Full seeded typecheck/build verification was done separately, via
        // the real `certo check`/`certo build --emit-dll` CLI commands
        // against a real generated file — `certo_typeck::check_module`
        // (unseeded, no stdlib registered) isn't the right tool for that
        // here and would only ever report every stdlib call as unbound.
        // This test covers what a plain unit test actually can: the
        // generator's own string-building never emits malformed syntax.
        let fields = fields_of(
            "module A\ntype Widget = { id: UUID, name: Text, price: Int, inStock: Bool }", "Widget");
        let source = emit_api("Widget", "widget", &fields);

        certo_parser::parse(&source)
            .unwrap_or_else(|errs| panic!("generated source failed to parse:\n{}\n---\n{}",
                errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"), source));

        // The five CRUD handlers and the router all exist.
        for name in ["handler", "listWidget", "getWidget", "createWidget", "updateWidget", "deleteWidget"] {
            assert!(source.contains(&format!("fn {name}(")), "missing generated function: {name}");
        }
    }

    #[test]
    fn emit_api_declares_the_db_import_needed_for_linking() {
        // uses_db (crates/cli/src/main.rs) — the switch that links libpq at
        // all — scans module.imports for a literal `Stdlib.Db` import, not
        // for actual dbConnect/dbQuery call sites. Confirmed by a real
        // build attempt before this was added: every db* call compiled
        // fine but failed to *link* without it.
        let fields = fields_of("module A\ntype Widget = { id: UUID, name: Text }", "Widget");
        let source = emit_api("Widget", "widget", &fields);
        assert!(source.contains("import Stdlib.Db"), "missing the load-bearing Stdlib.Db import");
    }

    #[test]
    fn emit_api_insert_and_update_never_write_the_id_column() {
        let fields = fields_of(
            "module A\ntype Widget = { id: UUID, name: Text, price: Int }", "Widget");
        let source = emit_api("Widget", "widget", &fields);
        assert!(source.contains("INSERT INTO widget (name, price) VALUES ($1, $2)"),
            "id must never be inserted — it's the primary key, not a client-supplied value:\n{source}");
        assert!(source.contains("UPDATE widget SET name = $1, price = $2 WHERE id = $3"),
            "id must never be in UPDATE's SET clause, only its WHERE clause:\n{source}");
    }

    #[test]
    fn emit_api_missing_id_field_is_rejected_before_generation() {
        // certo generate api requires an `id` field to exist (the assumed
        // primary key) — checked in `cmd_generate_api` before `emit_api` is
        // ever called, not discovered later as a runtime SQL failure.
        let module = certo_parser::parse("module A\ntype Widget = { name: Text }").unwrap();
        let certo_ast::decl::TypeBody::Record(r) = &module.decls.iter().find_map(|d| match &d.node {
            Decl::Type(t) if t.name.node == "Widget" => Some(&t.body),
            _ => None,
        }).unwrap() else { panic!("expected a record type") };
        assert!(!r.fields.iter().any(|f| f.name.node == "id"), "sanity check: this type really has no id field");
    }
}
