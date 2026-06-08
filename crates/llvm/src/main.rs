//! `certo-llvm` — emit LLVM IR from a Certo source file.
//!
//! Usage:
//!   certo-llvm [OPTIONS] <file.certo>
//!
//! Options:
//!   -o <file>                   Output file (default: <stem>.ll, or stdout if -)
//!   --target <triple>           Target triple (e.g. x86_64-unknown-linux-gnu)
//!   --data-layout <layout>      LLVM data layout string
//!   --wasm                      Shorthand for --target wasm32-wasi
//!   --annotate                  Emit comment annotations in the IR
//!
//! Exit codes:
//!   0  success
//!   1  compile error
//!   2  usage error

use std::path::{Path, PathBuf};
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
        eprintln!("usage: certo-llvm [OPTIONS] <file.certo>");
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
    };

    let ir = emit_module(&module, &opts).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });

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
