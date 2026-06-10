//! `certo-test` — run test blocks in Certo source files.
//!
//! Usage:
//!   certo-test [OPTIONS] <file.cto>...
//!
//! Options:
//!   --filter <pattern>   Only run tests whose name contains <pattern>
//!   --no-color           Disable ANSI colour output
//!
//! Exit codes:
//!   0  all tests passed (or no tests run with --filter)
//!   1  one or more tests failed
//!   2  compilation or parse error

use std::process;
use std::path::PathBuf;
use std::time::Duration;

use certo_testrunner::{run_file, run::RunOptions};

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut filter: Option<String> = None;
    let mut color = true;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-color" | "--no-colour" => color = false,
            "--filter"   => {
                filter = args.next().or_else(|| {
                    eprintln!("error: --filter requires an argument");
                    process::exit(2);
                });
            }
            other if other.starts_with("--filter=") => {
                filter = Some(other["--filter=".len()..].to_string());
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown option '{}'", other);
                eprintln!("usage: certo-test [--filter <pattern>] [--no-color] <file.cto>...");
                process::exit(2);
            }
            path => files.push(PathBuf::from(path)),
        }
    }

    if files.is_empty() {
        eprintln!("usage: certo-test [--filter <pattern>] [--no-color] <file.cto>...");
        process::exit(2);
    }

    let opts = RunOptions {
        timeout: Some(Duration::from_secs(30)),
        filter,
    };

    let mut all_ok = true;
    for file in &files {
        if files.len() > 1 {
            println!("\n=== {} ===", file.display());
        }
        match run_file(file, &opts, color) {
            Ok(passed) => {
                if !passed { all_ok = false; }
            }
            Err(certo_testrunner::error::TestRunnerError::ParseError(ref msg)) => {
                eprint!("{}", msg);
                process::exit(2);
            }
            Err(e) => {
                eprintln!("error: {}", e);
                process::exit(2);
            }
        }
    }

    process::exit(if all_ok { 0 } else { 1 });
}
