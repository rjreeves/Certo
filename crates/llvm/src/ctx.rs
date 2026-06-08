//! Shared mutable context accumulated while emitting an entire module.

use std::collections::HashMap;

/// Per-function SSA value counter (reset for each function).
pub type TmpId = u32;

/// Module-level accumulated state: string literals and external declarations.
#[derive(Debug, Default)]
pub struct ModuleCtx {
    /// String contents (raw Certo string, without null terminator).
    /// Index into this vec corresponds to the `@.str.N` global.
    pub str_lits: Vec<String>,

    /// External function declarations collected from call sites.
    /// Key: LLVM global name (e.g. `certo_pow`).
    /// Value: (return_type, arg_types).
    pub externs: HashMap<String, (String, Vec<String>)>,
}

impl ModuleCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a string literal and return its global index.
    pub fn intern_str(&mut self, s: &str) -> usize {
        // Deduplicate identical strings.
        if let Some(idx) = self.str_lits.iter().position(|x| x == s) {
            return idx;
        }
        let idx = self.str_lits.len();
        self.str_lits.push(s.to_string());
        idx
    }

    /// Record that a function with the given LLVM name must be declared
    /// externally.  Only the first declaration wins (types are determined
    /// from the first call site encountered).
    pub fn declare_extern(&mut self, name: String, ret_ty: String, arg_tys: Vec<String>) {
        self.externs.entry(name).or_insert((ret_ty, arg_tys));
    }
}
