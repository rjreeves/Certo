//! Error types for the WASM backend.

use std::fmt;

#[derive(Debug)]
pub enum WasmError {
    /// Parse or HIR lowering failed.
    Compile(String),
    /// No WASM-capable toolchain found on PATH.
    NoToolchain,
    /// `clang`/`emcc` exited with a non-zero status.
    ToolchainFailed { tool: String, exit_code: i32, stderr: String },
    /// I/O error (temp files, output path, …).
    Io(String),
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WasmError::Compile(e)    => write!(f, "compile error: {}", e),
            WasmError::NoToolchain   => write!(f,
                "no WASM toolchain found — install clang with wasm32 target support\n\
                 or Emscripten (emcc), then re-run.\n\
                 Alternatively, pass --emit-ir to write the .ll file for manual compilation:\n\
                   clang --target=wasm32-wasi -O2 -o out.wasm out.ll"),
            WasmError::ToolchainFailed { tool, exit_code, stderr } =>
                write!(f, "{} failed (exit {}): {}", tool, exit_code, stderr.trim()),
            WasmError::Io(e)         => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for WasmError {}
