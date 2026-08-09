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
pub mod coverage;

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
    run_file_opts(source_path, opts, color, false)
}

/// Same as `run_file`, with `coverage: true` additionally instrumenting the
/// build, source-mapping the generated C back to `source_path` via `#line`
/// directives, and printing an `llvm-cov` per-file report before returning
/// (BACKLOG item 126). `false` behaves identically to `run_file`.
pub fn run_file_opts(
    source_path: &Path,
    opts: &RunOptions,
    color: bool,
    coverage: bool,
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
    let coverage_source = if coverage { Some((filename.clone(), src.clone())) } else { None };
    let (c_src, entries) = build_harness(&module, coverage_source)?;
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

    compile::compile_c_opts(&c_src, &bin, coverage)?;

    // ── Run ────────────────────────────────────────────────────────────────
    // Coverage needs its own RunOptions with coverage_dir pointed at the
    // same tempdir the binary lives in, so every subprocess's .profraw ends
    // up somewhere `coverage::report` will find it — `opts` is a shared
    // `&RunOptions` from the caller, so this clones rather than mutates it.
    let run_opts = if coverage {
        let mut o = opts.clone();
        o.coverage_dir = Some(tmp.path().to_path_buf());
        o
    } else {
        opts.clone()
    };
    let results = run_tests(&bin, &entries, &run_opts);

    // ── Report ─────────────────────────────────────────────────────────────
    print_results(&results, color);
    let summary = Summary::from_results(&results);
    print_summary(&summary, color);

    if coverage {
        match coverage::report(tmp.path(), &bin, &c_src) {
            Ok(text) => {
                println!();
                println!("Coverage:");
                print!("{}", text);
            }
            Err(e) => eprintln!("warning: coverage report failed: {}", e),
        }
    }

    Ok(summary.all_passed())
}
