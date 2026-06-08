//! `certo-wasm` — WebAssembly target for the Certo compiler.
//!
//! # Pipeline
//!
//! ```text
//! .certo source
//!     │
//!     ▼ certo_parser::parse()
//!     │ Module
//!     ▼ certo_llvm::emit_module()   (target_triple = "wasm32-wasi" / "wasm32-unknown-unknown")
//!     │ LLVM IR text (.ll)
//!     ▼ toolchain::compile()        (clang --target=wasm32-* | emcc)
//!     │ .wasm binary
//!     ▼ bindgen::generate_dts/js()  (optional)
//!     │ .d.ts + .js loader
//! ```
//!
//! # Usage
//!
//! ```sh
//! certo-wasm src/main.certo -o out/main.wasm --bindings
//!
//! # If no toolchain is installed, emit IR for manual compilation:
//! certo-wasm src/main.certo --emit-ir -o out/main.ll
//! ```

pub mod error;
pub mod toolchain;
pub mod bindgen;

use std::path::{Path, PathBuf};

use certo_ast::module::Module;
use certo_llvm::{emit_module as emit_llvm_ir, LlvmOptions};

pub use error::WasmError;
pub use toolchain::{Toolchain, ToolchainKind};

// ------------------------------------------------------------------ //
// Options
// ------------------------------------------------------------------ //

/// Which WebAssembly execution environment to target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WasmTarget {
    /// `wasm32-wasi` — WASI system interface (Wasmtime, WasmEdge, Node.js …).
    /// Supports malloc, printf, file I/O through the WASI ABI.
    #[default]
    Wasi,
    /// `wasm32-unknown-unknown` — bare WebAssembly for browsers.
    /// No OS syscalls; numeric-only exports work without a WASI shim.
    Browser,
}

impl WasmTarget {
    pub fn triple(&self) -> &'static str {
        match self {
            WasmTarget::Wasi    => "wasm32-wasi",
            WasmTarget::Browser => "wasm32-unknown-unknown",
        }
    }
}

/// Full set of options for one WASM compilation.
#[derive(Debug, Clone, Default)]
pub struct WasmOptions {
    /// Execution target (default: WASI).
    pub target: WasmTarget,
    /// Path to WASI sysroot (`--sysroot`).  Auto-detected if `None`.
    pub wasi_sysroot: Option<PathBuf>,
    /// Optimisation level 0–3 (default: 2).
    pub opt_level: u8,
    /// Extra flags forwarded verbatim to the compiler.
    pub extra_flags: Vec<String>,
}

// ------------------------------------------------------------------ //
// High-level API
// ------------------------------------------------------------------ //

/// Compile a parsed Certo `Module` to a `.wasm` binary.
///
/// Returns the path to the produced binary (same as `out_path`).
///
/// # Errors
///
/// - `WasmError::Compile` — HIR lowering or IR emission failed.
/// - `WasmError::NoToolchain` — no WASM compiler found on PATH.
/// - `WasmError::ToolchainFailed` — compiler exited non-zero.
/// - `WasmError::Io` — temp file or output write error.
pub fn compile_module(
    module: &Module,
    out_path: &Path,
    opts: &WasmOptions,
) -> Result<(), WasmError> {
    // ── Emit LLVM IR ─────────────────────────────────────────────────
    let llvm_opts = LlvmOptions {
        target_triple: Some(opts.target.triple().into()),
        ..Default::default()
    };
    let ir = emit_llvm_ir(module, &llvm_opts)
        .map_err(WasmError::Compile)?;

    // ── Find toolchain ────────────────────────────────────────────────
    let tc = toolchain::detect().ok_or(WasmError::NoToolchain)?;

    // ── Compile ───────────────────────────────────────────────────────
    toolchain::compile(
        &tc,
        &ir,
        out_path,
        &opts.target,
        opts.opt_level.max(2),
        opts.wasi_sysroot.as_deref(),
        &opts.extra_flags,
    )
}

/// Emit LLVM IR with the WASM triple but do not invoke a compiler.
///
/// Useful when no WASM toolchain is installed — the user can compile
/// manually with `clang --target=wasm32-wasi -O2 -o out.wasm out.ll`.
pub fn emit_ir_only(module: &Module, target: &WasmTarget) -> Result<String, WasmError> {
    let llvm_opts = LlvmOptions {
        target_triple: Some(target.triple().into()),
        ..Default::default()
    };
    emit_llvm_ir(module, &llvm_opts).map_err(WasmError::Compile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;

    fn module(src: &str) -> Module {
        parse(src).expect("parse failed")
    }

    #[test]
    fn emit_ir_only_wasi() {
        let m  = module("module M\npub fn add(a: Int, b: Int): Int = a + b\n");
        let ir = emit_ir_only(&m, &WasmTarget::Wasi).expect("emit failed");
        assert!(ir.contains("target triple = \"wasm32-wasi\""), "missing triple\n{}", ir);
        assert!(ir.contains("define i64 @certo_add("), "missing fn def\n{}", ir);
    }

    #[test]
    fn emit_ir_only_browser() {
        let m  = module("module M\npub fn mul(a: Float, b: Float): Float = a * b\n");
        let ir = emit_ir_only(&m, &WasmTarget::Browser).expect("emit failed");
        assert!(ir.contains("wasm32-unknown-unknown"), "missing browser triple\n{}", ir);
        assert!(ir.contains("define double @certo_mul("), "missing fn def\n{}", ir);
    }

    #[test]
    fn wasm_target_triples() {
        assert_eq!(WasmTarget::Wasi.triple(),    "wasm32-wasi");
        assert_eq!(WasmTarget::Browser.triple(), "wasm32-unknown-unknown");
    }

    #[test]
    fn compile_without_toolchain_gives_no_toolchain_error() {
        // If there IS a toolchain on this machine, skip.
        if toolchain::detect().is_some() { return; }

        let m   = module("module M\npub fn add(a: Int, b: Int): Int = a + b\n");
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.wasm");
        let err = compile_module(&m, &out, &WasmOptions::default()).unwrap_err();
        assert!(matches!(err, WasmError::NoToolchain));
    }
}
