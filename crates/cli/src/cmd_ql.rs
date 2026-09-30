//! `certo ql` — typed queries against a schema.
//!
//!   certo ql check   <file.ql> --schema <schema.sdl|IR.json>
//!   certo ql compile <file.ql> --schema <schema.sdl|IR.json> [--json]

use certo_diagnostics::{render_all, Severity};
use certo_sdl::{describe_type, SchemaIR};
use certo_sql::Dialect;
use std::path::PathBuf;
use std::process;

use crate::{die, stderr_is_tty};

const USAGE: &str = "\
Usage: certo ql <subcommand> <file.ql> --schema <schema> [options]

Subcommands:
  check     Type-check the queries and show each one's parameters and typed result columns
  compile   Print the SQL for each query (with its parameter placeholders)

Options:
  --schema <file>   the schema: a .sdl file, or an IR.json
  --json            (compile) machine-readable output: params, columns, sql, param_order, ir
";

pub fn cmd_ql(args: &[String]) {
    let Some(sub) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        process::exit(2);
    };
    if matches!(sub, "--help" | "-h" | "help") {
        print!("{USAGE}");
        return;
    }
    if !matches!(sub, "check" | "compile") {
        eprintln!("Unknown ql subcommand: {sub}\n");
        eprint!("{USAGE}");
        process::exit(2);
    }
    let (mut file, mut schema, mut json) = (None::<PathBuf>, None::<PathBuf>, false);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--schema" => schema = Some(PathBuf::from(it.next().unwrap_or_else(|| die("--schema requires a file", 2)))),
            "--json" => json = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                return;
            }
            f if f.starts_with('-') => die(&format!("unknown option: {f}"), 2),
            p => {
                if file.is_some() { die("only one query file supported", 2); }
                file = Some(PathBuf::from(p));
            }
        }
    }
    let file = file.unwrap_or_else(|| die("no query file (usage: certo ql check <file.ql> --schema <schema.sdl>)", 2));
    let schema_path = schema.unwrap_or_else(|| die("--schema is required", 2));
    let schema = load_schema(&schema_path);

    let src = std::fs::read_to_string(&file).unwrap_or_else(|e| die(&format!("cannot read {}: {e}", file.display()), 2));
    let name = file.display().to_string();
    let (queries, diags) = certo_ql::compile(&schema, &src, Dialect::Postgres);
    eprint!("{}", render_all(&diags, &src, &name, stderr_is_tty()));
    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let Some(queries) = queries else {
        eprintln!("{name}: {errors} error(s)");
        process::exit(1);
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&certo_ql::to_json(&queries)).expect("json"));
        return;
    }
    let ty = |t: &certo_sdl::TypeIR, nullable: bool| format!("{}{}", describe_type(t), if nullable { "?" } else { "" });
    for q in &queries {
        if sub == "check" {
            let params: Vec<String> = q.params().iter().map(|p| format!("{}: {}", p.name, ty(&p.ty, p.nullable))).collect();
            let kind = match q {
                certo_ql::Statement::Query(_) => "query",
                certo_ql::Statement::Mutation(m) => match m.ir.kind {
                    certo_ql::MutationKind::Insert => "insert",
                    certo_ql::MutationKind::Update => "update",
                    certo_ql::MutationKind::Delete => "delete",
                },
            };
            println!("{kind} {}({})", q.name(), params.join(", "));
            let cols: Vec<String> = q.columns().iter().map(|c| format!("{} {}", c.name, ty(&c.ty, c.nullable))).collect();
            if !cols.is_empty() {
                println!("  -> {}", cols.join(", "));
            } else if q.as_mutation().is_some() {
                println!("  -> row count");
            }
        } else {
            let binds: Vec<String> = q.param_order().iter().enumerate().map(|(i, p)| format!("${}={p}", i + 1)).collect();
            println!("-- {}{}", q.name(), if binds.is_empty() { String::new() } else { format!("  ({})", binds.join(", ")) });
            println!("{};\n", q.sql());
        }
    }
    if sub == "check" {
        eprintln!("{name}: ok ({} statement(s))", queries.len());
    }
}

fn load_schema(path: &PathBuf) -> SchemaIR {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("cannot read {}: {e}", path.display()), 2));
    if path.extension().is_some_and(|e| e == "json") {
        return SchemaIR::from_json(&text).unwrap_or_else(|e| die(&format!("{}: invalid IR: {e}", path.display()), 2));
    }
    let (ir, diags) = certo_sdl::compile(&text);
    eprint!("{}", render_all(&diags, &text, &path.display().to_string(), stderr_is_tty()));
    ir.unwrap_or_else(|| die(&format!("{}: the schema has errors", path.display()), 1))
}
