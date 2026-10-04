//! The project's persisted views: `view name { ... }` declarations in QL, kept in `views.ql` (or the `views` key of `certo-db.toml`).
//!
//! Views are not part of the schema or of a migration, so a schema diff never mentions them. The runner owns them instead:
//! - before an `apply` that has pending migrations it drops every view it created (a view stops a table it reads from being
//!   changed), and afterwards it creates them all again, in dependency order, in one transaction;
//! - it records each view and a checksum of its SQL in `_certo_views`, so an `apply` that changes nothing leaves them alone,
//!   and one after `views.ql` was edited recreates them;
//! - drift reports a view that is missing, or whose definition changed since it was applied, and a managed view is not
//!   reported as "left out" by introspection.
//!
//! If a migration fails half-way, the views stay dropped (and unrecorded) until the next successful `apply` creates them.

use crate::error::RunnerError;
use crate::project::Project;
use certo_diagnostics::render_all;
use sha2::{Digest, Sha256};
use std::fs;

pub const VIEWS_TABLE: &str = "_certo_views";

/// A view the project defines, ready to create.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectView {
    pub name: String,
    pub create_sql: String,
    pub drop_sql: String,
    /// SHA-256 (hex) of `create_sql`.
    pub checksum: String,
}

/// A view the runner created, as the database remembers it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedView {
    pub name: String,
    pub checksum: String,
}

/// Compile the project's views against the schema as of the last migration (`IR.json`), in the order they must be created.
/// No views file means no views.
pub fn load(project: &Project) -> Result<Vec<ProjectView>, RunnerError> {
    let path = project.views_path();
    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(crate::error::io(&path, e)),
    };
    let label = project.config.views.clone().unwrap_or_else(|| "views.ql".to_string());
    let state = project.state_ir()?;
    let compiled = certo_ql::compile_full(&state, &src, project.dialect());
    if compiled.statements.is_none() {
        let rendered = render_all(&compiled.diagnostics, &src, &label, false);
        return Err(RunnerError::Compile { file: label, rendered, diagnostics: compiled.diagnostics, source: src });
    }
    Ok(compiled
        .views
        .into_iter()
        .map(|v| ProjectView { checksum: hex(&v.create_sql), name: v.name, create_sql: v.create_sql, drop_sql: v.drop_sql })
        .collect())
}

fn hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Is the database already exactly what the project defines (same views, same definitions, same order)?
pub fn in_sync(project: &[ProjectView], recorded: &[RecordedView]) -> bool {
    project.len() == recorded.len() && project.iter().zip(recorded).all(|(p, r)| p.name == r.name && p.checksum == r.checksum)
}

/// What to drop to clear the recorded views: all of them, the last created first.
pub fn drop_order(recorded: &[RecordedView]) -> Vec<String> {
    recorded.iter().rev().map(|r| r.name.clone()).collect()
}
