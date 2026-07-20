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
/// `state.applied` is included.
pub fn plan_up<'a>(
    migrations: &'a [MigrationDecl],
    state: &MigrationState,
) -> Vec<MigrationStep<'a>> {
    migrations.iter()
        .filter(|m| !state.is_applied(&m.name))
        .map(|m| MigrationStep { migration: m, direction: Direction::Up })
        .collect()
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
