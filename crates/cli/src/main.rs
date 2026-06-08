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
        eprintln!("Usage: certo <command> [args]");
        eprintln!("Commands: migrate up | migrate down [N] | migrate status | migrate create <name>");
        process::exit(1);
    }

    match args[1].as_str() {
        "migrate" => cmd_migrate(&args[2..]),
        other => {
            eprintln!("Unknown command: {}", other);
            process::exit(1);
        }
    }
}

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
            if steps.is_empty() {
                println!("Nothing to migrate.");
                return;
            }
            let opts = RunOptions { dry_run, manifest_path: &manifest };
            match run_steps(&steps, &opts) {
                Ok(sql) => {
                    for stmt in &sql {
                        println!("{}", stmt);
                    }
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
            if steps.is_empty() {
                println!("Nothing to roll back.");
                return;
            }
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
                        let applied = ts.as_deref().unwrap_or("(pending)");
                        println!("{:<40} {}", name, applied);
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
    if !migrations_dir.exists() {
        return Vec::new();
    }
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
            if let Decl::Migration(m) = &decl.node {
                result.push(m.clone());
            }
        }
    }
    result
}
