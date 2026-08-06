//! `certo-testrunner` — compile and run test blocks in Certo source files.
//!
//! # Pipeline
//!
//! ```text
//! .cto source
//!      │
//!      ▼
//!   parse()          (certo_parser)
//!      │  Module (with TestDecl / DbTestDecl / PropertyDecl)
//!      ▼
//!   build_harness()  (harness.rs)
//!      │  synthesises FnDecl wrappers, emits C + dispatching main()
//!      ▼
//!   compile_c()      (compile.rs)
//!      │  produces a native executable via cc/gcc/clang
//!      ▼
//!   run_tests()      (run.rs)
//!      │  spawns one subprocess per test, collects Outcome
//!      ▼
//!   print_results()  (report.rs)
//! ```

pub mod error;
pub mod harness;
pub mod compile;
pub mod run;
pub mod report;
pub mod gen;

use std::path::Path;

use certo_parser::parse;
use certo_diagnostics::{Diagnostic, render_all};
use tempfile::tempdir;

use compile::binary_path;
use error::TestRunnerError;
use harness::build_harness;
use run::{run_tests, RunOptions};
use report::{print_results, print_summary, Summary};

/// Parse `source`, build a C harness, compile it, run all tests.
///
/// Returns `true` if all tests passed, `false` otherwise.
pub fn run_file(
    source_path: &Path,
    opts: &RunOptions,
    color: bool,
) -> Result<bool, TestRunnerError> {
    // ── Parse ──────────────────────────────────────────────────────────────
    let src = std::fs::read_to_string(source_path)
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;

    // `parse` returns Result<Module, Vec<ParseError>>
    let filename = source_path.display().to_string();
    let colour   = std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").as_deref() != Ok("dumb");
    let module = parse(&src).map_err(|errs| {
        let diags: Vec<Diagnostic> = errs.iter().map(|e| {
            Diagnostic::error("", format!("{}", e)).with_span(e.span)
        }).collect();
        let rendered = render_all(&diags, &src, &filename, colour);
        TestRunnerError::ParseError(rendered)
    })?;

    // ── Build harness ──────────────────────────────────────────────────────
    let (c_src, entries) = build_harness(&module)?;
    if entries.is_empty() {
        return Err(TestRunnerError::NoTests);
    }

    // ── Compile ────────────────────────────────────────────────────────────
    let tmp = tempdir().map_err(|e| TestRunnerError::Io(e.to_string()))?;
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let bin = binary_path(stem, tmp.path());

    compile::compile_c(&c_src, &bin)?;

    // ── Run ────────────────────────────────────────────────────────────────
    let results = run_tests(&bin, &entries, opts);

    // ── Report ─────────────────────────────────────────────────────────────
    print_results(&results, color);
    let summary = Summary::from_results(&results);
    print_summary(&summary, color);

    Ok(summary.all_passed())
}
