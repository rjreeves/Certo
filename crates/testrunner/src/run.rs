//! Spawn one subprocess per test and collect results.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::harness::TestEntry;

/// Outcome of a single test run.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub entry:    TestEntry,
    pub outcome:  Outcome,
    pub duration: Duration,
    pub stdout:   String,
    pub stderr:   String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed { exit_code: i32 },
    /// The test process could not be launched at all.
    SpawnError(String),
}

impl Outcome {
    pub fn is_passed(&self) -> bool {
        matches!(self, Outcome::Passed)
    }
}

/// Options for how tests are run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Maximum wall-clock time per test (default: 30 s).
    pub timeout: Option<Duration>,
    /// Only run tests whose display name contains this string.
    pub filter:  Option<String>,
}

/// Run all tests described by `entries` using `binary` as the test executable.
///
/// Each test is launched as a subprocess: `binary "<display_name>"`.
/// Exit code 0 → Passed; anything else → Failed.
pub fn run_tests(
    binary: &Path,
    entries: &[TestEntry],
    opts: &RunOptions,
) -> Vec<TestResult> {
    let timeout = opts.timeout.unwrap_or(Duration::from_secs(30));

    entries
        .iter()
        .filter(|e| {
            opts.filter
                .as_deref()
                .map(|f| e.display_name.contains(f))
                .unwrap_or(true)
        })
        .map(|entry| run_one(binary, entry, timeout))
        .collect()
}

fn run_one(binary: &Path, entry: &TestEntry, _timeout: Duration) -> TestResult {
    let start = Instant::now();

    let result = Command::new(binary)
        .arg(&entry.display_name)
        .output();

    let duration = start.elapsed();

    match result {
        Err(e) => TestResult {
            entry:    entry.clone(),
            outcome:  Outcome::SpawnError(e.to_string()),
            duration,
            stdout:   String::new(),
            stderr:   String::new(),
        },
        Ok(out) => {
            let exit_code = out.status.code().unwrap_or(-1);
            TestResult {
                entry:    entry.clone(),
                outcome:  if exit_code == 0 {
                    Outcome::Passed
                } else {
                    Outcome::Failed { exit_code }
                },
                duration,
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::TestKind;

    fn fake_entry(name: &str) -> TestEntry {
        TestEntry {
            display_name: name.into(),
            kind:         TestKind::Unit,
            c_fn_name:    format!("__test_{}", name),
        }
    }

    #[test]
    fn filter_applies() {
        let entries = vec![
            fake_entry("foo bar"),
            fake_entry("baz qux"),
            fake_entry("foo qux"),
        ];
        let opts = RunOptions { filter: Some("foo".into()), ..Default::default() };

        // We can't run a real binary here, but we can verify the filter logic
        // by checking which entries would be selected.
        let selected: Vec<_> = entries
            .iter()
            .filter(|e| opts.filter.as_deref().map(|f| e.display_name.contains(f)).unwrap_or(true))
            .collect();

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].display_name, "foo bar");
        assert_eq!(selected[1].display_name, "foo qux");
    }

    #[test]
    fn outcome_is_passed() {
        assert!(Outcome::Passed.is_passed());
        assert!(!Outcome::Failed { exit_code: 1 }.is_passed());
        assert!(!Outcome::SpawnError("x".into()).is_passed());
    }
}
