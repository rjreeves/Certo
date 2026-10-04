//! The database, behind a trait so the runner logic is testable without one.

use crate::error::RunnerError;
use crate::introspect::{self, LiveSchema};
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
}

pub struct PgExecutor {
    client: Client,
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
        Ok(PgExecutor { client })
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
        self.client
            .execute(
                &Self::record_sql(),
                &[&(m.seq as i32), &m.name, &m.checksum, &m.script.compiler_version],
            )
            .map_err(|e| pg_err(e, Some("record migration in history")))?;
        Ok(())
    }

    fn introspect(&mut self) -> Result<LiveSchema, ExecError> {
        introspect::introspect(&mut self.client).map_err(|e| pg_err(e, None))
    }

    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        let (seq, name, sum, ver) = (
            m.seq as i32,
            m.name.clone(),
            m.checksum.clone(),
            m.script.compiler_version.clone(),
        );
        // the history row goes inside the last transactional batch, so it commits with the change
        let last_tx = m.script.batches.iter().rposition(|b| b.transactional);

        for (i, b) in m.script.batches.iter().enumerate() {
            if b.transactional {
                let mut tx = self.client.transaction().map_err(|e| pg_err(e, None))?;
                for s in &b.statements {
                    tx.batch_execute(s).map_err(|e| pg_err(e, Some(s)))?;
                }
                if Some(i) == last_tx {
                    tx.execute(&Self::record_sql(), &[&seq, &name, &sum, &ver])
                        .map_err(|e| pg_err(e, Some("record migration in history")))?;
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
}

/// Connect to the database `target` names, with the executor for the project's
/// dialect: a `postgres://` URL, or for SQLite a file path / `sqlite:<path>`.
pub fn connect(project: &crate::project::Project, target: &str) -> Result<Box<dyn Executor>, RunnerError> {
    connect_to(&project.config.dialect, target)
}

/// `connect` for a database of `dialect` (`"postgres"` or `"sqlite"`), with no project.
pub fn connect_to(dialect: &str, target: &str) -> Result<Box<dyn Executor>, RunnerError> {
    match dialect {
        "sqlite" => Ok(Box::new(crate::sqlite_exec::SqliteExecutor::open(target)?)),
        _ => Ok(Box::new(PgExecutor::connect(target)?)),
    }
}
