//! The database, behind a trait so the runner logic is testable without one.

use crate::error::RunnerError;
use crate::introspect::{self, LiveSchema};
use crate::journal::{self, JournalContext};
use crate::views::{ProjectView, RecordedView, VIEWS_TABLE};
use crate::migration::Migration;
use postgres::{Client, NoTls};
use std::fmt;

const HISTORY_TABLE: &str = "_certo_migrations";
/// Advisory-lock key ("certo" as digits); keeps two runners from interleaving.
const LOCK_KEY: i64 = 0x0063_6572_746f;

/// One row of the history table.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedRow {
    pub seq: u32,
    pub name: String,
    pub checksum: String,
    pub applied_at: String,
}

/// A migration MySQL started and did not finish (it commits DDL as it goes): `done` of its `total` statements ran.
#[derive(Debug, Clone, PartialEq)]
pub struct PartialRow {
    pub seq: u32,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug)]
pub struct ExecError {
    /// The statement that failed, when known.
    pub statement: Option<String>,
    pub message: String,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.message) }
}

pub trait Executor {
    /// The SQL dialect this executor talks (`Project` dialect names).
    fn dialect(&self) -> &'static str {
        "postgres"
    }
    /// Create the history table if it does not exist.
    fn ensure_history(&mut self) -> Result<(), ExecError>;
    /// Applied migrations in order. Empty (not an error) if the history table
    /// does not exist yet, so read-only commands never write.
    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError>;
    /// Run every batch of `m` and record it in the history. Transactional
    /// batches are atomic, and the history row is written inside the last one,
    /// so a migration is either fully applied and recorded, or not at all
    /// (apart from earlier non-transactional batches, which are idempotent).
    fn apply(&mut self, m: &Migration) -> Result<(), ExecError>;
    /// Record `m` as applied WITHOUT running it (used to adopt an existing
    /// database, whose baseline migration describes what is already there).
    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError>;
    /// Read the live schema (current schema only) back into an IR. Read-only.
    fn introspect(&mut self) -> Result<LiveSchema, ExecError>;
    /// Turn the journal on (`Some`) or off for the calls that follow: each `apply` and `record_applied` then also
    /// writes a row to `_certo_log`, inside the same transaction as the change. Off unless a host asks.
    fn set_journal(&mut self, _journal: Option<JournalContext>) {}
    /// Migrations begun and not finished (only a database without transactional DDL, MySQL, can have them).
    fn partial(&mut self) -> Result<Vec<PartialRow>, ExecError> { Ok(Vec::new()) }
    /// The views certo created, in the order it created them. Empty if it never created any.
    fn recorded_views(&mut self) -> Result<Vec<RecordedView>, ExecError> { Ok(Vec::new()) }
    /// In ONE transaction: drop the views named (in the order given), forget every recorded view, create `create` in order and
    /// record each with its checksum. With a journal, a `views` row is written too.
    fn replace_views(&mut self, _drop: &[String], _create: &[ProjectView]) -> Result<(), ExecError> { Ok(()) }
    /// The names of the views the database has (current schema), whoever made them.
    fn live_views(&mut self) -> Result<Vec<String>, ExecError> { Ok(Vec::new()) }
}

pub struct PgExecutor {
    client: Client,
    journal: Option<JournalContext>,
}

fn pg_err(e: postgres::Error, statement: Option<&str>) -> ExecError {
    let message = match e.as_db_error() {
        Some(d) => match d.detail() {
            Some(detail) => format!("{} ({detail})", d.message()),
            None => d.message().to_string(),
        },
        None => e.to_string(),
    };
    ExecError { statement: statement.map(str::to_string), message }
}

impl PgExecutor {
    /// Connect with a `postgres://` URL. `sslmode=require` uses TLS.
    pub fn connect(url: &str) -> Result<PgExecutor, RunnerError> {
        let conn_err = |e: String| RunnerError::Connection(format!("could not connect to the database: {e}"));
        let config: postgres::Config = url.parse().map_err(|e: postgres::Error| conn_err(e.to_string()))?;
        let mut client = if config.get_ssl_mode() == postgres::config::SslMode::Require {
            let tls = native_tls::TlsConnector::builder().build().map_err(|e| conn_err(e.to_string()))?;
            config.connect(postgres_native_tls::MakeTlsConnector::new(tls))
        } else {
            config.connect(NoTls)
        }
        .map_err(|e| conn_err(e.as_db_error().map_or_else(|| e.to_string(), |d| d.message().to_string())))?;

        let row = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK_KEY])
            .map_err(|e| conn_err(e.to_string()))?;
        if !row.get::<_, bool>(0) {
            return Err(RunnerError::Connection(
                "another migration run holds the lock on this database; wait for it to finish".into(),
            ));
        }
        Ok(PgExecutor { client, journal: None })
    }

    fn record_sql() -> String {
        format!(
            "INSERT INTO \"{HISTORY_TABLE}\" (seq, name, checksum, compiler_version) VALUES ($1, $2, $3, $4)"
        )
    }
}

impl Executor for PgExecutor {
    fn ensure_history(&mut self) -> Result<(), ExecError> {
        self.client
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS \"{HISTORY_TABLE}\" (
                    seq integer PRIMARY KEY,
                    name text NOT NULL,
                    checksum text NOT NULL,
                    compiler_version text NOT NULL,
                    applied_at timestamptz NOT NULL DEFAULT now()
                )"
            ))
            .map_err(|e| pg_err(e, None))
    }

    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError> {
        let exists = self
            .client
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&format!("\"{HISTORY_TABLE}\"")])
            .map_err(|e| pg_err(e, None))?
            .get::<_, bool>(0);
        if !exists {
            return Ok(Vec::new());
        }
        let rows = self
            .client
            .query(
                &format!("SELECT seq, name, checksum, applied_at::text FROM \"{HISTORY_TABLE}\" ORDER BY seq"),
                &[],
            )
            .map_err(|e| pg_err(e, None))?;
        Ok(rows
            .iter()
            .map(|r| AppliedRow {
                seq: r.get::<_, i32>(0) as u32,
                name: r.get(1),
                checksum: r.get(2),
                applied_at: r.get(3),
            })
            .collect())
    }

    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError> {
        self.ensure_history()?;
        let params: [&(dyn postgres::types::ToSql + Sync); 4] = [&(m.seq as i32), &m.name, &m.checksum, &m.script.compiler_version];
        let Some(j) = self.journal.clone() else {
            self.client.execute(&Self::record_sql(), &params).map_err(|e| pg_err(e, Some("record migration in history")))?;
            return Ok(());
        };
        // the baseline and its journal row commit together
        self.client.batch_execute(&journal::pg_create()).map_err(|e| pg_err(e, Some("create the journal table")))?;
        let (label, detail) = (m.label(), journal::detail(m));
        let mut tx = self.client.transaction().map_err(|e| pg_err(e, None))?;
        tx.execute(&Self::record_sql(), &params).map_err(|e| pg_err(e, Some("record migration in history")))?;
        tx.execute(&journal::pg_insert(), &[&"adopt", &label, &j.actor, &j.environment, &j.tool, &detail])
            .map_err(|e| pg_err(e, Some("write the journal")))?;
        tx.commit().map_err(|e| pg_err(e, Some("COMMIT")))
    }

    fn set_journal(&mut self, journal: Option<JournalContext>) { self.journal = journal; }

    fn recorded_views(&mut self) -> Result<Vec<RecordedView>, ExecError> {
        let exists = self
            .client
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&format!("\"{VIEWS_TABLE}\"")])
            .map_err(|e| pg_err(e, None))?
            .get::<_, bool>(0);
        if !exists {
            return Ok(Vec::new());
        }
        let rows = self
            .client
            .query(&format!("SELECT name, checksum FROM \"{VIEWS_TABLE}\" ORDER BY ord"), &[])
            .map_err(|e| pg_err(e, None))?;
        Ok(rows.iter().map(|r| RecordedView { name: r.get(0), checksum: r.get(1) }).collect())
    }

    fn replace_views(&mut self, drop: &[String], create: &[ProjectView]) -> Result<(), ExecError> {
        self.client
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS \"{VIEWS_TABLE}\" (ord integer NOT NULL, name text PRIMARY KEY, checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT now())"
            ))
            .map_err(|e| pg_err(e, None))?;
        let journal = self.journal.clone();
        if journal.is_some() {
            self.client.batch_execute(&journal::pg_create()).map_err(|e| pg_err(e, Some("create the journal table")))?;
        }
        let mut tx = self.client.transaction().map_err(|e| pg_err(e, None))?;
        for name in drop {
            let sql = format!("DROP VIEW IF EXISTS {}", certo_sql::quote_ident(certo_sql::Dialect::Postgres, name));
            tx.batch_execute(&sql).map_err(|e| pg_err(e, Some(&sql)))?;
        }
        tx.batch_execute(&format!("DELETE FROM \"{VIEWS_TABLE}\"")).map_err(|e| pg_err(e, None))?;
        for (i, v) in create.iter().enumerate() {
            tx.batch_execute(&v.create_sql).map_err(|e| pg_err(e, Some(&v.create_sql)))?;
            tx.execute(&format!("INSERT INTO \"{VIEWS_TABLE}\" (ord, name, checksum) VALUES ($1, $2, $3)"), &[&(i as i32), &v.name, &v.checksum])
                .map_err(|e| pg_err(e, Some("record the view")))?;
        }
        if let (Some(j), false) = (&journal, create.is_empty()) {
            let detail = serde_json::json!({ "views": create.iter().map(|v| v.name.clone()).collect::<Vec<_>>() }).to_string();
            tx.execute(&journal::pg_insert(), &[&"views", &format!("{} view(s)", create.len()), &j.actor, &j.environment, &j.tool, &detail])
                .map_err(|e| pg_err(e, Some("write the journal")))?;
        }
        tx.commit().map_err(|e| pg_err(e, Some("COMMIT")))
    }

    fn live_views(&mut self) -> Result<Vec<String>, ExecError> {
        let rows = self
            .client
            .query(
                "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = current_schema() AND c.relkind = 'v' ORDER BY c.relname",
                &[],
            )
            .map_err(|e| pg_err(e, None))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    fn introspect(&mut self) -> Result<LiveSchema, ExecError> {
        let mut live = introspect::introspect(&mut self.client).map_err(|e| pg_err(e, None))?;
        // a view certo created is part of the project, not something the schema language failed to express
        let managed = self.recorded_views()?;
        live.notes.retain(|n| !managed.iter().any(|m| n.starts_with(&format!("view {} is not represented", m.name))));
        Ok(live)
    }

    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        let (seq, name, sum, ver) = (
            m.seq as i32,
            m.name.clone(),
            m.checksum.clone(),
            m.script.compiler_version.clone(),
        );
        // the history row (and the journal row, if the journal is on) goes inside the last transactional batch, so
        // it commits with the change
        let last_tx = m.script.batches.iter().rposition(|b| b.transactional);
        let journal = self.journal.clone();
        let (label, detail) = (m.label(), journal::detail(m));
        if journal.is_some() {
            self.client.batch_execute(&journal::pg_create()).map_err(|e| pg_err(e, Some("create the journal table")))?;
        }

        for (i, b) in m.script.batches.iter().enumerate() {
            if b.transactional {
                let mut tx = self.client.transaction().map_err(|e| pg_err(e, None))?;
                for s in &b.statements {
                    tx.batch_execute(s).map_err(|e| pg_err(e, Some(s)))?;
                }
                if Some(i) == last_tx {
                    tx.execute(&Self::record_sql(), &[&seq, &name, &sum, &ver])
                        .map_err(|e| pg_err(e, Some("record migration in history")))?;
                    if let Some(j) = &journal {
                        tx.execute(&journal::pg_insert(), &[&"apply", &label, &j.actor, &j.environment, &j.tool, &detail])
                            .map_err(|e| pg_err(e, Some("write the journal")))?;
                    }
                }
                tx.commit().map_err(|e| pg_err(e, Some("COMMIT")))?;
            } else {
                for s in &b.statements {
                    self.client.batch_execute(s).map_err(|e| pg_err(e, Some(s)))?;
                }
            }
        }
        if last_tx.is_none() {
            self.client
                .execute(&Self::record_sql(), &[&seq, &name, &sum, &ver])
                .map_err(|e| pg_err(e, Some("record migration in history")))?;
            if let Some(j) = &journal {
                self.client
                    .execute(&journal::pg_insert(), &[&"apply", &label, &j.actor, &j.environment, &j.tool, &detail])
                    .map_err(|e| pg_err(e, Some("write the journal")))?;
            }
        }
        Ok(())
    }
}

impl Executor for Box<dyn Executor> {
    fn dialect(&self) -> &'static str { (**self).dialect() }
    fn ensure_history(&mut self) -> Result<(), ExecError> { (**self).ensure_history() }
    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError> { (**self).applied() }
    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> { (**self).apply(m) }
    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError> { (**self).record_applied(m) }
    fn introspect(&mut self) -> Result<LiveSchema, ExecError> { (**self).introspect() }
    fn set_journal(&mut self, journal: Option<JournalContext>) { (**self).set_journal(journal) }
    fn partial(&mut self) -> Result<Vec<PartialRow>, ExecError> { (**self).partial() }
    fn recorded_views(&mut self) -> Result<Vec<RecordedView>, ExecError> { (**self).recorded_views() }
    fn replace_views(&mut self, drop: &[String], create: &[ProjectView]) -> Result<(), ExecError> { (**self).replace_views(drop, create) }
    fn live_views(&mut self) -> Result<Vec<String>, ExecError> { (**self).live_views() }
}

/// Connect to the database `target` names, with the executor for the project's
/// dialect: a `postgres://` or `mysql://` URL, or for SQLite a file path / `sqlite:<path>`.
pub fn connect(project: &crate::project::Project, target: &str) -> Result<Box<dyn Executor>, RunnerError> {
    connect_to(&project.config.dialect, target)
}

/// `connect` for a database of `dialect` (`"postgres"` or `"sqlite"`), with no project.
pub fn connect_to(dialect: &str, target: &str) -> Result<Box<dyn Executor>, RunnerError> {
    match dialect {
        "sqlite" => Ok(Box::new(crate::sqlite_exec::SqliteExecutor::open(target)?)),
        "mysql" => Ok(Box::new(crate::mysql_exec::MysqlExecutor::connect(target)?)),
        _ => Ok(Box::new(PgExecutor::connect(target)?)),
    }
}
