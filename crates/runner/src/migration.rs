use crate::error::{io, RunnerError};
use crate::project::{write_synced, Project};
use certo_diagnostics::{render_all, Diagnostic, Severity};
use certo_mdl::{compile_migration, diff, MigrationPlan};
use certo_sdl::{compile, SchemaIR};
use certo_sql::{lower_batches_with, Schemas};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The executable content of a migration, frozen when it was created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub dialect: String,
    pub compiler_version: String,
    pub batches: Vec<BatchJson>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchJson {
    /// One transaction (`true`) or each statement on its own (`false`).
    pub transactional: bool,
    pub statements: Vec<String>,
}

impl Script {
    /// Readable rendering for `up.sql` and dry runs. Never executed.
    pub fn to_sql(&self) -> String {
        let mut lines = Vec::new();
        for b in &self.batches {
            if b.transactional { lines.push("BEGIN;".to_string()); }
            lines.extend(b.statements.iter().cloned());
            if b.transactional { lines.push("COMMIT;".to_string()); }
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone)]
pub struct Migration {
    pub seq: u32,
    pub name: String,
    pub dir: PathBuf,
    pub script: Script,
    /// SHA-256 (hex) of `up.json` exactly as stored.
    pub checksum: String,
}

impl Migration {
    pub fn label(&self) -> String { format!("{:04}_{}", self.seq, self.name) }
}

/// Result of `create`.
#[derive(Debug, Clone)]
pub struct Created {
    pub seq: u32,
    pub name: String,
    pub dir: PathBuf,
    /// One line per plan operation (`+` add, `~` change, `-` remove).
    pub summary: Vec<String>,
    pub destructive: bool,
    /// Rendered compiler warnings, if any.
    pub warnings: String,
}

pub fn checksum(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    s.split('_').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("_")
}

/// `0007_add_slug` -> (7, "add_slug")
fn parse_dir_name(n: &str) -> Option<(u32, String)> {
    let (num, rest) = n.split_once('_')?;
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) || rest.is_empty() {
        return None;
    }
    Some((num.parse().ok()?, rest.to_string()))
}

/// All migrations on disk, in order. Numbering must be 1, 2, 3, ... with no gaps.
pub fn list(project: &Project) -> Result<Vec<Migration>, RunnerError> {
    let dir = project.migrations_dir();
    let Ok(entries) = fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut found: Vec<(u32, String, PathBuf)> = Vec::new();
    for e in entries {
        let e = e.map_err(|e| io(&dir, e))?;
        if !e.path().is_dir() { continue; }
        let fname = e.file_name().to_string_lossy().to_string();
        if let Some((seq, name)) = parse_dir_name(&fname) {
            found.push((seq, name, e.path()));
        }
    }
    found.sort_by_key(|f| f.0);

    let mut out = Vec::new();
    for (i, (seq, name, path)) in found.into_iter().enumerate() {
        let expected = i as u32 + 1;
        if seq != expected {
            return Err(RunnerError::Project(format!(
                "migration numbering is broken in {}: expected {expected:04}, found {seq:04}_{name}",
                dir.display()
            )));
        }
        let up = path.join("up.json");
        let bytes = fs::read(&up).map_err(|e| io(&up, e))?;
        let script: Script = serde_json::from_slice(&bytes)
            .map_err(|e| RunnerError::Project(format!("{}: {e}", up.display())))?;
        if script.dialect != project.config.dialect {
            return Err(RunnerError::Project(format!(
                "{}: written for `{}` but the project dialect is `{}`",
                up.display(), script.dialect, project.config.dialect
            )));
        }
        out.push(Migration { seq, name, dir: path, script, checksum: checksum(&bytes) });
    }
    Ok(out)
}

/// The schema a migration leaves behind (its `ir.json`).
fn read_ir(dir: &std::path::Path) -> Result<SchemaIR, RunnerError> {
    let path = dir.join("ir.json");
    let text = fs::read_to_string(&path).map_err(|e| io(&path, e))?;
    SchemaIR::from_json(&text).map_err(|e| RunnerError::Project(format!("{}: {e}", path.display())))
}

/// Folders a killed `new` left behind while building (`.0003_x.tmp`): never part of the history, so just removed.
fn remove_stale_temp_folders(migrations: &std::path::Path) {
    let Ok(entries) = fs::read_dir(migrations) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name.ends_with(".tmp") && e.path().is_dir() {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

fn errors_only(d: &[Diagnostic]) -> bool { d.iter().any(|x| x.severity == Severity::Error) }

/// Compile `schema.sdl`, diff it against `IR.json` (steered by `mdl`, given as
/// `(label, source)`), lower it, and freeze the result as the next migration.
pub fn create(
    project: &Project,
    name: &str,
    mdl: Option<(&str, &str)>,
    allow_destructive: bool,
) -> Result<Created, RunnerError> {
    let name = slug(name);
    if name.is_empty() {
        return Err(RunnerError::Project("migration name must contain letters or digits".into()));
    }

    let schema_path = project.schema_path();
    let src = fs::read_to_string(&schema_path).map_err(|e| io(&schema_path, e))?;
    let (ir, diags) = compile(&src);
    let schema_label = project.config.schema.clone();
    let Some(new_ir) = ir else {
        let rendered = render_all(&diags, &src, &schema_label, false);
        return Err(RunnerError::Compile { file: schema_label, rendered, diagnostics: diags, source: src });
    };
    let warnings = render_all(&diags, &src, &schema_label, false);

    // ---- chain check: IR.json must be exactly where the last migration left off
    let mut state = project.state_ir()?;
    let existing = list(project)?;
    if let Some(last) = existing.last() {
        let last_ir = read_ir(&last.dir)?;
        if last_ir != state {
            // A `new` that was killed after the migration was published but before IR.json was updated leaves IR.json
            // exactly one step behind (the previous migration's schema, or empty for the first). That is the one
            // state that is safe to repair; anything else is somebody's edit and is refused.
            let before = match existing.len() {
                1 => SchemaIR::empty(),
                n => read_ir(&existing[n - 2].dir)?,
            };
            if state != before {
                return Err(RunnerError::Project(format!(
                    "IR.json does not match {}/ir.json; restore IR.json from version control or delete the newer migration",
                    last.label()
                )));
            }
            project.write_state_ir(&last_ir)?;
            state = last_ir;
        }
    }

    // ---- plan
    let plan: MigrationPlan = match mdl {
        None => diff(&state, &new_ir),
        Some((label, msrc)) => {
            let (plan, mdiags) = compile_migration(&state, &new_ir, msrc);
            match plan {
                Some(p) if !errors_only(&mdiags) => p,
                _ => {
                    return Err(RunnerError::Compile {
                        file: label.to_string(),
                        rendered: render_all(&mdiags, msrc, label, false),
                        diagnostics: mdiags,
                        source: msrc.to_string(),
                    })
                }
            }
        }
    };
    if plan.is_empty() {
        return Err(RunnerError::NoChanges);
    }
    let destructive_ops: Vec<String> =
        plan.ops.iter().filter(|o| o.is_destructive()).map(|o| o.describe()).collect();
    if !destructive_ops.is_empty() && !allow_destructive {
        return Err(RunnerError::Destructive(destructive_ops));
    }

    // ---- lower
    let batches = lower_batches_with(&plan, project.dialect(), Schemas { old: &state, new: &new_ir })
        .map_err(RunnerError::Unsupported)?;
    let script = Script {
        dialect: project.config.dialect.clone(),
        compiler_version: COMPILER_VERSION.to_string(),
        batches: batches
            .into_iter()
            .map(|b| BatchJson { transactional: b.transactional, statements: b.statements })
            .collect(),
    };

    // ---- write: the migration is built in a hidden folder and moved into place in one step, so nobody (a killed
    // process included) ever sees a folder with some of its files; then IR.json moves forward, also in one step
    let seq = existing.len() as u32 + 1;
    let migrations = project.migrations_dir();
    let dir = migrations.join(format!("{seq:04}_{name}"));
    if dir.exists() {
        return Err(RunnerError::Project(format!("{} already exists", dir.display())));
    }
    let tmp = migrations.join(format!(".{seq:04}_{name}.tmp"));
    remove_stale_temp_folders(&migrations);
    fs::create_dir_all(&tmp).map_err(|e| io(&tmp, e))?;
    let write = |file: &str, content: String| write_synced(&tmp.join(file), &content);
    let built = (|| {
        write("plan.json", plan.to_json() + "\n")?;
        write("ir.json", new_ir.to_json() + "\n")?;
        if let Some((_, msrc)) = mdl {
            write("migration.mdl", msrc.to_string())?;
        }
        write("up.sql", format!("-- GENERATED from up.json for review; this file is never executed.\n{}\n", script.to_sql()))?;
        write("up.json", serde_json::to_string_pretty(&script).expect("script serializes") + "\n")?;
        fs::rename(&tmp, &dir).map_err(|e| io(&dir, e))
    })();
    if let Err(e) = built {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if let Err(e) = project.write_state_ir(&new_ir) {
        let _ = fs::remove_dir_all(&dir); // not published after all
        return Err(e);
    }

    Ok(Created {
        seq,
        name,
        dir,
        summary: plan.ops.iter().map(|o| o.describe()).collect(),
        destructive: !destructive_ops.is_empty(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_dir_names() {
        assert_eq!(slug("Add User Slug!"), "add_user_slug");
        assert_eq!(slug("  --x--  "), "x");
        assert_eq!(slug("???"), "");
        assert_eq!(parse_dir_name("0007_add_slug"), Some((7, "add_slug".into())));
        assert_eq!(parse_dir_name("7_x"), Some((7, "x".into())));
        assert_eq!(parse_dir_name("README.md"), None);
        assert_eq!(parse_dir_name("abc_def"), None);
        assert_eq!(parse_dir_name("0001_"), None);
    }

    #[test]
    fn checksum_is_sha256_hex() {
        assert_eq!(
            checksum(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
