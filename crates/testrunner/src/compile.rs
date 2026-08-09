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
    compile_c_opts(c_src, out_path, false)
}

pub fn compile_c_opts(c_src: &str, out_path: &Path, coverage: bool) -> Result<(), TestRunnerError> {
    // Write source to a temp file (deleted when `_src_file` is dropped).
    let src_file = NamedTempFile::with_suffix(".c")
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;
    std::fs::write(src_file.path(), c_src)
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;

    let compiler = find_compiler();
    let mut cmd = Command::new(&compiler);
    cmd.args([
        "-x", "c",
        "-std=c11",
        "-O0",
        "-o", out_path.to_str().unwrap_or("certo_test_bin"),
        src_file.path().to_str().unwrap_or(""),
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
