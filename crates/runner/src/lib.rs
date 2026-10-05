//! Migration runner.
//!
//! On disk (all under the project root):
//!
//! ```text
//! certo-db.toml            config: dialect, schema path, migrations dir
//! schema.sdl               the schema you edit
//! IR.json                  the schema as of the last generated migration
//! migrations/
//!   0001_init/
//!     up.json              EXECUTABLE truth: SQL batches (checksummed)
//!     up.sql               human-readable copy of up.json (never executed)
//!     plan.json            the engine-agnostic plan it was lowered from
//!     ir.json              the schema after this migration
//!     migration.mdl        the MDL steering it (only if one was given)
//! ```
//!
//! Workflow: edit `schema.sdl`, `create` a migration (diff against `IR.json`,
//! lowered to SQL and frozen on disk), review it, then `apply` it. Applied
//! migrations are recorded in a history table with their checksum, so a
//! migration edited after it ran, a missing file, or a gap is caught before
//! anything else runs.
//!
//! Layers: `project` (files), `migration` (create / list), `exec` (the
//! database behind a trait), `runner` (apply / status logic).

pub mod adopt;
pub mod drift;
pub mod error;
pub mod exec;
pub mod introspect;
pub mod journal;
pub mod views;
pub mod migration;
pub mod mysql_exec;
pub mod mysql_introspect;
pub mod mysqlexpr;
pub mod pgexpr;
pub mod project;
pub mod runner;
pub mod sqlite_exec;
pub mod sqlite_introspect;

pub use adopt::{adopt, import_schema, AdoptOptions, AdoptReport, Counts, Prepared};
pub use drift::{Drift, DriftItem, DriftKind};
pub use error::RunnerError;
pub use exec::{connect, connect_to, AppliedRow, ExecError, Executor, PartialRow, PgExecutor};
pub use mysql_exec::MysqlExecutor;
pub use journal::JournalContext;
pub use views::{ProjectView, RecordedView};
pub use migration::{Created, Migration, Script};
pub use project::{Config, Project};
pub use sqlite_exec::SqliteExecutor;
pub use runner::{apply, apply_with_progress, status, ApplyOptions, ApplyReport, Status};

#[cfg(test)]
mod tests;
