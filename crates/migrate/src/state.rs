use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::error::MigrateError;

/// Persisted record of a single applied migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedMigration {
    pub name:       String,
    pub applied_at: String, // ISO-8601 UTC timestamp
}

/// The full manifest of applied migrations, stored as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationState {
    pub applied: Vec<AppliedMigration>,
}

impl MigrationState {
    pub fn load(path: &Path) -> Result<Self, MigrateError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), MigrateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn is_applied(&self, name: &str) -> bool {
        self.applied.iter().any(|a| a.name == name)
    }

    pub fn mark_applied(&mut self, name: &str) {
        self.applied.push(AppliedMigration {
            name:       name.to_string(),
            applied_at: now_utc(),
        });
    }

    pub fn mark_rolled_back(&mut self, name: &str) {
        self.applied.retain(|a| a.name != name);
    }
}

fn now_utc() -> String {
    // Minimal timestamp — avoids pulling in chrono/time for this crate.
    // In production this would use std::time::SystemTime.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as seconds-since-epoch; callers can convert if needed.
    format!("{}", secs)
}

/// Default location for the migration manifest relative to a project root.
pub fn default_manifest_path(project_root: &Path) -> PathBuf {
    project_root.join(".certo").join("migrations.json")
}
