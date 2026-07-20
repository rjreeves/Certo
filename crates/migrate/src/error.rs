use std::fmt;

#[derive(Debug)]
pub enum MigrateError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnknownMigration(String),
    AlreadyApplied(String),
    NotApplied(String),
    OutOfOrder { name: String, expected: usize, got: usize },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::Io(e)                  => write!(f, "I/O error: {}", e),
            MigrateError::Json(e)                => write!(f, "JSON error: {}", e),
            MigrateError::UnknownMigration(n)    => write!(f, "unknown migration: {}", n),
            MigrateError::AlreadyApplied(n)      => write!(f, "migration already applied: {}", n),
            MigrateError::NotApplied(n)          => write!(f, "migration not yet applied: {}", n),
            MigrateError::OutOfOrder { name, expected, got } =>
                write!(f, "migration {} is out of order (expected index {}, got {})", name, expected, got),
        }
    }
}

impl std::error::Error for MigrateError {}

impl From<std::io::Error> for MigrateError {
    fn from(e: std::io::Error) -> Self { MigrateError::Io(e) }
}

impl From<serde_json::Error> for MigrateError {
    fn from(e: serde_json::Error) -> Self { MigrateError::Json(e) }
}
