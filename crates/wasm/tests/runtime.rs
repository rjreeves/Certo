//! A program that prints builds for the browser target with the runtime shim
//! linked in (output through host imports); a purely numeric one gets no shim
//! and so no imports. Skipped where no wasm-capable clang is installed.

use certo_wasm::{compile_module, toolchain, WasmOptions, WasmTarget};

fn build_browser(src: &str) -> Option<Vec<u8>> {
    if toolchain::detect().is_none() {
        eprintln!("skipping: no wasm-capable clang found");
        return None;
    }
    let module = certo_parser::parse(src).expect("parse failed");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("m.wasm");
    let opts = WasmOptions { target: WasmTarget::Browser, opt_level: 2, ..Default::default() };
    compile_module(&module, &out, &opts).unwrap_or_else(|e| panic!("build failed: {e}"));
    Some(std::fs::read(&out).unwrap())
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn a_module_that_prints_imports_the_host_write_function() {
    let Some(wasm) = build_browser("module P\nimport Stdlib.Core\npub fn main(): Unit [io] = println(\"hi\")\n") else { return };
    assert!(wasm.starts_with(b"\0asm"), "not a wasm binary");
    assert!(contains(&wasm, "certo_write"), "println must import env.certo_write");
    assert!(contains(&wasm, "certo_main"), "main must stay exported so the loader can run it");
}

#[test]
fn text_and_number_formatting_needs_no_libc() {
    // The shim is freestanding: this would fail to link with an undefined
    // `strlen`/`memcpy` if the optimiser turned its loops into libc calls.
    let src = "module P\nimport Stdlib.Core\nfn label(n: Int): Text = \"n=\" ++ intToText(n)\n\
               pub fn main(): Unit [io] = {\n    println(label(7))\n    println(floatToText(1.5))\n    println(boolToText(true))\n}\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_float_to_text"), "floatToText goes through a host import");
}

#[test]
fn a_numeric_only_module_gets_no_runtime_and_no_imports() {
    let Some(wasm) = build_browser("module N\npub fn add(a: Int, b: Int): Int = a + b\n") else { return };
    assert!(!contains(&wasm, "certo_write"), "no shim should be linked into a numeric-only module");
    assert!(contains(&wasm, "certo_add"));
}

#[test]
fn records_build_for_the_browser() {
    // Record blocks come from the shim's allocator, which has no libc to lean on.
    let src = "module R\nimport Stdlib.Core\ntype P = { x: Int, label: Text }\n\
               fn mk(a: Int): P = P { label: \"p\", x: a }\n\
               pub fn main(): Unit [io] = {\n    val p = mk(5)\n    println(p.label ++ intToText(p.x))\n}\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_write"));
    assert!(contains(&wasm, "certo_rt_alloc"), "the allocator is exported from the shim");
}

#[test]
fn enums_and_match_build_for_the_browser() {
    let src = "module E\nimport Stdlib.Core\ntype Shape = Circle(r: Int) | Rect(w: Int, h: Int) | Dot\n\
               fn area(s: Shape): Int = match s {\n Circle(r) => r * r * 3\n Rect(w, h) => w * h\n Dot => 0\n}\n\
               pub fn main(): Unit [io] = println(intToText(area(Rect(3, 4))))\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_write"));
}

#[test]
fn options_results_and_coalesce_build_for_the_browser() {
    let src = "module O\nimport Stdlib.Core\n\
               fn find(n: Int): Int? = if n > 0 then Some(n) else None\n\
               fn half(n: Int): Result<Int, Text> = if n % 2 == 0 then Ok(n / 2) else Err(\"odd\")\n\
               fn show(r: Result<Int, Text>): Text = match r {\n Ok(v) => intToText(v)\n Err(e) => e\n}\n\
               pub fn main(): Unit [io] = {\n    println(intToText(find(3) ?? 0))\n    println(show(half(8)))\n}\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_write"));
}

#[test]
fn lists_and_tuples_build_for_the_browser() {
    // Slots are 8 bytes whatever the pointer width, and a Float list read must
    // not go through a 4-byte pointer.
    let src = "module L\nimport Stdlib.Core\n\
               fn fsum(xs: List<Float>): Float = {\n var t = 0.0\n for x in xs {\n  t = t + x\n }\n t\n}\n\
               pub fn main(): Unit [io] = {\n    val xs = List.push([1.5, 2.25], 4.0)\n    println(floatToText(fsum(xs)))\n    val (n, s) = (List.len(xs), \"items\")\n    println(intToText(n) ++ s)\n}\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_write"));
    assert!(contains(&wasm, "certo_rt_list_slot"), "the list runtime is exported from the shim");
}

#[test]
fn parse_int_arg_getenv_and_text_equality_build_for_the_browser() {
    // The browser has no argv or environment, so `arg` and `getEnv` are always
    // None there; parseInt is implemented in the shim without a libc.
    let src = "module P\nimport Stdlib.Core\n\
               pub fn main(): Unit [io] = {\n    val n = parseInt(\"123\") ?? -1\n    println(intToText(n))\n    println(arg(1) ?? \"no args\")\n    println(getEnv(\"HOME\") ?? \"no env\")\n    if \"a\" == \"a\" then println(\"equal\") else println(\"different\")\n}\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(contains(&wasm, "certo_write"));
    assert!(contains(&wasm, "certo_rt_parse_int"));
}

#[test]
fn closures_higher_order_lists_generic_enums_and_equality_build_for_the_browser() {
    // BACKLOG item 437: the browser shim has the closure-taking list functions, and the
    // freestanding runtime still needs no libc.
    let src = "module P\nimport Stdlib.Core\n\
               type Maybe<T> = Just(T) | Nothing\n\
               type Pt = { x: Int, label: Text }\n\
               fn pick(m: Maybe<Int>): Int = match m {\n    Just(v) => v\n    Nothing => 0\n}\n\
               fn same(a: Pt, b: Pt): Bool = a == b\n\
               pub fn main(): Unit [io] = {\n\
                   val k = 3\n\
                   val xs = List.map([1, 2, 3], (n) => n * k)\n\
                   var total = 0\n\
                   List.forEach(xs, (n) => { total = total + n })\n\
                   println(intToText(List.fold(xs, 0, (a, b) => a + b) + pick(Just(total))))\n\
                   println(boolToText(same(Pt { x: 1, label: \"a\" }, Pt { x: 1, label: \"a\" })))\n\
               }\n";
    let Some(wasm) = build_browser(src) else { return };
    assert!(wasm.starts_with(b"\0asm"), "not a wasm binary");
}
