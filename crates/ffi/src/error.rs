use std::fmt;

#[derive(Debug)]
pub enum FfiError {
    /// Source file parse errors.
    ParseError(String),
    /// REST schema JSON parse / validation error.
    SchemaError(String),
    /// I/O error.
    Io(String),
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfiError::ParseError(s)  => write!(f, "parse error: {}", s),
            FfiError::SchemaError(s) => write!(f, "schema error: {}", s),
            FfiError::Io(s)          => write!(f, "io error: {}", s),
        }
    }
}
