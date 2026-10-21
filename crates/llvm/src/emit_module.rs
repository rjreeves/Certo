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
use crate::emit_fn::{emit_fn, emit_lifted_fn, lifted_sig};
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

    /// Keep going past constructs the backend can't compile: declare an
    /// unknown runtime call as an external and emit a null placeholder for a
    /// value it can't build, instead of failing. For IR that is linked against
    /// a runtime the caller supplies; the default is to report them.
    pub allow_unresolved: bool,
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
    ctx.allow_unresolved = opts.allow_unresolved;
    let mut fn_bodies: Vec<String> = Vec::new();

    // Build HirFn lookup by name for MIR lowering.
    let hir_fn_map: std::collections::HashMap<&str, &certo_hir::HirFn> = hir.items.iter()
        .filter_map(|item| if let HirItem::Fn(f) = item { Some((f.name.as_str(), f)) } else { None })
        .collect();

    // Layout of every record type, in declared field order. The field types
    // come from HIR (already resolved); the names from the declaration.
    for sdecl in &module.decls {
        if let Decl::Type(t) = &sdecl.node {
            if let certo_ast::decl::TypeBody::Record(rec) = &t.body {
                if let Some(tys) = hir.record_field_types.get(&t.name.node) {
                    if tys.len() == rec.fields.len() {
                        ctx.record_tys.insert(t.name.node.clone(), tys.clone());
                        ctx.records.insert(t.name.node.clone(), crate::ctx::RecordLayout {
                            fields: rec.fields.iter().zip(tys)
                                .map(|(f, ty)| (f.name.node.clone(), crate::ty::llvm_ty(ty)))
                                .collect(),
                        });
                    }
                }
            }
        }
    }

    // Layout of every sum type, generic ones (`Maybe<T>`) included: a payload typed
    // by a type parameter is an erased pointer-sized slot. One whose payload types
    // HIR could not give us is left out, so using it is reported as unsupported.
    for sdecl in &module.decls {
        if let Decl::Type(t) = &sdecl.node {
            let certo_ast::decl::TypeBody::Sum(variants) = &t.body else { continue };
            let mut layout = Vec::new();
            let mut ok = true;
            for (tag, v) in variants.iter().enumerate() {
                let tys = hir.variant_field_types.get(&v.name.node);
                if !v.fields.is_empty() && tys.map(|t| t.len()) != Some(v.fields.len()) { ok = false; break; }
                let fields = v.fields.iter().enumerate().map(|(i, f)| (
                    f.name.as_ref().map(|n| n.node.clone()).unwrap_or_else(|| format!("f{i}")),
                    crate::ty::llvm_ty(&tys.unwrap()[i]),
                )).collect();
                layout.push(crate::ctx::VariantLayout { name: v.name.node.clone(), tag, fields });
            }
            if !ok { continue; }
            for v in &layout { ctx.variant_enum.insert(v.name.clone(), t.name.node.clone()); }
            ctx.variant_tys.insert(t.name.node.clone(), variants.iter().enumerate().map(|(tag, v)| (
                tag,
                if v.fields.is_empty() { Vec::new() } else { hir.variant_field_types.get(&v.name.node).cloned().unwrap_or_default() },
            )).collect());
            ctx.enums.insert(t.name.node.clone(), crate::ctx::EnumLayout { variants: layout });
        }
    }

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
    // Iterate AST decls (not HIR) to access declared param/ret types. Every
    // function is lowered first, so the lambdas lifted out of them can be
    // registered before any body is emitted.
    let mut lowered = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Fn(ast_fn) = &sdecl.node {
            if let Some(hir_fn) = hir_fn_map.get(ast_fn.name.node.as_str()) {
                let (mf, lifted) = lower_fn(hir_fn, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys, &hir.variant_to_type);
                lowered.push((ast_fn, mf, lifted));
            }
        }
    }
    for (_, _, lifted) in &lowered {
        for lf in lifted {
            let (ret, params) = lifted_sig(lf);
            ctx.register_fn(llvm_fn_name(&lf.name), ret, params);
        }
    }
    for (ast_fn, mf, lifted) in &lowered {
        fn_bodies.push(emit_fn(ast_fn, mf, &mut ctx));
        for lf in lifted {
            fn_bodies.push(emit_lifted_fn(lf, &mut ctx));
        }
    }

    // Emit global constants (from HIR)
    let mut const_lines: Vec<String> = Vec::new();
    for item in &hir.items {
        if let HirItem::Const(c) = item {
            use certo_hir::HirExprKind;
            let (llty, val): (String, String) = match &c.value.kind {
                HirExprKind::Int(n)   => ("i64".into(), n.to_string()),
                HirExprKind::Float(f) => ("double".into(), crate::ty::llvm_double(*f)),
                HirExprKind::Bool(b)  => ("i1".into(), if *b { "1".into() } else { "0".into() }),
                HirExprKind::Str(s)   => {
                    let idx = ctx.intern_str(s);
                    let len = s.len() + 1;
                    let gep = format!(
                        "getelementptr [{len} x i8], ptr @.str.{idx}, i32 0, i32 0"
                    );
                    ("ptr".into(), gep)
                }
                _ => {
                    ctx.unsupported(&c.name, "a module-level value that is not a plain number, Bool or Text is not supported by the LLVM backend");
                    ("ptr".into(), "null".into())
                }
            };
            let cname = format!("certo_{}", c.name.replace('.', "_"));
            const_lines.push(format!("@{cname} = global {llty} {val}"));
        }
    }

    // Everything the backend couldn't compile, reported together.
    if !ctx.errors.is_empty() {
        return Err(format!(
            "{} construct(s) the LLVM backend cannot compile:\n  {}\n(use --allow-unresolved to emit IR anyway, for linking against your own runtime)",
            ctx.errors.len(),
            ctx.errors.join("\n  "),
        ));
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

    // Payload-free variants used as values: one shared, zero-padded block each,
    // as large as any value of their type so a `match` can read payload slots
    // from it safely.
    for (ty, variant) in &ctx.used_variant_consts {
        let layout = &ctx.enums[ty];
        let tag = layout.variant(variant).map(|v| v.tag).unwrap_or(0);
        let mut slots = vec![format!("i64 {tag}")];
        slots.extend((1..layout.slots()).map(|_| "i64 0".to_string()));
        writeln!(out, "@certo_variant_{ty}_{variant} = internal constant [{} x i64] [{}]",
            layout.slots(), slots.join(", ")).unwrap();
    }
    if !ctx.used_variant_consts.is_empty() {
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

    // Structural-equality functions
    for body in &ctx.eq_bodies {
        out.push_str(body);
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
            // A program that reads its arguments gets them from the C `main`.
            let uses_args = out.contains("@certo_rt_arg(");
            if uses_args {
                writeln!(out, "declare void @{}(i32, ptr)", crate::rt::SET_ARGS).unwrap();
                writeln!(out, "define i32 @main(i32 %argc, ptr %argv) {{").unwrap();
            } else {
                writeln!(out, "define i32 @main() {{").unwrap();
            }
            writeln!(out, "entry:").unwrap();
            if uses_args {
                writeln!(out, "  call void @{}(i32 %argc, ptr %argv)", crate::rt::SET_ARGS).unwrap();
            }
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
        assert!(ir.contains("declare i64 @certo_rt_pow(i64, i64)"), "missing pow declare");
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
    fn supported_runtime_calls_are_declared_with_their_real_signature() {
        // Not defined in this module, so it is declared, but as the shim's
        // `certo_rt_*` symbol with the real parameter and return types.
        let ir = ir_of("module T
import Stdlib.Core
fn run(): Text = intToText(5)
");
        assert!(ir.contains("declare ptr @certo_rt_int_to_text(i64)"), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_int_to_text(i64 5)"), "{ir}");
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

    // ------------------------------------------------------------------ //
    // Runtime support (BACKLOG item 429): the calls the shim implements get
    // their real signatures, and anything else is reported instead of being
    // left as an unresolved external.
    // ------------------------------------------------------------------ //

    fn emit_err(src: &str) -> String {
        emit_module(&module_from(src), &LlvmOptions::default()).expect_err("expected an unsupported-construct error")
    }

    #[test]
    fn println_is_a_void_call_to_the_shim() {
        let ir = ir_of("module T
import Stdlib.Core
fn main(): Unit [io] = println(\"hi\")
");
        assert!(ir.contains("declare void @certo_rt_println(ptr)"), "{ir}");
        assert!(ir.contains("call void @certo_rt_println(ptr "), "{ir}");
    }

    #[test]
    fn text_concat_and_pow_use_the_shim_symbols() {
        let ir = ir_of("module T
fn join(a: Text, b: Text): Text = a ++ b
fn sq(x: Int): Int = x ** 2
");
        assert!(ir.contains("call ptr @certo_rt_text_concat(ptr "), "{ir}");
        assert!(ir.contains("call i64 @certo_rt_pow(i64 "), "{ir}");
        assert!(!ir.contains("@certo_text_concat") && !ir.contains("@certo_pow"), "{ir}");
    }

    #[test]
    fn bool_arguments_and_results_are_zero_extended_for_c() {
        // A C `bool` parameter assumes the caller zero-extended it; a bare
        // `i1` does not promise that.
        let ir = ir_of("module T
import Stdlib.Core
fn show(b: Bool): Text = boolToText(b)
fn same(a: Text, b: Text): Bool = Text.eq(a, b)
");
        assert!(ir.contains("declare ptr @certo_rt_bool_to_text(i1 zeroext)"), "{ir}");
        assert!(ir.contains("declare zeroext i1 @certo_rt_text_eq(ptr, ptr)"), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_bool_to_text(i1 zeroext "), "{ir}");
        assert!(ir.contains("call zeroext i1 @certo_rt_text_eq(ptr "), "{ir}");
    }

    #[test]
    fn float_constants_are_written_as_hex_bit_patterns() {
        // `2e0` (Rust's `{:e}`) is rejected by LLVM as an integer constant.
        let ir = ir_of("module T
fn half(x: Float): Float = x / 2.0
");
        assert!(ir.contains("fdiv double %v0, 0x4000000000000000"), "{ir}");
    }

    #[test]
    fn an_unsupported_runtime_call_is_an_error_naming_the_call() {
        let err = emit_err("module T
import Stdlib.Core
fn run(): Unit [io] = println(Json.stringify(Json.object()))
");
        assert!(err.contains("call to `Json.stringify` is not supported"), "{err}");
        assert!(err.contains("in `run`"), "{err}");
        assert!(err.contains("println"), "the message lists what is supported: {err}");
    }

    #[test]
    fn equality_on_records_calls_a_generated_structural_function() {
        // BACKLOG item 437.
        let ir = ir_of("module T
type P = { x: Int, name: Text }
fn same(a: P, b: P): Bool = a == b
");
        assert!(ir.contains("define internal zeroext i1 @certo_eq_0(ptr %a, ptr %b)"), "{ir}");
        assert!(ir.contains("call zeroext i1 @certo_eq_0(ptr "), "{ir}");
        assert!(ir.contains("@certo_rt_text_eq"), "the Text field is compared by content
{ir}");
    }

    #[test]
    fn equality_on_a_generic_sum_type_is_still_reported() {
        // The payload of `Just(T)` is erased, so there is no type to compare it as.
        let err = emit_err("module T
type Maybe<T> = Just(T) | Nothing
fn same(a: Maybe<Int>, b: Maybe<Int>): Bool = a == b
");
        assert!(err.contains("generic or unknown type"), "{err}");
    }

    #[test]
    fn text_equality_compares_contents_through_text_eq() {
        let ir = ir_of("module T
fn same(a: Text, b: Text): Bool = a == b
fn differ(a: Text): Bool = a != \"x\"
");
        assert!(ir.contains("call zeroext i1 @certo_rt_text_eq(ptr "), "not an address comparison
{ir}");
        assert!(ir.contains("xor i1 %v"), "!= negates the result
{ir}");
        assert!(!ir.contains("icmp eq ptr") && !ir.contains("icmp ne ptr"), "{ir}");
    }

    // ------------------------------------------------------------------ //
    // Records: a heap block with one 8-byte slot per field, in declared order.
    // ------------------------------------------------------------------ //

    const POINT: &str = "module T
import Stdlib.Core
type Point = { x: Int, y: Int, label: Text }
";

    #[test]
    fn a_record_literal_allocates_a_block_and_stores_each_field() {
        let ir = ir_of(&format!("{POINT}fn mk(a: Int): Point = Point {{ x: a, y: 2, label: \"p\" }}\n"));
        assert!(ir.contains("declare ptr @certo_rt_alloc(i64)"), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_alloc(i64 24)"), "three 8-byte slots\n{ir}");
        assert!(ir.contains("getelementptr i8, ptr %v0, i64 0"), "{ir}");
        assert!(ir.contains("getelementptr i8, ptr %v") && ir.contains("i64 16"), "the third slot\n{ir}");
    }

    #[test]
    fn field_access_loads_from_the_fields_declared_slot() {
        let ir = ir_of(&format!("{POINT}fn gety(p: Point): Int = p.y\nfn getl(p: Point): Text = p.label\n"));
        assert!(ir.contains("i64 8") && ir.contains("load i64, ptr %v"), "y is slot 1, an i64\n{ir}");
        assert!(ir.contains("i64 16") && ir.contains("load ptr, ptr %v"), "label is slot 2, a ptr\n{ir}");
    }

    #[test]
    fn a_literal_in_a_different_field_order_uses_the_declared_slots() {
        // Same layout whichever order the literal lists its fields in, or a
        // field read would pick up the wrong slot.
        let ir = ir_of(&format!("{POINT}fn mk(): Point = Point {{ label: \"p\", y: 2, x: 1 }}\n"));
        assert!(ir.contains("i64 0\n  store i64 1"), "x (declared first) must be stored at slot 0\n{ir}");
        assert!(ir.contains("i64 8\n  store i64 2"), "y must be stored at slot 1\n{ir}");
        assert!(ir.contains("i64 16\n  store ptr "), "label (listed first in the literal) must be stored at slot 2\n{ir}");
    }

    #[test]
    fn allow_unresolved_keeps_the_old_declare_everything_behaviour() {
        let opts = LlvmOptions { allow_unresolved: true, ..Default::default() };
        let ir = emit_module(&module_from("module T
import Stdlib.Core
fn run(): Text = Json.stringify(Json.object())
"), &opts).unwrap();
        assert!(ir.contains("declare ptr @certo_Json_stringify("), "{ir}");
        assert!(!ir.contains("@certo_coalesce"), "`??` is compiled inline now
{ir}");
    }

    #[test]
    fn calls_between_user_functions_still_need_no_runtime() {
        // Nothing here is a runtime call, so nothing is declared or reported.
        let ir = ir_of("module T
fn add(a: Int, b: Int): Int = a + b
fn run(): Int = add(1, 2)
");
        assert!(!ir.contains("declare"), "{ir}");
    }

    // ------------------------------------------------------------------ //
    // Sum types: a heap block of 8-byte slots, tag in slot 0, payload after.
    // ------------------------------------------------------------------ //

    const SHAPES: &str = "module T
import Stdlib.Core
type Color = Red | Green | Blue
type Shape = Circle(r: Int) | Rect(w: Int, h: Int) | Dot
";

    #[test]
    fn a_payload_free_variant_is_a_shared_constant_sized_for_its_type() {
        let ir = ir_of(&format!("{SHAPES}fn pick(): Shape = Dot
"));
        // Shape's widest variant (Rect) has two fields: tag + 2 = 3 slots, so a
        // match can read payload slots from Dot without running off the end.
        assert!(ir.contains("@certo_variant_Shape_Dot = internal constant [3 x i64] [i64 2, i64 0, i64 0]"), "{ir}");
        assert!(ir.contains("ret ptr"), "{ir}");
    }

    #[test]
    fn only_the_variant_constants_that_are_used_are_emitted() {
        let ir = ir_of(&format!("{SHAPES}fn pick(): Color = Green
"));
        assert!(ir.contains("@certo_variant_Color_Green"), "{ir}");
        assert!(!ir.contains("@certo_variant_Color_Red") && !ir.contains("@certo_variant_Shape_Dot"), "{ir}");
    }

    #[test]
    fn a_payload_variant_allocates_a_block_and_stores_tag_then_fields() {
        let ir = ir_of(&format!("{SHAPES}fn mk(a: Int, b: Int): Shape = Rect(a, b)
"));
        assert!(ir.contains("call ptr @certo_rt_alloc(i64 24)"), "tag + two payload slots
{ir}");
        assert!(ir.contains("@certo_rt_alloc(i64 24)
  store i64 1, ptr %v"), "Rect is the second variant: tag 1 goes in slot 0
{ir}");
        assert!(ir.contains("i64 8
  store i64 %"), "first field in slot 1
{ir}");
        assert!(ir.contains("i64 16
  store i64 %"), "second field in slot 2
{ir}");
    }

    #[test]
    fn match_compares_the_tag_slot_with_the_variants_ordinal() {
        let ir = ir_of(&format!("{SHAPES}fn tell(s: Shape): Int = match s {{
 Circle(r) => r
 Rect(w, h) => w
 Dot => 0
}}
"));
        assert!(ir.contains("getelementptr i8, ptr %v0, i64 0"), "the tag is read from slot 0
{ir}");
        assert!(ir.contains("icmp eq i64 %") && ir.contains(", 1
"), "Rect's tag is 1
{ir}");
        assert!(ir.contains("i64 8
") && ir.contains("i64 16
"), "payload fields read from slots 1 and 2
{ir}");
    }

    #[test]
    fn a_sum_type_with_type_parameters_gets_a_layout() {
        // BACKLOG item 437 - the payload of `Just(T)` is an erased slot.
        let ir = ir_of("module T
type Maybe<T> = Just(v: T) | Nothing
fn f(m: Maybe<Int>): Int = match m {
 Just(v) => v
 Nothing => 0
}
fn g(): Maybe<Int> = Just(4)
");
        assert!(ir.contains("define i64 @certo_f(ptr "), "{ir}");
    }

    // ------------------------------------------------------------------ //
    // Option and Result. Option is a nullable pointer to a boxed payload
    // (`None` is null); Result is a block {is_ok, payload}; `??` is inline.
    // ------------------------------------------------------------------ //

    #[test]
    fn some_boxes_the_payload_and_none_is_null() {
        let ir = ir_of("module T
fn find(n: Int): Int? = if n > 0 then Some(n) else None
");
        assert!(ir.contains("call ptr @certo_rt_alloc(i64 8)"), "Some allocates an 8-byte slot
{ir}");
        assert!(ir.contains("store ptr null") || ir.contains("ptr null"), "None is the null pointer
{ir}");
    }

    #[test]
    fn match_on_an_option_tests_the_pointer_against_null() {
        let ir = ir_of("module T
fn pick(o: Int?): Int = match o {
 Some(v) => v
 None => 0
}
");
        assert!(ir.contains("icmp ne ptr %v") && ir.contains(", null"), "present means non-null
{ir}");
        assert!(ir.contains("load i64, ptr %v"), "the payload is read back as an Int
{ir}");
    }

    #[test]
    fn coalesce_branches_on_null_and_loads_the_payload_as_the_result_type() {
        let ir = ir_of("module T
fn dflt(o: Int?): Int = o ?? 7
fn scale(o: Float?): Float = o ?? 1.5
");
        assert!(ir.contains("icmp eq ptr %v") && ir.contains("br i1"), "{ir}");
        assert!(ir.contains(".payload = load i64, ptr %v"), "an Int option loads an i64
{ir}");
        assert!(ir.contains(".payload = load double, ptr %v"), "a Float option loads a double
{ir}");
        assert!(ir.contains("phi i64 [") && ir.contains("phi double ["), "the two paths are joined with a phi
{ir}");
        assert!(!ir.contains("@certo_coalesce"), "no external helper
{ir}");
    }

    #[test]
    fn ok_and_err_build_a_two_slot_block_with_the_flag_first() {
        let ir = ir_of("module T
fn half(n: Int): Result<Int, Text> = if n % 2 == 0 then Ok(n / 2) else Err(\"odd\")
");
        assert!(ir.matches("call ptr @certo_rt_alloc(i64 16)").count() == 2, "one block per constructor
{ir}");
        assert!(ir.contains("store i64 1, ptr %v"), "Ok sets the flag
{ir}");
        assert!(ir.contains("store i64 0, ptr %v"), "Err clears it
{ir}");
    }

    #[test]
    fn match_on_a_result_reads_the_flag_and_the_payload_slot() {
        let ir = ir_of("module T
import Stdlib.Core
fn show(r: Result<Int, Text>): Text = match r {
 Ok(v) => intToText(v)
 Err(e) => e
}
");
        assert!(ir.contains("icmp ne i64 %v"), "is_ok is slot 0 != 0
{ir}");
        assert!(ir.contains("getelementptr i8, ptr %v") && ir.contains(", i64 8"), "the payload is slot 1
{ir}");
        assert!(!ir.contains("__result"), "the intrinsics are compiled inline
{ir}");
    }

    #[test]
    fn other_option_and_result_helpers_are_still_reported() {
        let err = emit_err("module T
import Stdlib.Core
fn f(o: Int?): Int = Option.unwrapOr(o, 0)
");
        assert!(err.contains("call to `Option.unwrapOr` is not supported"), "{err}");
    }

    // ------------------------------------------------------------------ //
    // Lists and tuples: a list is { len, cap, data } with one 8-byte slot per
    // element, built and indexed through the runtime shim; a tuple is a block of
    // 8-byte slots.
    // ------------------------------------------------------------------ //

    #[test]
    fn a_list_literal_asks_the_runtime_for_a_list_and_stores_each_element() {
        let ir = ir_of("module T
fn nums(): List<Int> = [1, 2, 3]
");
        assert!(ir.contains("declare ptr @certo_rt_list_new(i64)"), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_list_new(i64 3)"), "{ir}");
        assert!(ir.contains("getelementptr i8, ptr %v0, i64 16"), "the data pointer is at offset 16
{ir}");
        assert!(ir.contains("i64 8
  store i64 2,") && ir.contains("i64 16
  store i64 3,"), "elements go in 8-byte slots
{ir}");
    }

    #[test]
    fn list_len_and_get_or_panic_go_through_the_runtime_with_a_typed_load() {
        let ir = ir_of("module T
fn count(xs: List<Int>): Int = List.len(xs)
fn second(xs: List<Float>): Float = List.getOrPanic(xs, 1)
");
        assert!(ir.contains("call i64 @certo_rt_list_len(ptr "), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_list_slot(ptr %v"), "the runtime bounds-checks and returns the slot
{ir}");
        assert!(ir.contains("load double, ptr %v"), "the element is loaded as its own type
{ir}");
    }

    #[test]
    fn list_get_first_and_last_return_the_slot_as_an_option() {
        let ir = ir_of("module T
fn a(xs: List<Int>): Int? = List.get(xs, 0)
fn b(xs: List<Int>): Int? = List.first(xs)
fn c(xs: List<Int>): Int? = List.last(xs)
");
        assert!(ir.contains("declare ptr @certo_rt_list_get(ptr, i64)"), "{ir}");
        assert!(ir.contains("declare ptr @certo_rt_list_first(ptr)"), "{ir}");
        assert!(ir.contains("declare ptr @certo_rt_list_last(ptr)"), "{ir}");
    }

    #[test]
    fn list_push_passes_the_item_as_an_i64_slot_value() {
        let ir = ir_of("module T
fn grow(xs: List<Int>): List<Int> = List.push(xs, 4)
fn growf(xs: List<Float>): List<Float> = List.push(xs, 1.5)
");
        assert!(ir.contains("declare ptr @certo_rt_list_push(ptr, i64)"), "{ir}");
        assert!(ir.contains("bitcast double") && ir.contains("to i64"), "a Float travels as its bits
{ir}");
    }

    #[test]
    fn a_tuple_is_a_block_of_slots_read_by_position() {
        let ir = ir_of("module T
import Stdlib.Core
fn pair(): (Int, Text) = (1, \"x\")
fn use_it(): Int = {
 val (a, b) = pair()
 a + Text.len(b)
}
");
        assert!(ir.contains("call ptr @certo_rt_alloc(i64 16)"), "two slots
{ir}");
        assert!(ir.contains("i64 8
  %v") && ir.contains("load ptr, ptr %v"), "field 1 is slot 1
{ir}");
    }

    #[test]
    fn a_lambda_is_a_lifted_function_and_a_closure_block() {
        // BACKLOG item 437 - the lambda becomes its own function (environment first), the
        // closure is a block holding it, and List.map is a runtime call taking that block.
        let ir = ir_of("module T
fn f(xs: List<Int>, k: Int): List<Int> = List.map(xs, (x) => x + k)
");
        assert!(ir.contains("define i64 @certo___lam_f_0_boxed(ptr %p1, i64 %p2)"), "{ir}");
        assert!(ir.contains("call ptr @certo_rt_list_map(ptr "), "{ir}");
    }

    #[test]
    fn a_function_the_backend_has_no_runtime_for_is_still_reported() {
        let err = emit_err("module T
fn f(xs: List<Int>): List<Int> = List.reverse(xs)
");
        assert!(err.contains("`List.reverse`"), "{err}");
    }

    // ------------------------------------------------------------------ //
    // arg, getEnv, parseInt, Process.quit
    // ------------------------------------------------------------------ //

    #[test]
    fn arg_getenv_parseint_and_quit_are_runtime_calls() {
        let ir = ir_of("module T
import Stdlib.Core
fn a(): Text? = arg(1)
fn b(k: Text): Text? = getEnv(k)
fn c(s: Text): Int? = parseInt(s)
fn d(): Unit [io] = Process.quit(2)
");
        assert!(ir.contains("declare ptr @certo_rt_arg(i64)"), "{ir}");
        assert!(ir.contains("declare ptr @certo_rt_get_env(ptr)"), "{ir}");
        assert!(ir.contains("declare ptr @certo_rt_parse_int(ptr)"), "{ir}");
        assert!(ir.contains("declare void @certo_rt_quit(i64)"), "{ir}");
    }

    #[test]
    fn the_entry_point_takes_argc_and_argv_only_when_the_program_uses_arg() {
        let with = ir_of("module T
import Stdlib.Core
fn main(): Unit [io] = println(arg(1) ?? \"none\")
");
        assert!(with.contains("define i32 @main(i32 %argc, ptr %argv) {"), "{with}");
        assert!(with.contains("call void @certo_rt_set_args(i32 %argc, ptr %argv)"), "{with}");

        let without = ir_of("module T
fn main(): Unit = ()
");
        assert!(without.contains("define i32 @main() {"), "other programs keep the plain entry point
{without}");
        assert!(!without.contains("set_args"), "{without}");
    }
}
