//! MDL foundation: diff two SchemaIRs into an ordered, engine-agnostic
//! migration plan.
//!
//! The plan describes *what* changes, never SQL; engine adapters lower it.
//! Renames cannot be inferred from two snapshots, so by default a rename
//! appears as a drop plus an add (flagged destructive). An MDL migration
//! (`lang`, `migration`) states the intent explicitly: renames, enum value
//! remaps, backfills, and hand-written data or SQL steps.

mod diff;
pub mod lang;
mod migration;
mod plan;

pub use diff::diff;
pub use migration::{compile_migration, plan_migration, KNOWN_DIALECTS};
pub use plan::{action_sql, generation_lossy, widens, Assignment, EnumColumn, MigrationPlan, Op, VariantMapping, PLAN_VERSION};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod migration_tests;
