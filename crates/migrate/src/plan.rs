use certo_ast::decl::MigrationDecl;
use crate::state::MigrationState;

/// A single migration step to execute.
#[derive(Debug, Clone)]
pub struct MigrationStep<'a> {
    pub migration: &'a MigrationDecl,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { Up, Down }

/// Compute the list of migration steps needed to bring the database up to date.
///
/// Migrations are applied in declaration order. Any migration not yet in
/// `state.applied` is included. `step_limit` (BACKLOG item 265 — spec
/// §11.2's own documented `certo db migrate | --dry-run --step 1` row) caps
/// how many of those pending migrations are actually applied, in the same
/// declaration order — `None` means "all of them," matching this
/// function's own pre-item-265 behavior exactly, so every existing caller
/// that doesn't care about `--step` is unaffected.
pub fn plan_up<'a>(
    migrations: &'a [MigrationDecl],
    state: &MigrationState,
    step_limit: Option<usize>,
) -> Vec<MigrationStep<'a>> {
    let pending = migrations.iter()
        .filter(|m| !state.is_applied(&m.name))
        .map(|m| MigrationStep { migration: m, direction: Direction::Up });
    match step_limit {
        Some(n) => pending.take(n).collect(),
        None => pending.collect(),
    }
}

/// Compute the steps needed to roll back `n` migrations (most-recently-applied first).
pub fn plan_down<'a>(
    migrations: &'a [MigrationDecl],
    state: &MigrationState,
    count: usize,
) -> Vec<MigrationStep<'a>> {
    // Walk applied list in reverse, find matching declarations.
    let applied: Vec<&str> = state.applied.iter().rev().map(|a| a.name.as_str()).collect();
    let mut steps = Vec::new();
    for name in applied.iter().take(count) {
        if let Some(m) = migrations.iter().find(|m| m.name == *name) {
            steps.push(MigrationStep { migration: m, direction: Direction::Down });
        }
    }
    steps
}
