//! The optional journal: an append-only table, `_certo_log`, that records who changed the database and when.
//!
//! It is off unless a host asks for it (`ApplyOptions::journal`, `AdoptOptions::journal`). When on, each applied
//! migration (`apply`) and each adopted baseline (`adopt`) adds a row INSIDE the transaction that makes the change,
//! so the journal holds exactly the changes that committed: a migration that rolled back leaves no row, and one that
//! committed cannot be missing one. The table is created on first use and ignored by introspection (like the history
//! table), so it never shows up as drift.

use crate::migration::Migration;

pub const LOG_TABLE: &str = "_certo_log";

/// Who is making the change, for the journal.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JournalContext {
    /// The person or service, for example `alice@build-host`.
    pub actor: String,
    /// The named environment (`prod`), if the host has such a notion.
    pub environment: Option<String>,
    /// The tool and its version, for example `certo 0.10.0`.
    pub tool: String,
}

pub(crate) fn pg_create() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS \"{LOG_TABLE}\" (
            id bigserial PRIMARY KEY,
            at timestamptz NOT NULL DEFAULT now(),
            action text NOT NULL,
            subject text NOT NULL,
            actor text NOT NULL,
            environment text,
            tool text NOT NULL,
            detail text
        )"
    )
}

pub(crate) fn sqlite_create() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS \"{LOG_TABLE}\" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            action TEXT NOT NULL,
            subject TEXT NOT NULL,
            actor TEXT NOT NULL,
            environment TEXT,
            tool TEXT NOT NULL,
            detail TEXT
        )"
    )
}

pub(crate) fn pg_insert() -> String {
    format!("INSERT INTO \"{LOG_TABLE}\" (action, subject, actor, environment, tool, detail) VALUES ($1, $2, $3, $4, $5, $6)")
}

pub(crate) fn sqlite_insert() -> String {
    format!("INSERT INTO \"{LOG_TABLE}\" (action, subject, actor, environment, tool, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
}

/// What a migration's journal row says besides its label.
pub(crate) fn detail(m: &Migration) -> String {
    serde_json::json!({ "seq": m.seq, "checksum": m.checksum }).to_string()
}
