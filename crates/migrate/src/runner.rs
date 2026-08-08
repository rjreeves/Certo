use std::path::Path;
use certo_ast::decl::MigrationDecl;
use crate::error::MigrateError;
use crate::plan::{Direction, MigrationStep};
use crate::sql_gen::op_to_sql;
use crate::state::MigrationState;

/// Generate the SQL statements for a set of migration steps. Pure — no state
/// mutation, no execution against any database. Used both for `--dry-run`
/// (print and stop) and as the first half of a real run (generate, then the
/// caller actually executes it before calling `commit_steps`).
pub fn plan_sql(steps: &[MigrationStep<'_>]) -> Vec<String> {
    steps.iter()
        .flat_map(|step| {
            let ops: &[_] = match step.direction {
                Direction::Up   => &step.migration.up,
                Direction::Down => &step.migration.down,
            };
            ops.iter().map(op_to_sql).collect::<Vec<_>>()
        })
        .collect()
}

/// Records that `steps` were actually applied/rolled back, persisting to
/// `manifest_path`. Call this only *after* the SQL from `plan_sql` has been
/// executed successfully against the real database — never speculatively,
/// since the persisted state is what `certo db status` and future migration
/// planning both trust.
pub fn commit_steps(steps: &[MigrationStep<'_>], manifest_path: &Path) -> Result<(), MigrateError> {
    let mut state = MigrationState::load(manifest_path)?;
    for step in steps {
        match step.direction {
            Direction::Up   => state.mark_applied(&step.migration.name),
            Direction::Down => state.mark_rolled_back(&step.migration.name),
        }
    }
    state.save(manifest_path)
}

/// Produce a status table: (name, applied_at or None).
pub fn status(
    migrations: &[MigrationDecl],
    manifest_path: &Path,
) -> Result<Vec<(String, Option<String>)>, MigrateError> {
    let state = MigrationState::load(manifest_path)?;
    Ok(migrations.iter().map(|m| {
        let applied_at = state.applied.iter()
            .find(|a| a.name == m.name)
            .map(|a| a.applied_at.clone());
        (m.name.clone(), applied_at)
    }).collect())
}
