//! Compile a C source string to an executable using the host C compiler.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;

use crate::error::TestRunnerError;

/// Write `c_src` to a temporary file, invoke the host C compiler, and return
/// the path to the compiled executable.
///
/// `out_dir` is the directory where the binary is placed.  If `None`, a
/// temporary directory is used (caller is responsible for keeping the
/// `TempDir` alive as long as the binary is needed).
///
/// `coverage`, when true, adds clang's source-based coverage instrumentation
/// flags (`-fprofile-instr-generate -fcoverage-mapping`) — BACKLOG item 126.
/// Requires clang specifically (the flags are clang/LLVM-only, not
/// gcc/MSVC); `compile_c` doesn't itself enforce that, since `find_compiler`
/// already prefers clang first and this whole toolchain already requires it
/// (BACKLOG item 104).
pub fn compile_c(c_src: &str, out_path: &Path) -> Result<(), TestRunnerError> {
    compile_c_opts(c_src, out_path, false, false)
}

/// `uses_db`, when true, links libpq — required whenever the harness pulled
/// in the PostgreSQL C runtime (`harness::uses_db`, mirrored here since the
/// linker step is a separate process invocation from the C-generation step
/// in `harness.rs`). Previously this crate never linked libpq at all, a
/// real, pre-existing gap surfaced by BACKLOG item 165's `dbTest`
/// auto-rollback fix: a `dbTest`'s synthesized wrapper always calls real DB
/// functions now, and the resulting binary failed to link with `undefined
/// symbol: certo_db_connect` before this fix, confirmed via direct testing
/// against a real PostgreSQL server. Mirrors `certo build`'s own
/// already-working `uses_db` linking logic (`crates/cli/src/main.rs`).
pub fn compile_c_opts(c_src: &str, out_path: &Path, coverage: bool, uses_db: bool) -> Result<(), TestRunnerError> {
    // Write source to a temp file (deleted when `_src_file` is dropped).
    let src_file = NamedTempFile::with_suffix(".c")
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;
    std::fs::write(src_file.path(), c_src)
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;

    let compiler = find_compiler();
    let mut cmd = Command::new(&compiler);
    cmd.args([
        "-x", "c",
        // GNU dialect, not strict ISO `-std=c11` — matches `certo build`'s
        // own invocation (crates/cli/src/main.rs), which passes no `-std`
        // flag at all and so gets clang's GNU-by-default dialect. Codegen
        // already relies on the GNU `typeof` extension (`emit_mir.rs`'s
        // `AggregateKind::Record` emission, `(typeof(lhs)){ ... }`) for
        // *any* record-type literal, not just a closure's `certo_fn_t`
        // struct (BACKLOG item 140) — `certo test` had simply never
        // compiled one until now. Confirmed via direct testing: strict
        // `-std=c11` rejects `typeof` outright ("expected ';' after
        // expression"), a real, independent, pre-existing gap between
        // `certo test` and `certo build`'s C dialect.
        "-std=gnu11",
        "-O0",
        "-o", out_path.to_str().unwrap_or("certo_test_bin"),
        src_file.path().to_str().unwrap_or(""),
        // Resets clang's "treat every following file-like argument as C
        // source" mode that `-x c` above turns on — without this, the
        // `libpq.lib`/`-lpq` linker args added below (when `uses_db`) get
        // misinterpreted as more C source to compile instead of a library
        // to link, confirmed directly (clang tried to parse a `.lib`'s
        // binary contents as C and failed with "source file is not valid
        // UTF-8"). `certo build`'s own invocation never hits this because
        // it never passes `-x c` at all, relying on the `.c` extension
        // instead — this crate always writes its source to a `.c`-suffixed
        // temp file too, so `-x c` was already redundant even before this.
        "-x", "none",
    ]);
    cmd.args([
        "-Wno-int-to-pointer-cast",
        "-Wno-pointer-to-int-cast",
        "-Wno-int-conversion",
        "-Wno-implicit-function-declaration",
        "-Wno-deprecated-declarations",
    ]);
    if coverage {
        cmd.args(["-fprofile-instr-generate", "-fcoverage-mapping"]);
    }
    if uses_db {
        let (pg_inc, pg_lib) = resolve_pg_paths();
        if let Some(inc) = &pg_inc {
            cmd.arg(format!("-I{}", inc));
        }
        if cfg!(windows) {
            if let Some(lib_dir) = &pg_lib {
                cmd.arg(format!("{}/libpq.lib", lib_dir));
            } else {
                cmd.arg("libpq.lib");
            }
        } else {
            if let Some(lib) = &pg_lib {
                cmd.arg(format!("-L{}", lib));
            }
            cmd.arg("-lpq");
        }
    }
    if cfg!(windows) {
        cmd.arg("-Xlinker").arg("/subsystem:console");
    } else {
        cmd.arg("-lm");
    }
    let status = cmd.status().map_err(|e| TestRunnerError::CompilerNotFound {
        compiler: compiler.clone(),
        detail: e.to_string(),
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(TestRunnerError::CompileFailed {
            exit_code: status.code().unwrap_or(-1),
        })
    }
}

/// Locate an LLVM tool (`llvm-profdata`/`llvm-cov`) that ships alongside
/// whichever `clang` `find_compiler` resolved — these aren't reliably on
/// `PATH` even when `clang` itself is found only via its full install path
/// (confirmed: this is exactly the case on this project's own dev
/// environment). Falls back to the bare tool name (relying on `PATH`) if
/// the compiler was found via bare `PATH` lookup itself, or the sibling
/// binary doesn't exist next to it.
pub fn find_llvm_tool(tool: &str) -> String {
    let compiler = find_compiler();
    let compiler_path = Path::new(&compiler);
    if let Some(dir) = compiler_path.parent() {
        if compiler_path.file_name().is_some() {
            let candidate = dir.join(if cfg!(windows) { format!("{tool}.exe") } else { tool.to_string() });
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    tool.to_string()
}

/// Resolve PostgreSQL include/lib directories for linking `-lpq`/`libpq.lib`.
/// Duplicated from `crates/cli/src/main.rs`'s own `resolve_pg_paths` (kept
/// small and self-contained rather than pulled into a shared crate for one
/// ~40-line helper) — same resolution order: `PG_INCLUDE`/`PG_LIB` env vars
/// first, then versioned Windows install probing, then `pg_config` on Unix.
fn resolve_pg_paths() -> (Option<String>, Option<String>) {
    let env_inc = std::env::var("PG_INCLUDE").ok();
    let env_lib = std::env::var("PG_LIB").ok();
    if env_inc.is_some() || env_lib.is_some() {
        return (env_inc, env_lib);
    }

    if cfg!(windows) {
        for ver in (9u32..=20).rev() {
            let base = format!(r"C:\Program Files\PostgreSQL\{}", ver);
            let inc = format!(r"{}\include", base);
            let lib = format!(r"{}\lib", base);
            if std::path::Path::new(&inc).exists() {
                return (Some(inc), Some(lib));
            }
        }
    }

    if !cfg!(windows) {
        if let Ok(out) = Command::new("pg_config")
            .args(["--includedir", "--libdir"])
            .output()
        {
            if out.status.success() {
                let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
                    .unwrap_or("")
                    .lines()
                    .collect();
                let inc = lines.first().map(|s| s.trim().to_string());
                let lib = lines.get(1).map(|s| s.trim().to_string());
                return (inc, lib);
            }
        }
    }

    (None, None)
}

/// Find an available C compiler.
fn find_compiler() -> String {
    for candidate in &["clang", "gcc", "cc"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return (*candidate).to_string();
        }
    }
    // Fall back to common Windows install paths.
    for path in &[
        r"C:\Program Files\LLVM\bin\clang.exe",
        r"C:\Program Files (x86)\LLVM\bin\clang.exe",
    ] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }
    "cc".to_string()
}

/// Return a suitable path for the test binary given the source file stem.
pub fn binary_path(source_stem: &str, dir: &Path) -> PathBuf {
    let name = format!("certo_test_{}", source_stem.replace(std::path::MAIN_SEPARATOR, "_"));
    if cfg!(windows) {
        dir.join(format!("{}.exe", name))
    } else {
        dir.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_hello_world() {
        let src = r#"
#include <stdio.h>
int main(void) {
    printf("hello from certo test harness\n");
    return 0;
}
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = binary_path("hello", dir.path());
        // If no compiler is available, skip gracefully.
        match compile_c(src, &bin) {
            Ok(()) => {
                assert!(bin.exists(), "binary should exist");
                let out = std::process::Command::new(&bin)
                    .output()
                    .expect("run binary");
                let stdout = String::from_utf8_lossy(&out.stdout);
                assert!(stdout.contains("hello from certo test harness"));
            }
            Err(TestRunnerError::CompilerNotFound { .. }) => {
                eprintln!("skip: no C compiler available");
            }
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }
}
