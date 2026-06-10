//! `certo-wasm` — compile Certo source to WebAssembly.
//!
//! # Usage
//!
//!   certo-wasm [OPTIONS] <file.cto>
//!
//! # Options
//!
//!   -o <file>              Output path (.wasm default, .ll with --emit-ir)
//!   --target wasi          Target wasm32-wasi (default)
//!   --target browser       Target wasm32-unknown-unknown
//!   --wasi-sysroot <path>  Path to WASI sysroot (auto-detected if omitted)
//!   --opt <0-3>            Optimisation level (default: 2)
//!   --emit-ir              Emit LLVM IR (.ll) instead of compiling to .wasm
//!   --bindings             Generate .d.ts and .js binding files alongside .wasm
//!   --html                 Also generate a browser demo .html file
//!   --no-color             Disable ANSI colour output
//!
//! # Exit codes
//!
//!   0  success
//!   1  compile / toolchain error
//!   2  usage error

use std::path::PathBuf;
use std::process;

use certo_parser::parse;
use certo_wasm::{
    WasmOptions, WasmTarget, WasmError,
    compile_module, emit_ir_only,
    bindgen::{generate_dts, generate_js, generate_html},
    toolchain::detect as detect_toolchain,
};

fn main() {
    let mut args   = std::env::args().skip(1).peekable();
    let mut input:        Option<PathBuf>  = None;
    let mut output:       Option<String>   = None;
    let mut target        = WasmTarget::Wasi;
    let mut wasi_sysroot: Option<PathBuf>  = None;
    let mut opt_level:    u8               = 2;
    let mut emit_ir       = false;
    let mut gen_bindings  = false;
    let mut gen_html      = false;
    let mut color         = true;
    let mut extra_flags:  Vec<String>      = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => {
                output = Some(args.next().unwrap_or_else(|| die("-o requires a path", 2)));
            }
            "--target" => {
                let t = args.next().unwrap_or_else(|| die("--target requires wasi|browser", 2));
                target = match t.as_str() {
                    "wasi"    => WasmTarget::Wasi,
                    "browser" => WasmTarget::Browser,
                    other     => die(&format!("unknown target '{}' (use wasi or browser)", other), 2),
                };
            }
            "--wasi-sysroot" => {
                wasi_sysroot = Some(PathBuf::from(
                    args.next().unwrap_or_else(|| die("--wasi-sysroot requires a path", 2))
                ));
            }
            "--opt" => {
                let v = args.next().unwrap_or_else(|| die("--opt requires 0-3", 2));
                opt_level = v.parse().unwrap_or_else(|_| die("--opt must be 0-3", 2));
            }
            "--emit-ir"   => emit_ir      = true,
            "--bindings"  => gen_bindings = true,
            "--html"      => { gen_bindings = true; gen_html = true; }
            "--no-color"  => color        = false,
            "--"          => { extra_flags.extend(args); break; }
            other if other.starts_with('-') => {
                die(&format!("unknown option '{}'", other), 2);
            }
            path => {
                if input.is_some() { die("only one input file supported", 2); }
                input = Some(PathBuf::from(path));
            }
        }
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("usage: certo-wasm [OPTIONS] <file.cto>");
        eprintln!("       certo-wasm --help for option list");
        process::exit(2);
    });

    // ── Parse ─────────────────────────────────────────────────────────
    let src = std::fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", input.display(), e);
        process::exit(1);
    });
    let module = parse(&src).unwrap_or_else(|errs| {
        for e in &errs { eprintln!("parse error: {:?}", e); }
        process::exit(1);
    });

    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");

    // ── IR-only mode ─────────────────────────────────────────────────
    if emit_ir {
        let ir_path = output.clone().unwrap_or_else(|| format!("{}.ll", stem));
        let ir = emit_ir_only(&module, &target).unwrap_or_else(|e| {
            eprintln!("error: {}", e);
            process::exit(1);
        });
        if ir_path == "-" {
            print!("{}", ir);
        } else {
            std::fs::write(&ir_path, &ir).unwrap_or_else(|e| {
                eprintln!("error writing {}: {}", ir_path, e);
                process::exit(1);
            });
            status(color, &format!("wrote {}", ir_path));
            status(color, &format!(
                "compile with:  clang --target={} -O{} -o {}.wasm {}",
                target.triple(), opt_level, stem, ir_path
            ));
        }
        return;
    }

    // ── Check toolchain ───────────────────────────────────────────────
    match detect_toolchain() {
        None => {
            eprintln!("{}", WasmError::NoToolchain);
            eprintln!();
            eprintln!("Tip: use --emit-ir to write the LLVM IR for manual compilation.");
            process::exit(1);
        }
        Some(tc) => {
            status(color, &format!("using toolchain: {} ({})", tc.binary,
                match tc.kind {
                    certo_wasm::ToolchainKind::Clang      => "clang",
                    certo_wasm::ToolchainKind::Emscripten => "emscripten",
                }
            ));
        }
    }

    // ── Compile ───────────────────────────────────────────────────────
    let wasm_path = output.unwrap_or_else(|| format!("{}.wasm", stem));
    let opts = WasmOptions {
        target: target.clone(),
        wasi_sysroot,
        opt_level,
        extra_flags,
    };

    status(color, &format!("compiling {} → {}", input.display(), wasm_path));
    compile_module(&module, std::path::Path::new(&wasm_path), &opts)
        .unwrap_or_else(|e| {
            eprintln!("error: {}", e);
            process::exit(1);
        });
    status(color, &format!("wrote {}", wasm_path));

    // ── Bindings ──────────────────────────────────────────────────────
    if gen_bindings {
        let dts_path = format!("{}.d.ts", stem);
        let js_path  = format!("{}.js",   stem);

        let dts = generate_dts(&module, stem);
        std::fs::write(&dts_path, &dts).unwrap_or_else(|e| {
            eprintln!("warning: cannot write {}: {}", dts_path, e);
        });
        status(color, &format!("wrote {}", dts_path));

        let wasm_filename = std::path::Path::new(&wasm_path)
            .file_name().and_then(|n| n.to_str()).unwrap_or(&wasm_path);
        let js = generate_js(&module, stem, wasm_filename);
        std::fs::write(&js_path, &js).unwrap_or_else(|e| {
            eprintln!("warning: cannot write {}: {}", js_path, e);
        });
        status(color, &format!("wrote {}", js_path));

        if gen_html {
            let html_path = format!("{}.html", stem);
            let html = generate_html(stem, &js_path);
            std::fs::write(&html_path, &html).unwrap_or_else(|e| {
                eprintln!("warning: cannot write {}: {}", html_path, e);
            });
            status(color, &format!("wrote {}", html_path));
        }
    }
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}

fn status(color: bool, msg: &str) {
    if color {
        eprintln!("\x1b[32m✓\x1b[0m {}", msg);
    } else {
        eprintln!("{}", msg);
    }
}
