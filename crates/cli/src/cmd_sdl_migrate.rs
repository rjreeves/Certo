//! `certo sdl migrate` — project-based schema migrations.
//!
//!   certo sdl migrate init   [--dialect postgres]
//!   certo sdl migrate new    <name> [--mdl file.mdl] [--allow-destructive]
//!   certo sdl migrate apply  [--url URL] [--dry-run] [--to N]
//!   certo sdl migrate status [--url URL]
//!   certo sdl migrate drift  [--url URL] [--sql | --json]
//!   certo sdl migrate adopt  [--url URL] [--dry-run] [--force]
//!
//! Runs in the current directory (or `-C <dir>`). The database URL comes from
//! `--url` or the DATABASE_URL environment variable; it is never written to a file.

use certo_runner::{
    apply, drift, status, ApplyOptions, Drift, DriftKind, Executor, Project, RunnerError,
};
use std::path::{Path, PathBuf};
use std::process;

use crate::die;

const USAGE: &str = "\
Usage: certo sdl migrate <subcommand> [options]

Subcommands:
  init                Create certo-db.toml, schema.sdl, IR.json and migrations/
  new <name>          Freeze the difference between schema.sdl and IR.json as
                      the next migration (reviewable SQL in migrations/NNNN_name/)
  apply               Apply pending migrations in order
  status              Show applied and pending migrations
  drift               Compare the live database with the last applied migration
  adopt               Turn an existing database into schema.sdl + a baseline migration

Options:
  -C <dir>              Project directory (default: current directory)
  --dialect <name>      (init) SQL dialect: postgres or sqlite
  --mdl <file>          (new) MDL file steering renames, enum remaps, backfills, steps
  --allow-destructive   (new) accept operations that can lose data
  --url <url>           postgres:// URL, or a database file for sqlite; default: $DATABASE_URL
                        (add ?sslmode=require for TLS)
  --dry-run             (apply) print the SQL that would run; touch nothing
  --check-drift         (apply) refuse to run if the database has drifted
  --sql                 (drift) print the script that would repair the drift
  --json                (drift) machine-readable report
  --dry-run             (adopt) show the schema.sdl it would write and what it leaves out
  --force               (adopt) overwrite a schema.sdl that already has declarations
  --to <N>              (apply) stop after migration number N
";

pub fn cmd_migrate(args: &[String]) {
    let Some(sub) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        process::exit(2);
    };
    if matches!(sub, "--help" | "-h" | "help") {
        print!("{USAGE}");
        return;
    }
    let opts = Opts::parse(&args[1..]);
    let root = opts.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let result = match sub {
        "init" => init(&root, &opts),
        "new" => new(&root, &opts),
        "apply" => apply_cmd(&root, &opts),
        "status" => status_cmd(&root, &opts),
        "drift" => drift_cmd(&root, &opts),
        "adopt" => adopt_cmd(&root, &opts),
        other => {
            eprintln!("Unknown migrate subcommand: {other}\n");
            eprint!("{USAGE}");
            process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

#[derive(Default)]
struct Opts {
    positional: Vec<String>,
    dir: Option<PathBuf>,
    dialect: Option<String>,
    mdl: Option<PathBuf>,
    url: Option<String>,
    allow_destructive: bool,
    dry_run: bool,
    force: bool,
    check_drift: bool,
    sql: bool,
    json: bool,
    to: Option<u32>,
}

impl Opts {
    fn parse(args: &[String]) -> Opts {
        let mut o = Opts::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |flag: &str| -> String {
                it.next().cloned().unwrap_or_else(|| die(&format!("{flag} requires a value"), 2))
            };
            match a.as_str() {
                "-C" => o.dir = Some(PathBuf::from(value("-C"))),
                "--dialect" => o.dialect = Some(value("--dialect")),
                "--mdl" => o.mdl = Some(PathBuf::from(value("--mdl"))),
                "--url" => o.url = Some(value("--url")),
                "--to" => {
                    let v = value("--to");
                    o.to = Some(v.parse().unwrap_or_else(|_| die(&format!("--to expects a number, got `{v}`"), 2)));
                }
                "--allow-destructive" => o.allow_destructive = true,
                "--dry-run" => o.dry_run = true,
                "--check-drift" => o.check_drift = true,
                "--force" => o.force = true,
                "--sql" => o.sql = true,
                "--json" => o.json = true,
                "--help" | "-h" => {
                    print!("{USAGE}");
                    process::exit(0);
                }
                f if f.starts_with('-') => die(&format!("unknown option: {f}"), 2),
                p => o.positional.push(p.to_string()),
            }
        }
        o
    }

    fn url(&self) -> Option<String> {
        self.url.clone().or_else(|| std::env::var("DATABASE_URL").ok().filter(|u| !u.is_empty()))
    }
}

fn init(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let dialect = o.dialect.as_deref().unwrap_or("postgres");
    let p = Project::init(root, dialect)?;
    println!("created {} project in {}", p.config.dialect, p.root.display());
    println!("  edit {}, then run: certo sdl migrate new <name>", p.config.schema);
    Ok(())
}

fn new(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let name = o.positional.first().unwrap_or_else(|| die("usage: certo sdl migrate new <name>", 2));
    let project = Project::open(root)?;
    let mdl_src = o.mdl.as_ref().map(|path| {
        (
            path.display().to_string(),
            std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("cannot read {}: {e}", path.display()), 2)),
        )
    });
    let created = certo_runner::migration::create(
        &project,
        name,
        mdl_src.as_ref().map(|(l, s)| (l.as_str(), s.as_str())),
        o.allow_destructive,
    )?;
    eprint!("{}", created.warnings);
    println!("created {}", created.dir.display());
    for line in &created.summary {
        println!("  {line}");
    }
    if created.destructive {
        println!("  (contains destructive operations: review up.sql before applying)");
    }
    println!("review {}/up.sql, then run: certo sdl migrate apply", created.dir.display());
    Ok(())
}

fn connect(project: &Project, o: &Opts) -> Result<Box<dyn Executor>, RunnerError> {
    let url = o.url().ok_or_else(|| {
        RunnerError::Connection("no database: pass --url (a postgres:// URL, or a file path for sqlite) or set DATABASE_URL".into())
    })?;
    certo_runner::connect(project, &url)
}

fn apply_cmd(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let project = Project::open(root)?;
    let mut db = connect(&project, o)?;
    if o.check_drift {
        let d = drift::check(&project, &mut db)?;
        if !d.in_sync() {
            print_findings(&d);
            return Err(RunnerError::SchemaDrift(format!(
                "the database has drifted from {}; nothing was applied (run `certo sdl migrate drift` for details)",
                d.expected_from
            )));
        }
    }
    let report = apply(&project, &mut db, &ApplyOptions { dry_run: o.dry_run, to: o.to, ..Default::default() })?;
    if report.migrations.is_empty() {
        println!("nothing to apply: database is up to date");
        return Ok(());
    }
    if report.dry_run {
        for (label, sql) in &report.scripts {
            println!("-- {label}\n{sql}\n");
        }
        println!("dry run: {} migration(s) would be applied", report.migrations.len());
    } else {
        for label in &report.migrations {
            println!("applied {label}");
        }
    }
    Ok(())
}

fn status_cmd(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let project = Project::open(root)?;
    if o.url().is_none() {
        // no database: just the local view
        let local = certo_runner::migration::list(&project)?;
        if local.is_empty() {
            println!("no migrations yet");
        }
        for m in local {
            println!("  {}", m.label());
        }
        eprintln!("(no database URL: showing local migrations only)");
        return Ok(());
    }
    let mut db = connect(&project, o)?;
    let s = status(&project, &mut db)?;
    for a in &s.applied {
        println!("applied  {:04}_{}  ({})", a.seq, a.name, a.applied_at);
    }
    for (seq, name) in &s.pending {
        println!("pending  {seq:04}_{name}");
    }
    if s.pending.is_empty() {
        println!("up to date ({} applied)", s.applied.len());
    }
    Ok(())
}

fn print_findings(d: &Drift) {
    for (kind, heading) in [
        (DriftKind::Missing, "missing from the database"),
        (DriftKind::Different, "different in the database"),
        (DriftKind::Unexpected, "unexpected in the database"),
    ] {
        let group: Vec<_> = d.items.iter().filter(|i| i.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        println!("{heading}:");
        for i in group {
            println!("  {}", i.text);
        }
    }
}

fn drift_cmd(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let project = Project::open(root)?;
    let mut db = connect(&project, o)?;
    let d = drift::check(&project, &mut db)?;

    if o.json {
        let kind = |k: DriftKind| match k {
            DriftKind::Missing => "missing",
            DriftKind::Unexpected => "unexpected",
            DriftKind::Different => "different",
        };
        let doc = serde_json::json!({
            "in_sync": d.in_sync(),
            "expected_from": d.expected_from,
            "items": d.items.iter().map(|i| serde_json::json!({ "kind": kind(i.kind), "text": i.text })).collect::<Vec<_>>(),
            "notes": d.notes,
        });
        println!("{}", serde_json::to_string_pretty(&doc).expect("json"));
    } else {
        println!("comparing the database with {}", d.expected_from);
        if d.in_sync() {
            println!("no drift");
        } else {
            print_findings(&d);
        }
        for n in &d.notes {
            eprintln!("note: {n}");
        }
        if o.sql && !d.in_sync() {
            match d.repair_sql(project.dialect()) {
                Ok(script) => println!("\n-- script that would bring the database back to the migrations (review before running):\n{script}"),
                Err(e) => eprintln!("\nno repair script: {e}"),
            }
        }
    }
    if d.in_sync() { Ok(()) } else { process::exit(1) }
}

fn adopt_cmd(root: &Path, o: &Opts) -> Result<(), RunnerError> {
    let project = Project::open(root)?;
    let mut db = connect(&project, o)?;
    let r = certo_runner::adopt(
        &project,
        &mut db,
        &certo_runner::AdoptOptions { dry_run: o.dry_run, force: o.force, ..Default::default() },
    )?;
    let c = &r.adopted;
    let what = format!(
        "{} table(s) with {} column(s), {} enum(s), {} type(s), {} sequence(s), {} index(es), {} constraint(s)",
        c.tables, c.columns, c.enums, c.types, c.sequences, c.indexes, c.constraints
    );

    if r.dry_run {
        println!("{}", r.schema_sdl);
        println!("-- dry run: would adopt {what}");
    } else {
        println!("adopted {what}");
        println!("  wrote {}", project.schema_path().display());
        println!("  baseline migration {} (recorded as applied; it was not run)", r.migration.as_deref().unwrap_or("?"));
    }
    if !r.omissions.is_empty() {
        eprintln!("\nnot adopted ({}):", r.omissions.len());
        for om in &r.omissions {
            eprintln!("  {om}");
        }
    }
    if !r.dry_run {
        if r.known_drift.is_empty() {
            println!("\nthe database matches the adopted schema exactly");
        } else {
            println!("\nthe database still differs from the adopted schema in {} way(s) (the objects above):", r.known_drift.len());
            for i in &r.known_drift {
                println!("  {}", i.text);
            }
            println!("`certo sdl migrate drift` will keep reporting these until you add them to schema.sdl or remove them from the database.");
        }
        println!("\nnext: edit schema.sdl, then `certo sdl migrate new <name>`");
    }
    Ok(())
}
