//! The MySQL 8 executor.
//!
//! MySQL commits every DDL statement the moment it runs, so a migration cannot be atomic the way it is on PostgreSQL and
//! SQLite. Instead the runner applies a migration one statement at a time and keeps count in `_certo_progress`: after a
//! failure the statements before the failed one are in the database, the table says how many, and `apply` again carries on
//! from the failed statement (once whatever stopped it has been fixed). The history row (and the journal row) is written, in
//! one transaction with removing the progress row, only after the last statement succeeded: a migration is recorded when,
//! and only when, all of it ran. `status` shows a half-applied migration as such.
//!
//! A runner that is killed between a statement and its progress row leaves that statement's state unknown; the next `apply`
//! runs it again and (for DDL) reports the "already exists" for you to settle by hand.

use crate::error::RunnerError;
use crate::exec::{AppliedRow, ExecError, Executor, PartialRow};
use crate::introspect::LiveSchema;
use crate::journal::{self, JournalContext};
use crate::migration::Migration;
use crate::mysql_introspect;
use crate::views::{ProjectView, RecordedView, VIEWS_TABLE};
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, TxOpts};

const HISTORY_TABLE: &str = "_certo_migrations";
pub const PROGRESS_TABLE: &str = "_certo_progress";
/// What the message of a failed step says, so the error text does not claim a rollback that MySQL cannot do.
pub const RESUME_HINT: &str = "apply again to resume";

/// MySQL's named locks belong to the server, not to a database: the name carries the database's.
const LOCK_NAME: &str = "certo_migrations";

pub struct MysqlExecutor {
    conn: Conn,
    journal: Option<JournalContext>,
}

fn my_err(e: mysql::Error, statement: Option<&str>) -> ExecError {
    ExecError { statement: statement.map(str::to_string), message: e.to_string() }
}

impl MysqlExecutor {
    /// Connect with a `mysql://user:password@host:port/database` URL. The URL must name the database.
    pub fn connect(url: &str) -> Result<MysqlExecutor, RunnerError> {
        let conn_err = |e: String| RunnerError::Connection(format!("could not connect to the database: {e}"));
        let opts = Opts::from_url(url).map_err(|e| conn_err(e.to_string()))?;
        if opts.get_db_name().is_none_or(str::is_empty) {
            return Err(RunnerError::Connection("the MySQL URL must name a database (mysql://user@host/database)".into()));
        }
        // UTC, utf8mb4, and room for string_agg results: what the generated SQL and QL expect
        let builder = OptsBuilder::from_opts(opts).init(vec![
            "SET SESSION time_zone = '+00:00'".to_string(),
            "SET NAMES utf8mb4".to_string(),
            "SET SESSION group_concat_max_len = 1073741824".to_string(),
        ]);
        let mut conn = Conn::new(builder).map_err(|e| conn_err(e.to_string()))?;
        let locked: Option<i64> = conn
            .exec_first("SELECT GET_LOCK(CONCAT(?, ':', DATABASE()), 0)", (LOCK_NAME,))
            .map_err(|e| conn_err(e.to_string()))?
            .flatten();
        if locked != Some(1) {
            return Err(RunnerError::Connection("another migration run holds the lock on this database; wait for it to finish".into()));
        }
        Ok(MysqlExecutor { conn, journal: None })
    }

    fn table_exists(&mut self, name: &str) -> Result<bool, ExecError> {
        let n: Option<i64> = self
            .conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?",
                (name,),
            )
            .map_err(|e| my_err(e, None))?;
        Ok(n.unwrap_or(0) > 0)
    }

    fn ensure_progress(&mut self) -> Result<(), ExecError> {
        self.conn
            .query_drop(format!(
                "CREATE TABLE IF NOT EXISTS `{PROGRESS_TABLE}` (
                    seq INT PRIMARY KEY,
                    checksum VARCHAR(64) NOT NULL,
                    done INT NOT NULL,
                    total INT NOT NULL,
                    updated_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
                )"
            ))
            .map_err(|e| my_err(e, None))
    }

    fn ensure_journal(&mut self) -> Result<(), ExecError> {
        self.conn.query_drop(journal::mysql_create()).map_err(|e| my_err(e, Some("create the journal table")))
    }

    /// The history row, the journal row (if the journal is on) and, for `apply`, forgetting the progress row: one transaction.
    fn record(&mut self, m: &Migration, action: &str, clear_progress: bool) -> Result<(), ExecError> {
        let journal = self.journal.clone();
        if journal.is_some() {
            self.ensure_journal()?;
        }
        let (label, detail) = (m.label(), journal::detail(m));
        let mut tx = self.conn.start_transaction(TxOpts::default()).map_err(|e| my_err(e, None))?;
        tx.exec_drop(
            format!("INSERT INTO `{HISTORY_TABLE}` (seq, name, checksum, compiler_version) VALUES (?, ?, ?, ?)"),
            (m.seq, &m.name, &m.checksum, &m.script.compiler_version),
        )
        .map_err(|e| my_err(e, Some("record migration in history")))?;
        if let Some(j) = &journal {
            tx.exec_drop(journal::mysql_insert(), (action, &label, &j.actor, &j.environment, &j.tool, &detail))
                .map_err(|e| my_err(e, Some("write the journal")))?;
        }
        if clear_progress {
            tx.exec_drop(format!("DELETE FROM `{PROGRESS_TABLE}` WHERE seq = ?"), (m.seq,)).map_err(|e| my_err(e, None))?;
        }
        tx.commit().map_err(|e| my_err(e, Some("COMMIT")))
    }
}

impl Executor for MysqlExecutor {
    fn dialect(&self) -> &'static str { "mysql" }

    fn ensure_history(&mut self) -> Result<(), ExecError> {
        self.conn
            .query_drop(format!(
                "CREATE TABLE IF NOT EXISTS `{HISTORY_TABLE}` (
                    seq INT PRIMARY KEY,
                    name VARCHAR(255) NOT NULL,
                    checksum VARCHAR(64) NOT NULL,
                    compiler_version VARCHAR(64) NOT NULL,
                    applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
                )"
            ))
            .map_err(|e| my_err(e, None))
    }

    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError> {
        if !self.table_exists(HISTORY_TABLE)? {
            return Ok(Vec::new());
        }
        let rows: Vec<(u32, String, String, String)> = self
            .conn
            .query(format!("SELECT seq, name, checksum, CAST(applied_at AS CHAR) FROM `{HISTORY_TABLE}` ORDER BY seq"))
            .map_err(|e| my_err(e, None))?;
        Ok(rows.into_iter().map(|(seq, name, checksum, applied_at)| AppliedRow { seq, name, checksum, applied_at }).collect())
    }

    fn partial(&mut self) -> Result<Vec<PartialRow>, ExecError> {
        if !self.table_exists(PROGRESS_TABLE)? {
            return Ok(Vec::new());
        }
        let rows: Vec<(u32, u32, u32)> =
            self.conn.query(format!("SELECT seq, done, total FROM `{PROGRESS_TABLE}` ORDER BY seq")).map_err(|e| my_err(e, None))?;
        Ok(rows.into_iter().map(|(seq, done, total)| PartialRow { seq, done, total }).collect())
    }

    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        let statements: Vec<&String> = m.script.batches.iter().flat_map(|b| &b.statements).collect();
        let total = statements.len() as u32;
        self.ensure_history()?;
        self.ensure_progress()?;
        let existing: Option<(String, u32)> = self
            .conn
            .exec_first(format!("SELECT checksum, done FROM `{PROGRESS_TABLE}` WHERE seq = ?"), (m.seq,))
            .map_err(|e| my_err(e, None))?;
        let mut done = match existing {
            Some((checksum, _)) if checksum != m.checksum => {
                return Err(ExecError {
                    statement: None,
                    message: format!(
                        "migration {} was half applied and has changed since; put its files back as they were to resume, or repair the database by hand and remove its row from `{PROGRESS_TABLE}`",
                        m.label()
                    ),
                });
            }
            Some((_, done)) => done,
            None => {
                self.conn
                    .exec_drop(format!("INSERT INTO `{PROGRESS_TABLE}` (seq, checksum, done, total) VALUES (?, ?, 0, ?)"), (m.seq, &m.checksum, total))
                    .map_err(|e| my_err(e, None))?;
                0
            }
        };
        while (done as usize) < statements.len() {
            let s = statements[done as usize];
            self.conn.query_drop(s).map_err(|e| ExecError {
                statement: Some(s.clone()),
                message: format!(
                    "{e} (step {} of {total}; the {done} before it are applied: fix the cause and apply again to resume here)",
                    done + 1
                ),
            })?;
            done += 1;
            self.conn
                .exec_drop(format!("UPDATE `{PROGRESS_TABLE}` SET done = ?, updated_at = UTC_TIMESTAMP(6) WHERE seq = ?"), (done, m.seq))
                .map_err(|e| my_err(e, None))?;
        }
        self.record(m, "apply", true)
    }

    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError> {
        self.ensure_history()?;
        self.record(m, "adopt", false)
    }

    fn introspect(&mut self) -> Result<LiveSchema, ExecError> {
        let mut live = mysql_introspect::introspect(&mut self.conn).map_err(|e| my_err(e, None))?;
        // a view certo created is part of the project, not something the schema language failed to express
        let managed = self.recorded_views()?;
        live.notes.retain(|n| !managed.iter().any(|m| n.starts_with(&format!("view {} is not represented", m.name))));
        live.view_sql.retain(|(n, _)| !managed.iter().any(|m| &m.name == n));
        Ok(live)
    }

    fn set_journal(&mut self, journal: Option<JournalContext>) { self.journal = journal; }

    fn recorded_views(&mut self) -> Result<Vec<RecordedView>, ExecError> {
        if !self.table_exists(VIEWS_TABLE)? {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, String)> =
            self.conn.query(format!("SELECT name, checksum FROM `{VIEWS_TABLE}` ORDER BY ord")).map_err(|e| my_err(e, None))?;
        Ok(rows.into_iter().map(|(name, checksum)| RecordedView { name, checksum }).collect())
    }

    /// Without transactions, so in an order that keeps the record true at every step: the record is cleared, each view
    /// dropped, then each view created and recorded. A failure leaves the views made so far recorded; `apply` again makes the rest.
    fn replace_views(&mut self, drop: &[String], create: &[ProjectView]) -> Result<(), ExecError> {
        self.conn
            .query_drop(format!(
                "CREATE TABLE IF NOT EXISTS `{VIEWS_TABLE}` (ord INT NOT NULL, name VARCHAR(255) PRIMARY KEY, checksum VARCHAR(64) NOT NULL, applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)))"
            ))
            .map_err(|e| my_err(e, None))?;
        self.conn.query_drop(format!("DELETE FROM `{VIEWS_TABLE}`")).map_err(|e| my_err(e, None))?;
        for name in drop {
            let sql = format!("DROP VIEW IF EXISTS {}", certo_sql::quote_ident(certo_sql::Dialect::Mysql, name));
            self.conn.query_drop(&sql).map_err(|e| my_err(e, Some(&sql)))?;
        }
        for (i, v) in create.iter().enumerate() {
            self.conn.query_drop(&v.create_sql).map_err(|e| my_err(e, Some(&v.create_sql)))?;
            self.conn
                .exec_drop(format!("INSERT INTO `{VIEWS_TABLE}` (ord, name, checksum) VALUES (?, ?, ?)"), (i as u32, &v.name, &v.checksum))
                .map_err(|e| my_err(e, Some("record the view")))?;
        }
        if let (Some(j), false) = (self.journal.clone(), create.is_empty()) {
            self.ensure_journal()?;
            let detail = serde_json::json!({ "views": create.iter().map(|v| v.name.clone()).collect::<Vec<_>>() }).to_string();
            self.conn
                .exec_drop(journal::mysql_insert(), ("views", format!("{} view(s)", create.len()), &j.actor, &j.environment, &j.tool, &detail))
                .map_err(|e| my_err(e, Some("write the journal")))?;
        }
        Ok(())
    }

    fn live_views(&mut self) -> Result<Vec<String>, ExecError> {
        self.conn
            .query("SELECT table_name FROM information_schema.views WHERE table_schema = DATABASE() ORDER BY table_name")
            .map_err(|e| my_err(e, None))
    }
}
