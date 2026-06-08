use std::path::{Path, PathBuf};
use std::process;
use certo_ast::decl::{Decl, MigrationDecl};
use certo_migrate::{
    plan_up, plan_down, run_steps, status,
    default_manifest_path, RunOptions,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_top_help();
        process::exit(1);
    }

    // Allow `certo <file.certo> [options]` as shorthand for `certo build`.
    let first = args[1].as_str();
    if first.ends_with(".certo") || first == "build" {
        let build_args = if first == "build" { &args[2..] } else { &args[1..] };
        cmd_build(build_args);
    } else {
        match first {
            "migrate" => cmd_migrate(&args[2..]),
            "help" | "--help" | "-h" => { print_top_help(); }
            other => {
                eprintln!("Unknown command: {}", other);
                print_top_help();
                process::exit(1);
            }
        }
    }
}

// ------------------------------------------------------------------ //
// build
// ------------------------------------------------------------------ //

fn cmd_build(args: &[String]) {
    let mut input:   Option<PathBuf> = None;
    let mut output:  Option<PathBuf> = None;
    let mut verbose  = false;
    let mut emit_c   = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                output = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| die("-o requires a path", 2))
                ));
            }
            "--emit-c"  => emit_c   = true,
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                println!("Usage: certo <file.certo> [-o <out>] [--emit-c] [-v]");
                println!("       certo build <file.certo> [-o <out>] [--emit-c] [-v]");
                println!();
                println!("Options:");
                println!("  -o <file>    Output path (default: <stem>.exe on Windows, <stem> elsewhere)");
                println!("  --emit-c     Write the generated C to <stem>.c and stop (do not compile)");
                println!("  -v           Print the compiler command before running it");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => {
                if input.is_some() { die("only one input file supported", 2); }
                input = Some(PathBuf::from(path));
            }
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("error: no input file");
        eprintln!("usage: certo <file.certo> [-o <out>]");
        process::exit(2);
    });

    // ── Parse ─────────────────────────────────────────────────────────
    let src = std::fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", input.display(), e);
        process::exit(1);
    });
    let module = certo_parser::parse(&src).unwrap_or_else(|errs| {
        for e in &errs { eprintln!("parse error: {:?}", e); }
        process::exit(1);
    });

    // ── Check for entry point ─────────────────────────────────────────
    let has_main = module.decls.iter().any(|d| {
        matches!(&d.node, certo_ast::decl::Decl::Fn(f) if f.name.node == "main")
    });
    if !has_main && !emit_c {
        eprintln!("error: module has no `main` function");
        eprintln!("       use --emit-c to compile as a library, or add:");
        eprintln!("       fn main(): Unit [io] = {{ ... }}");
        process::exit(1);
    }

    // ── Emit C ────────────────────────────────────────────────────────
    // Order: system includes → runtime typedefs → stdlib impls → user code.
    let preamble = "#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\
                    #include <inttypes.h>\n#include <stdarg.h>\n\
                    #define _CRT_SECURE_NO_WARNINGS\n";
    let runtime_header = certo_codegen::RUNTIME_HEADER;
    let stdlib_c  = certo_stdlib::full_c_runtime();
    let module_c  = certo_codegen::emit_module(
        &module,
        &certo_codegen::CodegenOptions { inline_runtime: false },
    );
    // Strip the `#include "certo_runtime.h"` and duplicate system includes from
    // emit_module output — types are already supplied by preamble + runtime_header.
    let module_c = module_c.lines()
        .filter(|l| {
            !l.contains("certo_runtime.h") &&
            !l.contains("#include <stdint.h>") &&
            !l.contains("#include <stdbool.h>") &&
            !l.contains("#include <stddef.h>")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let full_c = format!("{}{}\n{}\n{}", preamble, runtime_header, stdlib_c, module_c);

    let stem = input.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");

    // --emit-c: just write the C and stop
    if emit_c {
        let c_path = output.unwrap_or_else(|| PathBuf::from(format!("{}.c", stem)));
        std::fs::write(&c_path, &full_c).unwrap_or_else(|e| {
            eprintln!("error writing {}: {}", c_path.display(), e);
            process::exit(1);
        });
        eprintln!("wrote {}", c_path.display());
        return;
    }

    // ── Write temp .c file ────────────────────────────────────────────
    let tmp = tempfile::Builder::new()
        .prefix("certo_")
        .suffix(".c")
        .tempfile()
        .unwrap_or_else(|e| { eprintln!("error creating temp file: {}", e); process::exit(1); });
    std::fs::write(tmp.path(), &full_c).unwrap_or_else(|e| {
        eprintln!("error writing temp file: {}", e);
        process::exit(1);
    });

    // ── Find a C compiler ─────────────────────────────────────────────
    let cc = find_cc().unwrap_or_else(|| {
        eprintln!("error: no C compiler found (tried cc, gcc, clang)");
        eprintln!("       install gcc or clang, or use --emit-c to get the C source");
        process::exit(1);
    });

    // ── Default output path ───────────────────────────────────────────
    let out_path = output.unwrap_or_else(|| {
        if cfg!(windows) {
            PathBuf::from(format!("{}.exe", stem))
        } else {
            PathBuf::from(stem)
        }
    });

    // ── Invoke compiler ───────────────────────────────────────────────
    let mut cmd = std::process::Command::new(&cc);
    cmd.arg(tmp.path())
       .arg("-o").arg(&out_path)
       .arg("-O2")
       .arg("-Wno-int-to-pointer-cast")
       .arg("-Wno-pointer-to-int-cast")
       .arg("-Wno-int-conversion")
       .arg("-Wno-implicit-function-declaration")
       .arg("-Wno-deprecated-declarations");
    // -lm is implicit on Windows (math is part of the UCRT)
    if !cfg!(windows) {
        cmd.arg("-lm");
    } else {
        // lld-link requires an explicit subsystem for console apps
        cmd.arg("-Xlinker").arg("/subsystem:console");
    }

    if verbose {
        let display: Vec<_> = std::iter::once(cc.as_str())
            .chain(cmd.get_args().filter_map(|a| a.to_str()))
            .collect();
        eprintln!("{}", display.join(" "));
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error running {}: {}", cc, e);
        process::exit(1);
    });

    if !status.success() {
        eprintln!("error: C compiler exited with code {}",
            status.code().unwrap_or(-1));
        process::exit(1);
    }

    eprintln!("wrote {}", out_path.display());
}

fn find_cc() -> Option<String> {
    // First try names on PATH.
    for candidate in &["clang", "gcc", "cc", "cl"] {
        if probe_cc(candidate) { return Some(candidate.to_string()); }
    }
    // Fall back to common Windows install locations.
    let windows_paths = [
        r"C:\Program Files\LLVM\bin\clang.exe",
        r"C:\Program Files (x86)\LLVM\bin\clang.exe",
    ];
    for path in &windows_paths {
        if std::path::Path::new(path).exists() && probe_cc(path) {
            return Some(path.to_string());
        }
    }
    None
}

fn probe_cc(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn print_top_help() {
    eprintln!("Certo compiler");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  certo <file.certo> [-o <out>]   Compile a Certo source file");
    eprintln!("  certo build <file.certo> ...     Same with explicit subcommand");
    eprintln!("  certo migrate <subcommand>       Database migration tools");
    eprintln!();
    eprintln!("Build options:");
    eprintln!("  -o <file>    Output binary path");
    eprintln!("  --emit-c     Stop after emitting C; write <stem>.c");
    eprintln!("  -v           Verbose: print the compiler command");
    eprintln!();
    eprintln!("Migrate subcommands:");
    eprintln!("  up [--dry-run]     Apply pending migrations");
    eprintln!("  down [N]           Roll back N migrations (default 1)");
    eprintln!("  status             Show applied/pending migrations");
    eprintln!("  create <name>      Scaffold a new migration file");
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}

// ------------------------------------------------------------------ //
// migrate (unchanged)
// ------------------------------------------------------------------ //

fn cmd_migrate(args: &[String]) {
    let sub = args.first().map(String::as_str).unwrap_or("");
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let manifest = default_manifest_path(&project_root);

    match sub {
        "up" => {
            let dry_run = args.contains(&"--dry-run".to_string());
            let migrations = load_migrations(&project_root);
            let state = certo_migrate::MigrationState::load(&manifest)
                .unwrap_or_else(|e| { eprintln!("error: {}", e); process::exit(1); });
            let steps = plan_up(&migrations, &state);
            if steps.is_empty() { println!("Nothing to migrate."); return; }
            let opts = RunOptions { dry_run, manifest_path: &manifest };
            match run_steps(&steps, &opts) {
                Ok(sql) => {
                    for stmt in &sql { println!("{}", stmt); }
                    if dry_run {
                        println!("-- dry run: {} statement(s) not executed", sql.len());
                    } else {
                        println!("Applied {} migration(s).", steps.len());
                    }
                }
                Err(e) => { eprintln!("error: {}", e); process::exit(1); }
            }
        }

        "down" => {
            let dry_run = args.contains(&"--dry-run".to_string());
            let count: usize = args.iter()
                .find(|a| a.parse::<usize>().is_ok())
                .and_then(|a| a.parse().ok())
                .unwrap_or(1);
            let migrations = load_migrations(&project_root);
            let state = certo_migrate::MigrationState::load(&manifest)
                .unwrap_or_else(|e| { eprintln!("error: {}", e); process::exit(1); });
            let steps = plan_down(&migrations, &state, count);
            if steps.is_empty() { println!("Nothing to roll back."); return; }
            let opts = RunOptions { dry_run, manifest_path: &manifest };
            match run_steps(&steps, &opts) {
                Ok(sql) => {
                    for stmt in &sql { println!("{}", stmt); }
                    if dry_run {
                        println!("-- dry run: {} statement(s) not executed", sql.len());
                    } else {
                        println!("Rolled back {} migration(s).", steps.len());
                    }
                }
                Err(e) => { eprintln!("error: {}", e); process::exit(1); }
            }
        }

        "status" => {
            let migrations = load_migrations(&project_root);
            match status(&migrations, &manifest) {
                Ok(rows) => {
                    println!("{:<40} {}", "Migration", "Applied At");
                    println!("{}", "-".repeat(60));
                    for (name, ts) in rows {
                        println!("{:<40} {}", name, ts.as_deref().unwrap_or("(pending)"));
                    }
                }
                Err(e) => { eprintln!("error: {}", e); process::exit(1); }
            }
        }

        "create" => {
            let name = args.get(1).map(String::as_str).unwrap_or("new_migration");
            let migrations_dir = project_root.join("migrations");
            std::fs::create_dir_all(&migrations_dir).unwrap_or_else(|e| {
                eprintln!("error creating migrations/: {}", e); process::exit(1);
            });
            let filename = migrations_dir.join(format!("{}.certo", name));
            let template = format!(
                "migration \"{}\" {{\n    up {{\n        // TODO: add operations\n    }}\n    down {{\n        // TODO: add rollback operations\n    }}\n}}\n",
                name
            );
            std::fs::write(&filename, template).unwrap_or_else(|e| {
                eprintln!("error writing file: {}", e); process::exit(1);
            });
            println!("Created {}", filename.display());
        }

        _ => {
            eprintln!("Unknown migrate subcommand: {}", sub);
            eprintln!("Usage: certo migrate up | down [N] | status | create <name>");
            process::exit(1);
        }
    }
}

fn load_migrations(project_root: &Path) -> Vec<MigrationDecl> {
    let migrations_dir = project_root.join("migrations");
    if !migrations_dir.exists() { return Vec::new(); }
    let mut result = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(&migrations_dir)
        .unwrap_or_else(|e| { eprintln!("error reading migrations/: {}", e); process::exit(1); })
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("certo"))
        .collect();
    paths.sort();
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("error reading {}: {}", path.display(), e); process::exit(1);
        });
        let module = certo_parser::parse(&src).unwrap_or_else(|e| {
            eprintln!("parse error in {}: {:?}", path.display(), e); process::exit(1);
        });
        for decl in &module.decls {
            if let Decl::Migration(m) = &decl.node { result.push(m.clone()); }
        }
    }
    result
}
