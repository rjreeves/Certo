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
    // A default Windows LLVM install puts clang in Program Files without
    // adding it to PATH (the main compiler checks these locations too).
    for path in fallback_clang_paths() {
        if Path::new(&path).exists() && has_wasm_support(&path) {
            return Some(Toolchain { kind: ToolchainKind::Clang, binary: path });
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
            // WASI sysroot (provides malloc, printf, …): the explicit one, else
            // the first common install location that exists.
            let sysroot = wasi_sysroot.map(|p| p.display().to_string()).or_else(|| {
                if matches!(target, WasmTarget::Wasi) {
                    wasi_sysroot_candidates().into_iter().find(|p| Path::new(p).exists())
                } else {
                    None
                }
            });
            args = clang_args(target, &opt_str, sysroot.as_deref(), out_str,
                src_file.path().to_str().unwrap_or(""));
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

/// The `clang` argument list for one wasm build. Pure, so it can be tested
/// without a toolchain.
fn clang_args(target: &WasmTarget, opt: &str, sysroot: Option<&str>, out: &str, src: &str) -> Vec<String> {
    let triple = match target {
        WasmTarget::Wasi    => "wasm32-wasi",
        WasmTarget::Browser => "wasm32-unknown-unknown",
    };
    let mut args = vec![format!("--target={}", triple), opt.to_string()];
    if let Some(sysroot) = sysroot {
        args.push(format!("--sysroot={}", sysroot));
    }
    if matches!(target, WasmTarget::Browser) {
        // No OS and no libc: disable default libs, export all public symbols.
        // Deliberately no `-lm` — there is no libm to find without a sysroot,
        // and asking for one made every browser build fail at link time.
        args.push("-nostdlib".into());
        args.push("-Wl,--export-all".into());
        args.push("-Wl,--no-entry".into());
    } else {
        args.push("-lm".into()); // math library (from the WASI sysroot)
    }
    args.push("-o".into());
    args.push(out.into());
    args.push(src.into());
    args
}

/// Where a default LLVM install puts clang when it isn't on PATH.
fn fallback_clang_paths() -> Vec<String> {
    if cfg!(windows) {
        vec![
            r"C:\Program Files\LLVM\bin\clang.exe".into(),
            r"C:\Program Files (x86)\LLVM\bin\clang.exe".into(),
        ]
    } else {
        Vec::new()
    }
}

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

    // Regression: the browser target has no libc/libm (it links with
    // -nostdlib), but `-lm` was always appended, so every browser build failed
    // with "wasm-ld: unable to find library -lm".
    #[test]
    fn browser_args_do_not_ask_for_libm() {
        let args = clang_args(&WasmTarget::Browser, "-O2", None, "out.wasm", "in.ll");
        assert!(!args.iter().any(|a| a == "-lm"), "{:?}", args);
        for want in ["--target=wasm32-unknown-unknown", "-nostdlib", "-Wl,--export-all", "-Wl,--no-entry"] {
            assert!(args.iter().any(|a| a == want), "missing {} in {:?}", want, args);
        }
    }

    #[test]
    fn wasi_args_still_link_libm_and_keep_the_sysroot() {
        let args = clang_args(&WasmTarget::Wasi, "-O2", Some("/sdk/sysroot"), "out.wasm", "in.ll");
        assert!(args.iter().any(|a| a == "-lm"), "{:?}", args);
        assert!(args.iter().any(|a| a == "--target=wasm32-wasi"), "{:?}", args);
        assert!(args.iter().any(|a| a == "--sysroot=/sdk/sysroot"), "{:?}", args);
        assert!(!args.iter().any(|a| a == "-nostdlib"), "{:?}", args);
    }

    #[test]
    fn args_end_with_output_then_source() {
        let args = clang_args(&WasmTarget::Browser, "-O1", None, "o.wasm", "i.ll");
        assert_eq!(&args[args.len() - 3..], ["-o", "o.wasm", "i.ll"]);
    }

    #[test]
    fn fallback_clang_paths_match_the_platform() {
        let paths = fallback_clang_paths();
        if cfg!(windows) {
            assert!(paths.iter().any(|p| p.ends_with(r"LLVM\bin\clang.exe")), "{:?}", paths);
        } else {
            assert!(paths.is_empty(), "no Windows paths off Windows: {:?}", paths);
        }
    }
}
