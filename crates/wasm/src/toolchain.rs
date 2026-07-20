//! Detect and invoke a WASM-capable compiler toolchain.
//!
//! Probe order:
//!  1. `clang` (any version) with `--target=wasm32-*` support
//!  2. `clang-18` … `clang-14` (versioned installs)
//!  3. `emcc` (Emscripten)
//!
//! If none is found, `detect()` returns `None` and the caller should offer
//! an `--emit-ir` fallback that writes the `.ll` file for manual compilation.

use std::path::Path;
use std::process::Command;

use crate::WasmTarget;
use crate::error::WasmError;

/// A detected WASM toolchain.
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub kind:    ToolchainKind,
    pub binary:  String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainKind {
    Clang,
    Emscripten,
}

/// Try to find a WASM-capable toolchain on PATH.
pub fn detect() -> Option<Toolchain> {
    // Clang candidates (most specific first so we pick the newest available)
    let clang_candidates = [
        "clang-18", "clang-17", "clang-16", "clang-15", "clang-14", "clang",
    ];
    for candidate in &clang_candidates {
        if has_wasm_support(candidate) {
            return Some(Toolchain { kind: ToolchainKind::Clang, binary: candidate.to_string() });
        }
    }
    // Emscripten fallback
    if command_exists("emcc") {
        return Some(Toolchain { kind: ToolchainKind::Emscripten, binary: "emcc".to_string() });
    }
    None
}

/// Compile an LLVM IR source string to a `.wasm` binary.
pub fn compile(
    toolchain:    &Toolchain,
    ll_src:       &str,
    out_path:     &Path,
    target:       &WasmTarget,
    opt_level:    u8,
    wasi_sysroot: Option<&Path>,
    extra_flags:  &[String],
) -> Result<(), WasmError> {
    use tempfile::NamedTempFile;

    let src_file = NamedTempFile::with_suffix(".ll")
        .map_err(|e| WasmError::Io(e.to_string()))?;
    std::fs::write(src_file.path(), ll_src)
        .map_err(|e| WasmError::Io(e.to_string()))?;

    let out_str = out_path.to_str().unwrap_or("out.wasm");
    let opt_str = format!("-O{}", opt_level.min(3));

    let mut args: Vec<String> = Vec::new();

    match toolchain.kind {
        ToolchainKind::Clang => {
            let triple = match target {
                WasmTarget::Wasi    => "wasm32-wasi",
                WasmTarget::Browser => "wasm32-unknown-unknown",
            };
            args.push(format!("--target={}", triple));
            args.push(opt_str);
            // WASI sysroot (provides malloc, printf, …)
            if let Some(sysroot) = wasi_sysroot {
                args.push(format!("--sysroot={}", sysroot.display()));
            } else if matches!(target, WasmTarget::Wasi) {
                // Try common install locations
                for path in wasi_sysroot_candidates() {
                    if Path::new(&path).exists() {
                        args.push(format!("--sysroot={}", path));
                        break;
                    }
                }
            }
            if matches!(target, WasmTarget::Browser) {
                // No OS; disable default libs, export all public symbols
                args.push("-nostdlib".into());
                args.push("-Wl,--export-all".into());
                args.push("-Wl,--no-entry".into());
            }
            args.push("-lm".into()); // math library
            args.push("-o".into());
            args.push(out_str.into());
            args.push(src_file.path().to_str().unwrap_or("").into());
        }

        ToolchainKind::Emscripten => {
            args.push(opt_str);
            args.push("-o".into());
            args.push(out_str.into());
            args.push(src_file.path().to_str().unwrap_or("").into());
        }
    }

    args.extend_from_slice(extra_flags);

    let result = Command::new(&toolchain.binary)
        .args(&args)
        .output()
        .map_err(|e| WasmError::ToolchainFailed {
            tool: toolchain.binary.clone(),
            exit_code: -1,
            stderr: e.to_string(),
        })?;

    if result.status.success() {
        Ok(())
    } else {
        Err(WasmError::ToolchainFailed {
            tool:      toolchain.binary.clone(),
            exit_code: result.status.code().unwrap_or(-1),
            stderr:    String::from_utf8_lossy(&result.stderr).into_owned(),
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn has_wasm_support(clang: &str) -> bool {
    // `clang --print-targets` includes "wasm32" if the target is compiled in.
    Command::new(clang)
        .arg("--print-targets")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm"))
        .unwrap_or(false)
}

/// Common locations where the WASI sysroot might be installed.
fn wasi_sysroot_candidates() -> Vec<String> {
    vec![
        // wasi-sdk on macOS/Linux
        "/opt/wasi-sdk/share/wasi-sysroot".into(),
        "/usr/local/share/wasi-sysroot".into(),
        // wasi-sdk installed via apt/brew
        "/usr/share/wasi-sysroot".into(),
        // LLVM's wasi-libc
        "/usr/lib/wasm32-wasi".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        // Just ensure detect() doesn't panic; it may return None in CI.
        let _ = detect();
    }

    #[test]
    fn command_exists_false_for_nonexistent() {
        assert!(!command_exists("__certo_nonexistent_binary__"));
    }
}
