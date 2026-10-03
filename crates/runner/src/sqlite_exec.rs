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
use crate::migration::Migration;
use crate::sqlite_introspect;
use rusqlite::Connection;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::ErrorKind;

pub struct SqliteExecutor {
    conn: Connection,
    /// Held for the executor's lifetime; unlocked when it is dropped.
    _lock: Option<File>,
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
        Ok(SqliteExecutor { conn, _lock: lock })
    }

    /// Wrap an already-open connection (tests, hosts that manage their own).
    pub fn from_connection(conn: Connection) -> SqliteExecutor { SqliteExecutor { conn, _lock: None } }

    pub fn connection(&self) -> &Connection { &self.conn }

    fn record_sql() -> String {
        format!("INSERT INTO \"{HISTORY_TABLE}\" (seq, name, checksum, compiler_version) VALUES (?1, ?2, ?3, ?4)")
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
        self.conn
            .execute(&Self::record_sql(), rusqlite::params![m.seq as i64, m.name, m.checksum, m.script.compiler_version])
            .map_err(|e| err(e, Some("record migration in history")))?;
        Ok(())
    }

    fn introspect(&mut self) -> Result<LiveSchema, ExecError> {
        sqlite_introspect::introspect(&self.conn).map_err(|e| err(e, None))
    }

    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        let params = rusqlite::params![m.seq as i64, m.name, m.checksum, m.script.compiler_version];
        let last = m.script.batches.len().checked_sub(1);
        let record_inside = last.is_some_and(|i| m.script.batches[i].transactional);

        for (i, b) in m.script.batches.iter().enumerate() {
            if b.transactional {
                let tx = self.conn.transaction().map_err(|e| err(e, None))?;
                for s in &b.statements {
                    tx.execute_batch(s).map_err(|e| err(e, Some(s)))?;
                }
                if record_inside && Some(i) == last {
                    tx.execute(&Self::record_sql(), params)
                        .map_err(|e| err(e, Some("record migration in history")))?;
                }
                tx.commit().map_err(|e| err(e, Some("COMMIT")))?;
            } else {
                for s in &b.statements {
                    self.conn.execute_batch(s).map_err(|e| err(e, Some(s)))?;
                }
            }
        }
        if !record_inside {
            self.conn
                .execute(&Self::record_sql(), params)
                .map_err(|e| err(e, Some("record migration in history")))?;
        }
        Ok(())
    }
}
