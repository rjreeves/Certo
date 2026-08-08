pub mod error;
pub mod sql_gen;
pub mod state;
pub mod plan;
pub mod runner;

pub use error::MigrateError;
pub use sql_gen::op_to_sql;
pub use state::{MigrationState, default_manifest_path};
pub use plan::{plan_up, plan_down, Direction, MigrationStep};
pub use runner::{plan_sql, commit_steps, status};

#[cfg(test)]
mod tests;
