//! `certo-ffi` — C header and REST client generator.
//!
//! # Usage
//!
//!   certo-ffi --header <file.certo> [-o <out.h>] [--guard <GUARD_H>]
//!   certo-ffi --rest-client <schema.json> [-o <out.certo>]
//!
//! # Options
//!
//!   --header <file>       Generate a C header from Certo pub fns
//!   --rest-client <file>  Generate Certo client stubs from a REST JSON schema
//!   -o <file>             Output path (default: stdout)
//!   --guard <NAME>        Override the C include-guard name (--header only)
//!
//! # Exit codes
//!
//!   0  success
//!   1  compile / generation error
//!   2  usage error

use std::path::PathBuf;
use std::process;

use certo_parser::parse;
use certo_ffi::{generate_header, parse_schema, generate_client};

fn main() {
    let mut args    = std::env::args().skip(1).peekable();
    let mut mode:     Option<Mode>    = None;
    let mut output:   Option<String>  = None;
    let mut guard:    Option<String>  = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--header" => {
                let path = args.next().unwrap_or_else(|| die("--header requires a path", 2));
                mode = Some(Mode::Header(PathBuf::from(path)));
            }
            "--rest-client" => {
                let path = args.next().unwrap_or_else(|| die("--rest-client requires a path", 2));
                mode = Some(Mode::Rest(PathBuf::from(path)));
            }
            "-o" => {
                output = Some(args.next().unwrap_or_else(|| die("-o requires a path", 2)));
            }
            "--guard" => {
                guard = Some(args.next().unwrap_or_else(|| die("--guard requires a name", 2)));
            }
            other => die(&format!("unknown option '{}'", other), 2),
        }
    }

    let mode = mode.unwrap_or_else(|| {
        eprintln!("usage: certo-ffi --header <file.certo> [-o <out.h>]");
        eprintln!("       certo-ffi --rest-client <schema.json> [-o <out.certo>]");
        process::exit(2);
    });

    let generated = match mode {
        Mode::Header(path) => {
            let src = read_file(&path);
            let module = parse(&src).unwrap_or_else(|errs| {
                for e in &errs { eprintln!("parse error: {:?}", e); }
                process::exit(1);
            });
            generate_header(&module, guard.as_deref())
        }
        Mode::Rest(path) => {
            let json = read_file(&path);
            let schema = parse_schema(&json).unwrap_or_else(|e| {
                eprintln!("error: {}", e);
                process::exit(1);
            });
            generate_client(&schema)
        }
    };

    match output {
        Some(ref path) if path != "-" => {
            std::fs::write(path, &generated).unwrap_or_else(|e| {
                eprintln!("error writing {}: {}", path, e);
                process::exit(1);
            });
            eprintln!("wrote {}", path);
        }
        _ => print!("{}", generated),
    }
}

// ------------------------------------------------------------------ //

enum Mode {
    Header(PathBuf),
    Rest(PathBuf),
}

fn read_file(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", path.display(), e);
        process::exit(1);
    })
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}
