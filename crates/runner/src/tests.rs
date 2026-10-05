use crate::exec::{AppliedRow, ExecError, Executor};
use crate::migration::{create, list};
use crate::project::Project;
use crate::runner::{apply, status, ApplyOptions};
use crate::{Migration, RunnerError};
use std::fs;
use tempfile::TempDir;

fn project() -> (TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "postgres").unwrap();
    (dir, p)
}

fn set_schema(p: &Project, src: &str) {
    fs::write(p.schema_path(), src).unwrap();
}

const V1: &str = "table users { id: uuid primary key  email: text not null }";
const V2: &str = "table users { id: uuid primary key  email: text not null  age: int }";

#[derive(Default)]
struct Fake {
    applied: Vec<AppliedRow>,
    ensured: bool,
    ran: Vec<u32>,
    fail_seq: Option<u32>,
    live: Option<crate::introspect::LiveSchema>,
    recorded: Vec<u32>,
    fail_record: bool,
    /// Simulates the process being killed while it records: a panic, so none of the cleanup code runs.
    die_on_record: bool,
}

impl Executor for Fake {
    fn ensure_history(&mut self) -> Result<(), ExecError> { self.ensured = true; Ok(()) }
    fn applied(&mut self) -> Result<Vec<AppliedRow>, ExecError> { Ok(self.applied.clone()) }
    fn record_applied(&mut self, m: &Migration) -> Result<(), ExecError> {
        if self.fail_record {
            return Err(ExecError { statement: None, message: "history table is read-only".into() });
        }
        if self.die_on_record {
            panic!("killed");
        }
        self.recorded.push(m.seq);
        self.applied.push(AppliedRow { seq: m.seq, name: m.name.clone(), checksum: m.checksum.clone(), applied_at: "adopted".into() });
        Ok(())
    }
    fn introspect(&mut self) -> Result<crate::introspect::LiveSchema, ExecError> {
        Ok(self.live.clone().expect("test did not configure a live schema"))
    }
    fn apply(&mut self, m: &Migration) -> Result<(), ExecError> {
        if self.fail_seq == Some(m.seq) {
            return Err(ExecError { statement: Some("BOOM".into()), message: "boom".into() });
        }
        self.ran.push(m.seq);
        self.applied.push(AppliedRow {
            seq: m.seq,
            name: m.name.clone(),
            checksum: m.checksum.clone(),
            applied_at: "now".into(),
        });
        Ok(())
    }
}

fn two_migrations(p: &Project) {
    set_schema(p, V1);
    create(p, "init", None, false).unwrap();
    set_schema(p, V2);
    create(p, "add age", None, false).unwrap();
}

// ---- project ------------------------------------------------------------ //

#[test]
fn init_and_open() {
    let (dir, p) = project();
    for f in ["certo-db.toml", "schema.sdl", "IR.json", "migrations"] {
        assert!(dir.path().join(f).exists(), "{f}");
    }
    assert!(p.state_ir().unwrap().tables.is_empty());
    assert_eq!(Project::open(dir.path()).unwrap().config, p.config);
    // no double init, no unknown dialect, no project here
    assert!(matches!(Project::init(dir.path(), "postgres"), Err(RunnerError::Project(_))));
    let other = tempfile::tempdir().unwrap();
    assert!(matches!(Project::init(other.path(), "oracle"), Err(RunnerError::Project(_))));
    assert!(matches!(Project::open(other.path()), Err(RunnerError::Project(m)) if m.contains("init")));
}

// ---- create ------------------------------------------------------------- //

#[test]
fn create_writes_a_frozen_migration_and_advances_state() {
    let (dir, p) = project();
    set_schema(&p, V1);
    let c = create(&p, "Initial Schema!", None, false).unwrap();
    assert_eq!((c.seq, c.name.as_str()), (1, "initial_schema"));
    assert_eq!(c.summary, ["+ table users"]);
    assert!(!c.destructive);
    let m = dir.path().join("migrations/0001_initial_schema");
    for f in ["up.json", "up.sql", "plan.json", "ir.json"] {
        assert!(m.join(f).exists(), "{f}");
    }
    assert!(!m.join("migration.mdl").exists());
    let sql = fs::read_to_string(m.join("up.sql")).unwrap();
    assert!(sql.contains("GENERATED") && sql.contains("CREATE TABLE \"users\""), "{sql}");
    assert_eq!(p.state_ir().unwrap().tables[0].name, "users");

    // nothing changed -> nothing to do
    assert!(matches!(create(&p, "again", None, false), Err(RunnerError::NoChanges)));
    // a change numbers 0002
    set_schema(&p, V2);
    let c = create(&p, "add age", None, false).unwrap();
    assert_eq!(c.seq, 2);
    assert_eq!(c.summary, ["+ column users.age"]);
    assert!(dir.path().join("migrations/0002_add_age/up.json").exists());
}

#[test]
fn create_rejects_a_bad_name_and_a_broken_schema() {
    let (_d, p) = project();
    set_schema(&p, V1);
    assert!(matches!(create(&p, "???", None, false), Err(RunnerError::Project(_))));
    set_schema(&p, "table t { id: nope }");
    let Err(RunnerError::Compile { file, rendered, diagnostics, source }) = create(&p, "x", None, false) else { panic!() };
    assert_eq!(file, "schema.sdl");
    assert!(rendered.contains("unknown type `nope`"), "{rendered}");
    assert_eq!(diagnostics[0].code, "SDL202");
    assert_eq!(source, "table t { id: nope }", "diagnostics carry the source their spans point into");
    // nothing was written
    assert!(list(&p).unwrap().is_empty());
    assert!(p.state_ir().unwrap().tables.is_empty());
}

#[test]
fn destructive_changes_need_permission() {
    let (_d, p) = project();
    two_migrations(&p);
    set_schema(&p, "table users { id: uuid primary key }");
    let Err(RunnerError::Destructive(ops)) = create(&p, "drop stuff", None, false) else { panic!() };
    assert!(ops.iter().any(|o| o.contains("column users.email")), "{ops:?}");
    assert_eq!(list(&p).unwrap().len(), 2, "refusal writes nothing");
    let c = create(&p, "drop stuff", None, true).unwrap();
    assert!(c.destructive);
}

#[test]
fn unsupported_changes_point_at_mdl_and_mdl_fixes_them() {
    let (d, p) = project();
    let old = "enum Role { admin, user, guest } table users { id: int primary key  role: Role }";
    let new = "enum Role { admin, user } table users { id: int primary key  role: Role }";
    set_schema(&p, old);
    create(&p, "init", None, false).unwrap();
    set_schema(&p, new);
    let Err(RunnerError::Unsupported(e)) = create(&p, "shrink", None, true) else { panic!() };
    assert!(e.reason.contains("remap"), "{e}");

    let c = create(&p, "shrink", Some(("shrink.mdl", "remap Role.guest -> user")), true).unwrap();
    assert!(c.summary.iter().any(|s| s.contains("recreate enum Role")));
    let saved = fs::read_to_string(d.path().join("migrations/0002_shrink/migration.mdl")).unwrap();
    assert_eq!(saved, "remap Role.guest -> user");
}

#[test]
fn a_bad_mdl_reports_against_its_own_file() {
    let (_d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    set_schema(&p, V2);
    let Err(RunnerError::Compile { file, rendered, .. }) =
        create(&p, "x", Some(("fix.mdl", "rename table ghost -> users")), false)
    else { panic!() };
    assert_eq!(file, "fix.mdl");
    assert!(rendered.contains("MDL301") && rendered.contains("fix.mdl"), "{rendered}");
    assert_eq!(list(&p).unwrap().len(), 1);
}

#[test]
fn ir_json_must_match_the_last_migration() {
    let (_d, p) = project();
    two_migrations(&p);
    p.write_state_ir(&certo_sdl::SchemaIR::empty()).unwrap(); // someone reset it
    set_schema(&p, "table users { id: uuid primary key }");
    let Err(RunnerError::Project(m)) = create(&p, "x", None, true) else { panic!() };
    assert!(m.contains("IR.json does not match"), "{m}");
}

// ---- `new` killed part-way --------------------------------------------- //

#[test]
fn a_migration_is_published_whole_and_leaves_no_temp_files() {
    let (d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    let names = |dir: std::path::PathBuf| {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(names(d.path().join("migrations")), ["0001_init"], "no hidden build folder is left");
    assert_eq!(names(d.path().join("migrations/0001_init")), ["ir.json", "plan.json", "up.json", "up.sql"]);
    assert!(!names(d.path().to_path_buf()).iter().any(|n| n.ends_with(".tmp")), "IR.json was replaced, not left beside a temp file");
}

#[test]
fn a_folder_left_by_a_killed_new_while_building_is_ignored_and_cleaned_up() {
    let (d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    // a killed `new` that was still building its folder: some files, no up.json, hidden name
    let stale = d.path().join("migrations/.0002_add_age.tmp");
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("plan.json"), "{").unwrap();
    // it is not part of the history, so nothing trips over it
    assert_eq!(list(&p).unwrap().len(), 1);
    let mut db = Fake::default();
    assert_eq!(status(&p, &mut db).unwrap().pending.len(), 1);
    // and the next `new` carries on, removing it
    set_schema(&p, V2);
    let c = create(&p, "add_age", None, false).unwrap();
    assert_eq!(c.seq, 2);
    assert!(!stale.exists());
    assert_eq!(list(&p).unwrap().len(), 2);
}

#[test]
fn a_new_killed_before_ir_json_moved_forward_is_repaired_by_the_next_one() {
    let (_d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    set_schema(&p, V2);
    create(&p, "add_age", None, false).unwrap();
    let migrations = list(&p).unwrap();
    let (first, second) = (read_ir_of(&migrations[0]), read_ir_of(&migrations[1]));
    // the process died after the second migration was published and before IR.json followed
    p.write_state_ir(&first).unwrap();
    assert_ne!(p.state_ir().unwrap(), second);
    // running `new` again: the migration is already there, so there is nothing new, and IR.json is brought forward
    assert!(matches!(create(&p, "add_age", None, false), Err(RunnerError::NoChanges)));
    assert_eq!(p.state_ir().unwrap(), second);
    // a real change after that carries on from the repaired state
    set_schema(&p, "table users { id: uuid primary key  email: text not null  age: int  nick: text }");
    assert_eq!(create(&p, "nick", None, false).unwrap().seq, 3);

    // the same for the very first migration: IR.json still empty
    let (_d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    let only = read_ir_of(&list(&p).unwrap()[0]);
    p.write_state_ir(&certo_sdl::SchemaIR::empty()).unwrap();
    assert!(matches!(create(&p, "init", None, false), Err(RunnerError::NoChanges)));
    assert_eq!(p.state_ir().unwrap(), only);
}

#[test]
fn only_that_one_state_is_repaired_other_edits_to_ir_json_are_still_refused() {
    let (_d, p) = project();
    set_schema(&p, V1);
    create(&p, "init", None, false).unwrap();
    set_schema(&p, V2);
    create(&p, "add_age", None, false).unwrap();
    // neither the last migration's schema nor the one before it
    let mut odd = certo_sdl::compile("table other { id: uuid primary key }").0.unwrap();
    odd.tables[0].name = "other".into();
    p.write_state_ir(&odd).unwrap();
    let Err(RunnerError::Project(m)) = create(&p, "x", None, true) else { panic!() };
    assert!(m.contains("IR.json does not match"), "{m}");
    assert_eq!(p.state_ir().unwrap(), odd, "an edit that is not the interrupted-new state is left alone");
}

fn read_ir_of(m: &Migration) -> certo_sdl::SchemaIR {
    certo_sdl::SchemaIR::from_json(&fs::read_to_string(m.dir.join("ir.json")).unwrap()).unwrap()
}

// ---- list --------------------------------------------------------------- //

#[test]
fn list_orders_checksums_and_validates() {
    let (d, p) = project();
    two_migrations(&p);
    fs::write(d.path().join("migrations/README.md"), "notes").unwrap(); // ignored
    let l = list(&p).unwrap();
    assert_eq!(l.iter().map(|m| m.label()).collect::<Vec<_>>(), ["0001_init", "0002_add_age"]);
    assert_ne!(l[0].checksum, l[1].checksum);
    assert_eq!(l[0].checksum.len(), 64);

    // a gap is an error
    fs::rename(d.path().join("migrations/0002_add_age"), d.path().join("migrations/0003_add_age")).unwrap();
    assert!(matches!(list(&p), Err(RunnerError::Project(m)) if m.contains("expected 0002")));
}

#[test]
fn list_rejects_a_migration_for_another_dialect() {
    let (d, p) = project();
    two_migrations(&p);
    let up = d.path().join("migrations/0001_init/up.json");
    let text = fs::read_to_string(&up).unwrap().replace("\"postgres\"", "\"mysql\"");
    fs::write(&up, text).unwrap();
    assert!(matches!(list(&p), Err(RunnerError::Project(m)) if m.contains("mysql")));
}

// ---- apply / status ----------------------------------------------------- //

#[test]
fn apply_runs_pending_in_order_and_is_idempotent() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();

    let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations, ["0001_init", "0002_add_age"]);
    assert!(db.ensured);
    assert_eq!(db.ran, [1, 2]);

    let again = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert!(again.migrations.is_empty());
    assert_eq!(db.ran, [1, 2], "nothing ran twice");
}

#[test]
fn apply_up_to_a_number() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    apply(&p, &mut db, &ApplyOptions { to: Some(1), ..Default::default() }).unwrap();
    assert_eq!(db.ran, [1]);
    assert!(matches!(
        apply(&p, &mut db, &ApplyOptions { to: Some(9), ..Default::default() }),
        Err(RunnerError::Project(m)) if m.contains("no migration numbered 9")
    ));
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(db.ran, [1, 2]);
}

#[test]
fn dry_run_touches_nothing() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    let r = apply(&p, &mut db, &ApplyOptions { dry_run: true, ..Default::default() }).unwrap();
    assert!(r.dry_run);
    assert_eq!(r.migrations, ["0001_init", "0002_add_age"]);
    assert_eq!(r.scripts.len(), 2);
    assert!(r.scripts[0].1.contains("CREATE TABLE \"users\""));
    assert!(!db.ensured && db.ran.is_empty(), "dry run must not create the history table or run anything");
}

#[test]
fn a_failure_stops_the_run_and_keeps_earlier_migrations() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake { fail_seq: Some(2), ..Default::default() };
    let err = apply(&p, &mut db, &ApplyOptions::default()).unwrap_err();
    let RunnerError::Database { seq, name, statement, message } = &err else { panic!("{err}") };
    assert_eq!((*seq, name.as_str(), statement.as_deref(), message.as_str()), (2, "add_age", Some("BOOM"), "boom"));
    assert!(err.to_string().contains("rolled back"));
    assert_eq!(db.ran, [1]);

    // after the cause is fixed, a re-run resumes at 2
    db.fail_seq = None;
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(db.ran, [1, 2]);
}

#[test]
fn status_reports_applied_and_pending() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    let s = status(&p, &mut db).unwrap();
    assert!(s.applied.is_empty());
    assert_eq!(s.pending.len(), 2);
    assert!(!db.ensured, "status is read-only");
    apply(&p, &mut db, &ApplyOptions { to: Some(1), ..Default::default() }).unwrap();
    let s = status(&p, &mut db).unwrap();
    assert_eq!(s.applied.len(), 1);
    assert_eq!(s.pending, [(2, "add_age".to_string())]);
}

// ---- drift -------------------------------------------------------------- //

fn drifted(p: &Project, db: &mut Fake) -> String {
    match apply(p, db, &ApplyOptions::default()).unwrap_err() {
        RunnerError::Drift(m) => m,
        e => panic!("expected drift, got {e}"),
    }
}

#[test]
fn editing_an_applied_migration_is_caught_before_anything_runs() {
    let (d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    apply(&p, &mut db, &ApplyOptions { to: Some(1), ..Default::default() }).unwrap();

    let up = d.path().join("migrations/0001_init/up.json");
    fs::write(&up, fs::read_to_string(&up).unwrap().replace("users", "people")).unwrap();
    let m = drifted(&p, &mut db);
    assert!(m.contains("0001_init") && m.contains("modified after it was applied"), "{m}");
    assert_eq!(db.ran, [1], "migration 2 must not run on top of a tampered history");
    // status refuses too
    assert!(matches!(status(&p, &mut db), Err(RunnerError::Drift(_))));
}

#[test]
fn editing_up_sql_does_not_matter_because_it_is_never_executed() {
    let (d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    apply(&p, &mut db, &ApplyOptions { to: Some(1), ..Default::default() }).unwrap();
    fs::write(d.path().join("migrations/0001_init/up.sql"), "-- whatever").unwrap();
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(db.ran, [1, 2]);
}

#[test]
fn missing_renamed_and_foreign_history_is_caught() {
    let (d, p) = project();
    two_migrations(&p);
    let mut db = Fake::default();
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();

    // history has a migration the files do not
    db.applied.push(AppliedRow { seq: 3, name: "ghost".into(), checksum: "x".into(), applied_at: "now".into() });
    assert!(drifted(&p, &mut db).contains("0003_ghost"));
    db.applied.pop();

    // renamed on disk
    fs::rename(d.path().join("migrations/0002_add_age"), d.path().join("migrations/0002_add_years")).unwrap();
    assert!(drifted(&p, &mut db).contains("now called 0002_add_years"));
    fs::rename(d.path().join("migrations/0002_add_years"), d.path().join("migrations/0002_add_age")).unwrap();

    // history rows out of step with the files
    db.applied[1].seq = 5;
    assert!(drifted(&p, &mut db).contains("0005_add_age"));
}

#[test]
fn progress_reports_each_applied_migration_even_when_a_later_one_fails() {
    let (_d, p) = project();
    two_migrations(&p);
    let mut db = Fake { fail_seq: Some(2), ..Default::default() };
    let mut seen: Vec<String> = Vec::new();
    let r = crate::apply_with_progress(&p, &mut db, &ApplyOptions::default(), &mut |l| seen.push(l.to_string()));
    assert!(matches!(r, Err(RunnerError::Database { seq: 2, .. })));
    assert_eq!(seen, ["0001_init"], "the caller learns migration 1 succeeded");
}


// ---- adopt ---------------------------------------------------------------- //

mod adopt_tests {
    use super::*;
    use crate::adopt::{adopt, prepare, AdoptOptions};
    use crate::introspect::LiveSchema;
    use certo_sdl::{compile, ExprIR, SchemaIR, TypeIR};

    fn live_of(src: &str) -> LiveSchema {
        let (ir, d) = compile(src);
        LiveSchema { ir: ir.unwrap_or_else(|| panic!("{d:?}")), notes: vec![], views: vec![] }
    }

    fn raw(sql: &str) -> Option<ExprIR> { Some(ExprIR::Raw { sql: sql.into() }) }

    const LEGACY: &str = "
        enum Role { admin, user }
        table users {
            id: uuid primary key
            email: text not null unique
            role: Role
            age: int
            score: int
            seq: int
        }
        table posts {
            id: uuid primary key
            author_id: uuid not null references users on delete cascade
        }
        index users_email on users (email)
        constraint adult on users using age >= 18
        constraint has_seq on users using seq > 0
    ";

    /// The schema as a live database would report it: defaults/CHECKs as SQL text.
    fn legacy_live() -> LiveSchema {
        let mut l = live_of(LEGACY);
        let users = l.ir.tables.iter_mut().find(|t| t.name == "users").unwrap();
        fn col<'a>(t: &'a mut certo_sdl::TableIR, n: &str) -> &'a mut certo_sdl::ColumnIR {
            t.columns.iter_mut().find(|c| c.name == n).unwrap()
        }
        col(users, "id").default = raw("gen_random_uuid()");
        col(users, "role").default = raw("'user'::\"Role\"");
        col(users, "age").default = raw("18");
        col(users, "score").default = raw("nextval('users_score_seq'::regclass)"); // untranslatable
        col(users, "seq").default = raw("md5('x'::text)"); // an unsupported function
        users.constraints[0].expr = ExprIR::Raw { sql: "((age >= 18))".into() };
        users.constraints[1].expr = ExprIR::Raw { sql: "(seq ~ '1'::text)".into() }; // an unsupported operator
        l
    }

    #[test]
    fn prepare_translates_what_it_can_and_reports_the_rest() {
        let p = prepare(legacy_live()).unwrap();
        let users = p.ir.table("users").unwrap();
        // translated
        assert!(matches!(users.column("id").unwrap().default, Some(ExprIR::Call { ref func, .. }) if func == "gen_uuid"));
        assert!(matches!(users.column("role").unwrap().default, Some(ExprIR::EnumVariant { ref variant, .. }) if variant == "user"));
        assert_eq!(users.column("age").unwrap().default, Some(ExprIR::Number { value: 18 }));
        assert_eq!(users.constraints.len(), 1);
        assert_eq!(users.constraints[0].name, "adult");
        // omitted, with reasons
        assert!(users.column("score").unwrap().default.is_none() && users.column("seq").unwrap().default.is_none());
        let om = p.omissions.join("\n");
        assert!(om.contains("default of users.score") && om.contains("nextval"), "{om}");
        assert!(om.contains("default of users.seq") && om.contains("md5"), "{om}");
        assert!(om.contains("constraint has_seq") && om.contains("operator `~`"), "{om}");
        // the SDL it printed compiles to exactly this IR
        assert_eq!(compile(&p.sdl).0.unwrap(), p.ir);
        assert!(p.sdl.contains("role: Role default user"), "{}", p.sdl);
        assert_eq!((p.counts.tables, p.counts.enums, p.counts.indexes, p.counts.constraints), (2, 1, 1, 1));
    }

    #[test]
    fn the_widened_language_lets_more_of_a_database_through() {
        let mut l = live_of(LEGACY);
        let users = l.ir.tables.iter_mut().find(|t| t.name == "users").unwrap();
        users.columns.iter_mut().find(|c| c.name == "seq").unwrap().default = raw("-1");
        users.columns.iter_mut().find(|c| c.name == "score").unwrap().default = raw("'-5'::integer");
        users.constraints[1].expr = ExprIR::Raw { sql: "(seq IS NOT NULL)".into() };
        users.constraints[0].expr = ExprIR::Raw { sql: "(age = ANY (ARRAY[18, 21, 65]))".into() };
        let p = prepare(l).unwrap();
        let users = p.ir.table("users").unwrap();
        assert_eq!(users.column("seq").unwrap().default, Some(ExprIR::Number { value: -1 }));
        assert_eq!(users.column("score").unwrap().default, Some(ExprIR::Number { value: -5 }));
        assert_eq!(users.constraints.len(), 2, "IS NOT NULL and IN both adopted: {:?}", p.omissions);
        assert!(p.sdl.contains("seq is not null") && p.sdl.contains("age in (18, 21, 65)"), "{}", p.sdl);
        assert!(p.sdl.contains("seq: int default -1"), "{}", p.sdl);
        assert!(p.omissions.is_empty(), "{:?}", p.omissions);
    }

    #[test]
    fn serial_identity_and_sequences_are_adopted() {
        let mut l = live_of(
            "sequence s start 100 increment 5
             table t { id: serial primary key  b: bigserial unique  n: int generated always  m: int generated by default  q: bigint default nextval(s) }",
        );
        // the live database reports a shared sequence's default as SQL text
        l.ir.tables[0].columns.iter_mut().find(|c| c.name == "q").unwrap().default =
            raw("nextval('s'::regclass)");
        let p = prepare(l).unwrap();
        assert!(p.omissions.is_empty(), "{:?}", p.omissions);
        let t = p.ir.table("t").unwrap();
        assert_eq!(t.column("id").unwrap().generated, Some(certo_sdl::Generation::Serial));
        assert_eq!(t.column("q").unwrap().default, Some(ExprIR::NextVal { sequence: "s".into() }));
        assert_eq!(p.counts.sequences, 1);
        for expected in [
            "sequence s start 100 increment 5",
            "id: serial primary key",
            "b: bigserial unique",
            "n: int generated always",
            "m: int generated by default",
            "q: bigint default nextval(s)",
        ] {
            assert!(p.sdl.contains(expected), "expected `{expected}` in:\n{}", p.sdl);
        }
        assert_eq!(compile(&p.sdl).0.unwrap(), p.ir);
    }

    #[test]
    fn sequences_that_cannot_be_adopted_take_their_defaults_with_them() {
        let mut l = live_of("sequence ok_seq  table t { id: int primary key  a: bigint  b: bigint  c: bigint }");
        let mk = |name: &str| certo_sdl::SequenceIR { name: name.into(), start: 1, increment: 1, min: 1, max: i64::MAX, cache: 1, cycle: false };
        l.ir.sequences.push(mk("bad name")); // not a valid SDL name
        l.ir.sequences.push(mk("t")); // collides with the table `t`
        let cols = &mut l.ir.tables[0].columns;
        cols.iter_mut().find(|c| c.name == "a").unwrap().default = raw("nextval('ok_seq'::regclass)");
        cols.iter_mut().find(|c| c.name == "b").unwrap().default = raw("nextval('\"bad name\"'::regclass)");
        cols.iter_mut().find(|c| c.name == "c").unwrap().default = raw("nextval('t'::regclass)");
        let p = prepare(l).unwrap();
        let om = p.omissions.join("\n");
        assert!(om.contains("sequence bad name not adopted"), "{om}");
        assert!(om.contains("sequence t not adopted"), "{om}");
        assert!(om.contains("default of t.b") && om.contains("default of t.c"), "{om}");
        let t = p.ir.table("t").unwrap();
        assert!(t.column("a").unwrap().default.is_some());
        assert!(t.column("b").unwrap().default.is_none() && t.column("c").unwrap().default.is_none());
        assert_eq!(p.counts.sequences, 1);
        assert_eq!(compile(&p.sdl).0.unwrap(), p.ir);
    }

    #[test]
    fn unexpressible_objects_take_their_dependents_with_them() {
        let mut l = live_of(LEGACY);
        // a table SDL cannot name, referenced by a foreign key
        let mut odd = l.ir.tables[1].clone(); // posts
        odd.name = "user data".into();
        l.ir.tables.push(odd);
        let mut c = l.ir.tables[0].columns[1].clone();
        c.name = "odd_ref".into();
        c.unique = false;
        c.ty = TypeIR::Builtin(certo_sdl::Builtin::Uuid);
        c.references = Some(certo_sdl::ForeignKeyIR { table: "user data".into(), column: "id".into(), ..Default::default() });
        let posts_idx = l.ir.tables.iter().position(|t| t.name == "posts").unwrap();
        l.ir.tables[posts_idx].columns.push(c);

        // an enum with a value SDL cannot spell; a column using it; an index on that column
        l.ir.enums.push(certo_sdl::EnumIR { name: "Mood".into(), variants: vec!["ok".into(), "not ok".into()] });
        let users = l.ir.tables.iter_mut().find(|t| t.name == "users").unwrap();
        let mut mood = users.columns[2].clone();
        mood.name = "mood".into();
        mood.ty = TypeIR::Enum("Mood".into());
        users.columns.push(mood);
        users.indexes.push(certo_sdl::IndexIR { name: "users_mood".into(), columns: vec!["mood".into()] });
        // a column name SDL cannot spell
        let mut bad = users.columns[3].clone();
        bad.name = "first-name".into();
        users.columns.push(bad);
        // an FK whose target is not unique
        let posts = l.ir.tables.iter_mut().find(|t| t.name == "posts").unwrap();
        let mut c = posts.columns[0].clone();
        c.name = "by_age".into();
        c.primary_key = false;
        c.nullable = true;
        c.ty = TypeIR::Builtin(certo_sdl::Builtin::Int);
        c.references = Some(certo_sdl::ForeignKeyIR { table: "users".into(), column: "age".into(), ..Default::default() });
        posts.columns.push(c);

        let p = prepare(l).unwrap();
        let om = p.omissions.join("\n");
        assert!(p.ir.table("user data").is_none() && om.contains("table user data"), "{om}");
        let odd_ref = p.ir.table("posts").unwrap().column("odd_ref").expect("the column stays, only its key goes");
        assert!(odd_ref.references.is_none());
        assert!(om.contains("foreign key posts.odd_ref") && om.contains("not adopted"), "{om}");
        assert!(p.ir.enums.iter().all(|e| e.name != "Mood") && om.contains("enum Mood"), "{om}");
        assert!(p.ir.table("users").unwrap().column("mood").is_none(), "columns of a dropped enum go too");
        assert!(p.ir.table("users").unwrap().indexes.iter().all(|i| i.name != "users_mood"), "and their indexes");
        assert!(p.ir.table("users").unwrap().column("first-name").is_none());
        assert!(om.contains("foreign key posts.by_age") && om.contains("not unique"), "{om}");
        // and what remains is valid SDL, identical after a round trip
        assert_eq!(compile(&p.sdl).0.unwrap(), p.ir);
    }

    #[test]
    fn constraint_names_must_be_unique_across_tables() {
        let mut l = live_of("table a { id: int primary key  x: int } table b { id: int primary key  y: int } constraint ca on a using x > 0 constraint cb on b using y > 0");
        l.ir.tables[1].constraints[0].name = "ca".into(); // PostgreSQL allows the same name on two tables
        let p = prepare(l).unwrap();
        assert_eq!(p.counts.constraints, 1);
        assert!(p.omissions.join("\n").contains("already used by another table"), "{:?}", p.omissions);
    }

    #[test]
    fn introspection_notes_are_carried_into_the_report() {
        let mut l = live_of("table t { id: int primary key }");
        l.notes.push("column t.c has type varchar(10), which SDL cannot express; ignored".into());
        assert!(prepare(l).unwrap().omissions[0].contains("varchar"));
    }

    fn fresh(live: LiveSchema) -> (TempDir, Project, Fake) {
        let (d, p) = project();
        (d, p, Fake { live: Some(live), ..Default::default() })
    }

    #[test]
    fn adopt_writes_the_schema_a_baseline_and_records_it_without_running_it() {
        let (d, p, mut db) = fresh(legacy_live());
        let r = adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        assert_eq!(r.migration.as_deref(), Some("0001_baseline"));
        assert!(!r.omissions.is_empty());

        // schema.sdl is the printed schema and compiles
        let sdl = std::fs::read_to_string(p.schema_path()).unwrap();
        assert_eq!(sdl, r.schema_sdl);
        assert!(compile(&sdl).0.is_some());
        // IR.json and the baseline agree with it
        assert_eq!(p.state_ir().unwrap(), compile(&sdl).0.unwrap());
        let m = &list(&p).unwrap()[0];
        assert_eq!(m.label(), "0001_baseline");
        // it holds the real CREATE statements (so an empty database can be built)...
        assert!(m.script.batches.iter().flat_map(|b| &b.statements).any(|s| s.starts_with("CREATE TABLE \"users\"")));
        // ...but it was only recorded, never executed
        assert_eq!(db.recorded, [1]);
        assert!(db.ran.is_empty());
        assert_eq!(db.applied[0].checksum, m.checksum);
        assert!(std::fs::read_to_string(d.path().join("migrations/0001_baseline/up.sql")).unwrap().contains("BASELINE"));

        // the history now lines up, so the normal workflow continues
        assert_eq!(status(&p, &mut db).unwrap().pending.len(), 0);
        set_schema(&p, &format!("{sdl}\ntable extra {{ id: int primary key }}\n"));
        let c = create(&p, "add extra", None, false).unwrap();
        assert_eq!((c.seq, c.summary.clone()), (2, vec!["+ table extra".to_string()]));
    }

    #[test]
    fn dry_run_changes_nothing() {
        let (_d, p, mut db) = fresh(legacy_live());
        let before = std::fs::read_to_string(p.schema_path()).unwrap();
        let r = adopt(&p, &mut db, &AdoptOptions { dry_run: true, ..Default::default() }).unwrap();
        assert!(r.dry_run && r.migration.is_none());
        assert!(r.schema_sdl.contains("table users"));
        assert_eq!(std::fs::read_to_string(p.schema_path()).unwrap(), before);
        assert!(list(&p).unwrap().is_empty());
        assert!(db.recorded.is_empty());
        assert!(p.state_ir().unwrap().tables.is_empty());
    }

    #[test]
    fn import_reads_a_schema_without_changing_anything_even_when_managed() {
        use crate::adopt::import_schema;
        let (_d, _p, mut db) = fresh(legacy_live());
        // a database with a migration history can still be read: only adopting it is refused
        db.applied.push(AppliedRow { seq: 1, name: "x".into(), checksum: "c".into(), applied_at: "t".into() });
        let p = import_schema(&mut db).unwrap();
        assert!(compile(&p.sdl).0.is_some(), "the SDL compiles");
        assert!(p.counts.tables > 0 && !p.omissions.is_empty());
        assert!(db.recorded.is_empty() && db.ran.is_empty(), "nothing was written");
        // an empty database is an empty schema, not an error
        let (_d, _p, mut empty) = fresh(LiveSchema { ir: SchemaIR::empty(), notes: vec![], views: vec![] });
        assert_eq!(import_schema(&mut empty).unwrap().counts.tables, 0);
    }

    const MARKER: &str = ".certo-adopt";

    fn state_matches_files(p: &Project) {
        let sdl = std::fs::read_to_string(p.schema_path()).unwrap();
        assert_eq!(p.state_ir().unwrap(), compile(&sdl).0.unwrap(), "IR.json is what schema.sdl compiles to");
    }

    #[test]
    fn an_adopt_killed_before_recording_is_undone_and_redone_by_the_next_one() {
        let (_d, p, mut db) = fresh(legacy_live());
        let stub = std::fs::read_to_string(p.schema_path()).unwrap();
        db.die_on_record = true;
        let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| adopt(&p, &mut db, &AdoptOptions::default())));
        assert!(died.is_err(), "the \"process\" was killed");
        // what a killed process leaves: the baseline on disk, nothing in the database, and the marker
        assert!(p.root.join(MARKER).exists());
        assert_eq!(list(&p).unwrap().len(), 1);
        assert!(db.applied.is_empty());

        // a dry run does not touch it, and says so
        db.die_on_record = false;
        assert!(matches!(adopt(&p, &mut db, &AdoptOptions { dry_run: true, ..Default::default() }), Err(RunnerError::Project(m)) if m.contains("interrupted")));
        assert!(p.root.join(MARKER).exists());

        // the next adopt undoes the partial work and does it all again
        let r = adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        assert!(r.recovered.as_deref().is_some_and(|s| s.contains("redone")), "{:?}", r.recovered);
        assert_eq!(r.migration.as_deref(), Some("0001_baseline"));
        assert!(!p.root.join(MARKER).exists(), "the marker is gone");
        assert_eq!(db.recorded, [1]);
        assert_eq!(list(&p).unwrap().len(), 1, "exactly one baseline");
        state_matches_files(&p);
        assert_eq!(status(&p, &mut db).unwrap().pending.len(), 0);
        assert_ne!(std::fs::read_to_string(p.schema_path()).unwrap(), stub);
    }

    #[test]
    fn an_adopt_killed_after_recording_is_just_acknowledged() {
        let (_d, p, mut db) = fresh(legacy_live());
        let stub = std::fs::read_to_string(p.schema_path()).unwrap();
        adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        let (sdl, checksum) = (std::fs::read_to_string(p.schema_path()).unwrap(), list(&p).unwrap()[0].checksum.clone());
        // the process died after the baseline was recorded and before the marker was removed
        std::fs::write(p.root.join(MARKER), &stub).unwrap();

        let r = adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        assert!(r.recovered.as_deref().is_some_and(|s| s.contains("already recorded")), "{:?}", r.recovered);
        assert_eq!(r.migration.as_deref(), Some("0001_baseline"));
        assert!(!p.root.join(MARKER).exists());
        // nothing was redone or recorded twice
        assert_eq!(db.recorded, [1]);
        assert_eq!(list(&p).unwrap()[0].checksum, checksum);
        assert_eq!(std::fs::read_to_string(p.schema_path()).unwrap(), sdl);
        assert_eq!(status(&p, &mut db).unwrap().pending.len(), 0);
    }

    #[test]
    fn an_adopt_killed_while_writing_files_is_undone_too() {
        let (_d, p, mut db) = fresh(legacy_live());
        let stub = std::fs::read_to_string(p.schema_path()).unwrap();
        // killed after schema.sdl and part of the baseline were written
        std::fs::write(p.root.join(MARKER), &stub).unwrap();
        set_schema(&p, "table half_written { id: int primary key }");
        std::fs::create_dir_all(p.migrations_dir().join("0001_baseline")).unwrap();
        std::fs::write(p.migrations_dir().join("0001_baseline").join("plan.json"), "{").unwrap();

        let r = adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        assert!(r.recovered.is_some());
        assert_eq!(list(&p).unwrap().len(), 1);
        assert!(!std::fs::read_to_string(p.schema_path()).unwrap().contains("half_written"));
        state_matches_files(&p);
        assert_eq!(db.recorded, [1]);
    }

    #[test]
    fn an_interrupted_adopt_does_not_touch_a_database_with_someone_elses_history() {
        let (_d, p, mut db) = fresh(legacy_live());
        let stub = std::fs::read_to_string(p.schema_path()).unwrap();
        std::fs::write(p.root.join(MARKER), &stub).unwrap();
        set_schema(&p, "table kept { id: int primary key }");
        db.applied.push(AppliedRow { seq: 1, name: "other".into(), checksum: "not-ours".into(), applied_at: "t".into() });
        let r = adopt(&p, &mut db, &AdoptOptions::default());
        assert!(matches!(r, Err(RunnerError::Project(m)) if m.contains("interrupted") && m.contains("nothing was changed")));
        assert!(p.root.join(MARKER).exists());
        assert!(std::fs::read_to_string(p.schema_path()).unwrap().contains("kept"), "untouched");
        assert!(db.recorded.is_empty());
    }

    #[test]
    fn a_normal_adopt_leaves_no_marker_and_a_failed_one_cleans_it_up() {
        let (_d, p, mut db) = fresh(legacy_live());
        adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
        assert!(!p.root.join(MARKER).exists());
        // an ordinary failure (not a kill) rolls back everything, the marker included
        let (_d, p, mut db) = fresh(legacy_live());
        db.fail_record = true;
        assert!(adopt(&p, &mut db, &AdoptOptions::default()).is_err());
        assert!(!p.root.join(MARKER).exists());
        assert!(list(&p).unwrap().is_empty());
    }

    #[test]
    fn adopt_refuses_when_it_should() {
        // project already has migrations
        let (_d, p) = project();
        two_migrations(&p);
        let mut db = Fake { live: Some(legacy_live()), ..Default::default() };
        assert!(matches!(adopt(&p, &mut db, &AdoptOptions::default()), Err(RunnerError::Project(m)) if m.contains("already has migrations")));

        // schema.sdl already has declarations: needs --force
        let (_d, p, mut db) = fresh(legacy_live());
        set_schema(&p, "table mine { id: int primary key }");
        assert!(matches!(adopt(&p, &mut db, &AdoptOptions::default()), Err(RunnerError::Project(m)) if m.contains("--force")));
        assert!(std::fs::read_to_string(p.schema_path()).unwrap().contains("mine"), "untouched");
        adopt(&p, &mut db, &AdoptOptions { force: true, ..Default::default() }).unwrap();
        assert!(!std::fs::read_to_string(p.schema_path()).unwrap().contains("mine"));

        // database already managed
        let (_d, p, mut db) = fresh(legacy_live());
        db.applied.push(AppliedRow { seq: 1, name: "x".into(), checksum: "c".into(), applied_at: "t".into() });
        assert!(matches!(adopt(&p, &mut db, &AdoptOptions::default()), Err(RunnerError::Project(m)) if m.contains("migration history")));

        // nothing there
        let (_d, p, mut db) = fresh(LiveSchema { ir: SchemaIR::empty(), notes: vec![], views: vec![] });
        assert!(matches!(adopt(&p, &mut db, &AdoptOptions::default()), Err(RunnerError::Project(m)) if m.contains("nothing to adopt")));
    }

    #[test]
    fn a_failed_record_rolls_everything_back() {
        let (d, p, mut db) = fresh(legacy_live());
        set_schema(&p, "// my notes\n");
        db.fail_record = true;
        let err = adopt(&p, &mut db, &AdoptOptions::default()).unwrap_err();
        assert!(matches!(&err, RunnerError::Connection(m) if m.contains("baseline")), "{err}");
        assert_eq!(std::fs::read_to_string(p.schema_path()).unwrap(), "// my notes\n", "schema.sdl restored");
        assert!(list(&p).unwrap().is_empty(), "no orphan migration");
        assert!(!d.path().join("migrations/0001_baseline").exists());
        assert!(p.state_ir().unwrap().tables.is_empty(), "IR.json restored");
        // and a retry works
        db.fail_record = false;
        assert!(adopt(&p, &mut db, &AdoptOptions::default()).is_ok());
    }
}

#[test]
fn a_sqlite_project_freezes_sqlite_scripts_but_the_postgres_executor_refuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "sqlite").unwrap();
    set_schema(&p, "enum Status { a, b }\ntable t { id: serial primary key  s: Status not null default a }");
    create(&p, "init", None, false).unwrap();
    let files = list(&p).unwrap();
    let sql = files[0].script.to_sql();
    assert_eq!(files[0].script.dialect, "sqlite");
    assert!(sql.contains("INTEGER PRIMARY KEY AUTOINCREMENT") && sql.contains("CHECK (\"s\" IN ('a', 'b'))"), "{sql}");

    // a second migration that needs a rebuild is written from the schemas of both sides
    set_schema(&p, "enum Status { a, b }\ntable t { id: serial primary key  s: Status not null default a  n: text not null }");
    create(&p, "more", None, true).unwrap();
    let sql = list(&p).unwrap()[1].script.to_sql();
    assert!(sql.contains("__certo_new_t"), "{sql}");

    let mut fake = Fake::default();
    let err = apply(&p, &mut fake, &ApplyOptions::default()).unwrap_err();
    assert!(matches!(&err, RunnerError::Project(m) if m.contains("the project dialect is `sqlite` but this executor talks `postgres`")), "{err}");
    assert!(fake.ran.is_empty() && !fake.ensured);
}
