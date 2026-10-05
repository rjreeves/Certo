use crate::error::RunnerError;
use crate::exec::{AppliedRow, Executor};
use crate::migration::{list, Migration};
use crate::project::Project;

#[derive(Debug, Clone)]
pub struct Status {
    pub applied: Vec<AppliedRow>,
    /// `(seq, name)` of migrations on disk that have not been applied.
    pub pending: Vec<(u32, String)>,
    /// The project's views (`views.ql`) against what the database has recorded.
    pub views: ViewsStatus,
    /// Migrations begun and not finished (MySQL commits DDL as it goes): how far each got.
    pub partial: Vec<crate::exec::PartialRow>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewsStatus {
    /// Names, in creation order.
    pub defined: Vec<String>,
    pub recorded: Vec<String>,
    /// The database has exactly the views the project defines, with the same definitions.
    pub in_sync: bool,
    /// The views file does not compile: what the compiler said (the other fields are then empty).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Report what would run, without touching the database.
    pub dry_run: bool,
    /// Stop after this migration number.
    pub to: Option<u32>,
    /// Write each applied migration to the journal (`_certo_log`), inside its transaction.
    pub journal: Option<crate::journal::JournalContext>,
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub dry_run: bool,
    /// Labels (`0003_add_slug`) of migrations applied, or that would be.
    pub migrations: Vec<String>,
    /// For dry runs: the SQL each pending migration would execute (and, last, what would happen to the views).
    pub scripts: Vec<(String, String)>,
    /// The views this run created (again), in the order created.
    pub views: Vec<String>,
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
            "the project dialect is `{}` but this executor talks `{}` (connect with a URL for the project's database: postgres://, mysql://, or a file for SQLite)",
            project.config.dialect, exec.dialect()
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
    let recorded = exec.recorded_views().map_err(connection_err)?;
    let pending = files[k..].iter().map(|m| (m.seq, m.name.clone())).collect();
    // a mistake in the views file is reported, not fatal: the migrations still say what they say
    let views = match crate::views::load(project) {
        Ok(defined) => ViewsStatus {
            in_sync: crate::views::in_sync(&defined, &recorded),
            defined: defined.iter().map(|v| v.name.clone()).collect(),
            recorded: recorded.iter().map(|v| v.name.clone()).collect(),
            error: None,
        },
        Err(RunnerError::Compile { rendered, .. }) => ViewsStatus {
            defined: Vec::new(),
            recorded: recorded.iter().map(|v| v.name.clone()).collect(),
            in_sync: false,
            error: Some(rendered),
        },
        Err(e) => return Err(e),
    };
    let partial = exec.partial().map_err(connection_err)?;
    Ok(Status { applied, pending, views, partial })
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
        exec.set_journal(opts.journal.clone());
    }
    let applied = exec.applied().map_err(connection_err)?;
    let k = reconcile(&files, &applied)?;

    let views = crate::views::load(project)?; // a mistake in views.ql stops everything before anything runs
    let recorded = exec.recorded_views().map_err(connection_err)?;
    let pending: Vec<&Migration> = files[k..].iter().filter(|m| opts.to.is_none_or(|to| m.seq <= to)).collect();
    // A view stops a table it reads from being changed, so with migrations to run the views certo made go first and are
    // created again afterwards. Stopping short with `to` leaves the views dropped: they are written against the latest schema.
    let drop_first = !pending.is_empty() && !recorded.is_empty();
    let complete = files.len() == k + pending.len();
    let view_error = |e: crate::exec::ExecError| RunnerError::Database { seq: 0, name: "views".into(), statement: e.statement, message: e.message };
    let mut report = ApplyReport { dry_run: opts.dry_run, migrations: Vec::new(), scripts: Vec::new(), views: Vec::new() };

    if drop_first && !opts.dry_run {
        exec.replace_views(&crate::views::drop_order(&recorded), &[]).map_err(view_error)?;
    }
    for m in &pending {
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
    let now: Vec<crate::views::RecordedView> = if drop_first { Vec::new() } else { recorded };
    // also when a view certo made was dropped by hand: what is recorded is not what exists
    let live_now = exec.live_views().map_err(connection_err)?;
    let intact = views.iter().all(|v| live_now.contains(&v.name));
    if complete && (!crate::views::in_sync(&views, &now) || !intact) {
        if opts.dry_run {
            let mut sql = String::new();
            for name in crate::views::drop_order(&now) {
                sql.push_str(&format!("DROP VIEW IF EXISTS {};\n", certo_sql::quote_ident(project.dialect(), &name)));
            }
            for v in &views {
                sql.push_str(&v.create_sql);
                sql.push_str(";\n");
            }
            report.scripts.push(("views".to_string(), sql));
        } else {
            exec.replace_views(&crate::views::drop_order(&now), &views).map_err(view_error)?;
        }
        report.views = views.iter().map(|v| v.name.clone()).collect();
    }
    Ok(report)
}
