//! `certo-ui` — compile Certo view/form declarations to a runnable HTTP server.
//!
//! # Usage
//!
//!   certo-ui [OPTIONS] <file.cto>
//!
//! # Options
//!
//!   -o <dir>       Output directory (default: current directory)
//!   --html         Legacy mode: emit one .html file per view/form instead of server.cto
//!   --stdout       Print generated output to stdout instead of writing files
//!   --no-color     Disable ANSI colour in status messages
//!
//! # Exit codes
//!
//!   0  success
//!   1  compile error
//!   2  usage error

use std::path::PathBuf;
use std::process;

use certo_parser::parse;
use certo_ui::{emit_module, emit_html};

fn main() {
    let mut args    = std::env::args().skip(1).peekable();
    let mut input:   Option<PathBuf> = None;
    let mut out_dir: PathBuf         = PathBuf::from(".");
    let mut stdout   = false;
    let mut color    = true;
    let mut html_mode = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => {
                out_dir = PathBuf::from(
                    args.next().unwrap_or_else(|| die("-o requires a directory", 2))
                );
            }
            "--stdout"   => stdout    = true,
            "--no-color" => color     = false,
            "--html"     => html_mode = true,
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
        eprintln!("usage: certo-ui [OPTIONS] <file.cto>");
        eprintln!("       --html       emit HTML files instead of server.cto");
        eprintln!("       -o <dir>     output directory (default: .)");
        eprintln!("       --stdout     print to stdout");
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

    // ── Emit ──────────────────────────────────────────────────────────
    let files = if html_mode {
        emit_html(&module)
    } else {
        emit_module(&module)
    }.unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });

    for (filename, content) in &files {
        if stdout {
            println!("=== {} ===", filename);
            print!("{}", content);
        } else {
            let path = out_dir.join(filename);
            std::fs::write(&path, content).unwrap_or_else(|e| {
                eprintln!("error writing {}: {}", path.display(), e);
                process::exit(1);
            });
            status(color, &format!("wrote {}", path.display()));
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
