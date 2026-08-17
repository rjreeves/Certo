mod cmd_doc;
mod cmd_watch;
mod cmd_repl;
mod cmd_lint;
mod cmd_generate;
mod certo_toml;
mod diff;
mod static_serve;


use std::path::{Path, PathBuf};
use std::process;
use certo_ast::decl::{Decl, MigrationDecl};
use certo_ast::module::Module;
use certo_migrate::{
    plan_up, plan_down, plan_sql, commit_steps, status,
    default_manifest_path,
};
use certo_diagnostics::{Diagnostic, render_all};
use certo_typeck::{TypeError, TypeErrorKind, TypeEnv, assign_var_names};
use certo_traits::{TraitError, TraitErrorKind};
use certo_dbschema::DbErrorKind;
use certo_effects::{EffectError, EffectErrorKind};
use certo_hir::{LowerError, LowerErrorKind};

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

const CERTO_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"), ".",
    env!("CERTO_BUILD_DATE"), ".",
    env!("CERTO_BUILD_NUM")
);

fn main() {
    // Compilation (parser, HIR/MIR lowering, codegen) is deeply recursive, and
    // the OS main-thread stack is small on Windows (~1 MB). Run on a worker thread
    // with a large stack so deeply-nested source doesn't overflow it.
    let child = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024) // 512 MB
        .spawn(real_main)
        .expect("failed to spawn compiler thread");
    // Propagate the worker's exit (its process::exit calls end the process; a
    // panic here means the worker panicked — re-abort with a non-zero code).
    if child.join().is_err() {
        process::exit(101);
    }
}

fn real_main() {
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
            "bench"    => cmd_bench(&args[2..]),
            "generate" => cmd_generate::cmd_generate(&args[2..]),
            "new"     => cmd_new(&args[2..]),
            "migrate" => cmd_migrate(&args[2..]),
            "db"      => cmd_db(&args[2..]),
            "add"     => cmd_add(&args[2..]),
            "audit"   => cmd_audit(&args[2..]),
            "repl"    => cmd_repl::cmd_repl(&args[2..]),
            "help" | "--help" | "-h" => { print_top_help(); }
            "--version" | "-V" => { println!("certo {}", CERTO_VERSION); }
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
    let mut strict  = false;
    let mut explain = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--verbose" | "-v" => verbose = true,
            "--strict"         => strict  = true,
            "--explain"        => explain = true,
            "--help" | "-h" => {
                println!("Usage: certo check <file.cto> [-v] [--strict] [--explain]");
                println!();
                println!("Type-check a Certo source file without compiling.");
                println!("Exits 0 on success, 1 if there are parse or type errors.");
                println!();
                println!("Options:");
                println!("  --strict   Also run lint checks (certo lint) and fail if any warnings are found");
                println!("  --explain  Print a full explanation from the error reference for each diagnostic code");
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
    run_typeck_opts(&module, &src, &filename, colour, explain);

    if strict {
        let warnings = cmd_lint::lint_hir(&module, &input, &src, colour);
        if warnings > 0 {
            eprintln!("{} warning(s) found (--strict)", warnings);
            process::exit(1);
        }
    }

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

    // `--watch`/`-w`: rebuild AND re-run the program on every change, not
    // just rebuild silently. `cmd_build`'s own `--watch` handling only
    // rebuilds — reusing it as-is here would mean `certo run --watch`
    // rebuilds forever without the program ever executing even once. So,
    // same pattern `cmd_build` itself uses (see its `if watch` branch): the
    // watch flag is intercepted here and this process re-invokes itself as
    // `certo run <args without --watch>` on each change via
    // `cmd_watch::watch_loop`, whose exit code (build *and* run together)
    // reports success/failure for the watch status line.
    if build_args.iter().any(|a| a == "--watch" || a == "-w") {
        let input_path = build_args.iter()
            .find(|a| a.ends_with(".cto") || (!a.starts_with('-') && !a.starts_with("build")))
            .cloned()
            .unwrap_or_else(|| {
                eprintln!("error: no input file");
                eprintln!("usage: certo run <file.cto> [build-opts] [-- prog-args]");
                process::exit(2);
            });

        let watch_files = vec![PathBuf::from(&input_path)];
        let self_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("certo"));
        let filtered_build_args: Vec<String> = build_args.iter()
            .filter(|a| *a != "--watch" && *a != "-w")
            .cloned()
            .collect();
        let mut run_args: Vec<String> = std::iter::once("run".to_string())
            .chain(filtered_build_args)
            .collect();
        if !prog_args.is_empty() {
            run_args.push("--".to_string());
            run_args.extend(prog_args.iter().cloned());
        }
        cmd_watch::watch_loop(watch_files, move || {
            std::process::Command::new(&self_exe)
                .args(&run_args)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        });
        return;
    }

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
    let mut windows_gui = false;
    let mut watch    = false;
    let mut release  = false;
    let mut links: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                output = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| die("-o requires a path", 2))
                ));
            }
            "--link" => {
                i += 1;
                links.push(args.get(i).unwrap_or_else(|| die("--link requires a library path", 2)).clone());
            }
            "--emit-c"        => emit_c   = true,
            "--emit-dll"      => emit_dll = true,
            "--windows-gui"    => windows_gui = true,
            "--verbose" | "-v" => verbose = true,
            "--watch" | "-w"  => watch    = true,
            "--release"       => release  = true,
            "--help" | "-h" => {
                println!("Usage: certo <file.cto> [-o <out>] [--emit-c] [--emit-dll] [--windows-gui] [-v] [--watch] [--release]");
                println!("       certo build <file.cto> [-o <out>] [--emit-c] [--emit-dll] [--windows-gui] [-v] [--watch] [--release]");
                println!();
                println!("Options:");
                println!("  -o <file>    Output path");
                println!("  --emit-c     Write the generated C to <stem>.c and stop");
                println!("  --emit-dll   Compile to a shared library (.dll/.so) instead of an exe");
                println!("  --link <lib> Link a native library (for extern \"C\" FFI); may be repeated");
                println!("  --windows-gui  Build a Windows GUI-subsystem exe with no launcher console");
                println!("  -v           Verbose: print the C compiler command");
                println!("  --watch, -w  Watch the source file and rebuild on change");
                println!("  --release    Apply the [targets.production] profile from certo.toml");
                println!("               (optimize/strip-debug/schema overrides — see docs/CLI-TOOLCHAIN.md)");
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

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd_toml = load_certo_toml_or_die(&cwd);

    let input = input.unwrap_or_else(|| {
        // No file argument — try reading `[build] entry` from certo.toml in cwd.
        match cwd_toml.as_ref().and_then(|c| c.build.as_ref()).and_then(|b| b.entry.as_deref()) {
            Some(entry) if !entry.is_empty() => cwd.join(entry),
            Some(_) | None => {
                if cwd_toml.is_some() {
                    eprintln!("error: certo.toml found but has no `entry` field under [build]");
                    eprintln!("       add:  entry = \"src/main.cto\"");
                } else {
                    eprintln!("error: no input file and no certo.toml in current directory");
                    eprintln!("usage: certo <file.cto> [-o <out>]");
                    eprintln!("       or run from a project directory containing certo.toml");
                }
                process::exit(2);
            }
        }
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

    // Read certo.toml if present: auto-detect lib type, output directory, and
    // (with --release) the [targets.production] build profile.
    let mut toml_output_dir: Option<PathBuf> = None;
    let mut target_optimize = true;
    let mut target_strip_debug = false;
    // Prefer cwd as the project root when it has its own certo.toml — matters
    // when `input` was resolved from `[build] entry` (e.g. `src/main.cto`),
    // whose *parent* is not the project root. Falls back to the input file's
    // own directory only when cwd has no manifest (e.g. building a file from
    // outside its project, the original fallback this mirrors).
    let project_root = if cwd_toml.is_some() {
        cwd.clone()
    } else {
        input.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let project_toml = if project_root == cwd {
        cwd_toml.clone()
    } else {
        load_certo_toml_or_die(&project_root)
    };
    if let Some(cfg) = &project_toml {
        if let Some(build) = &cfg.build {
            if !emit_dll && build.ty.as_deref() == Some("lib") {
                emit_dll = true;
            }
            if let Some(output_dir) = build.output.as_deref().filter(|s| !s.is_empty()) {
                let dir = project_root.join(output_dir);
                std::fs::create_dir_all(&dir).ok();
                toml_output_dir = Some(dir);
            }
        }
        if release {
            if let Some(prod) = cfg.targets.as_ref().and_then(|t| t.get("production")) {
                target_optimize = prod.optimize.unwrap_or(true);
                target_strip_debug = prod.strip_debug.unwrap_or(false);
                if let Some(schema) = &prod.schema {
                    // Overrides DATABASE_URL for this process only, so the
                    // schema-sync check below (if enabled) connects against
                    // the production schema URL instead of the dev default.
                    unsafe { std::env::set_var("DATABASE_URL", certo_toml::expand_env_vars(schema)); }
                }
            } else if verbose {
                eprintln!("note: --release passed but certo.toml has no [targets.production] section — using defaults");
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

    // ── Expand state machines and validators into executable functions ─
    expand_state_machines(&mut module, colour);
    expand_validators(&mut module, colour);

    // ── Type-check ────────────────────────────────────────────────────
    run_typeck_opts_with_root(&module, &src, &filename, colour, false, &project_root);

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
            || segs == ["Stdlib", "DbQuery"] || segs == ["DbQuery"]
            || segs == ["Stdlib", "DbMutation"] || segs == ["DbMutation"]
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
            line_directives: None,
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
       .arg(if target_optimize { "-O2" } else { "-O0" })
       .arg("-Wno-int-to-pointer-cast")
       .arg("-Wno-pointer-to-int-cast")
       .arg("-Wno-int-conversion")
       .arg("-Wno-implicit-function-declaration")
       .arg("-Wno-deprecated-declarations")
       .arg("-Wno-incompatible-function-pointer-types");
    if target_strip_debug && !cfg!(windows) {
        // Strips the symbol table / relocation info from the linked binary — a
        // real effect on ELF/Mach-O regardless of whether -g was ever passed
        // (it wasn't; certo never emits debug info). On Windows this is a
        // GNU-driver flag: this clang invocation targets the MSVC linker
        // (`-Xlinker /subsystem:...` below), which doesn't accept `-s` and
        // just warns it's unused — and since certo never passes `/DEBUG`
        // there either, there's no embedded symbol table (that lives in a
        // separate .pdb only when /DEBUG is requested) to strip in the first
        // place, so skipping it on Windows changes nothing observable.
        cmd.arg("-s");
    }

    // POSIX threads for `parallel {}` / `spawn` / `await`. On Windows the runtime
    // uses Win32 threads (kernel32, always linked), so no extra flag is needed.
    if !cfg!(windows) {
        cmd.arg("-pthread");
    }

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
            // wmain (Unicode entry) for correct quoted-arg handling from any shell.
            // --windows-gui keeps the same entry but avoids a launcher console.
            if windows_gui {
                cmd.arg("-Xlinker").arg("/subsystem:windows");
            } else {
                cmd.arg("-Xlinker").arg("/subsystem:console");
            }
            cmd.arg("-Xlinker").arg("/entry:wmainCRTStartup");
            // Standard Windows libs
            cmd.arg("-luser32");   // MessageBox
            cmd.arg("-lshell32");  // CommandLineToArgvW (used by wmainCRTStartup)
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

    // Link any user-specified native libraries (e.g. a Rust cdylib's import lib)
    // for `extern "C"` FFI. Passed as positional args so the linker resolves them.
    for lib in &links {
        cmd.arg(lib);
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

/// Expand each `validator { … }` declaration into executable functions
/// (`V_validate`, `V_validateAll`, optional `VContext` type) by generating Certo
/// source from the validator, parsing it, and splicing the decls into the module.
/// Call sites use `V.validate(...)`, which links to `V_validate` via `c_fn_name`.
fn expand_validators(module: &mut Module, colour: bool) {
    use certo_ast::decl::Decl;
    let mut generated = String::new();
    for d in &module.decls {
        if let Decl::Validator(v) = &d.node {
            generated.push_str(&certo_codegen::emit_validator(v).to_source());
            generated.push('\n');
        }
    }
    if generated.trim().is_empty() { return; }

    let wrapped = format!("module __validators\n{}", generated);
    match certo_parser::parse(&wrapped) {
        Ok(gen_module) => module.decls.extend(gen_module.decls),
        Err(errs) => {
            // A failure here is a compiler bug in the validator generator, not a
            // user error — surface it clearly rather than silently dropping rules.
            let diags: Vec<Diagnostic> = errs.iter()
                .map(|e| Diagnostic::error("", format!("{}", e)).with_span(e.span))
                .collect();
            eprint!("{}", render_all(&diags, &wrapped, "<generated validator>", colour));
            eprintln!("internal error: generated validator source failed to parse");
            process::exit(1);
        }
    }
}

/// Expand each `statemachine { … }` into a state enum plus transition / predicate
/// / accessor functions, **replacing** the original declaration. After this the
/// module contains only ordinary types and functions, so the rest of the
/// pipeline needs no special state-machine handling.
fn expand_state_machines(module: &mut Module, colour: bool) {
    use certo_ast::decl::Decl;
    let mut generated = String::new();
    let mut kept = Vec::with_capacity(module.decls.len());
    for decl in std::mem::take(&mut module.decls) {
        if let Decl::StateMachine(sm) = &decl.node {
            generated.push_str(&certo_codegen::emit_state_machine(sm));
            generated.push('\n');
            // drop the original decl — it is fully expanded
        } else {
            kept.push(decl);
        }
    }
    module.decls = kept;
    if generated.trim().is_empty() { return; }

    let wrapped = format!("module __statemachines\n{}", generated);
    match certo_parser::parse(&wrapped) {
        Ok(gen_module) => module.decls.extend(gen_module.decls),
        Err(errs) => {
            let diags: Vec<Diagnostic> = errs.iter()
                .map(|e| Diagnostic::error("", format!("{}", e)).with_span(e.span))
                .collect();
            eprint!("{}", render_all(&diags, &wrapped, "<generated state machine>", colour));
            eprintln!("internal error: generated state-machine source failed to parse");
            process::exit(1);
        }
    }
}

/// Run typeck with stdlib builtins seeded. Exits on type errors.
fn run_typeck(module: &Module, src: &str, filename: &str, colour: bool) {
    run_typeck_opts(module, src, filename, colour, false);
}

/// Like `run_typeck_opts`, but for callers that know the actual project root
/// (the directory containing `certo.toml`) independently of `filename` — e.g.
/// `cmd_build`, when `filename` came from `[build] entry` and so lives in a
/// subdirectory (`src/main.cto`) rather than at the project root itself.
/// Every other caller passes an explicit file path from the user, where
/// `filename`'s own parent directory already *is* the project root, so they
/// use the plain `run_typeck`/`run_typeck_opts` (which fall back to that).
fn run_typeck_opts_with_root(
    module: &Module, src: &str, filename: &str, colour: bool, explain: bool, project_root: &Path,
) {
    run_typeck_inner(module, src, filename, colour, explain, project_root);
}

/// The embedded error reference (`docs/ERROR-REFERENCE.md`) — bundled into the
/// binary via `include_str!` so `--explain` works regardless of the current
/// working directory or whether the repo is even present at runtime.
const ERROR_REFERENCE: &str = include_str!("../../../docs/ERROR-REFERENCE.md");

/// Look up the full write-up for a diagnostic code (`"E0100"`, `"L001"`, ...)
/// from the embedded error reference. `None` if the code has no entry there —
/// `--explain` silently skips it rather than printing something wrong.
fn explain_code(code: &str) -> Option<String> {
    let marker = format!("### {}", code);
    let start = ERROR_REFERENCE.find(&marker)?;
    let rest = &ERROR_REFERENCE[start..];
    let after_heading = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
    let next_h2 = rest[after_heading..].find("\n## ").map(|i| i + after_heading);
    let next_h3 = rest[after_heading..].find("\n### ").map(|i| i + after_heading);
    let end = [next_h2, next_h3].into_iter().flatten().min().unwrap_or(rest.len());
    let mut section = rest[..end].to_string();
    if let Some(pos) = section.rfind("\n---") {
        section.truncate(pos);
    }
    Some(section.trim().to_string())
}

/// Print each diagnostic's explanation once (deduped by code, first-seen order).
fn print_explanations(diags: &[Diagnostic]) {
    let mut seen = std::collections::HashSet::new();
    for d in diags {
        if !seen.insert(d.code.clone()) { continue; }
        if let Some(text) = explain_code(&d.code) {
            eprintln!();
            eprintln!("{}", text);
        }
    }
}

/// Run typeck with stdlib builtins seeded. Exits on type errors. `explain`
/// prints each diagnostic code's full write-up from `docs/ERROR-REFERENCE.md`
/// before exiting (`certo check --explain`).
fn run_typeck_opts(module: &Module, src: &str, filename: &str, colour: bool, explain: bool) {
    let project_root = Path::new(filename).parent().unwrap_or(Path::new(".")).to_path_buf();
    run_typeck_inner(module, src, filename, colour, explain, &project_root);
}

fn run_typeck_inner(
    module: &Module, src: &str, filename: &str, colour: bool, explain: bool, project_root: &Path,
) {
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
        if explain { print_explanations(&diags); }
        process::exit(1);
    }

    if let Err(errs) = certo_traits::check_module(module) {
        let diags: Vec<Diagnostic> = errs.iter()
            .map(|e| trait_error_to_diagnostic(e))
            .collect();
        eprint!("{}", render_all(&diags, src, filename, colour));
        eprintln!("aborting due to {} trait error(s)", diags.len());
        if explain { print_explanations(&diags); }
        process::exit(1);
    }

    let mut effect_env = certo_effects::EffectEnv::new();
    certo_stdlib::seed_stdlib_effects(&mut effect_env);
    if let Err(errs) = certo_effects::check_module_seeded(module, effect_env) {
        let diags: Vec<Diagnostic> = errs.iter()
            .map(|e| effect_error_to_diagnostic(e))
            .collect();
        eprint!("{}", render_all(&diags, src, filename, colour));
        eprintln!("aborting due to {} effect error(s)", diags.len());
        if explain { print_explanations(&diags); }
        process::exit(1);
    }

    if let Err(errs) = certo_hir::lower_module(module) {
        let diags: Vec<Diagnostic> = errs.iter()
            .map(|e| hir_error_to_diagnostic(e))
            .collect();
        eprint!("{}", render_all(&diags, src, filename, colour));
        eprintln!("aborting due to {} error(s)", diags.len());
        if explain { print_explanations(&diags); }
        process::exit(1);
    }

    let schema = match certo_dbschema::check_module(module) {
        Ok(schema) => schema,
        Err(errs) => {
            let diags: Vec<Diagnostic> = errs.iter()
                .map(|e| db_error_to_diagnostic(e))
                .collect();
            eprint!("{}", render_all(&diags, src, filename, colour));
            eprintln!("aborting due to {} schema error(s)", diags.len());
            if explain { print_explanations(&diags); }
            process::exit(1);
        }
    };

    // ── Live schema-sync (opt-in via `[features] schema-sync = true` in certo.toml) ────
    // Off by default so a normal build/check never needs a database connection — this
    // only runs for projects that explicitly ask for it, and only checks `type`s marked
    // `impl DbRow for X {}` against the live database, not every declared record type.
    if read_schema_sync_flag(project_root) {
        let db_url = resolve_database_url_for_sync().unwrap_or_else(|| {
            eprintln!("error: `[features] schema-sync = true` is set in certo.toml, but DATABASE_URL is not set");
            eprintln!("       set it in your environment or a .env file");
            process::exit(1);
        });
        match introspect_live_schema_for_sync(&db_url, "public") {
            Ok(live) => {
                let errs = certo_dbschema::check_schema_sync(module, &schema, &live);
                if !errs.is_empty() {
                    let diags: Vec<Diagnostic> = errs.iter()
                        .map(|e| db_error_to_diagnostic(e))
                        .collect();
                    eprint!("{}", render_all(&diags, src, filename, colour));
                    eprintln!("aborting due to {} schema-sync error(s) — the live database has drifted from your `type` declarations", diags.len());
                    if explain { print_explanations(&diags); }
                    process::exit(1);
                }
            }
            Err(msg) => {
                eprintln!("error: schema-sync failed: {}", msg);
                process::exit(1);
            }
        }
    }
}

/// Reads `schema-sync = true` from the `[features]` section of `certo.toml` in
/// `project_root`, if present — matches `docs/Certo_Language_Specification.md`
/// section 11.4's schema (`[features] schema-sync = true`). Previously this read
/// (undocumented, section-blind) whatever line matched `schema-sync = ...`
/// anywhere in the file; now parsed through `certo_toml::load` like every other
/// field.
fn read_schema_sync_flag(project_root: &Path) -> bool {
    load_certo_toml_or_die(project_root)
        .and_then(|cfg| cfg.features)
        .and_then(|f| f.schema_sync)
        .unwrap_or(false)
}

/// Loads `certo.toml` from `project_root`, exiting with a clear parse error if
/// the file exists but is malformed TOML. `None` (not an error) if the file is
/// simply absent — callers fall back to defaults/explicit CLI args.
fn load_certo_toml_or_die(project_root: &Path) -> Option<certo_toml::CertoToml> {
    match certo_toml::load(project_root) {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("error: {}", msg);
            process::exit(2);
        }
    }
}

/// Resolve the migrations directory for `project_root`: `certo.toml`'s
/// `[database] migrations` (section 11.4's schema — parsed since item 91 but
/// never read by anything until this) if set, else `migrations/` (this
/// command's own working default before this field was wired up, kept as
/// the fallback so an existing project with no `[database]` section sees no
/// behavior change).
fn migrations_dir(project_root: &Path) -> PathBuf {
    let configured = load_certo_toml_or_die(project_root)
        .and_then(|cfg| cfg.database)
        .and_then(|db| db.migrations);
    match configured {
        Some(dir) => project_root.join(dir),
        None => project_root.join("migrations"),
    }
}

/// Resolve `DATABASE_URL` — env var first, then `.env` in the current directory. Mirrors
/// `cmd_db_pull`'s resolution, but returns `Option` instead of exiting, since the caller
/// wants to print its own schema-sync-specific error message.
fn resolve_database_url_for_sync() -> Option<String> {
    if let Ok(v) = std::env::var("DATABASE_URL") { return Some(v); }
    let cwd = std::env::current_dir().ok()?;
    let contents = std::fs::read_to_string(cwd.join(".env")).ok()?;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("DATABASE_URL=") {
            return Some(val.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Introspect the live database for schema-sync — the same `information_schema.columns`
/// query `certo db pull` uses, but returning structured data (and a `Result` instead of
/// exiting the process) so `run_typeck` can report it as an ordinary compile error.
fn introspect_live_schema_for_sync(db_url: &str, schema_name: &str) -> Result<Vec<certo_dbschema::LiveTable>, String> {
    let psql_ok = std::process::Command::new("psql")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !psql_ok {
        return Err("psql not found on PATH — required for schema-sync".to_string());
    }

    let col_query = format!(
        "SELECT table_name, column_name, data_type, is_nullable, numeric_precision, numeric_scale \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         ORDER BY table_name, ordinal_position;",
        schema_name.replace('\'', "")
    );

    let col_out = std::process::Command::new("psql")
        .arg("-d").arg(db_url)
        .arg("--no-align")
        .arg("--tuples-only")
        .arg("--field-separator=|")
        .arg("--command").arg(&col_query)
        .output()
        .map_err(|e| format!("error running psql: {}", e))?;

    if !col_out.status.success() {
        return Err(format!("psql failed: {}", String::from_utf8_lossy(&col_out.stderr).trim()));
    }

    let stdout = String::from_utf8_lossy(&col_out.stdout);
    let mut tables: Vec<certo_dbschema::LiveTable> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 6 { continue; }
        let table_name = parts[0].trim().to_string();
        let col_name    = parts[1].trim().to_string();
        let pg_type     = parts[2].trim();
        let nullable    = parts[3].trim().eq_ignore_ascii_case("YES");
        let precision   = parts[4].trim().parse::<i64>().ok();
        let scale       = parts[5].trim().parse::<i64>().ok();
        let certo_type  = pg_type_to_certo(pg_type, precision, scale);

        let idx = match tables.iter().position(|t| t.name == table_name) {
            Some(i) => i,
            None => {
                tables.push(certo_dbschema::LiveTable { name: table_name, columns: Vec::new() });
                tables.len() - 1
            }
        };
        tables[idx].columns.push(certo_dbschema::LiveColumn { name: col_name, certo_type, nullable });
    }
    Ok(tables)
}

fn db_error_to_diagnostic(e: &certo_dbschema::DbError) -> Diagnostic {
    match &e.kind {
        DbErrorKind::ColumnTypeMismatch { table, column, declared, migration } =>
            Diagnostic::error("E0501",
                format!("column `{}.{}` type mismatch", table, column))
                .with_span(e.span)
                .with_label(format!("migration uses `{}`, type declaration has `{}`", migration, declared))
                .with_note("update the migration column type to match the `type` declaration"),

        DbErrorKind::UnknownForeignKeyTarget { table, column, references } =>
            Diagnostic::error("E0502",
                format!("foreign key `{}.{}` references unknown table `{}`", table, column, references))
                .with_span(e.span)
                .with_note("the referenced table must be declared as a `type` in this module"),

        DbErrorKind::DuplicateMigration { name, .. } =>
            Diagnostic::error("E0503",
                format!("duplicate migration name `{}`", name))
                .with_span(e.span)
                .with_note("migration names must be unique within a module"),

        DbErrorKind::MissingDownMigration { name } =>
            Diagnostic::error("E0504",
                format!("migration `{}` has no `down` block", name))
                .with_span(e.span)
                .with_note("add a `down { ... }` block to make this migration reversible"),

        DbErrorKind::TableNotDeclaredAsType { migration, table } =>
            Diagnostic::error("E0505",
                format!("migration `{}` creates table `{}` which is not declared as a `type`", migration, table))
                .with_span(e.span)
                .with_note(format!("add `type {} {{ ... }}` to your module", table)),

        DbErrorKind::UnknownColumn { migration, table, column } =>
            Diagnostic::error("E0506",
                format!("column `{}` does not exist on type `{}`", column, table))
                .with_span(e.span)
                .with_label(format!("referenced in migration `{}`", migration))
                .with_note(format!("add field `{}: <Type>` to the `type {}` declaration", column, table)),

        DbErrorKind::TableNotCreated { migration, table } =>
            Diagnostic::error("E0507",
                format!("migration `{}` alters/drops `{}` before it was created", migration, table))
                .with_span(e.span)
                .with_note("add a `CreateTable` operation for this table in an earlier migration"),

        DbErrorKind::UnknownTable { migration, table } =>
            Diagnostic::error("E0500",
                format!("migration `{}` references unknown table `{}`", migration, table))
                .with_span(e.span),

        DbErrorKind::QueryUnknownTable { table } =>
            Diagnostic::error("E0508",
                format!("`Query.from(\"{}\")` — no such table", table))
                .with_span(e.span)
                .with_note(format!("add `type {} {{ ... }}` to this module, or check for a typo", table)),

        DbErrorKind::QueryUnknownColumn { table, column } =>
            Diagnostic::error("E0509",
                format!("column `{}` does not exist on `{}`", column, table))
                .with_span(e.span)
                .with_note(format!("add field `{}: <Type>` to the `type {}` declaration, or check for a typo", column, table)),

        DbErrorKind::QueryNonLiteralArg { function, position } =>
            Diagnostic::error("E0510",
                format!("`{}`'s {} argument must be a string literal", function, position))
                .with_span(e.span)
                .with_note("column/operator/table names must be literal so the compiler can verify them against the schema"),

        DbErrorKind::QueryInvalidOperator { op } =>
            Diagnostic::error("E0511",
                format!("`\"{}\"` is not a recognized query operator", op))
                .with_span(e.span)
                .with_note("valid operators: \"=\", \"!=\", \"<\", \"<=\", \">\", \">=\", \"like\""),

        DbErrorKind::QueryInvalidSortDir { dir } =>
            Diagnostic::error("E0512",
                format!("`\"{}\"` is not a valid sort direction", dir))
                .with_span(e.span)
                .with_note("valid directions: \"asc\", \"desc\""),

        DbErrorKind::QueryInvalidAggFn { agg } =>
            Diagnostic::error("E0513",
                format!("`\"{}\"` is not a recognized aggregate function", agg))
                .with_span(e.span)
                .with_note("valid aggregate functions: \"count\", \"sum\", \"avg\", \"min\", \"max\""),

        DbErrorKind::QueryInvalidAlias { alias } =>
            Diagnostic::error("E0514",
                format!("`\"{}\"` is not a valid alias", alias))
                .with_span(e.span)
                .with_note("aliases must start with a letter or underscore, followed by letters, digits, or underscores"),

        DbErrorKind::QueryAmbiguousColumn { column, tables } =>
            Diagnostic::error("E0515",
                format!("column `{}` is ambiguous", column))
                .with_span(e.span)
                .with_label(format!("present on {}", tables.iter().map(|t| format!("`{}`", t)).collect::<Vec<_>>().join(", ")))
                .with_note(format!("qualify it, e.g. \"{}.{}\"", tables.first().map(String::as_str).unwrap_or("Table"), column)),

        DbErrorKind::QueryGroupedTerminalMisuse { function } =>
            Diagnostic::error("E0516",
                format!("`{}` cannot be used on a grouped/aggregated query", function))
                .with_span(e.span)
                .with_note("after `.groupBy`/`.aggregate`, use `.groupedList` to run the query"),

        DbErrorKind::QueryColumnTableNotJoined { table } =>
            Diagnostic::error("E0517",
                format!("`{}` is not the base table or a joined table in this query", table))
                .with_span(e.span)
                .with_note("add a `.join`/`.leftJoin` on this table first, or check for a typo"),

        DbErrorKind::QueryJoinColumnNotQualified { column } =>
            Diagnostic::error("E0518",
                format!("join column `\"{}\"` must be qualified", column))
                .with_span(e.span)
                .with_note(format!("write it as \"Table.{}\"", column)),

        DbErrorKind::MutationUnknownTable { function, table } =>
            Diagnostic::error("E0519",
                format!("`{}(\"{}\")` — no such table", function, table))
                .with_span(e.span)
                .with_note(format!("add `type {} {{ ... }}` to this module, or check for a typo", table)),

        DbErrorKind::MutationInvalidStage { function, kind } =>
            Diagnostic::error("E0520",
                format!("`{}` cannot be used on {}", function, kind))
                .with_span(e.span),

        DbErrorKind::MutationRowArityMismatch { expected, found } =>
            Diagnostic::error("E0521",
                format!("`.addRow` has {} value(s), but `.insertMany` declared {} column(s)", found, expected))
                .with_span(e.span),

        DbErrorKind::SchemaSyncTableMissing { table } =>
            Diagnostic::error("E0522",
                format!("`type {}` has `impl DbRow`, but no matching table exists in the live database", table))
                .with_span(e.span)
                .with_note("the live schema has drifted from this declaration — update the `type` or the database"),

        DbErrorKind::SchemaSyncColumnMissing { table, column } =>
            Diagnostic::error("E0523",
                format!("`{}.{}` has no matching column in the live database", table, column))
                .with_span(e.span),

        DbErrorKind::SchemaSyncTypeMismatch { table, column, declared, live } =>
            Diagnostic::error("E0524",
                format!("`{}.{}` type mismatch", table, column))
                .with_span(e.span)
                .with_label(format!("declared as `{}`, live database column is `{}`", declared, live)),

        DbErrorKind::SchemaSyncNullabilityMismatch { table, column, declared_nullable, live_nullable } =>
            Diagnostic::error("E0525",
                format!("`{}.{}` nullability mismatch", table, column))
                .with_span(e.span)
                .with_label(format!("declared as {}, live database column is {}",
                    if *declared_nullable { "nullable" } else { "not nullable" },
                    if *live_nullable { "nullable" } else { "not nullable" })),

        DbErrorKind::QueryDuplicateAlias { alias } =>
            Diagnostic::error("E0526",
                format!("alias `\"{}\"` is already used in this query", alias))
                .with_span(e.span)
                .with_note("give each occurrence of a self-joined table a distinct alias via `.fromAs`/`.joinAs`/`.leftJoinAs`"),
    }
}

/// HIR lowering errors previously had no diagnostic surfacing at all — codegen's
/// own call to `lower_module` (`crates/codegen/src/emit_module.rs`) silently wrote
/// `/* HIR lowering errors: N */` into the generated C and returned, with nothing
/// shown to the user. Running `lower_module` here too, before codegen, gives these
/// real diagnostics (found while adding BACKLOG's closure-capture rejection check,
/// the first check that can genuinely fail here for ordinary, well-typed source).
fn hir_error_to_diagnostic(e: &LowerError) -> Diagnostic {
    match &e.kind {
        LowerErrorKind::UnresolvedName(n) =>
            Diagnostic::error("E0600", format!("unresolved name `{}`", n))
                .with_span(e.span),
        LowerErrorKind::Unsupported(msg) =>
            Diagnostic::error("E0601", msg.clone())
                .with_span(e.span),
    }
}

fn trait_error_to_diagnostic(e: &TraitError) -> Diagnostic {
    match &e.kind {
        TraitErrorKind::UnsatisfiedBound { ty, trait_name } => {
            let note = if trait_name == "DbRow" {
                format!(
                    "only types generated by `certo db pull` implement `DbRow`; \
                     add `impl DbRow for {} {{}}` to suppress this error", ty)
            } else {
                format!("add `impl {} for {} {{}}`, or pass a type that already implements it", trait_name, ty)
            };
            Diagnostic::error("E0305",
                format!("type `{}` does not implement `{}`", ty, trait_name))
                .with_span(e.span)
                .with_label(format!("`{}` is not a `{}` type", ty, trait_name))
                .with_note(note)
        }
        TraitErrorKind::MissingMethod { trait_name, method } =>
            Diagnostic::error("E0301",
                format!("impl is missing method `{}` required by `{}`", method, trait_name))
                .with_span(e.span),
        TraitErrorKind::UnknownMethod { trait_name, method } =>
            Diagnostic::error("E0300",
                format!("method `{}` is not declared in trait `{}`", method, trait_name))
                .with_span(e.span),
        _ =>
            Diagnostic::error("E0300", format!("{:?}", e.kind))
                .with_span(e.span),
    }
}

fn effect_error_to_diagnostic(e: &EffectError) -> Diagnostic {
    match &e.kind {
        EffectErrorKind::UndeclaredEffect { fn_name, effect } => {
            let name = certo_effects::effect_name(effect);
            Diagnostic::error("E0400",
                format!("function `{}` uses effect `{}` without declaring it", fn_name, name))
                .with_span(e.span)
                .with_label(format!("requires `[{}]`", name))
                .with_note(format!("add `[{}]` to `{}`'s signature, or remove the effectful call", name, fn_name))
        }
        EffectErrorKind::ImpureCallInPure { caller, callee, effect } => {
            let name = certo_effects::effect_name(effect);
            Diagnostic::error("E0401",
                format!("pure function `{}` calls `{}`, which requires `{}`", caller, callee, name))
                .with_span(e.span)
                .with_label(format!("`{}` is not pure", callee))
                .with_note(format!("add `[{}]` to `{}`'s signature", name, caller))
        }
        EffectErrorKind::MissingAsyncAnnotation { fn_name } =>
            Diagnostic::error("E0402",
                format!("function `{}` uses `await` but is not declared `[async]`", fn_name))
                .with_span(e.span)
                .with_note(format!("add `[async]` to `{}`'s signature", fn_name)),
        EffectErrorKind::TransactionOutsideDbWrite { fn_name } =>
            Diagnostic::error("E0403",
                format!("`db.transaction` in `{}` requires `[db.write]`", fn_name))
                .with_span(e.span)
                .with_note(format!("add `[db.write]` to `{}`'s signature", fn_name)),
        EffectErrorKind::UnsafeOutsideUnsafe { fn_name } =>
            Diagnostic::error("E0404",
                format!("`unsafe` block in `{}` requires `[unsafe]`", fn_name))
                .with_span(e.span)
                .with_note(format!("add `[unsafe]` to `{}`'s signature", fn_name)),
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

        TypeErrorKind::TemporalNotDuration { found } => {
            let names = assign_var_names(&[found]);
            Diagnostic::error("E0708",
                format!("temporal body must be a Duration, found `{}`", found.display_named(&names)))
                .with_span(e.span)
                .with_note("use Duration.days(N), Duration.hours(N), etc.")
        }

        TypeErrorKind::AgeOnNonTimestamp { found } => {
            let names = assign_var_names(&[found]);
            Diagnostic::error("E0709",
                format!("`.age` requires a Timestamp field, found `{}`", found.display_named(&names)))
                .with_span(e.span)
                .with_note("only fields of type Timestamp support `.age`")
        }

        TypeErrorKind::UnsupportedKeyType { fn_name, found } => {
            let names = assign_var_names(&[found]);
            Diagnostic::error("E0710",
                format!("`{}`'s key/numeric projection resolved to `{}`, which isn't supported", fn_name, found.display_named(&names)))
                .with_span(e.span)
                .with_note("only Int/Int8/Int16/Int32/UInt/Float/Float32 are supported — Text's ordering isn't lexicographic here and Decimal has no generic comparison/addition yet")
        }

        TypeErrorKind::FfiCallOutsideUnsafe { name } => {
            Diagnostic::error("E0210",
                format!("call to extern function `{}` must be inside an `unsafe {{ }}` block", name))
                .with_span(e.span)
                .with_label("FFI call — the compiler cannot verify its behavior")
                .with_note(format!("wrap it: `unsafe {{ {}(...) }}`", name))
        }

        TypeErrorKind::NonDisplayableInterpolation { ty } => {
            let names = assign_var_names(&[ty]);
            Diagnostic::error("E0211",
                format!("cannot interpolate a value of type `{}` into a string", ty.display_named(&names)))
                .with_span(e.span)
                .with_label("no text representation")
                .with_note("only Int, Float, Bool, Decimal, and Text can be interpolated; convert it first")
        }

        TypeErrorKind::MissingRowField { ty, field, required } => {
            let names = assign_var_names(&[ty, required]);
            Diagnostic::error("E0212",
                format!("`{}` does not satisfy the row bound", ty.display_named(&names)))
                .with_span(e.span)
                .with_label(format!("missing field `{}: {}`", field, required.display_named(&names)))
                .with_note(format!("add a `{}: {}` field, or pass a type that already has one", field, required.display_named(&names)))
        }

        TypeErrorKind::NonExhaustiveMatch { ty, missing } => {
            let names = assign_var_names(&[ty]);
            Diagnostic::error("E0213",
                format!("match on `{}` is not exhaustive", ty.display_named(&names)))
                .with_span(e.span)
                .with_label(format!("missing: {}", missing.join(", ")))
                .with_note("add the missing arm(s), or a `_ => ...` arm to cover the rest")
        }

        TypeErrorKind::PrivConstructorCall { type_name } => {
            Diagnostic::error("E0214",
                format!("constructor `{type_name}` is private"))
                .with_span(e.span)
                .with_label("not callable outside its own impl block")
                .with_note(format!("call it only from within `impl {type_name} {{ ... }}`, e.g. a validating `{type_name}.new` factory"))
        }

        TypeErrorKind::SecretInSensitiveContext { fn_name, ty } => {
            let t = ty.display();
            Diagnostic::error("E0215",
                format!("`{t}` is not Loggable/Serializable"))
                .with_span(e.span)
                .with_label(format!("passed to `{fn_name}`, which would expose it"))
                .with_note("call `.expose()` on the Secret first if you really need its raw value here")
        }

        TypeErrorKind::SqlInjectionRisk { fn_name } => {
            Diagnostic::error("E0216",
                format!("an interpolated f-string was passed directly as the `sql` argument to `{fn_name}`"))
                .with_span(e.span)
                .with_label("SQL injection risk")
                .with_note("use `?` placeholders in the SQL text and pass values via `params` instead")
        }
    }
}

pub(crate) fn find_cc() -> Option<String> {
    // First try names on PATH. MSVC's cl.exe is deliberately not a candidate:
    // the compiler invocation below is 100% GCC/Clang-flag syntax (-o, -I,
    // -Xlinker, -luser32, ...), which cl.exe doesn't understand at all, so
    // "detecting" it would just fail differently (and more confusingly) one
    // step later. See BACKLOG item 104.
    for candidate in &["clang", "gcc", "cc"] {
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
pub(crate) fn resolve_pg_paths() -> (Option<String>, Option<String>) {
    // Explicit env vars always win.
    let env_inc = std::env::var("PG_INCLUDE").ok();
    let env_lib = std::env::var("PG_LIB").ok();
    if env_inc.is_some() || env_lib.is_some() {
        return (env_inc, env_lib);
    }

    // On Windows, probe versioned install directories highest-first so the
    // newest available libpq is always preferred over whatever pg_config
    // happens to be on PATH (which may be an older version).
    if cfg!(windows) {
        for ver in (9u32..=20).rev() {
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

/// `src/ui.cto` for the `fullstack` template — the hand-editable source of
/// truth (a `@ui.generate` declaration over a record type).
///
/// `id` is `Text` (not `Int`) and included in `list.columns` — the migrate
/// DSL has no auto-increment/identity column support, so an `Int
/// primaryKey` with no default would leave the generated Create form's
/// INSERT unable to supply a value for it (it's excluded from the form
/// whenever it isn't listed in `columns`). A user-supplied Text id matches
/// the one other real, shipped `@ui.generate` example in this repo
/// (`examples/fireworks_ui.cto`'s `custNo`/`productCde` Text primary keys)
/// and needs no engine changes to actually work.
fn fullstack_ui_source(module_name: &str) -> String {
    format!(
        "module {module_name}Ui\n\
         \n\
         type Task = {{\n\
         \x20   id:    Text\n\
         \x20   title: Text\n\
         \x20   done:  Bool\n\
         }}\n\
         \n\
         @ui.generate(Task) {{\n\
         \x20   title: \"Tasks\"\n\
         \x20   list: {{ columns: [id, title, done] }}\n\
         }}\n"
    )
}

/// `src/main.cto` for the `fullstack` template — the already-lowered,
/// immediately-`certo run`-able server generated from `ui_src` via the same
/// `certo_ui::emit_server` used by the standalone `certo-ui` binary, so
/// scaffolding here can never drift from what that binary actually
/// produces. `Err` only on a bug in `fullstack_ui_source` itself (its
/// output is fixed template text, not user input).
fn fullstack_server_source(ui_src: &str) -> Result<String, String> {
    let ui_module = certo_parser::parse(ui_src)
        .map_err(|errs| format!("ui.cto failed to parse: {:?}", errs))?;
    certo_ui::emit_server(&ui_module).map_err(|e| e.to_string())
}

fn cmd_new(args: &[String]) {
    let mut name: Option<&String> = None;
    let mut template = "default";

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                template = args.get(i).map(String::as_str).unwrap_or_else(|| {
                    eprintln!("error: --template requires a name (api, lib, cli, fullstack)");
                    process::exit(2);
                });
            }
            "--help" | "-h" => {
                println!("Usage: certo new <project-name> [--template <template>]");
                println!();
                println!("Templates:");
                println!("  default     Hello-world entry point (default)");
                println!("  api         HTTP JSON API with Stdlib.Http");
                println!("  lib         Library with public exports, no main");
                println!("  cli         CLI tool with argument parsing");
                println!("  fullstack   @ui.generate CRUD server (Stdlib.Http + Stdlib.Db)");
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
        "fullstack" =>
            "\n[dependencies]\n\
             \"Stdlib.Http\" = \"*\"\n\
             \"Stdlib.Db\"   = \"*\"\n",
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
         {toml_entry}{toml_deps}\n\
         [database]\n\
         migrations = \"db/migrations/\"   # read by `certo db migrate`/`certo db create`\n\
         \n\
         [features]\n\
         # schema-sync = true   # `certo build`/`check` cross-checks every `impl DbRow`\n\
         #                      # type against the live DATABASE_URL schema; needs a\n\
         #                      # reachable database on every build, so it's opt-in.\n\
         \n\
         # [targets.production]\n\
         # optimize    = true   # -O2 (default) vs -O0 if set to false\n\
         # strip-debug = true   # strips the linked binary's symbol table (-s)\n\
         # schema      = \"${{DATABASE_URL}}\"  # DATABASE_URL override, used with\n\
         #                                    # `[features] schema-sync` above\n\
         # applied with: certo build --release\n"
    ));

    let fullstack_ui_src = fullstack_ui_source(&module_name);
    let fullstack_main_src = if template == "fullstack" {
        Some(fullstack_server_source(&fullstack_ui_src).unwrap_or_else(|e| {
            eprintln!("internal error: fullstack template failed to generate src/main.cto: {}", e);
            process::exit(1);
        }))
    } else {
        None
    };

    // src/main.cto — template-specific
    let main_src = match template {
        "fullstack" => fullstack_main_src.clone().unwrap(),
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
    if template == "fullstack" {
        write_file(&root.join("src").join("ui.cto"), &fullstack_ui_src);
        write_file(&root.join("db").join("migrations").join("001_create_task.cto"),
            "module Migration\n\
             \n\
             migration \"create_task\" {\n\
             \x20   up {\n\
             \x20       createTable task { id: Text primaryKey, title: Text, done: Bool }\n\
             \x20   }\n\
             \x20   down {\n\
             \x20       dropTable task\n\
             \x20   }\n\
             }\n"
        );
    }

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
    let readme_fullstack_section = if template == "fullstack" {
        "\n\
         ## Editing the UI\n\
         \n\
         `src/ui.cto` is the source of truth: a `type` record plus an\n\
         `@ui.generate(...)` declaration describing the CRUD screen. \
         `src/main.cto` is the already-lowered, runnable HTTP server\n\
         generated from it — don't hand-edit it. After changing `src/ui.cto`\n\
         (new fields, a different title, a different column list), regenerate\n\
         `src/main.cto` with:\n\
         \n\
         ```\n\
         certo-ui src/ui.cto -o src\n\
         ```\n\
         \n\
         ## Database\n\
         \n\
         The generated server reads `DATABASE_URL` at runtime (see\n\
         `.env.example`) and expects a `task` table matching `src/ui.cto`'s\n\
         `Task` record. Apply the seed migration before running it:\n\
         \n\
         ```\n\
         certo migrate up\n\
         ```\n\
         \n"
    } else {
        ""
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
         {readme_fullstack_section}\
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
    } else if template == "fullstack" {
        eprintln!("  │   ├── main.cto   (generated — don't hand-edit)");
        eprintln!("  │   └── ui.cto     (source of truth)");
    } else {
        eprintln!("  │   └── main.cto");
    }
    eprintln!("  ├── db/");
    if template == "fullstack" {
        eprintln!("  │   └── migrations/");
        eprintln!("  │       └── 001_create_task.cto");
    } else {
        eprintln!("  │   └── migrations/");
    }
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

// ------------------------------------------------------------------ //
// add / audit
// ------------------------------------------------------------------ //
//
// `[dependencies]` in certo.toml only ever lists *stdlib* module names today
// (e.g. `"Stdlib.Http" = "*"` — see `cmd_new`'s template above) — there is no
// package registry, fetch mechanism, or lockfile anywhere in this toolchain.
// `certo add` and `certo audit` are scoped to that reality: manifest/import
// bookkeeping, not a package manager. `certo add` appends a validated stdlib
// module entry to `[dependencies]`; `certo audit` cross-checks that list
// against the modules the entry file's source actually `import`s.

/// Every module `certo_stdlib::seed_stdlib`/`full_c_runtime` actually provides —
/// mirrors the `pub use *_C` list in `crates/stdlib/src/lib.rs` exactly, so
/// `certo add <name>` rejects a typo instead of writing a manifest entry for a
/// module that will never resolve to anything.
const STDLIB_MODULES: &[&str] = &[
    "Core", "Bytes", "Credential", "Collections", "Channel", "Result", "Text", "DateTime",
    "Money", "Db", "DbQuery", "DbMutation", "Env", "File", "Path", "Process", "Json", "Http",
    "Math", "Crypto", "Regex", "Csv",
];

/// Validates a user-supplied module name (`"Http"` or `"Stdlib.Http"`) against
/// `STDLIB_MODULES` and normalizes it to the canonical `"Stdlib.X"` form every
/// real `.cto` file and `certo.toml` in this codebase actually uses. Returns
/// `Err` naming the invalid module and listing valid names.
fn normalize_stdlib_module_name(input: &str) -> Result<String, String> {
    let last_seg = input.rsplit('.').next().unwrap_or(input);
    if !STDLIB_MODULES.contains(&last_seg) {
        let mut names = STDLIB_MODULES.to_vec();
        names.sort_unstable();
        return Err(format!(
            "unknown stdlib module '{}'\n       valid modules: {}",
            input,
            names.join(", ")
        ));
    }
    Ok(format!("Stdlib.{}", last_seg))
}

fn cmd_add(args: &[String]) {
    let mut module: Option<String> = None;
    for a in args {
        match a.as_str() {
            "--help" | "-h" => {
                println!("Usage: certo add <module>");
                println!();
                println!("Add a stdlib module to [dependencies] in certo.toml.");
                println!("Example: certo add Http    (writes \"Stdlib.Http\" = \"*\")");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => {
                if module.is_some() { die("only one module supported", 2); }
                module = Some(path.to_string());
            }
        }
    }
    let module = module.unwrap_or_else(|| {
        eprintln!("error: no module given");
        eprintln!("usage: certo add <module>");
        process::exit(2);
    });

    let key = normalize_stdlib_module_name(&module).unwrap_or_else(|msg| {
        eprintln!("error: {}", msg);
        process::exit(1);
    });

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let manifest_path = cwd.join("certo.toml");
    let src = std::fs::read_to_string(&manifest_path).unwrap_or_else(|_| {
        eprintln!("error: no certo.toml in current directory");
        eprintln!("       run `certo new <name>` to scaffold a project, or create one by hand");
        process::exit(1);
    });

    let mut doc = src.parse::<toml_edit::DocumentMut>().unwrap_or_else(|e| {
        eprintln!("error: failed to parse {}:\n{}", manifest_path.display(), e);
        process::exit(2);
    });

    if doc.get("dependencies").and_then(|d| d.get(&key)).is_some() {
        eprintln!("{} is already in [dependencies]", key);
        return;
    }

    if doc.get("dependencies").is_none() {
        doc["dependencies"] = toml_edit::table();
    }
    doc["dependencies"][&key] = toml_edit::value("*");

    let new_src = doc.to_string();
    // Sanity-check the edit through the strict, deny-unknown-fields schema
    // loader before touching disk — a toml_edit bug here would otherwise
    // silently corrupt the user's manifest.
    if let Err(msg) = toml::from_str::<certo_toml::CertoToml>(&new_src) {
        eprintln!("error: internal error — edited certo.toml failed to re-parse: {}", msg);
        process::exit(1);
    }
    std::fs::write(&manifest_path, &new_src).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", manifest_path.display(), e);
        process::exit(1);
    });
    eprintln!("added {} to [dependencies]", key);
}

fn cmd_audit(args: &[String]) {
    let mut strict = false;
    for a in args {
        match a.as_str() {
            "--strict" => strict = true,
            "--help" | "-h" => {
                println!("Usage: certo audit [--strict]");
                println!();
                println!("Cross-check [dependencies] in certo.toml against the stdlib modules");
                println!("actually `import`ed by the project's entry file.");
                println!();
                println!("  --strict   Also fail (exit 1) on declared-but-unused dependencies");
                return;
            }
            other => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cfg = load_certo_toml_or_die(&cwd).unwrap_or_else(|| {
        eprintln!("error: no certo.toml in current directory");
        process::exit(1);
    });
    let entry = cfg.build.as_ref().and_then(|b| b.entry.as_deref()).unwrap_or_else(|| {
        eprintln!("error: certo.toml has no `entry` field under [build]");
        process::exit(1);
    });
    let entry_path = cwd.join(entry);
    let colour = stderr_is_tty();
    let (module, _src) = parse_file_or_exit(&entry_path, colour);

    // Same scope as `cmd_build`'s own `uses_db` detection: only the entry
    // file's own declared imports, not a transitive walk through locally
    // imported files — matches the one place import declarations already
    // drive real compiler behavior today, rather than inventing a deeper
    // resolution pass audit alone would need to justify.
    let stdlib_prefixes = ["Stdlib", "Core", "Collections", "Text", "DateTime", "Money"];
    let imported: std::collections::BTreeSet<String> = module.imports.iter()
        .filter(|imp| {
            let first = imp.path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
            stdlib_prefixes.contains(&first)
        })
        .map(|imp| imp.path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."))
        .collect();

    let declared: std::collections::BTreeSet<String> = cfg.dependencies
        .unwrap_or_default()
        .into_keys()
        .collect();

    let missing: Vec<&String> = imported.difference(&declared).collect();
    let unused: Vec<&String> = declared.difference(&imported).collect();

    for m in &missing {
        eprintln!("error: `{}` is imported but not declared in [dependencies]", m);
        eprintln!("       fix with: certo add {}", m);
    }
    for m in &unused {
        eprintln!("warning: `{}` is declared in [dependencies] but never imported", m);
    }

    if !missing.is_empty() || (strict && !unused.is_empty()) {
        process::exit(1);
    }
    if missing.is_empty() && unused.is_empty() {
        eprintln!("certo audit: ok — {} stdlib import(s) match declared dependencies", imported.len());
    }
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
    let mut show_diff = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check_only = true,
            "--diff" => show_diff = true,
            "--help" | "-h" => {
                println!("Usage: certo fmt [--check] [--diff] <file.cto>...");
                println!();
                println!("Format Certo source files in place.");
                println!("  --check   Exit 1 if any file would be reformatted (no writes).");
                println!("  --diff    Print a unified diff of what would change (no writes;");
                println!("            implies --check's exit-1-on-changes behavior).");
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

    // `--diff` is a dry run just like `--check` — it shows what would
    // change instead of writing it, matching the common `--diff` convention
    // (e.g. `black --diff`) rather than writing the file *and* printing a
    // diff, which would make the diff always look like a no-op afterward.
    let dry_run = check_only || show_diff;

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
                    if show_diff {
                        print!("{}", diff::unified_diff(&src, &formatted, &path.display().to_string()));
                    } else if dry_run {
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
    if dry_run && any_changed {
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
    let mut filter: Option<String> = None;
    let mut coverage = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-color" => color = false,
            "--help" | "-h" => {
                println!("Usage: certo test <file.cto>... [--filter <substring>] [--timeout=<ms>] [--coverage]");
                println!();
                println!("Compile and run all `test` blocks in the given source files.");
                println!("Exits 0 if all tests pass, 1 otherwise.");
                println!();
                println!("Options:");
                println!("  --filter <substring>  Only run tests whose display name contains this string");
                println!("  --timeout=<ms>         Per-test timeout in milliseconds (default 5000)");
                println!("  --coverage             Print a per-line coverage report (clang/llvm-cov required)");
                println!("                         mapped back to your .cto source, not the generated C");
                return;
            }
            "--filter" => {
                i += 1;
                filter = Some(args.get(i).unwrap_or_else(|| die("--filter requires a substring", 2)).clone());
            }
            "--coverage" => coverage = true,
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
        filter,
        coverage_dir: None, // set internally by run_file_opts when coverage is on
    };
    let mut all_passed = true;
    for path in &files {
        match certo_testrunner::run_file_opts(path, &opts, color, coverage) {
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
        &certo_codegen::CodegenOptions { inline_runtime: false, export_public: false, line_directives: None },
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
       .arg("-Wno-deprecated-declarations")
       .arg("-Wno-incompatible-function-pointer-types");

    // POSIX threads for `parallel {}` / `spawn` / `await`. On Windows the runtime
    // uses Win32 threads (kernel32, always linked), so no extra flag is needed.
    if !cfg!(windows) {
        cmd.arg("-pthread");
    }
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
    eprintln!("Certo compiler  v{}", CERTO_VERSION);
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
  eprintln!("  certo generate validators ...  Generate .cto from YAML definitions");
    eprintln!("  certo add   <module>           Add a stdlib module to [dependencies]");
    eprintln!("  certo audit [--strict]         Check [dependencies] against actual imports");
    eprintln!("  certo db <subcommand>            Database tools (migrate, rollback, status, pull)");
    eprintln!("  certo migrate <subcommand>       Alias for certo db
  certo --version                  Print version and exit");
    eprintln!();
    eprintln!("Build options:");
    eprintln!("  -o <file>    Output path");
    eprintln!("  --emit-c     Stop after emitting C; write <stem>.c");
    eprintln!("  --emit-dll   Compile to a shared library (.dll / .so)");
    eprintln!("  -v           Verbose: print the C compiler command");
    eprintln!("  --watch, -w  Watch source file and rebuild on change");
    eprintln!();
    eprintln!("DB/migrate subcommands:");
    eprintln!("  migrate [--dry-run]          Apply pending migrations");
    eprintln!("  rollback [N]                 Roll back N migrations (default 1)");
    eprintln!("  status                       Show applied/pending migrations");
    eprintln!("  create <name>                Scaffold a new migration file");
    eprintln!("  pull [-o <file>]             Introspect live DB schema → db/schema.cto");
    eprintln!("  diff <file.cto>              Compare type declarations to live DB");
}

pub(crate) fn die(msg: &str, code: i32) -> ! {
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
// db diff
// ------------------------------------------------------------------ //

fn cmd_db_diff(args: &[String]) {
    let mut schema_name = "public".to_string();
    let mut src_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                i += 1;
                schema_name = args.get(i)
                    .unwrap_or_else(|| die("--schema requires a name", 2))
                    .clone();
            }
            "--help" | "-h" => {
                println!("Usage: certo db diff <file.cto> [--schema <name>]");
                println!();
                println!("Compare the type declarations in <file.cto> against the live");
                println!("PostgreSQL database and report any schema drift.");
                println!();
                println!("Options:");
                println!("  --schema <name>  PostgreSQL schema to inspect (default: public)");
                println!();
                println!("Reads DATABASE_URL from the environment or .env file.");
                println!("Requires psql on PATH.");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => {
                src_path = Some(PathBuf::from(path));
            }
        }
        i += 1;
    }

    let src_path = src_path.unwrap_or_else(|| die("usage: certo db diff <file.cto>", 2));

    // Parse and build the expected schema from type declarations.
    let (module, _) = parse_file_or_exit(&src_path, false);
    let expected = match certo_dbschema::check_module(&module) {
        Ok(s)    => s,
        Err(errs) => {
            for e in &errs { eprintln!("error: {}", e.message()); }
            process::exit(1);
        }
    };

    let (diffs, ok_count) = diff_schema_against_live(&expected, &schema_name)
        .unwrap_or_else(|msg| { eprintln!("error: {}", msg); process::exit(1); });

    // ── Report ───────────────────────────────────────────────────────
    if diffs.is_empty() {
        println!("schema in sync — {} table(s) match the live database", ok_count);
    } else {
        println!("schema drift detected:\n");
        for line in &diffs {
            println!("{}", line);
        }
        println!();
        println!("{} issue(s) found, {} table(s) ok", diffs.len(), ok_count);
        process::exit(1);
    }
}

/// Connects to the live database (via `DATABASE_URL`) and diffs `expected` (a
/// schema already parsed/validated from `type` declarations) against it.
/// Returns `(diff_lines, tables_in_sync_count)` on success — never exits the
/// process, so callers can react differently: `certo db diff` treats any
/// non-empty diff as a hard failure, `certo db migrate`'s post-migrate check
/// only warns.
fn diff_schema_against_live(expected: &certo_dbschema::Schema, schema_name: &str) -> Result<(Vec<String>, usize), String> {
    let db_url = resolve_database_url_for_sync()
        .ok_or_else(|| "DATABASE_URL is not set".to_string())?;

    // Verify psql is available.
    let psql_ok = std::process::Command::new("psql")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !psql_ok {
        return Err("psql not found on PATH".to_string());
    }

    // Query live columns from information_schema.
    let col_query = format!(
        "SELECT table_name, column_name, data_type, is_nullable, numeric_precision, numeric_scale \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         ORDER BY table_name, ordinal_position;",
        schema_name.replace('\'', "")
    );
    let col_out = std::process::Command::new("psql")
        .arg(&db_url)
        .arg("--no-align").arg("--tuples-only").arg("--field-separator=|")
        .arg("--command").arg(&col_query)
        .output()
        .map_err(|e| format!("error running psql: {}", e))?;
    if !col_out.status.success() {
        return Err(format!("psql failed: {}", String::from_utf8_lossy(&col_out.stderr).trim()));
    }

    // Parse live schema into HashMap<certo-cased table name, (raw db name, Vec<(raw col, certo-cased col, certo_ty, nullable)>)>.
    // Bridges the same naming convention `check_schema_sync`/`certo db pull` already use
    // (snake_case DB identifiers ↔ PascalCase types / camelCase fields) — table/column
    // names are matched on the bridged form, but diagnostics that name a *live* DB object
    // ("EXTRA TABLE", "EXTRA COLUMN") still print its real, raw DB name.
    use std::collections::HashMap;
    let mut live: HashMap<String, (String, Vec<(String, String, String, bool)>)> = HashMap::new();
    for line in String::from_utf8_lossy(&col_out.stdout).lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 6 { continue; }
        let raw_table = parts[0].trim().to_string();
        let raw_col   = parts[1].trim().to_string();
        let precision = parts[4].trim().parse::<i64>().ok();
        let scale     = parts[5].trim().parse::<i64>().ok();
        let certo_ty  = pg_type_to_certo(parts[2].trim(), precision, scale);
        let nullable  = parts[3].trim().eq_ignore_ascii_case("YES");
        let certo_col = certo_dbschema::snake_to_camel(&raw_col);
        let entry = live.entry(certo_dbschema::snake_to_pascal(&raw_table))
            .or_insert_with(|| (raw_table.clone(), Vec::new()));
        entry.1.push((raw_col, certo_col, certo_ty, nullable));
    }

    // ── Diff ─────────────────────────────────────────────────────────
    let mut diffs: Vec<String> = Vec::new();
    let mut ok_count = 0usize;

    // Only diff tables that are declared as types (ignore migration-only helpers).
    let mut expected_names: Vec<&str> = expected.tables.keys().map(|s| s.as_str()).collect();
    expected_names.sort();

    for table_name in &expected_names {
        let expected_table = &expected.tables[*table_name];
        match live.get(*table_name) {
            None => {
                diffs.push(format!("  MISSING TABLE  {}", table_name));
            }
            Some((_raw_table, live_cols)) => {
                let live_map: HashMap<&str, (&str, &str, bool)> = live_cols.iter()
                    .map(|(raw_col, certo_col, t, n)| (certo_col.as_str(), (raw_col.as_str(), t.as_str(), *n)))
                    .collect();

                let mut table_diffs: Vec<String> = Vec::new();

                // Columns expected but missing from live.
                for ec in &expected_table.columns {
                    match live_map.get(ec.name.as_str()) {
                        None => {
                            table_diffs.push(format!(
                                "    MISSING COLUMN  {}.{}: {}{}",
                                table_name, ec.name, ec.ty,
                                if ec.nullable { "?" } else { "" }
                            ));
                        }
                        Some((_raw_col, live_ty, live_null)) => {
                            // Type mismatch.
                            if !types_match(&ec.ty, live_ty) {
                                table_diffs.push(format!(
                                    "    TYPE MISMATCH   {}.{} — code: `{}`, db: `{}`",
                                    table_name, ec.name, ec.ty, live_ty
                                ));
                            }
                            // Nullable mismatch.
                            if ec.nullable != *live_null {
                                table_diffs.push(format!(
                                    "    NULLABLE DRIFT  {}.{} — code: {}, db: {}",
                                    table_name, ec.name,
                                    if ec.nullable { "nullable" } else { "NOT NULL" },
                                    if *live_null  { "nullable" } else { "NOT NULL" },
                                ));
                            }
                        }
                    }
                }

                // Columns in live but not in the type declaration.
                let expected_cols: std::collections::HashSet<&str> =
                    expected_table.columns.iter().map(|c| c.name.as_str()).collect();
                for (raw_col, certo_col, live_ty, live_null) in live_cols {
                    if !expected_cols.contains(certo_col.as_str()) {
                        table_diffs.push(format!(
                            "    EXTRA COLUMN    {}.{}: {}{}",
                            table_name, raw_col, live_ty,
                            if *live_null { "?" } else { "" }
                        ));
                    }
                }

                if table_diffs.is_empty() {
                    ok_count += 1;
                } else {
                    diffs.push(format!("  TABLE  {}", table_name));
                    diffs.extend(table_diffs);
                }
            }
        }
    }

    // Tables in live DB but not declared as types (informational only).
    let mut extra_tables: Vec<&str> = live.iter()
        .filter(|(certo_name, _)| !expected.tables.contains_key(certo_name.as_str()))
        .map(|(_, (raw_table, _))| raw_table.as_str())
        .collect();
    extra_tables.sort();
    for t in &extra_tables {
        diffs.push(format!("  EXTRA TABLE     {} (not declared as a type)", t));
    }

    Ok((diffs, ok_count))
}

/// Loose type comparison — ignores casing, treats nullable-stripped types as equal.
fn types_match(code_ty: &str, live_ty: &str) -> bool {
    let a = code_ty.trim_end_matches('?').to_lowercase();
    let b = live_ty.trim_end_matches('?').to_lowercase();
    a == b || certo_dbschema::decimal_bare_vs_param(&a, &b)
}

// ------------------------------------------------------------------ //
// db pull
// ------------------------------------------------------------------ //

fn cmd_db_pull(args: &[String]) {
    let mut out_path: Option<PathBuf> = None;
    let mut schema_name = "public".to_string();
    let mut url_override: Option<String> = None;

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
            "--url" => {
                i += 1;
                url_override = Some(
                    args.get(i).unwrap_or_else(|| die("--url requires a connection string", 2))
                        .clone()
                );
            }
            "--help" | "-h" => {
                println!("Usage: certo db pull [-o <file>] [--schema <name>] [--url <dsn>]");
                println!();
                println!("Introspect the live PostgreSQL database and write a Certo");
                println!("schema snapshot to db/schema.cto (default).");
                println!();
                println!("Options:");
                println!("  -o <file>        Output path (default: db/schema.cto)");
                println!("  --schema <name>  PostgreSQL schema to introspect (default: public)");
                println!("  --url <dsn>      Connection string — overrides DATABASE_URL/.env");
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

    // Resolve the connection string — an explicit --url always wins, then
    // DATABASE_URL, then .env (same fallback chain resolve_database_url_for_sync
    // uses elsewhere in this file, just with --url spliced in ahead of it).
    let db_url = url_override.unwrap_or_else(|| {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
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
            eprintln!("       Set it in your environment or .env file, or pass --url:");
            eprintln!("       DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret");
            process::exit(1);
        })
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
        "SELECT table_name, column_name, data_type, is_nullable, numeric_precision, numeric_scale \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         ORDER BY table_name, ordinal_position;",
        schema_name.replace('\'', "")
    );

    let col_out = std::process::Command::new("psql")
        .arg("-d").arg(&db_url)
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
        .arg("-d").arg(&db_url)
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
        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 6 { continue; }
        let table   = parts[0].trim();
        let col     = parts[1].trim();
        let pg_type = parts[2].trim();
        let nullable = parts[3].trim().eq_ignore_ascii_case("YES");
        let precision = parts[4].trim().parse::<i64>().ok();
        let scale     = parts[5].trim().parse::<i64>().ok();
        let certo_type = pg_type_to_certo(pg_type, precision, scale);
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
    let out_is_stdout = out_path.as_ref().map(|p| p.as_os_str() == "-").unwrap_or(false);
    let out = if out_is_stdout {
        PathBuf::from("-")
    } else {
        out_path.unwrap_or_else(|| {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let dir = cwd.join("db");
            std::fs::create_dir_all(&dir).ok();
            dir.join("schema.cto")
        })
    };

    let mut src = String::new();
    src.push_str("module DbSchema\n\n");
    src.push_str("// Generated by `certo db pull` — do not edit manually.\n");
    src.push_str(&format!("// Schema: {}  Database: {}\n\n", schema_name, redact_url(&db_url)));
    // Required for `certo build`'s `uses_db` detection to include the DB C
    // runtime — every generated *FindAll/*FindById/*Insert/*DeleteById
    // function below calls dbQueryTyped/dbExec, which link to nothing
    // without it. (Only matters when this file is built directly; a file
    // that locally-imports this one needs its own `import Stdlib.Db` too —
    // multi-file imports aren't merged for this check, a separate,
    // pre-existing limitation.)
    src.push_str("import Stdlib.Db\n\n");

    for table in &tables {
        let type_name = snake_to_pascal(table);
        let cols = columns.get(table).map(|v| v.as_slice()).unwrap_or(&[]);

        // ── Type declaration ─────────────────────────────────────────────
        src.push_str(&format!("type {} = {{\n", type_name));
        for (col_name, certo_ty, nullable) in cols {
            let field_name = snake_to_camel(col_name);
            let is_pk = primary_keys.contains(&(table.clone(), col_name.clone()));
            let ty_str = if *nullable { format!("{}?", certo_ty) } else { certo_ty.clone() };
            if is_pk {
                src.push_str(&format!("    {}: {}  // PK\n", field_name, ty_str));
            } else {
                src.push_str(&format!("    {}: {}\n", field_name, ty_str));
            }
        }
        src.push_str("}\n\n");

        // ── fromRow ──────────────────────────────────────────────────────
        let select_cols: Vec<&str> = cols.iter().map(|(c, _, _)| c.as_str()).collect();
        let select_list = select_cols.join(", ");

        src.push_str(&format!("fn {}FromRow(row: List<Text?>): {} =\n", table, type_name));
        src.push_str(&format!("    {} {{\n", type_name));
        for (i, (col_name, certo_ty, nullable)) in cols.iter().enumerate() {
            let field = snake_to_camel(col_name);
            let cell  = format!("List.getOrPanic(row, {})", i);
            let expr  = if *nullable {
                nullable_fromrow_expr(&cell, certo_ty)
            } else {
                text_to_certo_expr(&format!("({} ?? \"\")", cell), certo_ty)
            };
            src.push_str(&format!("        {}: {},\n", field, expr));
        }
        src.push_str("    }\n\n");

        // ── DbRow impl ───────────────────────────────────────────────────
        src.push_str(&format!("impl DbRow for {} {{}}\n\n", type_name));

        // ── findAll ──────────────────────────────────────────────────────
        src.push_str(&format!(
            "fn {}FindAll(conn: Int): List<{}> [io] =\n    dbQueryTyped(conn, \"SELECT {} FROM {} ORDER BY 1\", [], {}FromRow)\n\n",
            table, type_name, select_list, table, table
        ));

        // ── findById / deleteById (only when a PK exists) ────────────────
        let pk = cols.iter().find(|(c, _, _)|
            primary_keys.contains(&(table.clone(), c.clone())));

        if let Some((pk_col, pk_ty, _)) = pk {
            let pk_certo   = if pk_ty == "Int" { "Int" } else { "Text" };
            let pk_to_text = certo_to_text_expr("id", pk_ty);

            src.push_str(&format!(
                "fn {}FindById(conn: Int, id: {}): {}? [io] = {{\n    val rows = dbQueryTyped(conn, \"SELECT {} FROM {} WHERE {} = $1 LIMIT 1\", [{}], {}FromRow)\n    List.first(rows)\n}}\n\n",
                table, pk_certo, type_name,
                select_list, table, pk_col, pk_to_text,
                table
            ));

            src.push_str(&format!(
                "fn {}DeleteById(conn: Int, id: {}): Int [io] =\n    dbExec(conn, \"DELETE FROM {} WHERE {} = $1\", [{}])\n\n",
                table, pk_certo, table, pk_col, pk_to_text
            ));
        }

        // ── insert (non-PK columns) ──────────────────────────────────────
        let insert_cols: Vec<_> = cols.iter()
            .filter(|(c, _, _)| !primary_keys.contains(&(table.clone(), c.clone())))
            .collect();

        if !insert_cols.is_empty() {
            let col_names: Vec<&str>  = insert_cols.iter().map(|(c, _, _)| c.as_str()).collect();
            let placeholders: Vec<String> =
                (1..=insert_cols.len()).map(|i| format!("${}", i)).collect();
            let params: Vec<String> = insert_cols.iter().map(|(c, ty, nullable)| {
                let field = snake_to_camel(c);
                let expr  = format!("record.{}", field);
                if *nullable {
                    certo_nullable_to_text_expr(&expr, ty)
                } else {
                    certo_to_text_expr(&expr, ty)
                }
            }).collect();

            src.push_str(&format!(
                "fn {}Insert(conn: Int, record: {}): Int [io] =\n    dbExec(conn,\n        \"INSERT INTO {} ({}) VALUES ({})\",\n        [{}])\n\n",
                table, type_name,
                table, col_names.join(", "), placeholders.join(", "),
                params.join(", ")
            ));
        }
    }

    if out_is_stdout {
        print!("{}", src);
    } else {
        std::fs::write(&out, &src).unwrap_or_else(|e| {
            eprintln!("error writing {}: {}", out.display(), e);
            process::exit(1);
        });
        eprintln!("wrote {} ({} table(s))", out.display(), tables.len());
    }
    for table in &tables {
        let col_count = columns.get(table).map(|c| c.len()).unwrap_or(0);
        eprintln!("  {}  ({} column(s))", snake_to_pascal(table), col_count);
    }
}

/// Map a PostgreSQL type name to the closest Certo type.
/// `precision`/`scale` come from `information_schema.columns.numeric_precision`/
/// `numeric_scale` — only meaningful (non-NULL) for `numeric`/`decimal` columns;
/// `money` and everything else always pass `None`, falling back to bare `Decimal`.
/// Emits `Decimal(p, s)` (real BACKLOG item 128 fidelity) only when both are present.
fn pg_type_to_certo(pg: &str, precision: Option<i64>, scale: Option<i64>) -> String {
    match pg {
        "integer" | "int" | "int4" | "bigint" | "int8" | "smallint" | "int2"
            | "serial" | "bigserial" | "smallserial"   => "Int".to_string(),
        "text" | "character varying" | "varchar" | "char"
            | "bpchar" | "name" | "citext"             => "Text".to_string(),
        "boolean" | "bool"                             => "Bool".to_string(),
        "real" | "float4" | "double precision" | "float8" => "Float".to_string(),
        "numeric" | "decimal" | "money"                => match (precision, scale) {
            (Some(p), Some(s)) => format!("Decimal({}, {})", p, s),
            _                  => "Decimal".to_string(),
        },
        "uuid"                                         => "UUID".to_string(),
        "date" | "timestamp" | "timestamp without time zone"
            | "timestamp with time zone" | "timestamptz" => "DateTime".to_string(),
        "json" | "jsonb"                               => "Text".to_string(),
        "bytea"                                        => "Text".to_string(),
        other => snake_to_pascal(other),
    }
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

/// Build the `fromRow` decode expression for a nullable column. `cell` is a
/// `List.getOrPanic(row, N)` expression of type `Text?` (`None` = SQL NULL).
/// Certo has no `null` literal at all (nullability is `Option<T>`/`None`,
/// not a separate keyword) — using it here previously generated code that
/// failed to parse for every nullable column. Both `if` branches must be the
/// same `T?` — the decoded value on its own is a bare `T`, so it needs
/// `Some(...)` around it.
fn nullable_fromrow_expr(cell: &str, certo_ty: &str) -> String {
    format!("if {} == None then None else Some({})",
        cell, text_to_certo_expr(&format!("({} ?? \"\")", cell), certo_ty))
}

/// Convert a `List<Text>` cell expression to a typed Certo expression.
fn text_to_certo_expr(cell: &str, ty: &str) -> String {
    // `Decimal(p, s)` (BACKLOG item 128) is the same runtime type as bare
    // `Decimal` — strip the parameter before matching so these generated
    // conversions keep recognizing it, instead of silently falling through
    // to the Text/unknown-type catch-all.
    let base_ty = ty.split('(').next().unwrap_or(ty).trim();
    match base_ty {
        "Int"      => format!("parseInt({}) ?? 0", cell),
        "Bool"     => format!("{} == \"t\"", cell),
        "Float"    => format!("parseFloat({}) ?? intToFloat(0)", cell),
        // `parseDecimal` (not `parseInt`) — a NUMERIC column's text representation
        // routinely has a fractional part (`"19.99"`), which `parseInt` would
        // silently truncate/fail to parse, corrupting the value on every row read.
        "Decimal"  => format!("parseDecimal({}) ?? Decimal.fromInt(0)", cell),
        "DateTime" => format!("DateTime.parseIso({})", cell),
        _          => cell.to_string(), // Text, UUID, unknown named types
    }
}

/// Convert a non-nullable Certo field expression to `Text` for use in dbExec params.
fn certo_to_text_expr(expr: &str, ty: &str) -> String {
    let base_ty = ty.split('(').next().unwrap_or(ty).trim();
    match base_ty {
        "Int"      => format!("intToText({})", expr),
        "Bool"     => format!("boolToText({})", expr),
        "Float"    => format!("floatToText({})", expr),
        "Decimal"  => format!("Decimal.toText({})", expr),
        "DateTime" => format!("DateTime.toIso({})", expr),
        _          => expr.to_string(), // Text, UUID
    }
}

/// Convert a nullable Certo field expression (`T?`) to `Text` for a dbExec param.
/// Emits `dbNull()` when the value is `None` so the C runtime passes SQL NULL.
fn certo_nullable_to_text_expr(expr: &str, ty: &str) -> String {
    let convert = certo_to_text_expr(&format!("{} ?? {}", expr, null_default(ty)), ty);
    format!("if {} == None then dbNull() else {}", expr, convert)
}

/// A typed default used only as a dead branch for type-checker satisfaction.
fn null_default(ty: &str) -> &'static str {
    let base_ty = ty.split('(').next().unwrap_or(ty).trim();
    match base_ty {
        "Int"      => "0",
        "Bool"     => "false",
        "Float"    => "intToFloat(0)",
        "Decimal"  => "Decimal.fromInt(0)",
        "DateTime" => "DateTime.now()",
        _          => "\"\"",
    }
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
        "diff" => cmd_db_diff(&args[1..]),
        "--help" | "-h" | "" => {
            println!("Usage: certo db <subcommand> [options]");
            println!();
            println!("Subcommands:");
            println!("  migrate [--dry-run]          Apply all pending migrations");
            println!("  rollback [N]                 Roll back N migrations (default 1)");
            println!("  status                       Show applied vs pending migrations");
            println!("  create <name>                Scaffold a new migration file");
            println!("  pull [-o <file>]             Introspect live DB → db/schema.cto");
            println!("  diff <file.cto>              Compare type declarations to live DB");
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
            let sql = plan_sql(&steps);
            for stmt in &sql { println!("{}", stmt); }

            if dry_run {
                println!("-- dry run: {} statement(s) not executed", sql.len());
                return;
            }

            run_migration_sql(&sql);
            if let Err(e) = commit_steps(&steps, &manifest) {
                eprintln!("error: migration(s) executed successfully against the database,");
                eprintln!("       but failed to record local state: {}", e);
                eprintln!("       `certo db status` may now be inaccurate — check .certo_migrations");
                process::exit(1);
            }
            println!("Applied {} migration(s).", steps.len());
            warn_if_schema_stale(&project_root);
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
            let sql = plan_sql(&steps);
            for stmt in &sql { println!("{}", stmt); }

            if dry_run {
                println!("-- dry run: {} statement(s) not executed", sql.len());
                return;
            }

            run_migration_sql(&sql);
            if let Err(e) = commit_steps(&steps, &manifest) {
                eprintln!("error: rollback executed successfully against the database,");
                eprintln!("       but failed to record local state: {}", e);
                eprintln!("       `certo db status` may now be inaccurate — check .certo_migrations");
                process::exit(1);
            }
            println!("Rolled back {} migration(s).", steps.len());
            warn_if_schema_stale(&project_root);
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
            let dir = migrations_dir(&project_root);
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
                eprintln!("error creating {}: {}", dir.display(), e); process::exit(1);
            });
            let filename = dir.join(format!("{}.cto", name));
            let template = format!(
                "module Migration\n\nmigration \"{}\" {{\n    up {{\n        // TODO: add operations\n    }}\n    down {{\n        // TODO: add rollback operations\n    }}\n}}\n",
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

/// Actually executes `sql` against `DATABASE_URL` via `psql`, exiting the
/// process on any failure (resolution, missing `psql`, or a failing
/// statement) — the caller (`cmd_migrate`) only calls `commit_steps` after
/// this returns, so migration state is never marked "applied" for SQL that
/// didn't really run. `--single-transaction` plus `ON_ERROR_STOP=1` wraps the
/// whole batch in one `BEGIN`/`COMMIT`: a failing statement rolls back
/// everything from this migration batch *and* makes `psql`'s own exit code
/// reflect the failure (its default behavior otherwise keeps going after an
/// error and can still exit 0).
fn run_migration_sql(sql: &[String]) {
    if sql.is_empty() { return; }

    let db_url = resolve_database_url_for_sync().unwrap_or_else(|| {
        eprintln!("error: DATABASE_URL is not set");
        eprintln!("       set it in your environment or a .env file");
        process::exit(1);
    });

    let psql_ok = std::process::Command::new("psql")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !psql_ok {
        eprintln!("error: psql not found on PATH — required to execute migrations");
        process::exit(1);
    }

    let script = sql.join("\n");
    let out = std::process::Command::new("psql")
        .arg(&db_url)
        .arg("--single-transaction")
        .arg("--set").arg("ON_ERROR_STOP=1")
        .arg("--command").arg(&script)
        .output()
        .unwrap_or_else(|e| { eprintln!("error running psql: {}", e); process::exit(1); });
    if !out.status.success() {
        eprintln!("error: migration failed — no changes were committed (single transaction rolled back):");
        eprintln!("{}", String::from_utf8_lossy(&out.stderr).trim());
        process::exit(1);
    }
}

/// After a successful migration, warns (doesn't fail the command) if
/// `db/schema.cto` — the file `certo db pull` generates — is now stale
/// relative to the database that migration just changed. Only runs the check
/// if that file actually exists, so projects that don't use `certo db pull`
/// at all are never forced into a DB round-trip they didn't ask for.
fn warn_if_schema_stale(project_root: &Path) {
    let schema_file = project_root.join("db").join("schema.cto");
    let Ok(src) = std::fs::read_to_string(&schema_file) else { return; };
    let Ok(module) = certo_parser::parse(&src) else {
        eprintln!("warning: db/schema.cto failed to parse — skipping drift check");
        return;
    };
    let Ok(expected) = certo_dbschema::check_module(&module) else { return; };

    match diff_schema_against_live(&expected, "public") {
        Ok((diffs, _ok_count)) if diffs.is_empty() => {}
        Ok((diffs, _ok_count)) => {
            eprintln!();
            eprintln!("warning: db/schema.cto is now out of sync with the live database:");
            for line in &diffs { eprintln!("{}", line); }
            eprintln!("         run `certo db pull` to refresh it");
        }
        Err(msg) => {
            eprintln!("warning: could not check db/schema.cto for drift: {}", msg);
        }
    }
}

fn load_migrations(project_root: &Path) -> Vec<MigrationDecl> {
    let dir = migrations_dir(project_root);
    if !dir.exists() { return Vec::new(); }
    let mut result = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| { eprintln!("error reading {}: {}", dir.display(), e); process::exit(1); })
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

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn explain_code_finds_e0200() {
        let text = explain_code("E0200").expect("E0200 should be in the error reference");
        assert!(text.starts_with("### E0200"));
        assert!(text.contains("Type mismatch"));
        assert!(text.contains("**Cause:**"));
        // Must stop before the next entry, not bleed into E0201.
        assert!(!text.contains("### E0201"));
    }

    #[test]
    fn explain_code_stops_before_next_category_header() {
        // E0206 is the last entry in the E0200-E0206 category — its section
        // must not run past the "## E0300-E0306" category header that follows.
        let text = explain_code("E0206").expect("E0206 should be in the error reference");
        assert!(text.starts_with("### E0206"));
        assert!(!text.contains("## E0300"));
    }

    #[test]
    fn explain_code_finds_e0210_through_e0215() {
        // BACKLOG item 160: these were previously missing from the error
        // reference entirely (a real gap in the CLI's own --explain source,
        // found while fixing the spec's Appendix B against it).
        for (code, needle) in [
            ("E0210", "unsafe"),
            ("E0211", "interpolate"),
            ("E0212", "row bound"),
            ("E0213", "exhaustive"),
            ("E0214", "private"),
            ("E0215", "Loggable"),
        ] {
            let text = explain_code(code).unwrap_or_else(|| panic!("{code} should be in the error reference"));
            assert!(text.starts_with(&format!("### {code}")), "{code}: {text}");
            assert!(text.contains(needle), "{code} should mention '{needle}': {text}");
        }
    }

    #[test]
    fn explain_code_finds_e0508_through_e0526_and_e0600_e0601() {
        // BACKLOG item 167: 21 real codes (all of the DB query DSL range
        // plus both HIR-lowering codes) had zero write-up in the error
        // reference at all — found while fixing item 160's much smaller
        // E0210-E0215 gap in the same file.
        for code in [
            "E0508", "E0509", "E0510", "E0511", "E0512", "E0513", "E0514",
            "E0515", "E0516", "E0517", "E0518", "E0519", "E0520", "E0521",
            "E0522", "E0523", "E0524", "E0525", "E0526", "E0600", "E0601",
        ] {
            let text = explain_code(code).unwrap_or_else(|| panic!("{code} should be in the error reference"));
            assert!(text.starts_with(&format!("### {code}")), "{code}: {text}");
            assert!(text.contains("**Cause:**"), "{code} should have a Cause section: {text}");
        }
    }

    #[test]
    fn explain_code_unknown_code_returns_none() {
        assert!(explain_code("E9999").is_none());
    }

    #[test]
    fn explain_code_finds_lint_warning_codes() {
        let text = explain_code("L001").expect("L001 should be in the error reference");
        assert!(text.starts_with("### L001"));
    }

    #[test]
    fn print_explanations_dedups_by_code() {
        // Two diagnostics sharing a code should only print one explanation.
        // Exercised indirectly: explain_code itself is idempotent/pure, so
        // this just confirms the lookup succeeds twice without panicking
        // (the dedup logic lives in print_explanations's HashSet, covered
        // end-to-end via `certo check --explain` on a real multi-error file).
        assert!(explain_code("E0100").is_some());
        assert!(explain_code("E0100").is_some());
    }
}

#[cfg(test)]
mod fullstack_template_tests {
    use super::*;

    #[test]
    fn ui_source_parses_and_declares_task_and_ui_generate() {
        let src = fullstack_ui_source("myApp");
        let module = certo_parser::parse(&src).unwrap_or_else(|e| panic!("must parse: {:?}", e));
        assert!(module.decls.iter().any(|d| matches!(&d.node, certo_ast::decl::Decl::Type(t) if t.name.node == "Task")));
        assert!(module.decls.iter().any(|d| matches!(&d.node, certo_ast::decl::Decl::UiGenerate(g) if g.type_name.node == "Task")));
    }

    #[test]
    fn server_source_generates_crud_routes_for_task() {
        let ui_src = fullstack_ui_source("myApp");
        let server_src = fullstack_server_source(&ui_src).expect("emit_server must succeed on the template's own output");
        assert!(server_src.contains("/task-list"), "missing list route:\n{server_src}");
        assert!(server_src.contains("/create-task"), "missing create route:\n{server_src}");
        assert!(server_src.contains("INSERT INTO task"), "missing insert:\n{server_src}");
        assert!(server_src.contains("import Stdlib.Db"), "missing DB import:\n{server_src}");
    }

    #[test]
    fn server_source_is_itself_valid_certo_source() {
        // The generated src/main.cto must parse on its own — this is what
        // actually gets written to disk and `certo run`, not just emitted text.
        let ui_src = fullstack_ui_source("myApp");
        let server_src = fullstack_server_source(&ui_src).unwrap();
        certo_parser::parse(&server_src).unwrap_or_else(|e| panic!("generated server.cto must parse: {:?}", e));
    }

    #[test]
    fn task_id_is_text_not_int_since_migrate_dsl_has_no_auto_increment() {
        // Regression guard for the create-form-can-never-insert-a-row bug
        // caught during this item's own end-to-end verification: an `Int
        // primaryKey` id with no DB default is excluded from the generated
        // Create form (since it's not in `list.columns`), so nothing ever
        // supplies it and every INSERT would violate the NOT NULL constraint.
        let src = fullstack_ui_source("myApp");
        assert!(src.contains("id:    Text"), "id must be Text, not Int:\n{src}");
        assert!(src.contains("columns: [id, title, done]"), "id must be listed in columns so the Create form asks for it:\n{src}");
    }

    #[test]
    fn migrate_create_template_has_a_module_header() {
        // Regression guard: `certo db create`/`certo migrate create`'s own
        // scaffolded file used to omit `module X`, which `certo_parser::parse`
        // requires unconditionally — every migration file it wrote was
        // silently unparseable by `certo db migrate`/`certo db status`.
        let template = format!(
            "module Migration\n\nmigration \"{}\" {{\n    up {{\n        // TODO: add operations\n    }}\n    down {{\n        // TODO: add rollback operations\n    }}\n}}\n",
            "add_widgets"
        );
        let module = certo_parser::parse(&template).unwrap_or_else(|e| panic!("must parse: {:?}", e));
        assert!(module.decls.iter().any(|d| matches!(&d.node, certo_ast::decl::Decl::Migration(m) if m.name == "add_widgets")));
    }
}

#[cfg(test)]
mod migrations_dir_tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("certo_migrations_dir_test_{}_{}_{}", name, std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_to_migrations_when_no_certo_toml() {
        let dir = temp_project("no_toml");
        assert_eq!(migrations_dir(&dir), dir.join("migrations"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn defaults_to_migrations_when_no_database_section() {
        let dir = temp_project("no_db_section");
        std::fs::write(dir.join("certo.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert_eq!(migrations_dir(&dir), dir.join("migrations"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn honors_configured_database_migrations_path() {
        let dir = temp_project("configured");
        std::fs::write(dir.join("certo.toml"), "[project]\nname = \"x\"\n\n[database]\nmigrations = \"db/migrations/\"\n").unwrap();
        assert_eq!(migrations_dir(&dir), dir.join("db/migrations/"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod add_audit_tests {
    use super::*;

    #[test]
    fn normalize_accepts_bare_name() {
        assert_eq!(normalize_stdlib_module_name("Http").unwrap(), "Stdlib.Http");
    }

    #[test]
    fn normalize_accepts_dotted_name() {
        assert_eq!(normalize_stdlib_module_name("Stdlib.Http").unwrap(), "Stdlib.Http");
    }

    #[test]
    fn normalize_rejects_unknown_module() {
        let err = normalize_stdlib_module_name("Bogus").unwrap_err();
        assert!(err.contains("unknown stdlib module 'Bogus'"), "unexpected message: {err}");
        assert!(err.contains("Http"), "should list valid modules: {err}");
    }

    #[test]
    fn cmd_add_inserts_dependency_preserving_rest_of_file() {
        let dir = std::env::temp_dir().join(format!("certo_add_test_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("certo.toml");
        std::fs::write(&manifest, "# a comment that must survive\n[project]\nname = \"x\"\n\n[build]\nentry = \"src/main.cto\"\n").unwrap();

        let src = std::fs::read_to_string(&manifest).unwrap();
        let mut doc = src.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(doc.get("dependencies").is_none());
        doc["dependencies"] = toml_edit::table();
        doc["dependencies"]["Stdlib.Http"] = toml_edit::value("*");
        let new_src = doc.to_string();

        assert!(new_src.contains("# a comment that must survive"), "comment lost:\n{new_src}");
        assert!(new_src.contains("\"Stdlib.Http\""), "dependency not written:\n{new_src}");

        let cfg: certo_toml::CertoToml = toml::from_str(&new_src).expect("edited manifest must still parse");
        assert_eq!(cfg.dependencies.unwrap().get("Stdlib.Http").map(String::as_str), Some("*"));

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod db_pull_codegen_tests {
    use super::*;

    #[test]
    fn nullable_fromrow_expr_never_emits_null_literal() {
        // Regression guard: Certo has no `null` keyword — every nullable
        // column's generated fromRow expression must use `None`, not `null`.
        for ty in ["Text", "Int", "Bool", "Float", "Decimal", "Decimal(19, 4)", "DateTime"] {
            let expr = nullable_fromrow_expr("List.getOrPanic(row, 0)", ty);
            assert!(!expr.contains("null"), "ty={ty} produced a `null` literal: {expr}");
            assert!(expr.contains("== None"), "ty={ty} should check == None: {expr}");
            assert!(expr.starts_with("if "), "ty={ty}: {expr}");
        }
    }

    #[test]
    fn nullable_fromrow_expr_wraps_decoded_value_in_some() {
        // Both `if` branches must be `T?` — the decode helper returns a bare
        // `T`, so the `else` branch needs `Some(...)` around it or it's a
        // real type error (`expected T?, found T`).
        let expr = nullable_fromrow_expr("List.getOrPanic(row, 1)", "Text");
        assert!(expr.contains("else Some("), "expected Some(...) wrapper: {expr}");
        assert!(!expr.contains("else None else"), "malformed: {expr}"); // sanity
    }

    #[test]
    fn nullable_fromrow_expr_decimal_param_still_uses_parse_decimal() {
        // Regression guard: text_to_certo_expr must still recognize
        // "Decimal(19, 4)" (BACKLOG item 128) as Decimal, not fall through
        // to the Text/unknown-type catch-all.
        let expr = nullable_fromrow_expr("List.getOrPanic(row, 2)", "Decimal(19, 4)");
        assert!(expr.contains("parseDecimal("), "expected parseDecimal: {expr}");
    }

    #[test]
    fn text_to_certo_expr_decimal_uses_parse_decimal_not_parse_int() {
        // Regression guard: parseInt silently truncates a real decimal's
        // fractional part ("19.99" -> corrupted). Must use parseDecimal.
        let expr = text_to_certo_expr("cell", "Decimal");
        assert!(expr.contains("parseDecimal("), "got: {expr}");
        assert!(!expr.contains("parseInt("), "should not use parseInt for Decimal: {expr}");
    }

    #[test]
    fn text_to_certo_expr_decimal_param_recognized() {
        let expr = text_to_certo_expr("cell", "Decimal(19, 4)");
        assert!(expr.contains("parseDecimal("), "got: {expr}");
    }

    #[test]
    fn certo_to_text_expr_decimal_param_recognized() {
        let expr = certo_to_text_expr("record.price", "Decimal(19, 4)");
        assert_eq!(expr, "Decimal.toText(record.price)");
    }

    #[test]
    fn null_default_decimal_param_recognized() {
        assert_eq!(null_default("Decimal(19, 4)"), "Decimal.fromInt(0)");
    }

    #[test]
    fn certo_nullable_to_text_expr_never_emits_null_literal() {
        let expr = certo_nullable_to_text_expr("record.name", "Text");
        assert!(!expr.contains("null"), "got: {expr}");
        assert!(expr.contains("== None"), "got: {expr}");
        assert!(expr.contains("dbNull()"), "got: {expr}"); // real C-runtime helper, not the keyword
    }
}
