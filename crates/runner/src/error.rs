use certo_diagnostics::Diagnostic;
use certo_sql::LowerError;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum RunnerError {
    Io { path: PathBuf, message: String },
    /// Missing or malformed project config, or unusable project state.
    Project(String),
    /// The schema (or MDL) has errors; `rendered` is ready to print.
    Compile { file: String, rendered: String, diagnostics: Vec<Diagnostic>, source: String },
    /// Nothing differs between `IR.json` and `schema.sdl`.
    NoChanges,
    /// The plan can lose data and `--allow-destructive` was not given.
    Destructive(Vec<String>),
    /// The dialect cannot express part of the plan.
    Unsupported(LowerError),
    /// Migration files and history disagree (edited, missing, renamed, gap).
    Drift(String),
    /// The live database no longer matches the applied migrations.
    SchemaDrift(String),
    /// A statement failed; the migration's transaction was rolled back.
    Database { seq: u32, name: String, statement: Option<String>, message: String },
    /// Could not connect or talk to the database outside a migration.
    Connection(String),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnerError::Io { path, message } => write!(f, "{}: {message}", path.display()),
            RunnerError::Project(m) | RunnerError::Connection(m) => write!(f, "{m}"),
            RunnerError::Compile { file, rendered, .. } => write!(f, "{rendered}{file}: has errors"),
            RunnerError::NoChanges => write!(f, "no changes: schema.sdl matches the last migration"),
            RunnerError::Destructive(ops) => write!(
                f,
                "the migration contains destructive operations (pass --allow-destructive to accept):\n  {}",
                ops.join("\n  ")
            ),
            RunnerError::Unsupported(e) => write!(f, "{e}"),
            RunnerError::Drift(m) => write!(f, "migration history problem: {m}"),
            RunnerError::SchemaDrift(m) => write!(f, "database drift: {m}"),
            RunnerError::Database { seq, name, statement, message } => {
                // MySQL cannot roll DDL back: its message says how to resume instead
                let outcome = if message.contains(crate::mysql_exec::RESUME_HINT) { "failed" } else { "failed and was rolled back" };
                write!(f, "migration {seq:04}_{name} {outcome}: {message}")?;
                if let Some(s) = statement {
                    write!(f, "\n  statement: {s}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RunnerError {}

pub(crate) fn io(path: impl Into<PathBuf>, e: impl fmt::Display) -> RunnerError {
    RunnerError::Io { path: path.into(), message: e.to_string() }
}
