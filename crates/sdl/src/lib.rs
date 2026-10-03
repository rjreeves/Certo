//! SDL (schema definition language) front end: source text -> AST -> SchemaIR.
//!
//! Grammar: `docs/database/ebnf.md`. Keywords are contextual (only
//! meaningful where the grammar expects them), so words like `key`, `one` or
//! `many` remain legal column names.
//!
//! Pipeline: `lexer` -> `parser` (AST) -> `symbols` (global names) ->
//! `sema` + `expr` (resolution, type checking) -> `ir` (SchemaIR).
//!
//! This crate is a pure library with owned, serializable data and no I/O, so
//! it can later be wrapped behind an FFI boundary for the C# CLI host.

mod ast;
mod expr;
mod ir;
mod lexer;
mod parser;
mod print;
mod sema;
mod symbols;

pub use ast::*;
pub use ir::*;
pub use lexer::{lex, TokKind, Token};
pub use expr::{
    compatible, default_assignable, describe as describe_type, from_rank, is_text, is_time, ordered, rank, ExprCx, Typed,
};
pub use parser::{parse, PResult, Parser};
pub use print::to_sdl;
pub use sema::analyze;
pub use symbols::{SymKind, SymbolTable};

use certo_diagnostics::{Diagnostic, Severity};

/// Full pipeline. The IR is `Some` only if there are no error diagnostics
/// (warnings are still returned alongside it).
pub fn compile(src: &str) -> (Option<SchemaIR>, Vec<Diagnostic>) {
    let (file, mut diags) = parse(src);
    // Analysing a file with syntax errors would only produce cascading noise.
    if has_errors(&diags) {
        return (None, diags);
    }
    let ir = analyze(&file, &mut diags);
    if has_errors(&diags) { (None, diags) } else { (Some(ir), diags) }
}

fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod sema_tests;
