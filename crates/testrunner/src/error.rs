//! Errors that can occur during test running.

use std::fmt;

#[derive(Debug)]
pub enum TestRunnerError {
    /// Could not read or parse the Certo source file.
    ParseError(String),
    /// No test declarations found in the file.
    NoTests,
    /// C compiler binary not found on PATH.
    CompilerNotFound { compiler: String, detail: String },
    /// C compiler exited with a non-zero status.
    CompileFailed { exit_code: i32 },
    /// I/O error (temp files, etc.).
    Io(String),
}

impl fmt::Display for TestRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestRunnerError::ParseError(e)    =>
                write!(f, "parse error: {}", e),
            TestRunnerError::NoTests          =>
                write!(f, "no test declarations found"),
            TestRunnerError::CompilerNotFound { compiler, detail } =>
                write!(f, "C compiler '{}' not found: {}", compiler, detail),
            TestRunnerError::CompileFailed { exit_code } =>
                write!(f, "C compilation failed (exit {})", exit_code),
            TestRunnerError::Io(e)            =>
                write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for TestRunnerError {}
