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
pub fn compile_c(c_src: &str, out_path: &Path) -> Result<(), TestRunnerError> {
    // Write source to a temp file (deleted when `_src_file` is dropped).
    let src_file = NamedTempFile::with_suffix(".c")
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;
    std::fs::write(src_file.path(), c_src)
        .map_err(|e| TestRunnerError::Io(e.to_string()))?;

    let compiler = find_compiler();
    let status = Command::new(&compiler)
        .args([
            "-x", "c",
            "-std=c11",
            "-O0",
            "-o", out_path.to_str().unwrap_or("certo_test_bin"),
            src_file.path().to_str().unwrap_or(""),
            "-lm",          // math library (needed for stdlib)
        ])
        .status()
        .map_err(|e| TestRunnerError::CompilerNotFound {
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

/// Find an available C compiler: prefer `cc`, fall back to `gcc`, then `clang`.
fn find_compiler() -> String {
    for candidate in &["cc", "gcc", "clang"] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return (*candidate).to_string();
        }
    }
    "cc".to_string() // let the OS give the real error
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
