mod cmd_doc;
mod cmd_watch;
mod cmd_repl;
mod cmd_lint;


use std::path::{Path, PathBuf};
use std::process;
use certo_ast::decl::{Decl, MigrationDecl};
use certo_ast::module::Module;
use certo_migrate::{
    plan_up, plan_down, run_steps, status,
    default_manifest_path, RunOptions,
};
use certo_diagnostics::{Diagnostic, render_all};
use certo_typeck::{TypeError, TypeErrorKind, TypeEnv, assign_var_names};

/// C preamble shared by the build pipeline and the REPL.
pub(crate) const REPL_PREAMBLE: &str =
    "#ifdef _WIN32\n\
     #  ifndef WIN32_LEAN_AND_MEAN\n\
     #    define WIN32_LEAN_AND_MEAN\n\
     #  endif\n\
     #  ifndef _USE_MATH_DEFINES\n\
     #    define _USE_MATH_DEFINES\n\
     #  endif\n\
     #  include <winsock2.h>\n\
     #  include <ws2tcpip.h>\n\
     #  pragma comment(lib, \"ws2_32.lib\")\n\
     #endif\n\
     #include <stdint.h>\n\
     #include <stdbool.h>\n\
     #include <stddef.h>\n\
     #include <inttypes.h>\n\
     #include <stdarg.h>\n\
     #define _CRT_SECURE_NO_WARNINGS\n";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Bare `certo` with no args: try certo.toml entry, else show help.
    if args.len() < 2 {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if cwd.join("certo.toml").exists() {
            cmd_build(&[], false);
        } else {
            print_top_help();
            process::exit(1);
        }
        return;
    }

    // Allow `certo <file.cto> [options]` as shorthand for `certo build`.
    let first = args[1].as_str();
    if first.ends_with(".cto") || first == "build" {
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
            "db"      => cmd_db(&args[2..]),
            "repl"    => cmd_repl::cmd_repl(),
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
                println!("Usage: certo check <file.cto> [-v]");
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
        eprintln!("usage: certo check <file.cto>");
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
        .find(|a| a.ends_with(".cto") || (!a.starts_with('-') && !a.starts_with("build")))
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("error: no input file");
            eprintln!("usage: certo run <file.cto> [build-opts] [-- prog-args]");
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
                println!("Usage: certo <file.cto> [-o <out>] [--emit-c] [--emit-dll] [-v] [--watch]");
                println!("       certo build <file.cto> [-o <out>] [--emit-c] [--emit-dll] [-v] [--watch]");
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
        // No file argument — try reading `entry` from certo.toml in cwd.
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if let Ok(toml_src) = std::fs::read_to_string(cwd.join("certo.toml")) {
            for line in toml_src.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("entry").and_then(|r| {
                    let r = r.trim_start();
                    r.strip_prefix('=').map(|v| v.trim().trim_matches('"'))
                }) {
                    if !rest.is_empty() {
                        return cwd.join(rest);
                    }
                }
            }
            eprintln!("error: certo.toml found but has no `entry` field under [build]");
            eprintln!("       add:  entry = \"src/main.cto\"");
        } else {
            eprintln!("error: no input file and no certo.toml in current directory");
            eprintln!("usage: certo <file.cto> [-o <out>]");
            eprintln!("       or run from a project directory containing certo.toml");
        }
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

    // Read certo.toml if present: auto-detect lib type and output directory.
    let mut toml_output_dir: Option<PathBuf> = None;
    {
        let project_root = input.parent().unwrap_or(Path::new("."));
        if let Ok(toml_src) = std::fs::read_to_string(project_root.join("certo.toml")) {
            for line in toml_src.lines() {
                let line = line.trim();
                if !emit_dll && (line == "type   = \"lib\"" || line == "type = \"lib\"") {
                    emit_dll = true;
                }
                // output = "dist/"  or  output = "dist"
                if let Some(rest) = line.strip_prefix("output").and_then(|r| {
                    let r = r.trim_start();
                    r.strip_prefix('=').map(|v| v.trim().trim_matches('"'))
                }) {
                    if !rest.is_empty() {
                        let dir = project_root.join(rest);
                        std::fs::create_dir_all(&dir).ok();
                        toml_output_dir = Some(dir);
                    }
                }
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
        let candidate = base_dir.join(rel).with_extension("cto");
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
    // Windows: winsock2.h before windows.h avoids IPPROTO_* redefinition.
    // _USE_MATH_DEFINES exposes M_PI / M_E from <math.h> on MSVC/clang-cl.
    let preamble = "#ifdef _WIN32\n\
                    #  ifndef WIN32_LEAN_AND_MEAN\n\
                    #    define WIN32_LEAN_AND_MEAN\n\
                    #  endif\n\
                    #  ifndef _USE_MATH_DEFINES\n\
                    #    define _USE_MATH_DEFINES\n\
                    #  endif\n\
                    #  include <winsock2.h>\n\
                    #  include <ws2tcpip.h>\n\
                    #  pragma comment(lib, \"ws2_32.lib\")\n\
                    #endif\n\
                    #include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\
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
        let base = toml_output_dir.as_deref().unwrap_or(Path::new("."));
        if emit_dll {
            if cfg!(windows) {
                base.join(format!("{}.dll", stem))
            } else if cfg!(target_os = "macos") {
                base.join(format!("lib{}.dylib", stem))
            } else {
                base.join(format!("lib{}.so", stem))
            }
        } else if cfg!(windows) {
            base.join(format!("{}.exe", stem))
        } else {
            base.join(stem)
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
            // Standard Windows libs (user32 for MessageBox, etc.)
            cmd.arg("-luser32");
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

/// Parse a `.cto` file, resolving its imports and exiting on parse errors.
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
pub(crate) fn type_error_to_diagnostic(e: &TypeError) -> Diagnostic {
    use certo_typeck::Ty;

    match &e.kind {
        TypeErrorKind::Mismatch { expected, found } => {
            let names = assign_var_names(&[expected, found]);
            let exp_s = expected.display_named(&names);
            let fnd_s = found.display_named(&names);

            let mut d = Diagnostic::error("E0200",
                format!("type mismatch: expected `{}`, found `{}`", exp_s, fnd_s))
                .with_span(e.span)
                .with_label(format!("this has type `{}`", fnd_s));

            // Context-sensitive hints
            d = match (expected, found) {
                // Unresolved var — user needs an annotation
                (_, Ty::Var(_)) | (Ty::Var(_), _) => d
                    .with_note("the compiler could not infer this type; add an explicit type annotation"),

                // Passed a plain value where a function was expected
                (Ty::Fn { params, .. }, _) if !found.is_fn() => d
                    .with_note(format!(
                        "`{}` is not a function — expected a function that takes {} argument(s)",
                        fnd_s, params.len())),

                // Passed a function where a plain value was expected
                (_, Ty::Fn { .. }) if !expected.is_fn() => d
                    .with_note(format!(
                        "you passed a function, but `{}` is required here — did you mean to call it?",
                        exp_s)),

                // Option unwrap mismatch: expected T, found T?
                (inner, Ty::Option(opt_inner)) if inner == opt_inner.as_ref() => d
                    .with_note(format!(
                        "`{}` is optional — use `match` or `?` to unwrap it before using it as `{}`",
                        fnd_s, exp_s)),

                // Missing ? on result
                (inner, Ty::Result(ok, _)) if inner == ok.as_ref() => d
                    .with_note(format!(
                        "`{}` is a Result — use `match` to handle the error case",
                        fnd_s)),

                // Int/Float confusion
                (Ty::Int, Ty::Float) => d
                    .with_note("use `floatToInt(x)` to convert a Float to Int"),
                (Ty::Float, Ty::Int) => d
                    .with_note("use `intToFloat(x)` to convert an Int to Float"),

                // Int/Text confusion
                (Ty::Text, Ty::Int) => d
                    .with_note("use `intToText(n)` to convert an Int to Text"),
                (Ty::Int, Ty::Text) => d
                    .with_note("use `parseInt(s)` to parse Text as an Int? (returns Option<Int>)"),

                // Float/Text confusion
                (Ty::Text, Ty::Float) => d
                    .with_note("use `floatToText(x)` to convert a Float to Text"),

                // Bool expected, non-bool found
                (Ty::Bool, _) => d
                    .with_note(format!(
                        "conditions must be `Bool`; `{}` is not a boolean value",
                        fnd_s)),

                // Unit return — function returns nothing but result used
                (_, Ty::Unit) => d
                    .with_note("this expression returns Unit (no value) — remove the assignment or use a different function"),

                _ => d.with_note(format!(
                    "the expression has type `{}` but `{}` is required here",
                    fnd_s, exp_s)),
            };
            d
        }

        TypeErrorKind::CannotUnify { left, right } => {
            let names = assign_var_names(&[left, right]);
            let l = left.display_named(&names);
            let r = right.display_named(&names);
            Diagnostic::error("E0201", format!("cannot unify `{}` with `{}`", l, r))
                .with_span(e.span)
                .with_label(format!("incompatible types `{}` and `{}`", l, r))
                .with_note("these two types must match but they have different shapes")
        }

        TypeErrorKind::OccursCheck { var: _, ty } => {
            let names = assign_var_names(&[ty]);
            Diagnostic::error("E0202",
                format!("infinite type: a type variable appears within its own inferred type `{}`",
                    ty.display_named(&names)))
                .with_span(e.span)
                .with_note("this usually means a recursive type alias without a base case")
                .with_note("if you meant a recursive function, make sure it has an explicit return type")
        }

        TypeErrorKind::MissingAnnotation { name } => {
            Diagnostic::error("E0203",
                format!("recursive function `{}` needs an explicit return type", name))
                .with_span(e.span)
                .with_label("return type required here")
                .with_note(format!(
                    "add `: ReturnType` after the parameter list — for example:\n  fn {}(...): Int = ...",
                    name))
        }

        TypeErrorKind::ArityMismatch { expected, found } => {
            let (exp, fnd) = (expected, found);
            let hint = if fnd > exp {
                format!("remove {} extra argument(s)", fnd - exp)
            } else {
                format!("add {} missing argument(s)", exp - fnd)
            };
            Diagnostic::error("E0204",
                format!("wrong number of arguments: expected {}, found {}", exp, fnd))
                .with_span(e.span)
                .with_label(format!("this call passes {} argument(s)", fnd))
                .with_note(hint)
        }

        TypeErrorKind::UnknownField { field, on } => {
            let names = assign_var_names(&[on]);
            let on_s = on.display_named(&names);
            let mut d = Diagnostic::error("E0205",
                format!("no field `{}` on type `{}`", field, on_s))
                .with_span(e.span)
                .with_label(format!("`{}` has no field named `{}`", on_s, field));
            // Hint for common module-style access on wrong receiver
            if on_s == "Unit" || on_s == "Int" || on_s == "Float" || on_s == "Bool" || on_s == "Text" {
                d = d.with_note(format!(
                    "primitive type `{}` has no fields — did you mean a function like `{}.someFunc(...)`?",
                    on_s, on_s));
            }
            d
        }

        TypeErrorKind::UnboundName(name) => {
            let mut d = Diagnostic::error("E0206", format!("undefined name `{}`", name))
                .with_span(e.span)
                .with_label("not defined in this scope");

            // Module-qualified names: suggest the module
            if let Some(dot) = name.find('.') {
                let module = &name[..dot];
                d = d.with_note(format!(
                    "`{}` looks like a module function — make sure `Stdlib.{}` is available",
                    name, module));
            } else {
                // Check for common casing mistakes
                let lower = name.to_lowercase();
                let suggestion: Option<&str> = match lower.as_str() {
                    "true"  => Some("`true` is already correct (lowercase)"),
                    "false" => Some("`false` is already correct (lowercase)"),
                    "int"   => Some("the type is `Int` (capital I), but values are just integer literals like `42`"),
                    "float" => Some("the type is `Float` (capital F), but values are float literals like `3.14`"),
                    "text"  => Some("the type is `Text` (capital T), but string literals are just `\"hello\"`"),
                    "bool"  => Some("the type is `Bool` (capital B), but values are `true` / `false`"),
                    "print"     => Some("`print` is a built-in function — no import needed"),
                    "println"   => Some("`println` is a built-in function — no import needed"),
                    "list"      => Some("try `List.empty` to start an empty list"),
                    "none"      => Some("use `None` (capital N) for the absent optional value"),
                    "some"      => Some("use `Some(value)` (capital S) to wrap an optional value"),
                    "ok"        => Some("use `Ok(value)` (capital O) to construct a Result"),
                    "err"       => Some("use `Err(value)` (capital E) to construct an error Result"),
                    _ => None,
                };
                if let Some(s) = suggestion {
                    d = d.with_note(s);
                } else {
                    d = d.with_note(format!(
                        "check the spelling; stdlib functions are accessed as `Module.function`, e.g. `Text.len`"));
                }
            }
            d
        }
    }
}

pub(crate) fn find_cc() -> Option<String> {
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

pub(crate) fn probe_cc(cmd: &str) -> bool {
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
    let toml_entry = if template == "lib" { "" } else { "entry  = \"src/main.cto\"\n" };
    let toml_deps = match template {
        "api" =>
            "\n[dependencies]\n\
             \"Stdlib.Http\" = \"*\"\n\
             \"Stdlib.Json\" = \"*\"\n",
        "cli" =>
            "\n[dependencies]\n\
             \"Stdlib.Text\" = \"*\"\n",
        "lib" =>
            "\n[dependencies]\n\
             # add stdlib modules your library uses, e.g.\n\
             # \"Stdlib.Text\" = \"*\"\n",
        _ =>
            "\n[dependencies]\n\
             # add stdlib modules your project uses, e.g.\n\
             # \"Stdlib.Http\" = \"*\"\n",
    };
    write_file(&root.join("certo.toml"), &format!(
        "[project]\n\
         name    = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2026\"\n\
         \n\
         [build]\n\
         type   = \"{toml_type}\"\n\
         target = \"native\"\n\
         output = \"dist/\"\n\
         {toml_entry}{toml_deps}"
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
         .cto\\\n\
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
                println!("Usage: certo fmt [--check] <file.cto>...");
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
                println!("Usage: certo test <file.cto>...");
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
                println!("Usage: certo lint <file.cto>...");
                println!();
                println!("Run lint checks on Certo source files (HIR dataflow pass).");
                println!("  L001  unused parameter");
                println!("  L002  unused variable (val/var declared but never read)");
                println!("  L003  assigned but never read (value written then overwritten)");
                println!("  L004  unreachable statement (after panic/todo/unreachable)");
                println!("  L005  guard condition is a literal true/false");
                println!();
                println!("Prefix a name with `_` to suppress all L001/L002 warnings for it.");
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
        let w = cmd_lint::lint_hir(&module, path, &src, color);
        warnings += w;
    }
    if warnings > 0 {
        eprintln!("{} warning(s) found", warnings);
        process::exit(1);
    } else {
        println!("No issues found.");
    }
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
                println!("Usage: certo bench <file.cto> [--iterations=N]");
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

fn run_bench_module(module: &certo_ast::module::Module, path: &Path, src: &str, iterations: u64) {
    use certo_ast::decl::Decl;

    // Collect bench function Certo names and their C equivalents.
    // Only zero-argument bench_ functions are entry points for the harness.
    // bench_* helpers that take parameters are support functions, not targets.
    let bench_fns: Vec<(String, String)> = module.decls.iter()
        .filter_map(|d| if let Decl::Fn(f) = &d.node { Some(f) } else { None })
        .filter(|f| f.name.node.starts_with("bench_") && f.params.is_empty())
        .map(|f| {
            let certo_name = f.name.node.clone();
            let c_name = certo_codegen::c_fn_name(&certo_name);
            (certo_name, c_name)
        })
        .collect();

    if bench_fns.is_empty() {
        eprintln!("no bench_ functions found (prefix bench functions with `bench_`)");
        return;
    }

    // Type-check the module.
    let colour = stderr_is_tty();
    let filename = path.display().to_string();
    run_typeck(module, src, &filename, colour);

    // Codegen: emit module C and mask any `main` so our harness can provide it.
    let preamble   = REPL_PREAMBLE;
    let runtime    = certo_codegen::RUNTIME_HEADER;
    let stdlib_c   = certo_stdlib::full_c_runtime_with_db(false);
    let raw_module = certo_codegen::emit_module(
        module,
        &certo_codegen::CodegenOptions { inline_runtime: false, export_public: false },
    );
    // Strip duplicate system includes already in preamble.
    let module_c = raw_module.lines()
        .filter(|l| {
            !l.contains("certo_runtime.h") &&
            !l.contains("#include <stdint.h>") &&
            !l.contains("#include <stdbool.h>") &&
            !l.contains("#include <stddef.h>")
        })
        .collect::<Vec<_>>()
        .join("\n")
        // Rename any user `main` so the harness `main` doesn't clash.
        .replace("\nint main(", "\nint _bench_user_main(");

    // Split the harness: forward declarations + timer code go BEFORE module_c
    // (so noinline attributes precede the definitions), main() goes after.
    let (harness_pre, harness_main) = build_bench_harness_c(&bench_fns, iterations);

    let full_c = format!("{}{}\n{}\n{}\n{}\n{}",
        preamble, runtime, stdlib_c, harness_pre, module_c, harness_main);

    // Write to a temp file, compile, run.
    let tmp_c = tempfile::Builder::new()
        .prefix("certo_bench_")
        .suffix(".c")
        .tempfile()
        .unwrap_or_else(|e| { eprintln!("error: {}", e); process::exit(1); });
    std::fs::write(tmp_c.path(), &full_c)
        .unwrap_or_else(|e| { eprintln!("error: {}", e); process::exit(1); });

    let cc = find_cc().unwrap_or_else(|| {
        eprintln!("error: no C compiler found");
        process::exit(1);
    });
    let bin_path = std::env::temp_dir().join(
        if cfg!(windows) { "certo_bench.exe" } else { "certo_bench" }
    );

    let mut cmd = std::process::Command::new(&cc);
    cmd.arg(tmp_c.path())
       .arg("-o").arg(&bin_path)
       .arg("-O1")  // O1: preserve real work; O2 can eliminate benchmark loops entirely
       .arg("-Wno-int-to-pointer-cast")
       .arg("-Wno-pointer-to-int-cast")
       .arg("-Wno-int-conversion")
       .arg("-Wno-implicit-function-declaration")
       .arg("-Wno-deprecated-declarations");
    if cfg!(windows) {
        cmd.arg("-Xlinker").arg("/subsystem:console");
    } else {
        cmd.arg("-lm");
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error invoking {}: {}", cc, e);
        process::exit(1);
    });
    if !status.success() {
        eprintln!("error: C compiler failed");
        process::exit(1);
    }

    eprintln!("running {} benchmarks ({} iterations each) …", bench_fns.len(), iterations);
    eprintln!();
    std::process::Command::new(&bin_path)
        .status()
        .unwrap_or_else(|e| { eprintln!("error running bench: {}", e); process::exit(1); });
}

/// Generate the bench harness split into two C fragments.
/// Returns `(pre, main_fn)` where `pre` must appear BEFORE the module code
/// (so noinline attributes precede function definitions) and `main_fn` after.
fn build_bench_harness_c(bench_fns: &[(String, String)], iterations: u64) -> (String, String) {
    let mut pre = String::new();

    pre.push_str("\n/* ---- certo bench harness ---- */\n");
    pre.push_str("#include <stdio.h>\n");
    pre.push_str("#include <inttypes.h>\n");

    // High-resolution timer — platform specific.
    pre.push_str(r#"
#ifdef _WIN32
static uint64_t bench_now_ns(void) {
    LARGE_INTEGER freq, count;
    QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&count);
    /* freq is ticks/sec (e.g. 10 MHz); multiply count first to avoid truncation */
    return (uint64_t)(count.QuadPart * 1000000000LL / freq.QuadPart);
}
#else
#include <time.h>
static uint64_t bench_now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * UINT64_C(1000000000) + (uint64_t)ts.tv_nsec;
}
#endif
"#);

    // Volatile sink prevents the optimizer from eliminating benchmark calls.
    pre.push_str(&format!(r#"
static volatile int64_t _bench_sink = 0;

static void bench_run(const char* name, int64_t (*fn)(void)) {{
    uint64_t iters  = {iters}ULL;
    uint64_t warmup = iters / 10 < 1 ? 1 : iters / 10;
    for (uint64_t i = 0; i < warmup; i++) _bench_sink = fn();
    uint64_t t0 = bench_now_ns();
    for (uint64_t i = 0; i < iters; i++) _bench_sink = fn();
    uint64_t t1 = bench_now_ns();
    uint64_t ns_per = (t1 > t0) ? (t1 - t0) / iters : 0;
    printf("bench  %-38s %12" PRIu64 " ns/iter\n", name, ns_per);
}}
"#, iters = iterations));

    // noinline forward declarations — must come BEFORE the definitions so the
    // attribute is applied when the compiler sees the function body.
    pre.push_str("#if defined(__GNUC__) || defined(__clang__)\n");
    pre.push_str("#  define BENCH_NOINLINE __attribute__((noinline))\n");
    pre.push_str("#elif defined(_MSC_VER)\n");
    pre.push_str("#  define BENCH_NOINLINE __declspec(noinline)\n");
    pre.push_str("#else\n");
    pre.push_str("#  define BENCH_NOINLINE\n");
    pre.push_str("#endif\n");
    for (_, c_name) in bench_fns {
        pre.push_str(&format!("BENCH_NOINLINE int64_t {}(void);\n", c_name));
    }

    // main() — goes after the module code.
    let mut main_fn = String::new();
    main_fn.push_str("\nint main(void) {\n");
    for (certo_name, c_name) in bench_fns {
        main_fn.push_str(&format!(
            "    bench_run(\"{}\", {});\n",
            certo_name, c_name
        ));
    }
    main_fn.push_str("    return 0;\n}\n");

    (pre, main_fn)
}

fn print_top_help() {
    eprintln!("Certo compiler");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  certo                            Start the interactive REPL");
    eprintln!("  certo repl                       Start the interactive REPL");
    eprintln!("  certo new <project-name>         Scaffold a new project");
    eprintln!("  certo check <file.cto>         Type-check without compiling");
    eprintln!("  certo run   <file.cto> [-- args]  Compile and run");
    eprintln!("  certo <file.cto> [-o <out>]    Compile a Certo source file");
    eprintln!("  certo build <file.cto> ...     Same with explicit subcommand");
    eprintln!("  certo doc   <file.cto>         Generate HTML documentation");
    eprintln!("  certo fmt   <file.cto>...      Format source files in place");
    eprintln!("  certo test  <file.cto>...      Run test blocks");
    eprintln!("  certo lint  <file.cto>...      Lint for unused params / dead code");
    eprintln!("  certo bench <file.cto>...      Run bench_ functions");
    eprintln!("  certo db <subcommand>            Database tools (migrate, rollback, status, pull)");
    eprintln!("  certo migrate <subcommand>       Alias for certo db");
    eprintln!();
    eprintln!("Build options:");
    eprintln!("  -o <file>    Output path");
    eprintln!("  --emit-c     Stop after emitting C; write <stem>.c");
    eprintln!("  --emit-dll   Compile to a shared library (.dll / .so)");
    eprintln!("  -v           Verbose: print the C compiler command");
    eprintln!("  --watch, -w  Watch source file and rebuild on change");
    eprintln!();
    eprintln!("DB/migrate subcommands:");
    eprintln!("  migrate [--dry-run]    Apply pending migrations");
    eprintln!("  rollback [N]           Roll back N migrations (default 1)");
    eprintln!("  status                 Show applied/pending migrations");
    eprintln!("  create <name>          Scaffold a new migration file");
    eprintln!("  pull [-o <file>]       Introspect live DB schema → db/schema.cto");
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}

/// Returns true when colour output is appropriate for stderr.
/// Respects the `NO_COLOR` env var and `TERM=dumb` convention.
pub(crate) fn stderr_is_tty() -> bool {
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
// db pull
// ------------------------------------------------------------------ //

fn cmd_db_pull(args: &[String]) {
    let mut out_path: Option<PathBuf> = None;
    let mut schema_name = "public".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                out_path = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| die("-o requires a path", 2))
                ));
            }
            "--schema" => {
                i += 1;
                schema_name = args.get(i)
                    .unwrap_or_else(|| die("--schema requires a name", 2))
                    .clone();
            }
            "--help" | "-h" => {
                println!("Usage: certo db pull [-o <file>] [--schema <name>]");
                println!();
                println!("Introspect the live PostgreSQL database and write a Certo");
                println!("schema snapshot to db/schema.cto (default).");
                println!();
                println!("Options:");
                println!("  -o <file>        Output path (default: db/schema.cto)");
                println!("  --schema <name>  PostgreSQL schema to introspect (default: public)");
                println!();
                println!("Reads DATABASE_URL from the environment or .env file.");
                println!("Requires psql on PATH.");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            _ => {}
        }
        i += 1;
    }

    // Resolve DATABASE_URL — env var first, then .env file.
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if let Ok(contents) = std::fs::read_to_string(cwd.join(".env")) {
            for line in contents.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("DATABASE_URL=") {
                    return val.trim().trim_matches('"').to_string();
                }
            }
        }
        eprintln!("error: DATABASE_URL is not set");
        eprintln!("       Set it in your environment or .env file:");
        eprintln!("       DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret");
        process::exit(1);
    });

    // Verify psql is available.
    let psql_ok = std::process::Command::new("psql")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !psql_ok {
        eprintln!("error: psql not found on PATH");
        eprintln!("       Install PostgreSQL client tools to use certo db pull.");
        process::exit(1);
    }

    // Query columns from information_schema.
    let col_query = format!(
        "SELECT table_name, column_name, data_type, is_nullable \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         ORDER BY table_name, ordinal_position;",
        schema_name.replace('\'', "")
    );

    let col_out = std::process::Command::new("psql")
        .arg(&db_url)
        .arg("--no-align")
        .arg("--tuples-only")
        .arg("--field-separator=|")
        .arg("--command").arg(&col_query)
        .output()
        .unwrap_or_else(|e| { eprintln!("error running psql: {}", e); process::exit(1); });

    if !col_out.status.success() {
        let stderr = String::from_utf8_lossy(&col_out.stderr);
        eprintln!("error: psql failed: {}", stderr.trim());
        process::exit(1);
    }

    // Query primary key columns.
    let pk_query = format!(
        "SELECT kcu.table_name, kcu.column_name \
         FROM information_schema.key_column_usage kcu \
         JOIN information_schema.table_constraints tc \
           ON kcu.constraint_name = tc.constraint_name \
          AND kcu.table_schema    = tc.table_schema \
         WHERE tc.constraint_type = 'PRIMARY KEY' \
           AND tc.table_schema    = '{}' \
         ORDER BY kcu.table_name, kcu.ordinal_position;",
        schema_name.replace('\'', "")
    );

    let pk_out = std::process::Command::new("psql")
        .arg(&db_url)
        .arg("--no-align")
        .arg("--tuples-only")
        .arg("--field-separator=|")
        .arg("--command").arg(&pk_query)
        .output()
        .unwrap_or_else(|e| { eprintln!("error running psql: {}", e); process::exit(1); });

    // Parse primary keys into a set of (table, column).
    let pk_text = String::from_utf8_lossy(&pk_out.stdout);
    let mut primary_keys: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for line in pk_text.lines() {
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() == 2 {
            primary_keys.insert((parts[0].trim().to_string(), parts[1].trim().to_string()));
        }
    }

    // Parse columns and group by table.
    let col_text = String::from_utf8_lossy(&col_out.stdout);
    let mut tables: Vec<String> = Vec::new();
    let mut columns: std::collections::HashMap<String, Vec<(String, String, bool)>> =
        std::collections::HashMap::new();

    for line in col_text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 { continue; }
        let table   = parts[0].trim();
        let col     = parts[1].trim();
        let pg_type = parts[2].trim();
        let nullable = parts[3].trim().eq_ignore_ascii_case("YES");
        let certo_type = pg_type_to_certo(pg_type);
        if !tables.contains(&table.to_string()) {
            tables.push(table.to_string());
        }
        columns.entry(table.to_string())
            .or_default()
            .push((col.to_string(), certo_type, nullable));
    }

    if tables.is_empty() {
        eprintln!("warning: no tables found in schema '{}'", schema_name);
        eprintln!("         Check that DATABASE_URL points to the right database.");
        return;
    }

    // Emit db/schema.cto.
    let out = out_path.unwrap_or_else(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let dir = cwd.join("db");
        std::fs::create_dir_all(&dir).ok();
        dir.join("schema.cto")
    });

    let mut src = String::new();
    src.push_str("module DbSchema\n\n");
    src.push_str("// Generated by `certo db pull` — do not edit manually.\n");
    src.push_str(&format!("// Schema: {}  Database: {}\n\n", schema_name, redact_url(&db_url)));

    for table in &tables {
        let type_name = snake_to_pascal(table);
        src.push_str(&format!("type {} {{\n", type_name));
        if let Some(cols) = columns.get(table) {
            for (col_name, certo_ty, nullable) in cols {
                let field_name = snake_to_camel(col_name);
                let is_pk = primary_keys.contains(&(table.clone(), col_name.clone()));
                let ty_str = if *nullable {
                    format!("{}?", certo_ty)
                } else {
                    certo_ty.clone()
                };
                if is_pk {
                    src.push_str(&format!("    {}: {}  // PK\n", field_name, ty_str));
                } else {
                    src.push_str(&format!("    {}: {}\n", field_name, ty_str));
                }
            }
        }
        src.push_str("}\n\n");
    }

    std::fs::write(&out, &src).unwrap_or_else(|e| {
        eprintln!("error writing {}: {}", out.display(), e);
        process::exit(1);
    });

    eprintln!("wrote {} ({} table(s))", out.display(), tables.len());
    for table in &tables {
        let col_count = columns.get(table).map(|c| c.len()).unwrap_or(0);
        eprintln!("  {}  ({} column(s))", snake_to_pascal(table), col_count);
    }
}

/// Map a PostgreSQL type name to the closest Certo type.
fn pg_type_to_certo(pg: &str) -> String {
    match pg {
        "integer" | "int" | "int4" | "bigint" | "int8" | "smallint" | "int2"
            | "serial" | "bigserial" | "smallserial"   => "Int",
        "text" | "character varying" | "varchar" | "char"
            | "bpchar" | "name" | "citext"             => "Text",
        "boolean" | "bool"                             => "Bool",
        "real" | "float4" | "double precision" | "float8" => "Float",
        "numeric" | "decimal" | "money"                => "Decimal",
        "uuid"                                         => "UUID",
        "date" | "timestamp" | "timestamp without time zone"
            | "timestamp with time zone" | "timestamptz" => "DateTime",
        "json" | "jsonb"                               => "Text",
        "bytea"                                        => "Text",
        other => return snake_to_pascal(other),
    }.to_string()
}

/// `users_table` → `UsersTable`
fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

/// `created_at` → `createdAt`
fn snake_to_camel(s: &str) -> String {
    let mut parts = s.split('_');
    let first = parts.next().unwrap_or("").to_string();
    let rest: String = parts.map(|w| {
        let mut c = w.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }).collect();
    first + &rest
}

/// Redact the password from a connection URL for display.
fn redact_url(url: &str) -> String {
    // Handle URL format: postgres://user:pass@host/db
    if let Some(at) = url.find('@') {
        if let Some(colon) = url[..at].rfind(':') {
            return format!("{}:****{}", &url[..colon], &url[at..]);
        }
    }
    // Keyword format: just hide the password= value.
    url.split_whitespace()
        .map(|kv| {
            if kv.to_lowercase().starts_with("password=") { "password=****".to_string() }
            else { kv.to_string() }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------------------------ //
// db
// ------------------------------------------------------------------ //

fn cmd_db(args: &[String]) {
    let sub = args.first().map(String::as_str).unwrap_or("");
    match sub {
        // certo db migrate [--dry-run]  →  certo migrate up
        "migrate" => {
            let mut fwd = vec!["up".to_string()];
            fwd.extend_from_slice(&args[1..]);
            cmd_migrate(&fwd);
        }
        // certo db rollback [N] [--dry-run]  →  certo migrate down [N]
        "rollback" => {
            let mut fwd = vec!["down".to_string()];
            fwd.extend_from_slice(&args[1..]);
            cmd_migrate(&fwd);
        }
        // certo db status  →  certo migrate status
        "status" => cmd_migrate(&["status".to_string()]),
        // certo db create <name>  →  certo migrate create <name>
        "create" => {
            let mut fwd = vec!["create".to_string()];
            fwd.extend_from_slice(&args[1..]);
            cmd_migrate(&fwd);
        }
        "pull" => cmd_db_pull(&args[1..]),
        "--help" | "-h" | "" => {
            println!("Usage: certo db <subcommand> [options]");
            println!();
            println!("Subcommands:");
            println!("  migrate [--dry-run]    Apply all pending migrations");
            println!("  rollback [N]           Roll back N migrations (default 1)");
            println!("  status                 Show applied vs pending migrations");
            println!("  create <name>          Scaffold a new migration file");
            println!("  pull [-o <file>]       Introspect live DB → db/schema.cto");
            println!();
            println!("Set DATABASE_URL in your environment or .env file.");
        }
        other => {
            eprintln!("Unknown db subcommand: {}", other);
            eprintln!("Usage: certo db migrate | rollback [N] | status | create <name> | pull");
            process::exit(1);
        }
    }
}

// ------------------------------------------------------------------ //
// migrate
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
            let filename = migrations_dir.join(format!("{}.cto", name));
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
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cto"))
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
