//! The SQLite executor: the same history, apply and introspection contract as
//! `PgExecutor`, on a database file (or `:memory:`).
//!
//! SQLite has no advisory locks, so the executor takes an operating-system file lock on a
//! sidecar file, `<database>.certo-lock`, for as long as it lives: a second runner on the same
//! database fails at once ("another migration run holds the lock"), as with PostgreSQL's
//! advisory lock, instead of starting its own migration between two batches. The lock goes
//! away with the process, however it ends, so a crash leaves nothing stale; the small empty
//! file stays (deleting it would let two runners hold "the" lock on different files).
//! An in-memory database, and one in a folder the process cannot write to, use no lock.

use crate::error::RunnerError;
use crate::exec::{AppliedRow, ExecError, Executor};
use crate::introspect::{LiveSchema, HISTORY_TABLE};
use crate::journal::{self, JournalContext};
use crate::views::{ProjectView, RecordedView, VIEWS_TABLE};
use crate::migration::Migration;
use crate::sqlite_introspect;
use rusqlite::Connection;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::ErrorKind;

pub struct SqliteExecutor {
    conn: Connection,
    /// Held for the executor's lifetime; unlocked when it is dropped.
    _lock: Option<File>,
    journal: Option<JournalContext>,
}

/// Take the lock on `<path>.certo-lock` without waiting.
fn lock_beside(path: &str) -> Result<Option<File>, RunnerError> {
    if path.is_empty() || path == ":memory:" {
        return Ok(None);
    }
    let lock_path = format!("{path}.certo-lock");
    let file = match OpenOptions::new().create(true).write(true).truncate(false).open(&lock_path) {
        Ok(f) => f,
        // a database opened just to be read, in a folder this process cannot write to
        Err(e) if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem) => return Ok(None),
        Err(e) => return Err(RunnerError::Connection(format!("could not create the lock file `{lock_path}`: {e}"))),
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Err(RunnerError::Connection(
            "another migration run holds the lock on this database; wait for it to finish".into(),
        )),
        Err(TryLockError::Error(e)) => Err(RunnerError::Connection(format!("could not lock `{lock_path}`: {e}"))),
    }
}

fn err(e: rusqlite::Error, statement: Option<&str>) -> ExecError {
    ExecError { statement: statement.map(str::to_string), message: e.to_string() }
}

impl SqliteExecutor {
    /// Open (creating if needed) the database at `target`: a file path,
    /// `sqlite:<path>` / `sqlite://<path>`, or `:memory:`. Foreign keys are
    /// enforced on this connection.
    pub fn open(target: &str) -> Result<SqliteExecutor, RunnerError> {
        let path = target
            .strip_prefix("sqlite://")
            .or_else(|| target.strip_prefix("sqlite:"))
            .unwrap_or(target);
        let conn_err = |e: rusqlite::Error| RunnerError::Connection(format!("could not open the database `{path}`: {e}"));
        let conn = Connection::open(path).map_err(conn_err)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(conn_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(conn_err)?;
        let lock = lock_beside(path)?;
        Ok(SqliteExecutor { conn, _lock: lock, journal: None })
    }

    /// Wrap an already-open connection (tests, hosts that manage their own).
    pub fn from_connection(conn: Connection) -> SqliteExecutor { SqliteExecutor { conn, _lock: None, journal: None } }

    pub fn connection(&self) -> &Connection { &self.conn }

    fn record_sql() -> String {
        format!("INSERT INTO \"{HISTORY_TABLE}\" (seq, name, checksum, compiler_version) VALUES (?1, ?2, ?3, ?4)")
    }
}

impl SqliteExecutor {
    /// Run every batch of `m` and record it in the history. The history row goes inside the last
    /// transactional batch, so it commits (or rolls back) together with the schema change. A table-rebuild
    /// script ends with `PRAGMA foreign_keys = ON`, which cannot run inside a transaction; recording after it
    /// would leave a window in which the change is committed but unrecorded.
    fn run_batches(&mut self, m: &Migration) -> Result<(), ExecError> {
        let params = rusqlite::params![m.seq as i64, m.name, m.checksum, m.script.compiler_version];
        let last_tx = m.script.batches.iter().rposition(|b| b.transactional);
        let journal = self.journal.clone();
        let (label, detail) = (m.label(), journal::detail(m));
        if journal.is_some() {
            self.conn.execute_batch(&journal::sqlite_create()).map_err(|e| err(e, Some("create the journal table")))?;
        }

        for (i, b) in m.script.batches.iter().enumerate() {
            if b.transactional {
                let tx = self.conn.transaction().map_err(|e| err(e, None))?;
                for s in &b.statements {
                    tx.execute_batch(s).map_err(|e| err(e, Some(s)))?;
                }
                if Some(i) == last_tx {
                    tx.execute(&Self::record_sql(), params)
                        .map_err(|e| err(e, Some("record migration in history")))?;
                    if let Some(j) = &journal {
                        tx.execute(&journal::sqlite_insert(), rusqlite::params!["apply", label, j.actor, j.environment, j.tool, detail])
                            .map_err(|e| err(e, Some("write the journal")))?;
                    }
                }
                tx.commit().map_err(|e| err(e, Some("COMMIT")))?;
            } else {
                for s in &b.statements {
                    self.conn.execute_batch(s).map_err(|e| err(e, Some(s)))?;
                }
            }
        }
        if last_tx.is_none() {
            // nothing transactional to hold the row (an empty script)
            self.conn
                .execute(&Self::record_sql(), params)
                .map_err(|e| err(e, Some("record migration in history")))?;
            if let Some(j) = &journal {
                self.conn
                    .execute(&journal::sqlite_insert(), rusqlite::params!["apply", label, j.actor, j.environment, j.tool, detail])
                    .map_err(|e| err(e, Some("write the journal")))?;
            }
        }
        Ok(())
    }
}

impl Executor for SqliteExecutor {
    fn dialect(&self) -> &'static str { "sqlite" }

    fn ensure_history(&mut self) -> Result<(), ExecError> {
        self.conn
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS \"{HISTORY_TABLE}\" (
                    seq INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    checksum TEXT NOT NULL,
                    compiler_version TEXT NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                )"
            ))
            .map_err(|e| err(e, None))
    }

    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError> {
        let exists: bool = self
            .conn
            .query_row("SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1", [HISTORY_TABLE], |r| r.get(0))
            .map_err(|e| err(e, None))?;
        if !exists {
            return Ok(Vec::new());
        }
        let mut st = self
            .conn
            .prepare(&format!("SELECT seq, name, checksum, applied_at FROM \"{HISTORY_TABLE}\" ORDER BY seq"))
            .map_err(|e| err(e, None))?;
        let rows = st
            .query_map([], |r| {
                Ok(AppliedRow { seq: r.get::<_, i64>(0)? as u32, name: r.get(1)?, checksum: r.get(2)?, applied_at: r.get(3)? })
            })
            .map_err(|e| err(e, None))?;
        rows.collect::<rusqlite::Result<_>>().map_err(|e| err(e, None))
    }

    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError> {
        self.ensure_history()?;
        let params = rusqlite::params![m.seq as i64, m.name, m.checksum, m.script.compiler_version];
        let Some(j) = self.journal.clone() else {
            self.conn.execute(&Self::record_sql(), params).map_err(|e| err(e, Some("record migration in history")))?;
            return Ok(());
        };
        // the baseline and its journal row commit together
        self.conn.execute_batch(&journal::sqlite_create()).map_err(|e| err(e, Some("create the journal table")))?;
        let (label, detail) = (m.label(), journal::detail(m));
        let tx = self.conn.transaction().map_err(|e| err(e, None))?;
        tx.execute(&Self::record_sql(), params).map_err(|e| err(e, Some("record migration in history")))?;
        tx.execute(&journal::sqlite_insert(), rusqlite::params!["adopt", label, j.actor, j.environment, j.tool, detail])
            .map_err(|e| err(e, Some("write the journal")))?;
        tx.commit().map_err(|e| err(e, Some("COMMIT")))
    }

    fn set_journal(&mut self, journal: Option<JournalContext>) { self.journal = journal; }

    fn recorded_views(&mut self) -> Result<Vec<RecordedView>, ExecError> {
        let exists: bool = self
            .conn
            .query_row("SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1", [VIEWS_TABLE], |r| r.get(0))
            .map_err(|e| err(e, None))?;
        if !exists {
            return Ok(Vec::new());
        }
        let mut st = self.conn.prepare(&format!("SELECT name, checksum FROM \"{VIEWS_TABLE}\" ORDER BY ord")).map_err(|e| err(e, None))?;
        let rows = st
            .query_map([], |r| Ok(RecordedView { name: r.get(0)?, checksum: r.get(1)? }))
            .map_err(|e| err(e, None))?;
        rows.collect::<rusqlite::Result<_>>().map_err(|e| err(e, None))
    }

    fn replace_views(&mut self, drop: &[String], create: &[ProjectView]) -> Result<(), ExecError> {
        self.conn
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS \"{VIEWS_TABLE}\" (ord INTEGER NOT NULL, name TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ))
            .map_err(|e| err(e, None))?;
        let journal = self.journal.clone();
        if journal.is_some() {
            self.conn.execute_batch(&journal::sqlite_create()).map_err(|e| err(e, Some("create the journal table")))?;
        }
        let tx = self.conn.transaction().map_err(|e| err(e, None))?;
        for name in drop {
            let sql = format!("DROP VIEW IF EXISTS {}", certo_sql::quote_ident(certo_sql::Dialect::Sqlite, name));
            tx.execute_batch(&sql).map_err(|e| err(e, Some(&sql)))?;
        }
        tx.execute_batch(&format!("DELETE FROM \"{VIEWS_TABLE}\"")).map_err(|e| err(e, None))?;
        for (i, v) in create.iter().enumerate() {
            tx.execute_batch(&v.create_sql).map_err(|e| err(e, Some(&v.create_sql)))?;
            tx.execute(&format!("INSERT INTO \"{VIEWS_TABLE}\" (ord, name, checksum) VALUES (?1, ?2, ?3)"), rusqlite::params![i as i64, v.name, v.checksum])
                .map_err(|e| err(e, Some("record the view")))?;
        }
        if let (Some(j), false) = (&journal, create.is_empty()) {
            let detail = serde_json::json!({ "views": create.iter().map(|v| v.name.clone()).collect::<Vec<_>>() }).to_string();
            let subject = format!("{} view(s)", create.len());
            tx.execute(&journal::sqlite_insert(), rusqlite::params!["views", subject, j.actor, j.environment, j.tool, detail])
                .map_err(|e| err(e, Some("write the journal")))?;
        }
        tx.commit().map_err(|e| err(e, Some("COMMIT")))
    }

    fn live_views(&mut self) -> Result<Vec<String>, ExecError> {
        let mut st = self.conn.prepare("SELECT name FROM sqlite_master WHERE type = 'view' ORDER BY name").map_err(|e| err(e, None))?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(|e| err(e, None))?;
        rows.collect::<rusqlite::Result<_>>().map_err(|e| err(e, None))
    }

    fn introspect(&mut self) -> Result<LiveSchema, ExecError> {
        let mut live = sqlite_introspect::introspect(&self.conn).map_err(|e| err(e, None))?;
        // a view certo created is part of the project, not something the schema language failed to express
        let managed = self.recorded_views()?;
        live.notes.retain(|n| !managed.iter().any(|m| n.starts_with(&format!("view {} is not represented", m.name))));
        Ok(live)
    }

    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        let result = self.run_batches(m);
        if result.is_err() {
            // a failed rebuild may have left `PRAGMA foreign_keys = OFF` behind; this connection enforces them
            let _ = self.conn.execute_batch("PRAGMA foreign_keys = ON;");
        }
        result
    }
}
