//! `certo-llvm` — emit LLVM IR from a Certo source file.
//!
//! Usage:
//!   certo-llvm [OPTIONS] <file.cto>
//!
//! Options:
//!   -o <file>                   Output file (default: <stem>.ll, or stdout if -)
//!   --target <triple>           Target triple (e.g. x86_64-unknown-linux-gnu)
//!   --data-layout <layout>      LLVM data layout string
//!   --wasm                      Shorthand for --target wasm32-wasi
//!   --annotate                  Emit comment annotations in the IR
//!   --no-entry                  Don't emit a program entry point (`main`), e.g. when
//!                               linking into a host that has its own
//!   --exe <file>                Also build a native executable: compiles the IR
//!                               together with the bundled runtime shim using
//!                               clang. Without -o, no .ll file is written.
//!   --allow-unresolved          Emit IR even for constructs the backend can't
//!                               compile (unknown runtime calls become external
//!                               declarations); for linking your own runtime
//!
//! Supported runtime calls: print, println, eprint, eprintln, intToText,
//! floatToText, boolToText, Text.eq, Text.len, plus `++` on Text and `**` on
//! Int. Anything else is reported as an error.
//!
//! Exit codes:
//!   0  success
//!   1  compile error
//!   2  usage error

use std::path::PathBuf;
use std::process;

use certo_llvm::{emit_module, LlvmOptions};
use certo_parser::parse;

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut input:       Option<PathBuf> = None;
    let mut output:      Option<String>  = None;
    let mut target:      Option<String>  = None;
    let mut data_layout: Option<String>  = None;
    let mut annotate = false;
    let mut no_entry = false;
    let mut allow_unresolved = false;
    let mut exe: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => {
                output = Some(args.next().unwrap_or_else(|| {
                    eprintln!("error: -o requires a filename");
                    process::exit(2);
                }));
            }
            "--target" => {
                target = Some(args.next().unwrap_or_else(|| {
                    eprintln!("error: --target requires a triple");
                    process::exit(2);
                }));
            }
            "--data-layout" => {
                data_layout = Some(args.next().unwrap_or_else(|| {
                    eprintln!("error: --data-layout requires a string");
                    process::exit(2);
                }));
            }
            "--wasm" => {
                target = Some("wasm32-wasi".into());
            }
            "--annotate" => annotate = true,
            "--no-entry" => no_entry = true,
            "--allow-unresolved" => allow_unresolved = true,
            "--exe" => {
                exe = Some(args.next().unwrap_or_else(|| {
                    eprintln!("error: --exe requires a filename");
                    process::exit(2);
                }));
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown option '{}'", other);
                process::exit(2);
            }
            path => {
                if input.is_some() {
                    eprintln!("error: only one input file supported");
                    process::exit(2);
                }
                input = Some(PathBuf::from(path));
            }
        }
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("usage: certo-llvm [OPTIONS] <file.cto>");
        process::exit(2);
    });

    // Parse
    let src = std::fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", input.display(), e);
        process::exit(1);
    });

    let module = parse(&src).unwrap_or_else(|errs| {
        for e in &errs {
            eprintln!("parse error: {:?}", e);
        }
        process::exit(1);
    });

    // Determine output path
    let explicit_output = output.is_some();
    let out_path = output.unwrap_or_else(|| {
        let stem = input.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out");
        format!("{}.ll", stem)
    });

    // Emit
    let opts = LlvmOptions {
        target_triple: target,
        data_layout,
        annotate,
        no_entry,
        allow_unresolved,
    };

    let ir = emit_module(&module, &opts).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });

    // Build a native executable from the IR plus the runtime shim.
    if let Some(exe) = &exe {
        if let Err(e) = certo_llvm::link_executable(&ir, std::path::Path::new(exe)) {
            eprintln!("error: {}", e);
            process::exit(1);
        }
        eprintln!("wrote {}", exe);
        // Only write the .ll as well when it was asked for with -o.
        if !explicit_output { return; }
    }

    // Write
    if out_path == "-" {
        print!("{}", ir);
    } else {
        std::fs::write(&out_path, &ir).unwrap_or_else(|e| {
            eprintln!("error: cannot write {}: {}", out_path, e);
            process::exit(1);
        });
        eprintln!("wrote {}", out_path);
    }
}
