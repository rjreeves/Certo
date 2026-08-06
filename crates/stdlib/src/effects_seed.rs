use std::collections::HashSet;
use certo_ast::types::Effect;
use certo_effects::{EffectEnv, DeclaredEffects};

/// Register the declared effects of every stdlib function that is actually
/// effectful. Hand-maintained, mirroring how `seed.rs` hand-registers stdlib
/// *type* signatures rather than deriving them from anything parseable —
/// there's no machine-readable source of truth for stdlib signatures at all
/// (see `docs/STDLIB-QUICKREF.md`, the hand-maintained manual), so the
/// names/effects here are cross-checked against that doc by hand.
///
/// Without this, `certo_effects` has no idea that `println`/`dbConnect`/etc.
/// are effectful, so a `[pure]` function calling them directly goes
/// unchecked (BACKLOG item 105).
///
/// Dot-qualified names (`Query.list`, `Http.get`, ...) are registered too,
/// even though `Type.method(...)` call sites aren't routed through the
/// effects checker yet (the same `Expr::Field`-vs-`Expr::Path` limitation as
/// BACKLOG items 101/102) — so this stays complete once that routing gap is
/// fixed, with no further changes needed here.
pub fn seed_stdlib_effects(env: &mut EffectEnv) {
    for name in IO_FUNCTIONS {
        define(env, name, Effect::Io);
    }
    for name in FALLIBLE_FUNCTIONS {
        define(env, name, Effect::Fallible);
    }
}

fn define(env: &mut EffectEnv, name: &str, effect: Effect) {
    env.insert(name.to_string(), DeclaredEffects {
        effects: HashSet::from([effect]),
        is_pure: false,
    });
}

const IO_FUNCTIONS: &[&str] = &[
    // Core — console / process
    "print", "println", "eprint", "eprintln", "flush",
    "readLine", "readAll", "monotonicMillis", "sleep",
    // Env
    "setEnv", "unsetEnv",
    // File
    "readFile", "readFileBytes", "writeFile", "writeFileBytes",
    "appendFile", "deleteFile", "fileExists", "listDir", "makeDir",
    // Db (connection / raw query / transaction)
    "dbConnect", "dbClose", "dbServerVersion", "dbVersionString",
    "dbExec", "dbRunScript", "dbRunScriptResult",
    "dbQuery", "dbQueryTyped", "dbQueryRow", "dbQueryOne", "dbColumns", "dbStream",
    "dbBegin", "dbCommit", "dbRollback",
    "withTransaction", "withConnection",
    // DbQuery / DbMutation builders' terminal (executing) methods
    "Query.list", "Query.first", "Query.groupedList",
    "Query.count", "Query.sum", "Query.avg", "Query.min", "Query.max",
    "Mutation.run",
    // Http
    "Http.get", "Http.post", "Http.put", "Http.delete",
    "Http.request", "Http.requestBytes", "Http.serve",
    // Credential
    "Credential.get", "Credential.getBytes", "Credential.set", "Credential.delete",
    // Process
    "Process.exec", "Process.execWithInput", "Process.spawnDetached",
    "Process.spawnDetachedHidden", "Process.quit", "Process.lines",
    // DateTime
    "DateTime.now", "Date.today",
    // Json (in-place mutation)
    "JsonValue.push", "JsonValue.set",
];

const FALLIBLE_FUNCTIONS: &[&str] = &[
    "DateTime.parseIso",
    "Decimal.div",
];
