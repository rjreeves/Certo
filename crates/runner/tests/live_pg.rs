//! Live PostgreSQL tests for introspection and drift detection.
//!
//! Skipped unless `CERTO_TEST_PG_URL` is set, e.g.
//!   CERTO_TEST_PG_URL=postgres://user@localhost:5432/scratch cargo test -p certo-runner --test live_pg
//!
//! WARNING: the test DROPS and recreates the `public` schema of that
//! database. Point it at a throwaway database only.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{apply, ApplyOptions, DriftKind, Executor, PgExecutor, Project};
use postgres::{Client, NoTls};

const SCHEMA: &str = r#"
enum Role { admin, user, guest }
type Address { street: text  city: text  zip: text }

table users {
    id: uuid primary key default gen_uuid()
    email: text not null unique
    role: Role default user
    age: int default 18
    score: float default 1
    balance: decimal
    visits: bigint default 0
    active: bool not null default true
    meta: json
    avatar: bytes
    born: date
    created: timestamp not null default now()
    home: Address
    nick: text default "anon"
    code: text default "it's"
}

table posts {
    id: uuid primary key
    author_id: uuid not null references users on delete cascade on update restrict
    editor_id: uuid references users on delete set null
    title: text not null default "untitled"
    parent_id: uuid references posts
}

table tags {
    a: int primary key
    b: int primary key
    label: text
}

index users_email_role on users (email, role)
index posts_title on posts (title)
constraint adult on users using age >= 18 and role != admin
constraint sane_score on users using score >= 0 or is_null(score)
constraint tags_label on tags using length(label) > 0 or is_null(label)
"#;

fn url() -> Option<String> { std::env::var("CERTO_TEST_PG_URL").ok().filter(|u| !u.is_empty()) }

fn texts(d: &drift::Drift) -> Vec<String> {
    d.items.iter().map(|i| format!("{:?}: {}", i.kind, i.text)).collect()
}

fn has(d: &drift::Drift, kind: DriftKind, needle: &str) -> bool {
    d.items.iter().any(|i| i.kind == kind && i.text.contains(needle))
}

#[test]
fn introspection_matches_a_rich_schema_and_detects_hand_edits() {
    let Some(url) = url() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut raw = Client::connect(&url, NoTls).expect("connect");
    raw.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").unwrap();

    // ---- apply a rich schema through the real runner ----------------------
    let dir = tempfile::tempdir().unwrap();
    let project = Project::init(dir.path(), "postgres").unwrap();
    std::fs::write(project.schema_path(), SCHEMA).unwrap();
    create(&project, "init", None, false).unwrap();
    let mut db = PgExecutor::connect(&url).unwrap();
    apply(&project, &mut db, &ApplyOptions::default()).unwrap();

    // ---- no false positives ------------------------------------------------
    let d = drift::check(&project, &mut db).unwrap();
    assert!(d.in_sync(), "a freshly applied schema must show no drift, got:\n{}", texts(&d).join("\n"));
    assert!(d.notes.is_empty(), "nothing in this schema is unrepresentable: {:?}", d.notes);

    // the introspected IR really contains the interesting bits (not just "no diff because empty")
    let live = db.introspect().unwrap().ir;
    let users = live.table("users").unwrap();
    assert!(users.column("id").unwrap().primary_key);
    assert!(users.column("email").unwrap().unique);
    assert_eq!(users.constraints.len(), 2);
    assert_eq!(users.indexes[0].columns, ["email", "role"]);
    let posts = live.table("posts").unwrap();
    let fk = posts.column("author_id").unwrap().references.clone().unwrap();
    assert_eq!((fk.table.as_str(), fk.on_delete, fk.on_update), ("users", certo_sdl::ReferentialAction::Cascade, certo_sdl::ReferentialAction::Restrict));
    assert!(live.table("tags").unwrap().column("a").unwrap().primary_key);
    assert!(live.table("tags").unwrap().column("b").unwrap().primary_key);
    assert_eq!(live.enums[0].variants, ["admin", "user", "guest"]);
    assert_eq!(live.types[0].fields.len(), 3);

    // ---- hand edits: each must be reported, and the repair must heal it ----
    let cases: &[(&str, &str, DriftKind, &str)] = &[
        ("dropped column (cascades to its check)", "ALTER TABLE users DROP COLUMN age CASCADE", DriftKind::Missing, "column users.age"),
        ("nullability loosened", "ALTER TABLE users ALTER COLUMN email DROP NOT NULL", DriftKind::Different, "nullability: database has NULL"),
        ("default changed", "ALTER TABLE users ALTER COLUMN nick SET DEFAULT 'other'", DriftKind::Different, "column users.nick: default"),
        ("default dropped", "ALTER TABLE users ALTER COLUMN code DROP DEFAULT", DriftKind::Different, "column users.code: default"),
        ("index dropped", "DROP INDEX posts_title", DriftKind::Missing, "index posts_title"),
        ("foreign key dropped", "ALTER TABLE posts DROP CONSTRAINT fk_posts_author_id", DriftKind::Missing, "foreign key posts.author_id"),
        ("unique dropped", "ALTER TABLE users DROP CONSTRAINT users_email_key", DriftKind::Different, "column users.email: unique"),
        ("check body changed", "ALTER TABLE users DROP CONSTRAINT sane_score; ALTER TABLE users ADD CONSTRAINT sane_score CHECK (score >= 5 OR score IS NULL)", DriftKind::Different, "check constraint sane_score on users"),
        ("stray table", "CREATE TABLE stray (id integer PRIMARY KEY)", DriftKind::Unexpected, "table stray"),
        ("stray column", "ALTER TABLE posts ADD COLUMN extra integer", DriftKind::Unexpected, "column posts.extra"),
    ];
    for (what, sql, kind, needle) in cases {
        raw.batch_execute(sql).unwrap_or_else(|e| panic!("{what}: {e}"));
        let d = drift::check(&project, &mut db).unwrap();
        assert!(has(&d, *kind, needle), "{what}: expected {kind:?} `{needle}` in:\n{}", texts(&d).join("\n"));

        // the repair script (live -> expected) must restore the schema exactly
        let script = certo_sql::render(&d.plan, certo_sql::Dialect::Postgres)
            .unwrap_or_else(|e| panic!("{what}: repair not lowerable: {e}"));
        raw.batch_execute(&script).unwrap_or_else(|e| panic!("{what}: repair failed: {e}\n{script}"));
        let after = drift::check(&project, &mut db).unwrap();
        assert!(after.in_sync(), "{what}: still drifting after repair:\n{}\nrepair was:\n{script}", texts(&after).join("\n"));
    }

    // ---- things SDL cannot express are noted, not called drift --------------
    raw.batch_execute(
        "ALTER TABLE tags ADD COLUMN v inet;
         CREATE UNIQUE INDEX tags_label_uq ON tags (label);
         CREATE INDEX posts_lower_title ON posts (lower(title))",
    )
    .unwrap();
    let d = drift::check(&project, &mut db).unwrap();
    assert!(d.in_sync(), "unrepresentable objects must not count as drift: {:?}", texts(&d));
    let notes = d.notes.join("\n");
    assert!(notes.contains("tags.v") && notes.contains("inet"), "{notes}");
    assert!(notes.contains("tags_label_uq") && notes.contains("posts_lower_title"), "{notes}");

    // ---- an enum value added by hand: detected; repair cannot be lowered ----
    raw.batch_execute("ALTER TYPE \"Role\" ADD VALUE 'ghost'").unwrap();
    let d = drift::check(&project, &mut db).unwrap();
    assert!(has(&d, DriftKind::Unexpected, "enum variant Role.ghost"), "{:?}", texts(&d));
    assert!(certo_sql::render(&d.plan, certo_sql::Dialect::Postgres).is_err(), "removing an enum value needs a hand-written migration");

    // ---- history tampering is still caught first ----------------------------
    let up = dir.path().join("migrations/0001_init/up.json");
    std::fs::write(&up, std::fs::read_to_string(&up).unwrap().replace("users", "people")).unwrap();
    assert!(matches!(drift::check(&project, &mut db), Err(certo_runner::RunnerError::Drift(_))));
}

#[test]
fn a_database_with_no_migrations_applied_reports_everything_as_unexpected() {
    let Some(url) = url() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut raw = Client::connect(&url, NoTls).unwrap();
    raw.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE TABLE legacy (id integer PRIMARY KEY, name text);")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let project = Project::init(dir.path(), "postgres").unwrap();
    let mut db = PgExecutor::connect(&url).unwrap();
    let d = drift::check(&project, &mut db).unwrap();
    assert_eq!(texts(&d), ["Unexpected: table legacy"]);
    assert!(d.expected_from.contains("no migrations applied"));
    // the history table itself is never reported
    db.ensure_history().unwrap();
    assert_eq!(texts(&drift::check(&project, &mut db).unwrap()), ["Unexpected: table legacy"]);
}

#[test]
fn the_journal_records_applies_and_is_not_drift_on_postgresql() {
    let Some(url) = url() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut raw = Client::connect(&url, NoTls).expect("connect");
    raw.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let project = Project::init(dir.path(), "postgres").unwrap();
    std::fs::write(project.schema_path(), SCHEMA).unwrap();
    create(&project, "init", None, false).unwrap();
    {
        let mut db = PgExecutor::connect(&url).unwrap();
        let who = certo_runner::JournalContext { actor: "alice@build".into(), environment: Some("prod".into()), tool: "certo 9.9.9".into() };
        apply(&project, &mut db, &ApplyOptions { journal: Some(who), ..Default::default() }).unwrap();
        // certo's own tables are not part of the schema, so there is no drift and nothing extra to import
        assert!(drift::check(&project, &mut db).unwrap().in_sync());
        let imported = certo_runner::import_schema(&mut db).unwrap();
        assert!(!imported.sdl.contains("_certo_log") && !imported.sdl.contains("_certo_migrations"));
    }
    let row = raw.query_one("SELECT action, subject, actor, environment, tool, detail, at IS NOT NULL FROM _certo_log", &[]).unwrap();
    assert_eq!(row.get::<_, String>(0), "apply");
    assert_eq!(row.get::<_, String>(1), "0001_init");
    assert_eq!(row.get::<_, String>(2), "alice@build");
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some("prod"));
    assert_eq!(row.get::<_, String>(4), "certo 9.9.9");
    assert!(row.get::<_, String>(5).contains("checksum"));
    assert!(row.get::<_, bool>(6));
    assert_eq!(raw.query_one("SELECT count(*) FROM _certo_log", &[]).unwrap().get::<_, i64>(0), 1);
}

