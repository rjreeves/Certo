//! `certo sdl` — schema definition language tools.
//!
//!   certo sdl check   <file.sdl>              validate; exit 1 on errors
//!   certo sdl compile <file.sdl> [-o out.json] emit SchemaIR as JSON (stdout by default)
//!   certo sdl diff <old.sdl> <new.sdl> [--mdl <file.mdl>] [--json | --sql <dialect>] [--deny-destructive]
//!                                             show the migration plan between two schemas

use certo_diagnostics::{render_all, Severity};
use certo_sdl::compile;
use std::path::PathBuf;
use std::process;

use crate::{die, stderr_is_tty};

const USAGE: &str = "\
Usage: certo sdl <subcommand> <file.sdl> [options]

Subcommands:
  check    Parse and validate a schema; exits 0 on success, 1 on errors
  compile  Validate and emit the canonical SchemaIR as JSON
  diff     Plan the migration from <old.sdl> to <new.sdl>
  migrate  Project-based migrations: init | new | apply | status (see `certo sdl migrate --help`)

Options (compile):
  -o <file>   Write JSON to <file> instead of stdout

Options (diff):
  --mdl <file>         Apply an MDL migration (renames, remaps, backfills, data steps)
  --json               Print the plan as JSON instead of a summary
  --sql <dialect>      Print SQL instead of a summary (dialects: postgres, sqlite)
  --deny-destructive   Exit 1 if the plan can lose data
";

pub fn cmd_sdl(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("check") => run(&args[1..], false),
        Some("compile") => run(&args[1..], true),
        Some("diff") => run_diff(&args[1..]),
        Some("migrate") => crate::cmd_sdl_migrate::cmd_migrate(&args[1..]),
        Some("--help" | "-h" | "help") => print!("{USAGE}"),
        Some(other) => {
            eprintln!("Unknown sdl subcommand: {other}\n");
            eprint!("{USAGE}");
            process::exit(2);
        }
        None => {
            eprint!("{USAGE}");
            process::exit(2);
        }
    }
}

fn run(args: &[String], emit: bool) {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if emit => {
                i += 1;
                match args.get(i) {
                    Some(p) => output = Some(PathBuf::from(p)),
                    None => die("-o requires a path", 2),
                }
            }
            "--help" | "-h" => { print!("{USAGE}"); return; }
            o if o.starts_with('-') => die(&format!("unknown option: {o}"), 2),
            p => {
                if input.is_some() { die("only one input file supported", 2); }
                input = Some(PathBuf::from(p));
            }
        }
        i += 1;
    }
    let input = input.unwrap_or_else(|| die("no input file (usage: certo sdl check <file.sdl>)", 2));
    let src = std::fs::read_to_string(&input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", input.display()), 2));

    let (ir, diags) = compile(&src);
    let filename = input.display().to_string();
    eprint!("{}", render_all(&diags, &src, &filename, stderr_is_tty()));

    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
    let Some(ir) = ir else {
        eprintln!("{filename}: {errors} error(s), {warnings} warning(s)");
        process::exit(1);
    };

    if !emit {
        eprintln!("{filename}: ok ({} table(s), {} enum(s), {} type(s), {warnings} warning(s))",
            ir.tables.len(), ir.enums.len(), ir.types.len());
        return;
    }
    let json = ir.to_json();
    match output {
        Some(p) => std::fs::write(&p, json + "\n")
            .unwrap_or_else(|e| die(&format!("cannot write {}: {e}", p.display()), 2)),
        None => println!("{json}"),
    }
}

/// Load, compile and report one schema file; exits 1 if it has errors.
fn load(path: &PathBuf) -> certo_sdl::SchemaIR {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", path.display()), 2));
    let (ir, diags) = compile(&src);
    let filename = path.display().to_string();
    eprint!("{}", render_all(&diags, &src, &filename, stderr_is_tty()));
    ir.unwrap_or_else(|| {
        eprintln!("{filename}: schema has errors");
        process::exit(1);
    })
}

fn run_diff(args: &[String]) {
    let mut files: Vec<PathBuf> = Vec::new();
    let (mut json, mut deny) = (false, false);
    let mut dialect: Option<certo_sql::Dialect> = None;
    let mut mdl: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--mdl" => {
                let p = it.next().unwrap_or_else(|| die("--mdl requires a file", 2));
                mdl = Some(PathBuf::from(p));
            }
            "--sql" => {
                let name = it.next().unwrap_or_else(|| die("--sql requires a dialect (postgres, sqlite)", 2));
                dialect = Some(certo_sql::Dialect::from_name(name)
                    .unwrap_or_else(|| die(&format!("unknown SQL dialect `{name}` (supported: postgres, sqlite)"), 2)));
            }
            "--deny-destructive" => deny = true,
            "--help" | "-h" => { print!("{USAGE}"); return; }
            o if o.starts_with('-') => die(&format!("unknown option: {o}"), 2),
            p => files.push(PathBuf::from(p)),
        }
    }
    if files.len() != 2 {
        die("usage: certo sdl diff <old.sdl> <new.sdl> [--mdl <file.mdl>] [--json | --sql <dialect>] [--deny-destructive]", 2);
    }
    let (old, new) = (load(&files[0]), load(&files[1]));
    let plan = match &mdl {
        None => certo_mdl::diff(&old, &new),
        Some(path) => {
            let src = std::fs::read_to_string(path)
                .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", path.display()), 2));
            let (plan, diags) = certo_mdl::compile_migration(&old, &new, &src);
            eprint!("{}", render_all(&diags, &src, &path.display().to_string(), stderr_is_tty()));
            plan.unwrap_or_else(|| {
                eprintln!("{}: migration has errors", path.display());
                process::exit(1);
            })
        }
    };

    if json && dialect.is_some() {
        die("--json and --sql are mutually exclusive", 2);
    }
    if let Some(d) = dialect {
        match certo_sql::render_with(&plan, d, certo_sql::Schemas { old: &old, new: &new }) {
            Ok(sql) => if !sql.is_empty() { println!("{sql}") },
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
    } else if json {
        println!("{}", plan.to_json());
    } else if plan.is_empty() {
        eprintln!("no changes");
    } else {
        for op in &plan.ops {
            let mark = if op.is_destructive() { "  (destructive)" } else { "" };
            println!("{}{mark}", op.describe());
        }
    }
    if deny && plan.has_destructive() {
        eprintln!("error: plan contains destructive operations");
        process::exit(1);
    }
}
