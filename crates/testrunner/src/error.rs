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
    /// A `property` parameter's type has no value generator yet.
    UnsupportedPropertyParamType { property_name: String, param_name: String },
    /// BACKLOG item 222 — `expand_validators`/`expand_state_machines`
    /// (`certo_codegen::expand`) failed to parse their own generated source.
    /// Always a compiler bug in the generator, never a user error — same
    /// class as a `ParseError`, just with the generated text attached so a
    /// caller can render it.
    ValidatorExpansionFailed(String),
    /// BACKLOG item 222 — a `ruleTest`/`validatorTest` names a validator, or
    /// (for `ruleTest`) a rule on it, that isn't declared anywhere in the
    /// module.
    UnknownValidatorRef { label: String, path: String },
}

impl fmt::Display for TestRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestRunnerError::ParseError(e)    =>
                write!(f, "{}", e),
            TestRunnerError::NoTests          =>
                write!(f, "no test declarations found"),
            TestRunnerError::CompilerNotFound { compiler, detail } =>
                write!(f, "C compiler '{}' not found: {}", compiler, detail),
            TestRunnerError::CompileFailed { exit_code } =>
                write!(f, "C compilation failed (exit {})", exit_code),
            TestRunnerError::Io(e)            =>
                write!(f, "I/O error: {}", e),
            TestRunnerError::UnsupportedPropertyParamType { property_name, param_name } =>
                write!(
                    f,
                    "property \"{}\" parameter `{}`: value generation is only supported for \
                     Int, Float, Bool, Text, List<T> of those, and named record/sum types built \
                     from them today (not anonymous record types, Map/Decimal/DateTime/tuples/\
                     Option/Result, a List of a record/sum type, or a self-referential type)",
                    property_name, param_name
                ),
            TestRunnerError::ValidatorExpansionFailed(detail) =>
                write!(f, "internal error: generated validator/state-machine source failed to parse\n{}", detail),
            TestRunnerError::UnknownValidatorRef { label, path } =>
                write!(f, "test \"{}\": `{}` doesn't name a declared validator/rule in this module", label, path),
        }
    }
}

impl std::error::Error for TestRunnerError {}
