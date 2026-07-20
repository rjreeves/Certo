//! `certo-llvm` — LLVM IR backend for the Certo compiler.
//!
//! # Pipeline
//!
//! ```text
//! Module  ──► lower_module() ──► HirModule
//!                                    │
//!                          per-function lower_fn()
//!                                    │ MirFn
//!                              emit_fn() ──► LLVM IR text
//!                                    │
//!                             emit_module() ──► .ll file
//! ```
//!
//! The `.ll` file is then passed to `clang`/`llc`:
//!
//! ```sh
//! certo-llvm src/main.cto -o out.ll
//! clang -O2 -o myapp out.ll
//! # or for WebAssembly (#18):
//! clang --target=wasm32-wasi -O2 -o myapp.wasm out.ll
//! ```

pub mod ty;
pub mod ctx;
pub mod emit_fn;
pub mod emit_module;

pub use emit_module::{emit_module, LlvmOptions};
