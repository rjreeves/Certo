//! Shared mutable context accumulated while emitting an entire module.

use std::collections::HashMap;

/// Per-function SSA value counter (reset for each function).
pub type TmpId = u32;

/// The LLVM-level signature of a function the module itself defines.
#[derive(Debug, Clone)]
pub struct FnSig {
    /// LLVM return type; `"void"` for `Unit`.
    pub ret: String,
    pub params: Vec<String>,
}

/// Module-level accumulated state: string literals and external declarations.
#[derive(Debug, Default)]
pub struct ModuleCtx {
    /// Every function this module defines, keyed by LLVM global name
    /// (`certo_add`). Filled before any body is emitted so a call can see a
    /// callee defined later in the file — calls to these must NOT be declared
    /// extern (LLVM rejects a `declare` plus a `define` of the same symbol)
    /// and take their return type from here, not from the call site.
    pub fn_sigs: HashMap<String, FnSig>,

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

    /// Record a function this module defines (see `fn_sigs`).
    pub fn register_fn(&mut self, name: String, ret: String, params: Vec<String>) {
        self.fn_sigs.insert(name, FnSig { ret, params });
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
