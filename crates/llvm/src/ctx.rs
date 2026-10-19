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

/// How a record type is laid out in memory: a heap block with one 8-byte slot
/// per field, in declared order (so a literal that lists its fields in a
/// different order still agrees with every field access).
#[derive(Debug, Clone)]
pub struct RecordLayout {
    /// `(field name, LLVM type)` in declared order.
    pub fields: Vec<(String, String)>,
}

impl RecordLayout {
    /// Byte size of the block.
    pub fn size(&self) -> usize { self.fields.len().max(1) * 8 }

    /// Slot index and LLVM type of a field.
    pub fn field(&self, name: &str) -> Option<(usize, &str)> {
        self.fields.iter().position(|(n, _)| n == name).map(|i| (i, self.fields[i].1.as_str()))
    }
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

    /// Layout of every record type the module declares, by type name.
    pub records: HashMap<String, RecordLayout>,

    /// String contents (raw Certo string, without null terminator).
    /// Index into this vec corresponds to the `@.str.N` global.
    pub str_lits: Vec<String>,

    /// External function declarations collected from call sites.
    /// Key: LLVM global name (e.g. `certo_pow`).
    /// Value: (return_type, arg_types).
    pub externs: HashMap<String, (String, Vec<String>)>,

    /// Constructs the backend cannot compile correctly (an unsupported runtime
    /// call, a record, …). Emission carries on so every problem is reported at
    /// once, then `emit_module` fails with the whole list.
    pub errors: Vec<String>,

    /// Restore the old behaviour for unsupported constructs: declare an
    /// unknown runtime call as an external and emit a null placeholder for
    /// values the backend can't build, for output that is linked against a
    /// runtime the caller supplies.
    pub allow_unresolved: bool,
}

impl ModuleCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a function this module defines (see `fn_sigs`).
    pub fn register_fn(&mut self, name: String, ret: String, params: Vec<String>) {
        self.fn_sigs.insert(name, FnSig { ret, params });
    }

    /// Record a construct the backend can't compile (see `errors`). Does
    /// nothing under `allow_unresolved`; returns whether it was recorded.
    pub fn unsupported(&mut self, in_fn: &str, what: &str) -> bool {
        if self.allow_unresolved { return false; }
        let msg = format!("in `{in_fn}`: {what}");
        if !self.errors.contains(&msg) { self.errors.push(msg); }
        true
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
