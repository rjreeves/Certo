use crate::error::{io, RunnerError};
use certo_sdl::SchemaIR;
use certo_sql::Dialect;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "certo-db.toml";
pub const IR_FILE: &str = "IR.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub dialect: String,
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default = "default_migrations")]
    pub migrations: String,
}

fn default_schema() -> String { "schema.sdl".into() }
fn default_migrations() -> String { "migrations".into() }

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: Config,
}

impl Project {
    /// Create a new project in `root` (which must not already be one).
    pub fn init(root: &Path, dialect: &str) -> Result<Project, RunnerError> {
        if Dialect::from_name(dialect).is_none() {
            return Err(RunnerError::Project(format!("unknown dialect `{dialect}` (supported: postgres, sqlite)")));
        }
        let config_path = root.join(CONFIG_FILE);
        if config_path.exists() {
            return Err(RunnerError::Project(format!("{} already exists", config_path.display())));
        }
        let config = Config { dialect: dialect.into(), schema: default_schema(), migrations: default_migrations() };
        fs::create_dir_all(root).map_err(|e| io(root, e))?;
        let project = Project { root: root.to_path_buf(), config };
        fs::create_dir_all(project.migrations_dir()).map_err(|e| io(project.migrations_dir(), e))?;

        let schema = project.schema_path();
        if !schema.exists() {
            fs::write(&schema, "// Describe your schema here, then run `certo sdl migrate new <name>`.\n")
                .map_err(|e| io(&schema, e))?;
        }
        project.write_state_ir(&SchemaIR::empty())?;
        let text = toml::to_string_pretty(&project.config).expect("config serializes");
        fs::write(&config_path, text).map_err(|e| io(&config_path, e))?;
        Ok(project)
    }

    /// Open the project rooted at `root`.
    pub fn open(root: &Path) -> Result<Project, RunnerError> {
        let path = root.join(CONFIG_FILE);
        let text = fs::read_to_string(&path).map_err(|_| {
            RunnerError::Project(format!(
                "no {CONFIG_FILE} in {} (run `certo sdl migrate init` first)",
                root.display()
            ))
        })?;
        let config: Config = toml::from_str(&text)
            .map_err(|e| RunnerError::Project(format!("{}: {e}", path.display())))?;
        if Dialect::from_name(&config.dialect).is_none() {
            return Err(RunnerError::Project(format!(
                "{}: unknown dialect `{}` (supported: postgres, sqlite)",
                path.display(),
                config.dialect
            )));
        }
        Ok(Project { root: root.to_path_buf(), config })
    }

    pub fn dialect(&self) -> Dialect {
        Dialect::from_name(&self.config.dialect).expect("validated on open/init")
    }

    pub fn schema_path(&self) -> PathBuf { self.root.join(&self.config.schema) }
    pub fn migrations_dir(&self) -> PathBuf { self.root.join(&self.config.migrations) }
    pub fn ir_path(&self) -> PathBuf { self.root.join(IR_FILE) }

    /// The schema as of the last generated migration.
    pub fn state_ir(&self) -> Result<SchemaIR, RunnerError> {
        let path = self.ir_path();
        let text = fs::read_to_string(&path).map_err(|e| io(&path, e))?;
        SchemaIR::from_json(&text)
            .map_err(|e| RunnerError::Project(format!("{}: invalid IR: {e}", path.display())))
    }

    pub fn write_state_ir(&self, ir: &SchemaIR) -> Result<(), RunnerError> {
        write_atomic(&self.ir_path(), &(ir.to_json() + "\n"))
    }
}

/// Write `content` to `path` so a reader (or a process killed half-way) sees the old file or the whole new one,
/// never part of it: it goes to a sibling file, is flushed to disk, and replaces `path` in one step.
pub(crate) fn write_atomic(path: &Path, content: &str) -> Result<(), RunnerError> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    write_synced(&tmp, content)?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io(path, e)
    })
}

/// Write a file and flush it to disk before returning.
pub(crate) fn write_synced(path: &Path, content: &str) -> Result<(), RunnerError> {
    use std::io::Write;
    let mut f = fs::File::create(path).map_err(|e| io(path, e))?;
    f.write_all(content.as_bytes()).map_err(|e| io(path, e))?;
    f.sync_all().map_err(|e| io(path, e))
}
