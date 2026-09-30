use crate::error::RunnerError;
use crate::exec::{AppliedRow, Executor};
use crate::migration::{list, Migration};
use crate::project::Project;

#[derive(Debug, Clone)]
pub struct Status {
    pub applied: Vec<AppliedRow>,
    /// `(seq, name)` of migrations on disk that have not been applied.
    pub pending: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Report what would run, without touching the database.
    pub dry_run: bool,
    /// Stop after this migration number.
    pub to: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub dry_run: bool,
    /// Labels (`0003_add_slug`) of migrations applied, or that would be.
    pub migrations: Vec<String>,
    /// For dry runs: the SQL each pending migration would execute.
    pub scripts: Vec<(String, String)>,
}

/// Check that the history is a faithful prefix of the files on disk.
/// Returns how many migrations are already applied.
pub(crate) fn reconcile(files: &[Migration], applied: &[AppliedRow]) -> Result<usize, RunnerError> {
    for (i, a) in applied.iter().enumerate() {
        let label = format!("{:04}_{}", a.seq, a.name);
        let Some(f) = files.get(i) else {
            return Err(RunnerError::Drift(format!(
                "{label} is recorded as applied but its files are missing from the migrations directory"
            )));
        };
        if f.seq != a.seq {
            return Err(RunnerError::Drift(format!(
                "history has {label} where the files have {}",
                f.label()
            )));
        }
        if f.name != a.name {
            return Err(RunnerError::Drift(format!(
                "{label} was applied under that name but is now called {}",
                f.label()
            )));
        }
        if f.checksum != a.checksum {
            return Err(RunnerError::Drift(format!(
                "{label} was modified after it was applied (checksum differs); restore it, and put further changes in a new migration"
            )));
        }
    }
    Ok(applied.len())
}

pub(crate) fn connection_err(e: crate::exec::ExecError) -> RunnerError {
    RunnerError::Connection(e.message)
}

/// A project's migrations are written for one dialect; refuse an executor for another.
pub(crate) fn check_dialect(project: &Project, exec: &dyn Executor) -> Result<(), RunnerError> {
    if project.config.dialect != exec.dialect() {
        return Err(RunnerError::Project(format!(
            "the project dialect is `{}` but this executor talks `{}` (the built-in executor is PostgreSQL only;              a `{}` project can still write migrations with `migrate new` and run the scripts with your own driver)",
            project.config.dialect, exec.dialect(), project.config.dialect
        )));
    }
    Ok(())
}

/// Compare the migration files with the database history. Never writes.
pub fn status(project: &Project, exec: &mut dyn Executor) -> Result<Status, RunnerError> {
    check_dialect(project, exec)?;
    let files = list(project)?;
    let applied = exec.applied().map_err(connection_err)?;
    let k = reconcile(&files, &applied)?;
    Ok(Status {
        applied,
        pending: files[k..].iter().map(|m| (m.seq, m.name.clone())).collect(),
    })
}

/// Apply pending migrations in order. Stops at the first failure; earlier
/// migrations stay applied and recorded.
pub fn apply(project: &Project, exec: &mut dyn Executor, opts: &ApplyOptions) -> Result<ApplyReport, RunnerError> {
    apply_with_progress(project, exec, opts, &mut |_| {})
}

/// Like `apply`, calling `progress(label)` after each migration is applied and
/// recorded, so a caller still knows how far it got when a later one fails.
pub fn apply_with_progress(
    project: &Project,
    exec: &mut dyn Executor,
    opts: &ApplyOptions,
    progress: &mut dyn FnMut(&str),
) -> Result<ApplyReport, RunnerError> {
    check_dialect(project, exec)?;
    let files = list(project)?;
    if let Some(to) = opts.to
        && !files.iter().any(|m| m.seq == to)
    {
        return Err(RunnerError::Project(format!("there is no migration numbered {to}")));
    }
    if !opts.dry_run {
        exec.ensure_history().map_err(connection_err)?;
    }
    let applied = exec.applied().map_err(connection_err)?;
    let k = reconcile(&files, &applied)?;

    let pending = files[k..].iter().filter(|m| opts.to.is_none_or(|to| m.seq <= to));
    let mut report = ApplyReport { dry_run: opts.dry_run, migrations: Vec::new(), scripts: Vec::new() };
    for m in pending {
        if opts.dry_run {
            report.scripts.push((m.label(), m.script.to_sql()));
        } else {
            exec.apply(m).map_err(|e| RunnerError::Database {
                seq: m.seq,
                name: m.name.clone(),
                statement: e.statement,
                message: e.message,
            })?;
            progress(&m.label());
        }
        report.migrations.push(m.label());
    }
    Ok(report)
}
