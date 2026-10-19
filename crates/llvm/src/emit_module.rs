//! Emit a complete LLVM IR translation unit (`.ll` file) from a Certo module.
//!
//! # Output structure
//!
//! ```text
//! ; header comment
//! target triple = "..."      ; optional
//!
//! @.str.N = private …        ; string literal globals
//!
//! declare <type> @<name>(…)  ; external function declarations
//!
//! define …                   ; function definitions
//! ```
//!
//! # Compatibility
//!
//! The emitted IR uses **opaque pointers** (`ptr`) which requires LLVM 14+.
//! Pass the generated `.ll` file to `clang` or `llc`:
//!
//! ```sh
//! clang -O2 -o out out.ll          # native binary
//! llc -filetype=obj -o out.o out.ll # object file
//! ```

use std::fmt::Write as FmtWrite;

use certo_ast::decl::Decl;
use certo_ast::module::Module;
use certo_hir::{lower_module, HirItem};
use certo_mir::lower_fn;

use crate::ctx::ModuleCtx;
use crate::emit_fn::emit_fn;
use crate::ty::{ast_ret_ty_to_llvm, ast_ty_to_llvm, llvm_escape, llvm_fn_name};

// ------------------------------------------------------------------ //
// Options
// ------------------------------------------------------------------ //

/// Controls what the LLVM IR emitter includes in the output.
#[derive(Debug, Clone, Default)]
pub struct LlvmOptions {
    /// LLVM target triple, e.g. `"x86_64-unknown-linux-gnu"` or
    /// `"wasm32-wasi"`.  If `None`, no `target triple` line is emitted
    /// (letting `clang` infer the host target).
    pub target_triple: Option<String>,

    /// Data layout string.  Optional — normally inferred by the toolchain.
    pub data_layout: Option<String>,

    /// Emit `; comment` annotations describing each block's purpose.
    pub annotate: bool,

    /// Don't emit the program entry point (`define i32 @main()`). Needed when
    /// linking the output into a host program that supplies its own `main`.
    pub no_entry: bool,
}

/// Wasm targets use their own entry-point convention (WASI looks for a
/// mangled `__main_void`, not `main`), which hasn't been verified here, so
/// no `main` is emitted for them.
fn is_wasm_triple(triple: Option<&str>) -> bool {
    triple.is_some_and(|t| t.starts_with("wasm"))
}

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

/// Compile `module` to a complete LLVM IR string.
///
/// Returns `Err` if HIR lowering fails (parse / resolve errors should have
/// been caught before this point).
pub fn emit_module(module: &Module, opts: &LlvmOptions) -> Result<String, String> {
    // ── HIR lowering ─────────────────────────────────────────────────
    let hir = lower_module(module).map_err(|errs| {
        format!("{} HIR lowering error(s)", errs.len())
    })?;

    let mut ctx = ModuleCtx::new();
    let mut fn_bodies: Vec<String> = Vec::new();

    // Build HirFn lookup by name for MIR lowering.
    let hir_fn_map: std::collections::HashMap<&str, &certo_hir::HirFn> = hir.items.iter()
        .filter_map(|item| if let HirItem::Fn(f) = item { Some((f.name.as_str(), f)) } else { None })
        .collect();

    // Record every function we are about to define *before* emitting any
    // body, so a call to a function defined later in the file (or to itself)
    // is recognised as a local call rather than declared as an external one.
    for sdecl in &module.decls {
        if let Decl::Fn(ast_fn) = &sdecl.node {
            if hir_fn_map.contains_key(ast_fn.name.node.as_str()) {
                ctx.register_fn(
                    llvm_fn_name(&ast_fn.name.node),
                    ast_ret_ty_to_llvm(ast_fn.ret_ty.as_ref().map(|s| &s.node)),
                    ast_fn.params.iter().map(|p| ast_ty_to_llvm(&p.ty.node)).collect(),
                );
            }
        }
    }

    // ── MIR lowering + IR emission per function ───────────────────────
    // Iterate AST decls (not HIR) to access declared param/ret types.
    for sdecl in &module.decls {
        if let Decl::Fn(ast_fn) = &sdecl.node {
            if let Some(hir_fn) = hir_fn_map.get(ast_fn.name.node.as_str()) {
                let (mf, _lifted) = lower_fn(hir_fn, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys, &hir.variant_to_type);
                let body = emit_fn(ast_fn, &mf, &mut ctx);
                fn_bodies.push(body);
            }
        }
    }

    // Emit global constants (from HIR)
    let mut const_lines: Vec<String> = Vec::new();
    for item in &hir.items {
        if let HirItem::Const(c) = item {
            use certo_hir::HirExprKind;
            let (llty, val): (String, String) = match &c.value.kind {
                HirExprKind::Int(n)   => ("i64".into(), n.to_string()),
                HirExprKind::Float(f) => ("double".into(), format!("{:e}", f)),
                HirExprKind::Bool(b)  => ("i1".into(), if *b { "1".into() } else { "0".into() }),
                HirExprKind::Str(s)   => {
                    let idx = ctx.intern_str(s);
                    let len = s.len() + 1;
                    let gep = format!(
                        "getelementptr [{len} x i8], ptr @.str.{idx}, i32 0, i32 0"
                    );
                    ("ptr".into(), gep)
                }
                _ => ("ptr".into(), "null".into()),
            };
            let cname = format!("certo_{}", c.name.replace('.', "_"));
            const_lines.push(format!("@{cname} = global {llty} {val}"));
        }
    }

    // ── Assemble output ───────────────────────────────────────────────
    let mut out = String::new();

    writeln!(out, "; Certo LLVM IR — generated by certo-llvm").unwrap();
    writeln!(out, "; Requires LLVM 14+ (opaque pointer mode)").unwrap();
    writeln!(out, "; Compile:  clang -O2 -o <out> <file>.ll").unwrap();
    writeln!(out, ";           clang --target=wasm32-wasi -O2 -o <out>.wasm <file>.ll").unwrap();
    writeln!(out).unwrap();

    if let Some(dl) = &opts.data_layout {
        writeln!(out, "target datalayout = \"{}\"", dl).unwrap();
    }
    if let Some(triple) = &opts.target_triple {
        writeln!(out, "target triple = \"{}\"", triple).unwrap();
    }
    if opts.data_layout.is_some() || opts.target_triple.is_some() {
        writeln!(out).unwrap();
    }

    // String literal globals
    for (idx, s) in ctx.str_lits.iter().enumerate() {
        let escaped = llvm_escape(s);
        let len     = s.len() + 1; // include NUL terminator
        writeln!(out,
            "@.str.{idx} = private unnamed_addr constant [{len} x i8] c\"{escaped}\\00\", align 1"
        ).unwrap();
    }
    if !ctx.str_lits.is_empty() {
        writeln!(out).unwrap();
    }

    // Module-level constants
    for line in &const_lines {
        writeln!(out, "{}", line).unwrap();
    }
    if !const_lines.is_empty() {
        writeln!(out).unwrap();
    }

    // External declarations (sorted for determinism)
    let mut externs: Vec<_> = ctx.externs.iter().collect();
    externs.sort_by_key(|(name, _)| name.as_str());
    for (name, (ret_ty, arg_tys)) in &externs {
        let arg_str = arg_tys.join(", ");
        writeln!(out, "declare {ret_ty} @{name}({arg_str})").unwrap();
    }
    if !ctx.externs.is_empty() {
        writeln!(out).unwrap();
    }

    // Function bodies
    for body in fn_bodies {
        out.push_str(&body);
        writeln!(out).unwrap();
    }

    // Program entry point. Mirrors the C backend's `main`: call the user's
    // zero-argument `main` and exit 0 (its result, if any, is ignored). The
    // user's function is `@certo_main`, so this never clashes with it.
    if !opts.no_entry && !is_wasm_triple(opts.target_triple.as_deref()) {
        if let Some(sig) = ctx.fn_sigs.get("certo_main").filter(|s| s.params.is_empty()) {
            writeln!(out, "define i32 @main() {{").unwrap();
            writeln!(out, "entry:").unwrap();
            if sig.ret == "void" {
                writeln!(out, "  call void @certo_main()").unwrap();
            } else {
                writeln!(out, "  %r = call {} @certo_main()", sig.ret).unwrap();
            }
            writeln!(out, "  ret i32 0").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;
    

    fn module_from(src: &str) -> Module {
        parse(src).expect("parse failed")
    }

    #[test]
    fn empty_module_emits_header() {
        let m = module_from("module Test\n");
        let ir = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("; Certo LLVM IR"), "missing header");
    }

    #[test]
    fn simple_fn_emits_define() {
        // Function body uses `= expr` syntax in Certo
        let src = "module Test\n\nfn add(a: Int, b: Int): Int = a + b\n";
        let m   = module_from(src);
        let ir  = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("define i64 @certo_add("), "missing define");
        assert!(ir.contains("add i64"), "missing add instruction");
        assert!(ir.contains("ret i64"), "missing ret");
    }

    #[test]
    fn bool_fn_emits_i1_ret() {
        let src = "module Test\n\nfn isZero(x: Int): Bool = x == 0\n";
        let m   = module_from(src);
        let ir  = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("define i1 @certo_isZero("), "missing bool define");
        assert!(ir.contains("icmp eq"), "missing icmp eq");
    }

    #[test]
    fn unit_fn_emits_void_ret() {
        let src = "module Test\n\nfn noop(): Unit = ()\n";
        let m   = module_from(src);
        let ir  = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("define void @certo_noop("), "missing void define");
        assert!(ir.contains("ret void"), "missing ret void");
    }

    #[test]
    fn string_constant_emits_global() {
        let src = "module Test\n\nfn greet(): Text = \"hello\"\n";
        let m   = module_from(src);
        let ir  = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("@.str.0"), "missing string global");
        assert!(ir.contains("hello"), "missing string content");
        assert!(ir.contains("getelementptr"), "missing GEP for string");
    }

    #[test]
    fn target_triple_emitted_when_set() {
        let src = "module Test\n";
        let m   = module_from(src);
        let opts = LlvmOptions {
            target_triple: Some("x86_64-unknown-linux-gnu".into()),
            ..Default::default()
        };
        let ir = emit_module(&m, &opts).unwrap();
        assert!(ir.contains("target triple = \"x86_64-unknown-linux-gnu\""));
    }

    #[test]
    fn pow_emits_extern_declare() {
        // `**` is the exponentiation operator in Certo
        let src = "module Test\n\nfn square(x: Int): Int = x ** x\n";
        let m   = module_from(src);
        let ir  = emit_module(&m, &LlvmOptions::default()).unwrap();
        assert!(ir.contains("declare i64 @certo_pow(i64, i64)"), "missing pow declare");
    }

    // ------------------------------------------------------------------ //
    // Calls between functions defined in the same module (BACKLOG item 429).
    // Before: every callee was `declare`d extern *and* defined, which clang
    // rejects ("invalid redefinition of function"), and the call's result was
    // typed `ptr` rather than the callee's real return type.
    // ------------------------------------------------------------------ //

    fn ir_of(src: &str) -> String {
        emit_module(&module_from(src), &LlvmOptions::default()).unwrap()
    }

    #[test]
    fn call_to_a_user_fn_is_not_declared_extern() {
        let ir = ir_of("module T
fn add(a: Int, b: Int): Int = a + b
fn run(): Int = add(1, 2)
");
        assert!(ir.contains("define i64 @certo_add("), "missing define
{ir}");
        assert!(!ir.contains("declare"), "a function the module defines must not also be declared
{ir}");
        assert!(ir.contains("call i64 @certo_add(i64 1, i64 2)"), "call must use the callee's i64 return type
{ir}");
    }

    #[test]
    fn forward_reference_and_recursion_are_local_calls() {
        let ir = ir_of(
            "module T
fn main(): Int = fact(4)
fn fact(n: Int): Int = if n <= 1 then 1 else n * fact(n - 1)
");
        assert!(!ir.contains("declare"), "callee defined after its caller / itself must not be declared
{ir}");
        assert_eq!(ir.matches("call i64 @certo_fact(").count(), 2, "one call from main, one recursive
{ir}");
    }

    #[test]
    fn unit_returning_user_fn_is_called_as_void() {
        let ir = ir_of("module T
fn noop(x: Int): Unit = ()
fn run(): Unit = noop(1)
");
        assert!(ir.contains("define void @certo_noop("), "{ir}");
        assert!(ir.contains("call void @certo_noop(i64 1)"), "{ir}");
        assert!(!ir.contains("declare"), "{ir}");
    }

    #[test]
    fn stdlib_calls_are_still_declared_extern() {
        // Not defined in this module, so it keeps the old behaviour.
        let ir = ir_of("module T
import Stdlib.Core
fn run(): Text = intToText(5)
");
        assert!(ir.contains("declare ptr @certo_intToText("), "{ir}");
    }

    // ------------------------------------------------------------------ //
    // Program entry point: a module with a zero-argument `main` gets a real
    // `define i32 @main()` so it links and runs on its own. Mirrors the C
    // backend: call the user's main, exit 0, ignore its result.
    // ------------------------------------------------------------------ //

    #[test]
    fn unit_main_gets_a_program_entry_point() {
        let ir = ir_of("module T
fn noop(x: Int): Unit = ()
fn main(): Unit = noop(1)
");
        assert!(ir.contains("define void @certo_main()"), "the user's main keeps its own name
{ir}");
        assert!(ir.contains("define i32 @main() {
entry:
  call void @certo_main()
  ret i32 0
}"), "{ir}");
    }

    #[test]
    fn non_unit_main_result_is_ignored_by_the_entry_point() {
        let ir = ir_of("module T
fn main(): Int = 42
");
        assert!(ir.contains("%r = call i64 @certo_main()"), "{ir}");
        assert!(ir.contains("ret i32 0"), "exit code stays 0 like the C backend
{ir}");
    }

    #[test]
    fn no_entry_point_without_a_main_fn() {
        let ir = ir_of("module T
fn add(a: Int, b: Int): Int = a + b
");
        assert!(!ir.contains("@main"), "a library module must not gain a main
{ir}");
    }

    #[test]
    fn main_with_parameters_gets_no_entry_point() {
        // The C backend always calls `certo_main()` with no arguments, so a
        // main that expects some can't be wired up the same way.
        let ir = ir_of("module T
fn main(x: Int): Unit = ()
");
        assert!(!ir.contains("@main()"), "{ir}");
    }

    #[test]
    fn no_entry_option_suppresses_the_entry_point() {
        let opts = LlvmOptions { no_entry: true, ..Default::default() };
        let ir = emit_module(&module_from("module T
fn main(): Unit = ()
"), &opts).unwrap();
        assert!(ir.contains("define void @certo_main()"), "{ir}");
        assert!(!ir.contains("define i32 @main"), "for linking into a host that has its own main
{ir}");
    }

    #[test]
    fn wasm_targets_get_no_entry_point() {
        // WASI's entry convention differs (a mangled __main_void) and is not
        // verified here, so wasm output is left as it was.
        for triple in ["wasm32-wasi", "wasm32-unknown-unknown"] {
            let opts = LlvmOptions { target_triple: Some(triple.into()), ..Default::default() };
            let ir = emit_module(&module_from("module T
fn main(): Unit = ()
"), &opts).unwrap();
            assert!(!ir.contains("define i32 @main"), "{triple}
{ir}");
        }
    }
}
