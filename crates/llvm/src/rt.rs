//! The small runtime the LLVM backend can actually run.
//!
//! The emitter models `Text` as a NUL-terminated `ptr`; the C backend's
//! runtime uses structs and different symbol names, so it cannot be linked
//! into LLVM output. Instead `runtime/certo_llvm_rt.c` (embedded below)
//! implements the few functions listed in [`RT_FNS`], and [`link_executable`]
//! compiles it together with the emitted IR. Any other runtime call is
//! reported as unsupported rather than left as an unresolved external.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The runtime shim's C source.
pub const RUNTIME_SHIM_C: &str = include_str!("../runtime/certo_llvm_rt.c");

/// The same functions for browser WebAssembly (`wasm32-unknown-unknown`),
/// which has no libc: output and Float formatting go through host imports
/// (`env.certo_write`, `env.certo_float_to_text`) that the JS loader supplies.
pub const RUNTIME_SHIM_WASM_BROWSER_C: &str = include_str!("../runtime/certo_llvm_rt_wasm.c");

/// Does this IR call into the runtime shim? A module that doesn't (numeric
/// functions only) must not have the shim linked in: on the browser target
/// that would add imports the host has to provide for nothing.
pub fn ir_needs_runtime(ir: &str) -> bool {
    ir.contains("@certo_rt_")
}

/// A runtime function the LLVM backend supports.
#[derive(Debug)]
pub struct RtFn {
    /// The Certo-level name as it appears in MIR (`println`, `Text.eq`).
    pub certo: &'static str,
    /// The symbol the shim defines.
    pub symbol: &'static str,
    /// LLVM return type; `"void"` for `Unit`.
    pub ret: &'static str,
    pub params: &'static [&'static str],
}

pub const RT_FNS: &[RtFn] = &[
    RtFn { certo: "print",       symbol: "certo_rt_print",         ret: "void",   params: &["ptr"] },
    RtFn { certo: "println",     symbol: "certo_rt_println",       ret: "void",   params: &["ptr"] },
    RtFn { certo: "eprint",      symbol: "certo_rt_eprint",        ret: "void",   params: &["ptr"] },
    RtFn { certo: "eprintln",    symbol: "certo_rt_eprintln",      ret: "void",   params: &["ptr"] },
    RtFn { certo: "intToText",   symbol: "certo_rt_int_to_text",   ret: "ptr",    params: &["i64"] },
    RtFn { certo: "floatToText", symbol: "certo_rt_float_to_text", ret: "ptr",    params: &["double"] },
    RtFn { certo: "boolToText",  symbol: "certo_rt_bool_to_text",  ret: "ptr",    params: &["i1"] },
    RtFn { certo: "Text.eq",     symbol: "certo_rt_text_eq",       ret: "i1",     params: &["ptr", "ptr"] },
    RtFn { certo: "Text.len",    symbol: "certo_rt_text_len",      ret: "i64",    params: &["ptr"] },
    RtFn { certo: "List.len",    symbol: "certo_rt_list_len",      ret: "i64",    params: &["ptr"] },
    RtFn { certo: "List.get",    symbol: "certo_rt_list_get",      ret: "ptr",    params: &["ptr", "i64"] },
    RtFn { certo: "List.first",  symbol: "certo_rt_list_first",    ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "List.last",   symbol: "certo_rt_list_last",     ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "List.empty",  symbol: "certo_rt_list_empty",    ret: "ptr",    params: &[] },
    RtFn { certo: "List.map",     symbol: "certo_rt_list_map",      ret: "ptr",    params: &["ptr", "ptr"] },
    RtFn { certo: "List.filter",  symbol: "certo_rt_list_filter",   ret: "ptr",    params: &["ptr", "ptr"] },
    RtFn { certo: "List.fold",    symbol: "certo_rt_list_fold",     ret: "ptr",    params: &["ptr", "ptr", "ptr"] },
    RtFn { certo: "List.reduce",  symbol: "certo_rt_list_fold",     ret: "ptr",    params: &["ptr", "ptr", "ptr"] },
    RtFn { certo: "List.forEach", symbol: "certo_rt_list_for_each", ret: "void",   params: &["ptr", "ptr"] },
    RtFn { certo: "List.find",    symbol: "certo_rt_list_find",     ret: "ptr",    params: &["ptr", "ptr"] },
    RtFn { certo: "List.any",     symbol: "certo_rt_list_any",      ret: "i1",     params: &["ptr", "ptr"] },
    RtFn { certo: "List.all",     symbol: "certo_rt_list_all",      ret: "i1",     params: &["ptr", "ptr"] },
    RtFn { certo: "__certo_cell_new", symbol: "certo_rt_cell_new",  ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "__certo_cell_get", symbol: "certo_rt_cell_get",  ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "__certo_cell_set", symbol: "certo_rt_cell_set",  ret: "void",   params: &["ptr", "ptr"] },
    RtFn { certo: "arg",         symbol: "certo_rt_arg",           ret: "ptr",    params: &["i64"] },
    RtFn { certo: "getEnv",      symbol: "certo_rt_get_env",       ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "parseInt",    symbol: "certo_rt_parse_int",     ret: "ptr",    params: &["ptr"] },
    RtFn { certo: "Process.quit", symbol: "certo_rt_quit",         ret: "void",   params: &["i64"] },
];

/// Symbols for operators the emitter lowers to runtime calls itself.
pub const TEXT_CONCAT: &str = "certo_rt_text_concat";
pub const INT_POW: &str = "certo_rt_pow";
/// Allocates a record's heap block: `ptr certo_rt_alloc(i64 bytes)`.
pub const ALLOC: &str = "certo_rt_alloc";
/// Hands the program's `argc`/`argv` to the runtime; the entry point calls it
/// when the program uses `arg` (native only: a browser module has no arguments).
pub const SET_ARGS: &str = "certo_rt_set_args";
/// `ptr certo_rt_list_new(i64 n)`: a list of `n` zeroed slots, filled in by compiled code.
pub const LIST_NEW: &str = "certo_rt_list_new";
/// `ptr certo_rt_list_slot(ptr list, i64 i)`: a slot's address, panicking when out of range.
pub const LIST_SLOT: &str = "certo_rt_list_slot";
/// `ptr certo_rt_list_push(ptr list, i64 item)`: a new list with `item` (an 8-byte slot value) appended.
pub const LIST_PUSH: &str = "certo_rt_list_push";

/// The supported runtime function for a Certo name, if there is one.
pub fn lookup(certo_name: &str) -> Option<&'static RtFn> {
    RT_FNS.iter().find(|f| f.certo == certo_name)
}

/// The Certo names of every supported runtime function, for error messages.
pub fn supported_names() -> String {
    RT_FNS.iter().map(|f| f.certo).collect::<Vec<_>>().join(", ")
}

/// A parameter type as written in a `declare`/`call`. A C `bool` argument is
/// zero-extended by the caller, which a bare `i1` does not promise.
pub fn abi_param(ty: &str) -> String {
    if ty == "i1" { "i1 zeroext".into() } else { ty.into() }
}

/// A return type as written in a `declare`/`call`.
pub fn abi_ret(ty: &str) -> String {
    if ty == "i1" { "zeroext i1".into() } else { ty.into() }
}

// ------------------------------------------------------------------ //
// Linking
// ------------------------------------------------------------------ //

/// Find a `clang`: on PATH, else in a default Windows LLVM install.
pub fn find_clang() -> Option<String> {
    let mut candidates: Vec<String> = vec!["clang".into()];
    if cfg!(windows) {
        candidates.push(r"C:\Program Files\LLVM\bin\clang.exe".into());
        candidates.push(r"C:\Program Files (x86)\LLVM\bin\clang.exe".into());
    }
    candidates.into_iter().find(|c| {
        (c == "clang" || Path::new(c).exists())
            && Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
    })
}

/// Compile `ir` together with the runtime shim into a native executable.
pub fn link_executable(ir: &str, out: &Path) -> Result<(), String> {
    let clang = find_clang().ok_or_else(|| {
        "clang not found (looked on PATH and in C:\\Program Files\\LLVM\\bin). \
         Install LLVM, or write the .ll with -o and link it yourself with the runtime from \
         crates/llvm/runtime/certo_llvm_rt.c".to_string()
    })?;
    let dir = tempfile::tempdir().map_err(|e| format!("cannot create a temp directory: {e}"))?;
    let ll: PathBuf = dir.path().join("program.ll");
    let rt: PathBuf = dir.path().join("certo_llvm_rt.c");
    std::fs::write(&ll, ir).map_err(|e| format!("cannot write {}: {e}", ll.display()))?;
    std::fs::write(&rt, RUNTIME_SHIM_C).map_err(|e| format!("cannot write {}: {e}", rt.display()))?;
    let output = Command::new(&clang)
        .arg("-O2")
        .arg("-Wno-override-module")
        .arg(&ll)
        .arg(&rt)
        .arg("-o")
        .arg(out)
        .output()
        .map_err(|e| format!("cannot run {clang}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "clang failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_defines_every_symbol_the_emitter_can_call() {
        let mut symbols: Vec<&str> = RT_FNS.iter().map(|f| f.symbol).collect();
        symbols.push(TEXT_CONCAT);
        symbols.push(INT_POW);
        symbols.extend([ALLOC, LIST_NEW, LIST_SLOT, LIST_PUSH]);
        for sym in symbols {
            assert!(
                RUNTIME_SHIM_C.contains(&format!(" {sym}(")) || RUNTIME_SHIM_C.contains(&format!("*{sym}(")),
                "runtime/certo_llvm_rt.c does not define `{sym}`"
            );
        }
    }

    #[test]
    fn the_browser_shim_defines_the_same_symbols_without_a_libc() {
        let mut symbols: Vec<&str> = RT_FNS.iter().map(|f| f.symbol).collect();
        symbols.push(TEXT_CONCAT);
        symbols.push(INT_POW);
        symbols.extend([ALLOC, LIST_NEW, LIST_SLOT, LIST_PUSH]);
        for sym in symbols {
            assert!(
                RUNTIME_SHIM_WASM_BROWSER_C.contains(&format!(" {sym}(")) || RUNTIME_SHIM_WASM_BROWSER_C.contains(&format!("*{sym}(")),
                "runtime/certo_llvm_rt_wasm.c does not define `{sym}`"
            );
        }
        assert!(!RUNTIME_SHIM_WASM_BROWSER_C.contains("#include"), "the browser shim has no libc to include");
    }

    #[test]
    fn only_ir_that_calls_the_shim_needs_it_linked() {
        assert!(ir_needs_runtime("  call void @certo_rt_println(ptr %v0)"));
        assert!(!ir_needs_runtime("define i64 @certo_add(i64 %p1, i64 %p2) {"));
    }

    #[test]
    fn lookup_finds_supported_names_only() {
        assert_eq!(lookup("println").unwrap().symbol, "certo_rt_println");
        assert_eq!(lookup("Text.len").unwrap().ret, "i64");
        assert!(lookup("Json.parse").is_none());
    }

    #[test]
    fn only_bool_gets_zeroext() {
        assert_eq!(abi_param("i1"), "i1 zeroext");
        assert_eq!(abi_ret("i1"), "zeroext i1");
        assert_eq!(abi_param("i64"), "i64");
        assert_eq!(abi_ret("ptr"), "ptr");
    }
}
