use std::path::Path;
use certo_ast::decl::MigrationDecl;
use crate::error::MigrateError;
use crate::plan::{Direction, MigrationStep};
use crate::sql_gen::op_to_sql;
use crate::state::MigrationState;

pub struct RunOptions<'a> {
    pub dry_run:       bool,
    pub manifest_path: &'a Path,
}

/// Execute a list of migration steps, updating state unless dry-run.
///
/// Returns the SQL statements that were (or would be) executed.
pub fn run_steps(
    steps: &[MigrationStep<'_>],
    opts:  &RunOptions<'_>,
) -> Result<Vec<String>, MigrateError> {
    let mut state = MigrationState::load(opts.manifest_path)?;
    let mut all_sql: Vec<String> = Vec::new();

    for step in steps {
        let ops = match step.direction {
            Direction::Up   => &step.migration.up,
            Direction::Down => &step.migration.down,
        };
        let sql_stmts: Vec<String> = ops.iter().map(op_to_sql).collect();
        all_sql.extend(sql_stmts);

        if !opts.dry_run {
            match step.direction {
                Direction::Up   => state.mark_applied(&step.migration.name),
                Direction::Down => state.mark_rolled_back(&step.migration.name),
            }
        }
    }

    if !opts.dry_run {
        state.save(opts.manifest_path)?;
    }

    Ok(all_sql)
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
