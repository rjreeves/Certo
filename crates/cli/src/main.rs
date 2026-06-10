mod cmd_doc;
mod cmd_watch;


use std::path::{Path, PathBuf};
use std::process;
use certo_ast::decl::{Decl, MigrationDecl};
use certo_ast::module::Module;
use certo_migrate::{
    plan_up, plan_down, run_steps, status,
    default_manifest_path, RunOptions,
};
use certo_diagnostics::{Diagnostic, render_all};
use certo_typeck::{TypeError, TypeErrorKind, TypeEnv};

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
        cmd_build(build_args, false);
    } else {
        match first {
            "check"   => cmd_check(&args[2..]),
            "run"     => cmd_run(&args[2..]),
            "doc"     => cmd_doc::cmd_doc(&args[2..]),
            "fmt"     => cmd_fmt(&args[2..]),
            "test"    => cmd_test(&args[2..]),
            "lint"    => cmd_lint(&args[2..]),
            "bench"   => cmd_bench(&args[2..]),
            "new"     => cmd_new(&args[2..]),
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

// ------------------------------------------------------------------ //
// check
// ------------------------------------------------------------------ //

fn cmd_check(args: &[String]) {
    let mut input: Option<PathBuf> = None;
    let mut verbose = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                println!("Usage: certo check <file.certo> [-v]");
                println!();
                println!("Type-check a Certo source file without compiling.");
                println!("Exits 0 on success, 1 if there are parse or type errors.");
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
        eprintln!("usage: certo check <file.certo>");
        process::exit(2);
    });

    let colour = stderr_is_tty();
    let module = parse_file_or_exit(&input, colour);
    let filename = input.display().to_string();

    if verbose { eprintln!("checking {}...", filename); }

    let (module, src) = module;
    run_typeck(&module, &src, &filename, colour);

    eprintln!("ok");
}

// ------------------------------------------------------------------ //
// run
// ------------------------------------------------------------------ //

fn cmd_run(args: &[String]) {
    // Split args at `--`: everything before is build args, after is program args.
    let (build_args, prog_args) = if let Some(sep) = args.iter().position(|a| a == "--") {
        (&args[..sep], &args[sep + 1..])
    } else {
        (args, [].as_slice())
    };

    // Build to a temp directory.
    let tmp_dir = tempfile::TempDir::new().unwrap_or_else(|e| {
        eprintln!("error creating temp dir: {}", e);
        process::exit(1);
    });

    // Find the input file from build_args so we can derive the exe name.
    let input_path = build_args.iter()
        .find(|a| a.ends_with(".certo") || (!a.starts_with('-') && !a.starts_with("build")))
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("error: no input file");
            eprintln!("usage: certo run <file.certo> [build-opts] [-- prog-args]");
            process::exit(2);
        });

    let stem = PathBuf::from(&input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out")
        .to_string();

    let exe_name = if cfg!(windows) {
        format!("{}.exe", stem)
    } else {
        stem.clone()
    };
    let exe_path = tmp_dir.path().join(&exe_name);

    // Synthesise build args with our temp output path.
    let mut full_build_args: Vec<String> = build_args.to_vec();
    full_build_args.push("-o".into());
    full_build_args.push(exe_path.display().to_string());

    cmd_build(&full_build_args, true);

    // Execute the compiled binary.
    let status = std::process::Command::new(&exe_path)
        .args(prog_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("error: could not run {}: {}", exe_path.display(), e);
            process::exit(1);
        });

    process::exit(status.code().unwrap_or(1));
}

// ------------------------------------------------------------------ //
// build
// ------------------------------------------------------------------ //

/// `quiet`: suppress the "wrote <path>" line (used by `certo run`).
fn cmd_build(args: &[String], quiet: bool) {
    let mut input:   Option<PathBuf> = None;
    let mut output:  Option<PathBuf> = None;
    let mut verbose  = false;
    let mut emit_c   = false;
    let mut emit_dll = false;
    let mut watch    = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                output = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| die("-o requires a path", 2))
                ));
            }
            "--emit-c"        => emit_c   = true,
            "--emit-dll"      => emit_dll = true,
            "--verbose" | "-v" => verbose = true,
            "--watch" | "-w"  => watch    = true,
            "--help" | "-h" => {
                println!("Usage: certo <file.certo> [-o <out>] [--emit-c] [--emit-dll] [-v] [--watch]");
                println!("       certo build <file.certo> [-o <out>] [--emit-c] [--emit-dll] [-v] [--watch]");
                println!();
                println!("Options:");
                println!("  -o <file>    Output path");
                println!("  --emit-c     Write the generated C to <stem>.c and stop");
                println!("  --emit-dll   Compile to a shared library (.dll/.so) instead of an exe");
                println!("  -v           Verbose: print the C compiler command");
                println!("  --watch, -w  Watch the source file and rebuild on change");
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

    // Watch mode: re-invoke this binary (minus --watch) on each change.
    if watch {
        let watch_files = vec![input.clone()];
        let self_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("certo"));
        let build_args: Vec<String> = std::iter::once("build".to_string())
            .chain(args.iter().filter(|a| *a != "--watch" && *a != "-w").cloned())
            .collect();
        cmd_watch::watch_loop(watch_files, move || {
            std::process::Command::new(&self_exe)
                .args(&build_args)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        });
        return;
    }

    // Auto-detect lib type from certo.toml if present
    if !emit_dll {
        let project_root = input.parent().unwrap_or(Path::new("."));
        if let Ok(toml_src) = std::fs::read_to_string(project_root.join("certo.toml")) {
            if toml_src.lines().any(|l| l.trim() == "type   = \"lib\"" || l.trim() == "type = \"lib\"") {
                emit_dll = true;
            }
        }
    }

    let colour   = stderr_is_tty();
    let filename = input.display().to_string();

    let (mut module, src) = parse_file_or_exit(&input, colour);

    // ── Resolve local imports (multi-file) ────────────────────────────
    let base_dir = input.parent().unwrap_or(Path::new("."));
    let stdlib_prefixes = ["Stdlib", "Core", "Collections", "Text", "DateTime", "Money"];
    for imp in &module.imports.clone() {
        let first_seg = imp.path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
        if stdlib_prefixes.contains(&first_seg) { continue; }
        let rel: PathBuf = imp.path.segments.iter()
            .map(|s| s.node.as_str())
            .collect::<Vec<_>>()
            .join("/")
            .into();
        let candidate = base_dir.join(rel).with_extension("certo");
        if candidate.exists() {
            let (imp_module, imp_src) = parse_file_or_exit(&candidate, colour);
            let imp_filename = candidate.display().to_string();
            let _ = (imp_src, imp_filename); // already reported inside helper
            if verbose { eprintln!("importing {}", candidate.display()); }
            module.decls.extend(imp_module.decls);
        }
    }

    // ── Type-check ────────────────────────────────────────────────────
    run_typeck(&module, &src, &filename, colour);

    // ── Check for entry point ─────────────────────────────────────────
    let has_main = module.decls.iter().any(|d| {
        matches!(&d.node, certo_ast::decl::Decl::Fn(f) if f.name.node == "main")
    });
    if !has_main && !emit_c && !emit_dll {
        eprintln!("error: module has no `main` function");
        eprintln!("       use --emit-dll to build a shared library, or add:");
        eprintln!("       fn main(): Unit [io] = {{ ... }}");
        process::exit(1);
    }

    // ── Detect stdlib imports (db, etc.) ─────────────────────────────
    let uses_db = module.imports.iter().any(|imp| {
        let segs: Vec<&str> = imp.path.segments.iter().map(|s| s.node.as_str()).collect();
        segs == ["Stdlib", "Db"] || segs == ["Db"]
    });

    // ── Emit C ────────────────────────────────────────────────────────
    // Order: system includes → runtime typedefs → stdlib impls → user code.
    let preamble = "#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\
                    #include <inttypes.h>\n#include <stdarg.h>\n\
                    #define _CRT_SECURE_NO_WARNINGS\n";
    let runtime_header = certo_codegen::RUNTIME_HEADER;
    let stdlib_c  = certo_stdlib::full_c_runtime_with_db(uses_db);
    let module_c  = certo_codegen::emit_module(
        &module,
        &certo_codegen::CodegenOptions {
            inline_runtime: false,
            export_public:  emit_dll,
        },
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
        if emit_dll {
            if cfg!(windows) {
                PathBuf::from(format!("{}.dll", stem))
            } else if cfg!(target_os = "macos") {
                PathBuf::from(format!("lib{}.dylib", stem))
            } else {
                PathBuf::from(format!("lib{}.so", stem))
            }
        } else if cfg!(windows) {
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

    if emit_dll {
        cmd.arg("-shared");
        if cfg!(windows) {
            // Generate an import library alongside the DLL.
            let implib = out_path.with_extension("lib");
            cmd.arg("-Xlinker").arg(format!("/IMPLIB:{}", implib.display()));
            cmd.arg("-Xlinker").arg("/DLL");
        } else {
            cmd.arg("-fPIC");
            if !cfg!(target_os = "macos") {
                cmd.arg("-lm");
            }
        }
    } else {
        // -lm is implicit on Windows (math is part of the UCRT)
        if !cfg!(windows) {
            cmd.arg("-lm");
        } else {
            // lld-link requires an explicit subsystem for console apps
            cmd.arg("-Xlinker").arg("/subsystem:console");
        }
    }

    // Link libpq when the program uses Stdlib.Db
    if uses_db {
        // Resolve PostgreSQL include/lib paths:
        // 1. Honour PG_LIB / PG_INCLUDE env vars if set.
        // 2. On Windows, probe common PostgreSQL install locations.
        // 3. Otherwise assume the system package manager put them on the path.
        let (pg_inc, pg_lib) = resolve_pg_paths();

        if let Some(inc) = &pg_inc {
            cmd.arg(format!("-I{}", inc));
        }
        if cfg!(windows) {
            // On Windows, PostgreSQL ships libpq.lib (not pq.lib).
            // Pass the full path directly so lld-link finds it.
            if let Some(lib_dir) = &pg_lib {
                let libpq = format!("{}/libpq.lib", lib_dir);
                cmd.arg(&libpq);
            } else {
                cmd.arg("libpq.lib");
            }
        } else {
            if let Some(lib) = &pg_lib {
                cmd.arg(format!("-L{}", lib));
            }
            cmd.arg("-lpq");
        }
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

    if !quiet {
        eprintln!("wrote {}", out_path.display());
    }
}

// ------------------------------------------------------------------ //
// Shared helpers
// ------------------------------------------------------------------ //

/// Parse a `.certo` file, resolving its imports and exiting on parse errors.
/// Returns (Module, source_text).
fn parse_file_or_exit(path: &Path, colour: bool) -> (Module, String) {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", path.display(), e);
        process::exit(1);
    });
    let filename = path.display().to_string();
    let module = certo_parser::parse(&src).unwrap_or_else(|errs| {
        let diags: Vec<Diagnostic> = errs.iter().map(|e| {
            Diagnostic::error("", format!("{}", e)).with_span(e.span)
        }).collect();
        eprint!("{}", render_all(&diags, &src, &filename, colour));
        eprintln!("aborting due to {} parse error(s)", diags.len());
        process::exit(1);
    });
    (module, src)
}

/// Run typeck with stdlib builtins seeded. Exits on type errors.
fn run_typeck(module: &Module, src: &str, filename: &str, colour: bool) {
    let mut env     = TypeEnv::new();
    let mut counter = 0u32;
    env.seed_builtins(&mut counter);
    certo_stdlib::seed_stdlib(&mut env, &mut counter);

    if let Err(errs) = certo_typeck::check_module_seeded(module, env, counter) {
        let diags: Vec<Diagnostic> = errs.iter()
            .map(|e| type_error_to_diagnostic(e))
            .collect();
        eprint!("{}", render_all(&diags, src, filename, colour));
        eprintln!("aborting due to {} type error(s)", diags.len());
        process::exit(1);
    }
}

/// Convert a `TypeError` into a human-friendly `Diagnostic` with labels and notes.
fn type_error_to_diagnostic(e: &TypeError) -> Diagnostic {
    match &e.kind {
        TypeErrorKind::Mismatch { expected, found } => {
            Diagnostic::error("E0200",
                format!("type mismatch: expected `{}`, found `{}`",
                    expected.display(), found.display()))
                .with_span(e.span)
                .with_label(format!("expected `{}`", expected.display()))
                .with_note(format!(
                    "the expression has type `{}` but `{}` is required here",
                    found.display(), expected.display()))
        }
        TypeErrorKind::CannotUnify { left, right } => {
            Diagnostic::error("E0201",
                format!("cannot unify `{}` with `{}`", left.display(), right.display()))
                .with_span(e.span)
                .with_label(format!("types `{}` and `{}` are incompatible", left.display(), right.display()))
        }
        TypeErrorKind::OccursCheck { var, ty } => {
            Diagnostic::error("E0202",
                format!("infinite type: type variable ?t{} would contain itself in `{}`",
                    var, ty.display()))
                .with_span(e.span)
                .with_note("this usually means a recursive type alias without a base case")
        }
        TypeErrorKind::MissingAnnotation { name } => {
            Diagnostic::error("E0203",
                format!("recursive function `{}` needs an explicit return type", name))
                .with_span(e.span)
                .with_label("return type required here")
                .with_note(format!("add `: ReturnType` after the parameter list, e.g.  fn {}(...): Int = ...", name))
        }
        TypeErrorKind::ArityMismatch { expected, found } => {
            Diagnostic::error("E0204",
                format!("wrong number of arguments: expected {}, found {}", expected, found))
                .with_span(e.span)
                .with_label(format!("this call has {} argument(s)", found))
        }
        TypeErrorKind::UnknownField { field, on } => {
            Diagnostic::error("E0205",
                format!("no field `{}` on type `{}`", field, on.display()))
                .with_span(e.span)
                .with_label(format!("`{}` has no such field", on.display()))
        }
        TypeErrorKind::UnboundName(name) => {
            Diagnostic::error("E0206", format!("undefined name `{}`", name))
                .with_span(e.span)
                .with_label("not found in this scope")
                .with_note(format!(
                    "if `{}` is a stdlib function, make sure to `import` the module",
                    name))
        }
    }
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

/// Locate PostgreSQL include and lib directories.
/// Returns (include_dir, lib_dir), either of which may be None if not needed
/// (i.e. already on the system search path).
fn resolve_pg_paths() -> (Option<String>, Option<String>) {
    // Explicit env vars always win.
    let env_inc = std::env::var("PG_INCLUDE").ok();
    let env_lib = std::env::var("PG_LIB").ok();
    if env_inc.is_some() || env_lib.is_some() {
        return (env_inc, env_lib);
    }

    // On Windows, probe common PostgreSQL install locations.
    if cfg!(windows) {
        // Try pg_config first (works if PostgreSQL\bin is on PATH)
        if let Ok(out) = std::process::Command::new("pg_config")
            .args(["--includedir", "--libdir"])
            .output()
        {
            if out.status.success() {
                let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
                    .unwrap_or("")
                    .lines()
                    .collect();
                let inc = lines.first().map(|s| s.trim().to_string());
                let lib = lines.get(1).map(|s| s.trim().to_string());
                return (inc, lib);
            }
        }

        // Probe common install directories for versions 14-17
        for ver in (14u32..=17).rev() {
            let base = format!(r"C:\Program Files\PostgreSQL\{}", ver);
            let inc = format!(r"{}\include", base);
            let lib = format!(r"{}\lib", base);
            if std::path::Path::new(&inc).exists() {
                return (Some(inc), Some(lib));
            }
        }
    }

    // On Unix pg_config usually works without probing.
    if !cfg!(windows) {
        if let Ok(out) = std::process::Command::new("pg_config")
            .args(["--includedir", "--libdir"])
            .output()
        {
            if out.status.success() {
                let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
                    .unwrap_or("")
                    .lines()
                    .collect();
                let inc = lines.first().map(|s| s.trim().to_string());
                let lib = lines.get(1).map(|s| s.trim().to_string());
                return (inc, lib);
            }
        }
    }

    (None, None)
}

// ------------------------------------------------------------------ //
// new
// ------------------------------------------------------------------ //

fn cmd_new(args: &[String]) {
    let mut name: Option<&String> = None;
    let mut template = "default";

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                template = args.get(i).map(String::as_str).unwrap_or_else(|| {
                    eprintln!("error: --template requires a name (api, lib, cli)");
                    process::exit(2);
                });
            }
            "--help" | "-h" => {
                println!("Usage: certo new <project-name> [--template <template>]");
                println!();
                println!("Templates:");
                println!("  default   Hello-world entry point (default)");
                println!("  api       HTTP JSON API with Stdlib.Http");
                println!("  lib       Library with public exports, no main");
                println!("  cli       CLI tool with argument parsing");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            n => {
                if name.is_some() { eprintln!("error: unexpected argument '{}'", n); process::exit(2); }
                name = Some(&args[i]);
            }
        }
        i += 1;
    }

    let name = name.unwrap_or_else(|| {
        eprintln!("error: missing project name");
        eprintln!("usage: certo new <project-name> [--template api|lib|cli]");
        process::exit(1);
    });

    let root = PathBuf::from(name);
    if root.exists() {
        eprintln!("error: directory '{}' already exists", name);
        process::exit(1);
    }

    // Directories
    let mut dirs: Vec<PathBuf> = vec![
        root.join("src"),
        root.join("db").join("migrations"),
        root.join("tests").join("unit"),
        root.join("tests").join("integration"),
        root.join("dist"),
    ];
    if template == "api" {
        dirs.push(root.join("src").join("handlers"));
    }
    for dir in &dirs {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| {
            eprintln!("error: cannot create {}: {}", dir.display(), e);
            process::exit(1);
        });
    }

    let module_name = to_module_name(name);

    // certo.toml
    let toml_type = if template == "lib" { "lib" } else { "app" };
    let toml_entry = if template == "lib" { "" } else { "entry  = \"src\\\\main.cto\"\n" };
    write_file(&root.join("certo.toml"), &format!(
        "[project]\n\
         name    = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2026\"\n\
         \n\
         [build]\n\
         type   = \"{toml_type}\"\n\
         target = \"native\"\n\
         output = \"dist\\\\\"\n\
         {toml_entry}"
    ));

    // src/main.cto — template-specific
    let main_src = match template {
        "api" => format!(
            "module {module_name}\n\
             \n\
             import Stdlib.Http\n\
             \n\
             fn handleHealth(req: HttpRequest): HttpResponse {{\n\
                 Http.ok(\"application/json\", \"{{\\\"status\\\": \\\"ok\\\"}}\")\n\
             }}\n\
             \n\
             fn main(): Unit {{\n\
                 println(\"Listening on :8080\")\n\
                 Http.serve(8080, fn(req) {{\n\
                     if req.path == \"/health\" then handleHealth(req)\n\
                     else Http.notFound(\"not found\")\n\
                 }})\n\
             }}\n"
        ),
        "lib" => format!(
            "module {module_name}\n\
             \n\
             /// The public API of this library.\n\
             pub fn greet(name: Text): Text {{\n\
                 \"Hello, \" ++ name ++ \"!\"\n\
             }}\n"
        ),
        "cli" => format!(
            "module {module_name}\n\
             \n\
             fn printUsage(): Unit {{\n\
                 println(\"Usage: {name} <command>\")\n\
                 println(\"\")\n\
                 println(\"Commands:\")\n\
                 println(\"  help    Show this message\")\n\
             }}\n\
             \n\
             fn main(): Unit {{\n\
                 val cmd = arg(1)\n\
                 match cmd {{\n\
                     None    => printUsage()\n\
                     Some(c) => match c {{\n\
                         \"help\" => printUsage()\n\
                         other  => println(\"Unknown command: \" ++ other)\n\
                     }}\n\
                 }}\n\
             }}\n",
            name = name,
        ),
        _ => format!(
            "module {module_name}\n\
             \n\
             fn main(): Unit {{\n\
                 println(\"Hello from {name}!\")\n\
             }}\n",
            name = name,
        ),
    };
    write_file(&root.join("src").join("main.cto"), &main_src);

    // .env.example
    write_file(&root.join(".env.example"),
        "# Database\n\
         DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret\n\
         \n\
         # App\n\
         APP_ENV=development\n\
         PORT=8080\n"
    );

    // .gitignore
    write_file(&root.join(".gitignore"),
        "dist\\\n\
         .env\n\
         .certo\\\n\
         *.log\n"
    );

    // README.md
    let readme_build = match template {
        "lib"  => format!("certo build src/main.cto --emit-dll -o dist/{name}.dll"),
        _      => format!("certo build src/main.cto -o dist/{name}.exe"),
    };
    write_file(&root.join("README.md"), &format!(
        "# {name}\n\
         \n\
         A Certo project.\n\
         \n\
         ## Build\n\
         \n\
         ```\n\
         {readme_build}\n\
         ```\n\
         \n\
         ## Run\n\
         \n\
         ```\n\
         certo run src/main.cto\n\
         ```\n\
         \n\
         ## Docs\n\
         \n\
         ```\n\
         certo doc src/main.cto\n\
         ```\n"
    ));

    // Print tree
    eprintln!("Created '{}' (template: {})", name, template);
    eprintln!();
    eprintln!("  {name}/");
    eprintln!("  ├── certo.toml");
    eprintln!("  ├── .env.example");
    eprintln!("  ├── .gitignore");
    eprintln!("  ├── README.md");
    eprintln!("  ├── src/");
    if template == "api" {
        eprintln!("  │   ├── main.cto");
        eprintln!("  │   └── handlers/");
    } else {
        eprintln!("  │   └── main.cto");
    }
    eprintln!("  ├── db/");
    eprintln!("  │   └── migrations/");
    eprintln!("  ├── tests/");
    eprintln!("  │   ├── unit/");
    eprintln!("  │   └── integration/");
    eprintln!("  └── dist/");
    eprintln!();
    eprintln!("  cd {name}");
    eprintln!("  certo run src/main.cto");
}

fn write_file(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", path.display(), e);
        process::exit(1);
    });
}

/// Convert a project name like "Lattice-Project" to a module name "latticeProject".
fn to_module_name(name: &str) -> String {
    let mut out = String::new();
    let mut cap_next = false;
    for (i, c) in name.chars().enumerate() {
        if c == '-' || c == '_' {
            cap_next = true;
        } else if cap_next {
            out.extend(c.to_uppercase());
            cap_next = false;
        } else if i == 0 {
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------------------ //
// fmt
// ------------------------------------------------------------------ //

fn cmd_fmt(args: &[String]) {
    let mut files: Vec<PathBuf> = vec![];
    let mut check_only = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check_only = true,
            "--help" | "-h" => {
                println!("Usage: certo fmt [--check] <file.certo>...");
                println!();
                println!("Format Certo source files in place.");
                println!("  --check   Exit 1 if any file would be reformatted (no writes).");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => files.push(PathBuf::from(path)),
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("error: no input files");
        process::exit(2);
    }

    let mut any_changed = false;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {}: {}", path.display(), e);
            process::exit(1);
        });
        match certo_fmt::format_source(&src) {
            Ok(formatted) => {
                if formatted != src {
                    any_changed = true;
                    if check_only {
                        eprintln!("would reformat: {}", path.display());
                    } else {
                        std::fs::write(path, &formatted).unwrap_or_else(|e| {
                            eprintln!("error writing {}: {}", path.display(), e);
                            process::exit(1);
                        });
                        println!("formatted: {}", path.display());
                    }
                }
            }
            Err(_) => {
                eprintln!("parse error: {} (skipped)", path.display());
            }
        }
    }
    if check_only && any_changed {
        process::exit(1);
    }
}

// ------------------------------------------------------------------ //
// test
// ------------------------------------------------------------------ //

fn cmd_test(args: &[String]) {
    let mut files: Vec<PathBuf> = vec![];
    let mut color = stderr_is_tty();
    let mut timeout_ms: u64 = 5000;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-color" => color = false,
            "--help" | "-h" => {
                println!("Usage: certo test <file.certo>...");
                println!();
                println!("Compile and run all `test` blocks in the given source files.");
                println!("Exits 0 if all tests pass, 1 otherwise.");
                return;
            }
            other if other.starts_with("--timeout=") => {
                let v = other.trim_start_matches("--timeout=");
                timeout_ms = v.parse().unwrap_or_else(|_| {
                    eprintln!("error: --timeout= requires a number in milliseconds");
                    process::exit(2);
                });
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => files.push(PathBuf::from(path)),
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("error: no input files");
        process::exit(2);
    }

    let opts = certo_testrunner::run::RunOptions {
        timeout: Some(std::time::Duration::from_millis(timeout_ms)),
        filter: None,
    };
    let mut all_passed = true;
    for path in &files {
        match certo_testrunner::run_file(path, &opts, color) {
            Ok(passed) => { if !passed { all_passed = false; } }
            Err(certo_testrunner::error::TestRunnerError::NoTests) => {
                eprintln!("{}: no tests found", path.display());
            }
            Err(e) => {
                eprintln!("error: {}", e);
                all_passed = false;
            }
        }
    }
    if !all_passed { process::exit(1); }
}

// ------------------------------------------------------------------ //
// lint
// ------------------------------------------------------------------ //

fn cmd_lint(args: &[String]) {
    let mut files: Vec<PathBuf> = vec![];
    let mut color = stderr_is_tty();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-color" => color = false,
            "--help" | "-h" => {
                println!("Usage: certo lint <file.certo>...");
                println!();
                println!("Run lint checks on Certo source files.");
                println!("Reports: unused parameters, unreachable code after panic/todo.");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => files.push(PathBuf::from(path)),
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("error: no input files");
        process::exit(2);
    }

    let mut warnings = 0usize;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {}: {}", path.display(), e);
            process::exit(1);
        });
        let module = match certo_parser::parse(&src) {
            Ok(m) => m,
            Err(errs) => {
                for e in &errs {
                    eprintln!("{}:{}: parse error: {}", path.display(), e.span.start, e);
                }
                process::exit(1);
            }
        };
        let w = lint_module(&module, path, &src, color);
        warnings += w;
    }
    if warnings > 0 {
        eprintln!("{} warning(s) found", warnings);
        process::exit(1);
    } else {
        println!("No issues found.");
    }
}

fn lint_module(module: &certo_ast::module::Module, path: &Path, src: &str, _color: bool) -> usize {
    use certo_ast::decl::Decl;
    let mut count = 0;

    for decl in &module.decls {
        let Decl::Fn(f) = &decl.node else { continue };

        for param in &f.params {
            let name = &param.name.node;
            if name.starts_with('_') { continue; }
            let body_src = if let Some(b) = &f.body {
                let s = b.span.start as usize;
                let e = b.span.end as usize;
                src.get(s..e).unwrap_or("")
            } else { continue };
            let occurrences = body_src.matches(name.as_str()).count();
            if occurrences == 0 {
                let line = src[..param.name.span.start as usize].chars().filter(|&c| c == '\n').count() + 1;
                eprintln!("{}:{}: warning: unused parameter `{}`", path.display(), line, name);
                count += 1;
            }
        }

        if let Some(body) = &f.body {
            count += lint_unreachable_after_terminal(&body.node, path, src);
        }
    }
    count
}

fn lint_unreachable_after_terminal(expr: &certo_ast::expr::Expr, path: &Path, src: &str) -> usize {
    use certo_ast::expr::{Expr, Stmt};
    let mut count = 0;
    if let Expr::Block { stmts, .. } = expr {
        let mut found_terminal = false;
        for stmt in stmts {
            if found_terminal {
                let pos = match stmt {
                    Stmt::Val { span, .. } | Stmt::Var { span, .. }
                    | Stmt::Assign { span, .. } | Stmt::Defer { span, .. }
                    | Stmt::Expr { span, .. } => span.start as usize,
                };
                let line = src[..pos].chars().filter(|&c| c == '\n').count() + 1;
                eprintln!("{}:{}: warning: unreachable statement", path.display(), line);
                count += 1;
                break;
            }
            if let Stmt::Expr { expr: e, .. } = stmt {
                if let Expr::App { func, .. } = &e.node {
                    if let Expr::Path { path: p, .. } = &func.node {
                        let name = p.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                        if matches!(name, "panic" | "todo" | "unreachable") {
                            found_terminal = true;
                        }
                    }
                }
            }
        }
    }
    count
}

// ------------------------------------------------------------------ //
// bench
// ------------------------------------------------------------------ //

fn cmd_bench(args: &[String]) {
    let mut files: Vec<PathBuf> = vec![];
    let mut iterations: u64 = 1000;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                println!("Usage: certo bench <file.certo> [--iterations=N]");
                println!();
                println!("Compile and run all `bench` blocks in the source file.");
                println!("  --iterations=N   Number of iterations per benchmark (default 1000).");
                return;
            }
            other if other.starts_with("--iterations=") => {
                let v = other.trim_start_matches("--iterations=");
                iterations = v.parse().unwrap_or_else(|_| {
                    eprintln!("error: --iterations= requires a positive integer");
                    process::exit(2);
                });
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => files.push(PathBuf::from(path)),
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("error: no input files");
        process::exit(2);
    }

    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {}: {}", path.display(), e);
            process::exit(1);
        });
        let module = match certo_parser::parse(&src) {
            Ok(m) => m,
            Err(errs) => {
                for e in &errs { eprintln!("parse error: {}", e); }
                process::exit(1);
            }
        };
        run_bench_module(&module, path, &src, iterations);
    }
}

fn run_bench_module(module: &certo_ast::module::Module, _path: &Path, _src: &str, iterations: u64) {
    use certo_ast::decl::Decl;
    let mut found = 0;
    for decl in &module.decls {
        if let Decl::Fn(f) = &decl.node {
            let name = &f.name.node;
            if !name.starts_with("bench_") { continue; }
            found += 1;
            // For now: report that bench discovery works; full timing harness
            // requires compiling a C wrapper with clock_gettime around the call.
            println!("bench  {:<40} {} iterations  (timing harness: pending)", name, iterations);
        }
    }
    if found == 0 {
        eprintln!("no bench_ functions found (prefix bench functions with `bench_`)");
    }
}

fn print_top_help() {
    eprintln!("Certo compiler");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  certo new <project-name>         Scaffold a new project");
    eprintln!("  certo check <file.certo>         Type-check without compiling");
    eprintln!("  certo run   <file.certo> [-- args]  Compile and run");
    eprintln!("  certo <file.certo> [-o <out>]    Compile a Certo source file");
    eprintln!("  certo build <file.certo> ...     Same with explicit subcommand");
    eprintln!("  certo doc   <file.certo>         Generate HTML documentation");
    eprintln!("  certo fmt   <file.certo>...      Format source files in place");
    eprintln!("  certo test  <file.certo>...      Run test blocks");
    eprintln!("  certo lint  <file.certo>...      Lint for unused params / dead code");
    eprintln!("  certo bench <file.certo>...      Run bench_ functions");
    eprintln!("  certo migrate <subcommand>       Database migration tools");
    eprintln!();
    eprintln!("Build options:");
    eprintln!("  -o <file>    Output path");
    eprintln!("  --emit-c     Stop after emitting C; write <stem>.c");
    eprintln!("  --emit-dll   Compile to a shared library (.dll / .so)");
    eprintln!("  -v           Verbose: print the C compiler command");
    eprintln!("  --watch, -w  Watch source file and rebuild on change");
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

/// Returns true when colour output is appropriate for stderr.
/// Respects the `NO_COLOR` env var and `TERM=dumb` convention.
fn stderr_is_tty() -> bool {
    if std::env::var_os("NO_COLOR").is_some() { return false; }
    if std::env::var("TERM").as_deref() == Ok("dumb") { return false; }
    // On Windows the console supports ANSI escapes from Windows 10+.
    // On Unix-like systems, assume colour when TERM is set.
    #[cfg(windows)]
    { std::env::var_os("TERM").is_some() || std::env::var_os("WT_SESSION").is_some() }
    #[cfg(not(windows))]
    { std::env::var_os("TERM").is_some() }
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
    let colour = stderr_is_tty();
    for path in paths {
        let (module, _) = parse_file_or_exit(&path, colour);
        for decl in &module.decls {
            if let Decl::Migration(m) = &decl.node { result.push(m.clone()); }
        }
    }
    result
}
