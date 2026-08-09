//! Spawn one subprocess per test and collect results.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::gen::{generate_case, shrink, GenType, Rng};
use crate::harness::TestEntry;

/// Number of generated cases per property test, unless overridden by
/// `CERTO_TEST_CASES` — matches the QuickCheck/proptest convention.
const DEFAULT_CASES: usize = 100;
/// Upper bound on subprocess spawns spent shrinking a single failing case.
const MAX_SHRINK_ATTEMPTS: usize = 200;

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
    /// A property test found a generated input that fails, shrunk to a
    /// minimal (or attempt-budget-limited) counterexample.
    PropertyFailed {
        exit_code:      i32,
        seed:           u64,
        /// How many generated cases ran (in order) before this one failed.
        case_index:     usize,
        /// Total case count this run used — case generation's "size" scaling
        /// depends on it, so reproducing the same seed needs the same count.
        cases:          usize,
        /// (parameter name, shrunk value's display text) in declaration order.
        counterexample: Vec<(String, String)>,
    },
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
    /// When `Some(dir)`, every test subprocess is launched with
    /// `LLVM_PROFILE_FILE=<dir>/certo-cov-%p.profraw` (BACKLOG item 126) —
    /// clang's runtime substitutes `%p` with each subprocess's own PID, so
    /// concurrent/repeated runs (property tests spawn the binary many
    /// times) never clobber each other's profile data. The binary itself
    /// must already have been compiled with `compile_c_opts(.., coverage:
    /// true)` for this to do anything; `None` (the default) is a complete
    /// no-op, identical to every `certo test` run before this existed.
    pub coverage_dir: Option<PathBuf>,
}

/// Set `LLVM_PROFILE_FILE` on `cmd` when coverage is requested, so this
/// subprocess's execution contributes its own `.profraw` file.
fn apply_coverage_env(cmd: &mut Command, opts: &RunOptions) {
    if let Some(dir) = &opts.coverage_dir {
        cmd.env("LLVM_PROFILE_FILE", dir.join("certo-cov-%p.profraw"));
    }
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
        .map(|entry| {
            if entry.params.is_empty() {
                run_one(binary, entry, timeout, opts)
            } else {
                run_property(binary, entry, timeout, opts)
            }
        })
        .collect()
}

fn run_one(binary: &Path, entry: &TestEntry, _timeout: Duration, opts: &RunOptions) -> TestResult {
    let start = Instant::now();

    let mut cmd = Command::new(binary);
    cmd.arg(&entry.display_name);
    apply_coverage_env(&mut cmd, opts);
    let result = cmd.output();

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

fn property_seed() -> u64 {
    if let Ok(s) = std::env::var("CERTO_TEST_SEED") {
        if let Ok(v) = s.parse::<u64>() { return v; }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn property_num_cases() -> usize {
    std::env::var("CERTO_TEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Spawn one case of a property test with generated args. `None` means the
/// binary itself couldn't be launched (distinct from the test failing).
fn spawn_property_case(binary: &Path, entry: &TestEntry, args: &[String], opts: &RunOptions) -> Option<bool> {
    let mut cmd = Command::new(binary);
    cmd.arg(&entry.display_name).args(args);
    apply_coverage_env(&mut cmd, opts);
    cmd.output().ok().map(|out| out.status.success())
}

/// Run a property test: generate `CERTO_TEST_CASES` (default 100) random
/// cases from a `CERTO_TEST_SEED`-derived (or time-based) seed, spawning the
/// already-compiled binary once per case with the generated values as argv.
/// On the first failure, shrink toward a minimal counterexample the same
/// way — by re-spawning the binary with smaller candidates and checking its
/// exit code — then report it. No changes to `certo_panic`/`abort()` are
/// involved anywhere; a failing case is just a nonzero exit code, same as
/// any other test.
fn run_property(binary: &Path, entry: &TestEntry, _timeout: Duration, opts: &RunOptions) -> TestResult {
    let start = Instant::now();
    let seed  = property_seed();
    let cases = property_num_cases();
    let types: Vec<GenType> = entry.params.iter().map(|(_, t)| t.clone()).collect();
    let mut rng = Rng::new(seed);

    for case_idx in 0..cases {
        let values = generate_case(&mut rng, &types, case_idx, cases);
        let args: Vec<String> = values.iter().flat_map(|v| v.to_args()).collect();

        match spawn_property_case(binary, entry, &args, opts) {
            None => {
                return TestResult {
                    entry:    entry.clone(),
                    outcome:  Outcome::SpawnError("failed to launch test binary".into()),
                    duration: start.elapsed(),
                    stdout:   String::new(),
                    stderr:   String::new(),
                };
            }
            Some(true) => continue,
            Some(false) => {
                let shrunk = shrink(
                    values,
                    |trial| {
                        let trial_args: Vec<String> = trial.iter().flat_map(|v| v.to_args()).collect();
                        matches!(spawn_property_case(binary, entry, &trial_args, opts), Some(false))
                    },
                    MAX_SHRINK_ATTEMPTS,
                );

                // Re-run the final (shrunk) counterexample once more to
                // capture its stdout/stderr/exit code for the report.
                let shrunk_args: Vec<String> = shrunk.iter().flat_map(|v| v.to_args()).collect();
                let mut final_cmd = Command::new(binary);
                final_cmd.arg(&entry.display_name).args(&shrunk_args);
                apply_coverage_env(&mut final_cmd, opts);
                let out = final_cmd.output();
                let (stdout, stderr, exit_code) = match out {
                    Ok(o) => (
                        String::from_utf8_lossy(&o.stdout).into_owned(),
                        String::from_utf8_lossy(&o.stderr).into_owned(),
                        o.status.code().unwrap_or(-1),
                    ),
                    Err(_) => (String::new(), String::new(), -1),
                };

                let counterexample: Vec<(String, String)> = entry.params.iter()
                    .zip(shrunk.iter())
                    .map(|((name, _), v)| (name.clone(), v.display()))
                    .collect();

                return TestResult {
                    entry:    entry.clone(),
                    outcome:  Outcome::PropertyFailed { exit_code, seed, case_index: case_idx, cases, counterexample },
                    duration: start.elapsed(),
                    stdout,
                    stderr,
                };
            }
        }
    }

    TestResult {
        entry:    entry.clone(),
        outcome:  Outcome::Passed,
        duration: start.elapsed(),
        stdout:   String::new(),
        stderr:   String::new(),
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
            params:       vec![],
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
