//! Human-readable test result formatting (no external dependencies).

use std::time::Duration;

use crate::run::{Outcome, TestResult};

/// Overall summary after all tests have run.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub passed:  usize,
    pub failed:  usize,
    pub errored: usize,
    pub total:   usize,
    pub elapsed: Duration,
}

impl Summary {
    pub fn from_results(results: &[TestResult]) -> Self {
        let mut s = Summary { total: results.len(), ..Default::default() };
        for r in results {
            match r.outcome {
                Outcome::Passed         => s.passed  += 1,
                Outcome::Failed { .. }  => s.failed  += 1,
                Outcome::SpawnError(_)  => s.errored += 1,
            }
            s.elapsed += r.duration;
        }
        s
    }

    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.errored == 0
    }
}

/// Print the results to stdout in a human-readable format.
///
/// Uses ANSI codes when `color` is true.
pub fn print_results(results: &[TestResult], color: bool) {
    for r in results {
        let (symbol, label) = match &r.outcome {
            Outcome::Passed            => ("✓", "PASS"),
            Outcome::Failed { .. }     => ("✗", "FAIL"),
            Outcome::SpawnError(_)     => ("!", "ERR "),
        };

        let (sym_col, reset) = if color {
            match &r.outcome {
                Outcome::Passed           => ("\x1b[32m", "\x1b[0m"), // green
                Outcome::Failed { .. }    => ("\x1b[31m", "\x1b[0m"), // red
                Outcome::SpawnError(_)    => ("\x1b[33m", "\x1b[0m"), // yellow
            }
        } else {
            ("", "")
        };

        println!(
            "{sym_col}{symbol} [{label}]{reset} {} [{kind}] ({ms:.0}ms)",
            r.entry.display_name,
            kind = r.entry.kind.label(),
            ms   = r.duration.as_secs_f64() * 1000.0,
            sym_col = sym_col,
            symbol  = symbol,
            label   = label,
            reset   = reset,
        );

        // Show captured output for failures.
        if !r.outcome.is_passed() {
            if !r.stdout.is_empty() {
                println!("  stdout:\n{}", indent(&r.stdout, "    "));
            }
            if !r.stderr.is_empty() {
                println!("  stderr:\n{}", indent(&r.stderr, "    "));
            }
            if let Outcome::SpawnError(msg) = &r.outcome {
                println!("  spawn error: {}", msg);
            }
        }
    }
}

/// Print the final summary line.
pub fn print_summary(summary: &Summary, color: bool) {
    let (ok_col, fail_col, reset) = if color {
        ("\x1b[32m", "\x1b[31m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    println!();
    if summary.all_passed() {
        println!(
            "{ok_col}All {} tests passed{reset} ({:.2}s)",
            summary.total,
            summary.elapsed.as_secs_f64(),
            ok_col = ok_col, reset = reset
        );
    } else {
        println!(
            "{fail_col}{} failed{reset}, {} passed, {} errored — {} total ({:.2}s)",
            summary.failed, summary.passed, summary.errored, summary.total,
            summary.elapsed.as_secs_f64(),
            fail_col = fail_col, reset = reset
        );
    }
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{}{}", prefix, l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{TestEntry, TestKind};
    use std::time::Duration;

    fn passed(name: &str) -> TestResult {
        TestResult {
            entry:    TestEntry { display_name: name.into(), kind: TestKind::Unit, c_fn_name: String::new() },
            outcome:  Outcome::Passed,
            duration: Duration::from_millis(5),
            stdout:   String::new(),
            stderr:   String::new(),
        }
    }

    fn failed(name: &str) -> TestResult {
        TestResult {
            entry:    TestEntry { display_name: name.into(), kind: TestKind::Unit, c_fn_name: String::new() },
            outcome:  Outcome::Failed { exit_code: 1 },
            duration: Duration::from_millis(12),
            stdout:   "assertion failed".into(),
            stderr:   String::new(),
        }
    }

    #[test]
    fn summary_all_passed() {
        let results = vec![passed("a"), passed("b"), passed("c")];
        let s = Summary::from_results(&results);
        assert_eq!(s.passed, 3);
        assert_eq!(s.failed, 0);
        assert!(s.all_passed());
    }

    #[test]
    fn summary_with_failure() {
        let results = vec![passed("a"), failed("b")];
        let s = Summary::from_results(&results);
        assert_eq!(s.passed, 1);
        assert_eq!(s.failed, 1);
        assert!(!s.all_passed());
    }
}
