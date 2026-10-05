//! The runner on a real MySQL 8 server: apply, status, drift (which exercises reading a schema back), a migration that stops
//! half-way and is resumed, views, the journal, and adopting an existing database.
//!
//! Skipped unless `CERTO_TEST_MYSQL_URL` is set (for example `mysql://root@127.0.0.1:54398/certo_test`; the URL's database
//! name is replaced). WARNING: it drops and recreates the databases `certo_runner_*` on that server.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{adopt, apply, status, AdoptOptions, ApplyOptions, DriftKind, JournalContext, MysqlExecutor, Project, RunnerError};
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder};
use std::fs;
use tempfile::TempDir;

const V1: &str = r#"
enum Role { admin, user, guest }
table users {
    id: serial primary key
    token: uuid default gen_uuid()
    email: varchar(100) not null unique
    role: Role not null default user
    age: int default 18
    score: float default 1
    balance: decimal(10,2)
    big: decimal
    visits: bigint default 0
    active: bool not null default true
    meta: json
    avatar: bytes
    born: date
    created: timestamp not null default now()
    local: timestamp_naive
    nick: varchar(40) default "anon"
    code: varchar(40) default "it's a \\ test"
    bio: text
}
table posts {
    id: serial primary key
    author: int not null references users on delete cascade
    title: varchar(100) not null
    body: text not null default ""
}
index posts_title on posts (title)
index posts_author_title on posts (author, title)
constraint adult on users using age >= 18 and role != admin
constraint sane on users using score >= 0 or is_null(score)
constraint titled on posts using length(title) > 0
"#;

const V2: &str = r#"
enum Role { admin, user, guest, banned }
table users {
    id: serial primary key
    token: uuid default gen_uuid()
    email: varchar(200) not null unique
    role: Role not null default user
    age: int default 21
    score: float default 1
    balance: decimal(10,2)
    big: decimal
    visits: bigint default 0
    active: bool not null default true
    meta: json
    avatar: bytes
    born: date
    created: timestamp not null default now()
    local: timestamp_naive
    nick: varchar(40) default "anon"
    code: varchar(40) default "it's a \\ test"
    bio: text
    slug: varchar(50) unique
}
table posts {
    id: serial primary key
    author: int references users on delete set null
    title: varchar(100) not null
    body: text not null default ""
}
index posts_title on posts (title)
constraint adult on users using age >= 21 and role != admin
constraint sane on users using score >= 0 or is_null(score)
constraint titled on posts using length(title) > 0
"#;

fn server() -> Option<String> {
    std::env::var("CERTO_TEST_MYSQL_URL").ok()
}

/// A fresh database on the server, and a URL for it.
fn fresh(name: &str) -> Option<(String, Conn)> {
    let url = server()?;
    let opts = Opts::from_url(&url).expect("CERTO_TEST_MYSQL_URL");
    let mut admin = Conn::new(OptsBuilder::from_opts(opts.clone()).db_name(Some("mysql".to_string()))).expect("connect");
    admin.query_drop(format!("DROP DATABASE IF EXISTS {name}")).unwrap();
    admin.query_drop(format!("CREATE DATABASE {name}")).unwrap();
    let conn = Conn::new(OptsBuilder::from_opts(opts.clone()).db_name(Some(name.to_string()))).expect("connect");
    let (user, pass) = (opts.get_user().unwrap_or("root").to_string(), opts.get_pass().unwrap_or("").to_string());
    let host = opts.get_ip_or_hostname().to_string();
    let url = format!("mysql://{user}{}@{host}:{}/{name}", if pass.is_empty() { String::new() } else { format!(":{pass}") }, opts.get_tcp_port());
    Some((url, conn))
}

fn project(schema: &str) -> (TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "mysql").unwrap();
    fs::write(p.schema_path(), schema).unwrap();
    (dir, p)
}

fn count(c: &mut Conn, sql: &str) -> i64 {
    c.query_first::<i64, _>(sql).unwrap().unwrap()
}

#[test]
fn apply_status_and_drift_round_trip() {
    let Some((url, mut raw)) = fresh("certo_runner_apply") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let (_dir, p) = project(V1);
    create(&p, "init", None, false).unwrap();
    {
        let mut db = MysqlExecutor::connect(&url).unwrap();
        let s = status(&p, &mut db).unwrap();
        assert_eq!((s.applied.len(), s.pending.len(), s.partial.len()), (0, 1, 0));
        let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
        assert_eq!(r.migrations, ["0001_init"]);
        let s = status(&p, &mut db).unwrap();
        assert_eq!((s.applied.len(), s.pending.len()), (1, 0));
        // reading the schema back gives what was applied: no drift, nothing left out
        let d = drift::check(&p, &mut db).unwrap();
        assert!(d.in_sync(), "{:?}\nnotes: {:?}", d.items, d.notes);
        assert!(d.notes.is_empty(), "{:?}", d.notes);
    }
    raw.query_drop("INSERT INTO users (email, age) VALUES ('a@x.com', 30), ('A@x.com', 30)").unwrap();
    raw.query_drop("INSERT INTO posts (author, title) VALUES (1, 't')").unwrap();

    // a change to tables with rows in them: a new enum value, a new column, a changed default, key and constraint
    fs::write(p.schema_path(), V2).unwrap();
    let c = create(&p, "v2", None, true).unwrap();
    assert_eq!(c.name, "v2");
    let mut db = MysqlExecutor::connect(&url).unwrap();
    let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations, ["0002_v2"]);
    let d = drift::check(&p, &mut db).unwrap();
    assert!(d.in_sync(), "{:?}\nnotes: {:?}", d.items, d.notes);
    assert_eq!(count(&mut raw, "SELECT count(*) FROM users"), 2);
    assert_eq!(count(&mut raw, "SELECT count(*) FROM posts WHERE author IS NOT NULL"), 1);

    // hand edits are found
    raw.query_drop("ALTER TABLE users ADD COLUMN extra INT").unwrap();
    raw.query_drop("ALTER TABLE posts MODIFY COLUMN title VARCHAR(100) NULL").unwrap();
    raw.query_drop("DROP INDEX posts_title ON posts").unwrap();
    let d = drift::check(&p, &mut db).unwrap();
    assert!(!d.in_sync());
    let text: Vec<String> = d.items.iter().map(|i| i.text.clone()).collect();
    assert!(text.iter().any(|t| t.contains("extra")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("title")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("posts_title")), "{text:?}");
    assert!(d.items.iter().any(|i| i.kind == DriftKind::Unexpected));
    // and the repair script, run, puts it right
    let sql = d.repair_sql(certo_sql::Dialect::Mysql).unwrap();
    for s in sql.lines().filter(|l| !l.trim().is_empty()) {
        raw.query_drop(s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
    assert!(drift::check(&p, &mut db).unwrap().in_sync());
}

#[test]
fn a_migration_that_stops_half_way_is_resumed() {
    let Some((url, mut raw)) = fresh("certo_runner_resume") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let (_dir, p) = project("table t { id: serial primary key }");
    create(&p, "init", None, false).unwrap();
    let mut db = MysqlExecutor::connect(&url).unwrap();
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    raw.query_drop("INSERT INTO t () VALUES ()").unwrap();

    // two new columns, then a step of its own that fails because the table it names is not there yet
    fs::write(p.schema_path(), "table t { id: serial primary key  a: int  b: varchar(10) }").unwrap();
    create(&p, "grow", Some(("grow.mdl", "after { sql mysql \"ALTER TABLE later ADD COLUMN x INT\" }")), true).unwrap();
    let err = apply(&p, &mut db, &ApplyOptions::default()).unwrap_err();
    let RunnerError::Database { seq, message, .. } = &err else { panic!("{err}") };
    assert_eq!(*seq, 2);
    assert!(message.contains("step 3 of 3") && message.contains("apply again to resume"), "{message}");

    // the two columns are there, nothing is recorded, and status says how far it got
    assert_eq!(count(&mut raw, "SELECT count(*) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 't'"), 3);
    let s = status(&p, &mut db).unwrap();
    assert_eq!((s.applied.len(), s.pending.len()), (1, 1));
    assert_eq!(s.partial.iter().map(|r| (r.seq, r.done, r.total)).collect::<Vec<_>>(), [(2, 2, 3)]);

    // once the cause is fixed, applying again runs only the step that was left
    raw.query_drop("CREATE TABLE later (id INT PRIMARY KEY)").unwrap();
    let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations, ["0002_grow"]);
    let s = status(&p, &mut db).unwrap();
    assert_eq!((s.applied.len(), s.pending.len(), s.partial.len()), (2, 0, 0));
    assert_eq!(count(&mut raw, "SELECT count(*) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'later'"), 2);
}

#[test]
fn views_follow_the_migrations_on_mysql() {
    let Some((url, mut raw)) = fresh("certo_runner_views") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let schema = "table people { id: serial primary key  name: varchar(50) not null  age: int }
                  view adults on people (id, name) where age >= 18";
    let (_dir, p) = project(schema);
    create(&p, "init", None, false).unwrap();
    fs::write(p.views_path(), "view adult_names { from adults a select a.name }").unwrap();
    let mut db = MysqlExecutor::connect(&url).unwrap();
    let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(r.views, ["adult_names"]);
    raw.query_drop("INSERT INTO people (name, age) VALUES ('a', 30), ('b', 5)").unwrap();
    assert_eq!(count(&mut raw, "SELECT count(*) FROM adult_names"), 1);
    let d = drift::check(&p, &mut db).unwrap();
    assert!(d.in_sync(), "{:?} {:?}", d.items, d.notes);
    assert!(d.notes.iter().all(|n| !n.contains("adults") && !n.contains("adult_names")), "{:?}", d.notes);
    assert!(status(&p, &mut db).unwrap().views.in_sync);

    // a type change under both views: they go first and come back
    fs::write(p.schema_path(), schema.replace("name: varchar(50)", "name: varchar(80)")).unwrap();
    create(&p, "wider", None, true).unwrap();
    let r = apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!((r.migrations.len(), r.views.clone()), (1, vec!["adult_names".to_string()]));
    assert_eq!(count(&mut raw, "SELECT count(*) FROM adult_names"), 1);
    assert!(drift::check(&p, &mut db).unwrap().in_sync());

    // a view dropped by hand is noticed
    raw.query_drop("DROP VIEW adult_names").unwrap();
    assert!(drift::check(&p, &mut db).unwrap().items.iter().any(|i| i.text.contains("adult_names")));
    apply(&p, &mut db, &ApplyOptions::default()).unwrap();
    assert!(drift::check(&p, &mut db).unwrap().in_sync());
}

#[test]
fn the_journal_notes_applies_and_is_not_drift() {
    let Some((url, mut raw)) = fresh("certo_runner_journal") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let (_dir, p) = project("table t { id: serial primary key }");
    create(&p, "init", None, false).unwrap();
    let mut db = MysqlExecutor::connect(&url).unwrap();
    let who = JournalContext { actor: "alice@build".into(), environment: Some("prod".into()), tool: "certo 9.9.9".into() };
    apply(&p, &mut db, &ApplyOptions { journal: Some(who), ..Default::default() }).unwrap();
    let row: (String, String, String, Option<String>, String) =
        raw.query_first("SELECT action, subject, actor, environment, tool FROM _certo_log").unwrap().unwrap();
    assert_eq!(row, ("apply".into(), "0001_init".into(), "alice@build".into(), Some("prod".into()), "certo 9.9.9".into()));
    assert!(drift::check(&p, &mut db).unwrap().in_sync(), "the journal and history tables are not part of the schema");
}

#[test]
fn an_existing_mysql_database_can_be_adopted() {
    let Some((url, mut raw)) = fresh("certo_runner_adopt") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    // a database made by hand: types, defaults and constraints written the way people write them
    for s in [
        "CREATE TABLE accounts (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, name VARCHAR(80) NOT NULL, plan VARCHAR(20) NOT NULL DEFAULT 'free',
            credit DECIMAL(10,2) NOT NULL DEFAULT 0, active BOOLEAN NOT NULL DEFAULT TRUE, created DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            note TEXT, UNIQUE KEY accounts_name (name), CONSTRAINT credit_ok CHECK (credit >= 0)) ENGINE=InnoDB",
        "CREATE TABLE invoices (id INT NOT NULL AUTO_INCREMENT PRIMARY KEY, account BIGINT NOT NULL, total DECIMAL(10,2) NOT NULL,
            KEY by_account (account), CONSTRAINT fk_inv FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE) ENGINE=InnoDB",
        "CREATE VIEW big_invoices AS SELECT id, total FROM invoices WHERE total > 100",
    ] {
        raw.query_drop(s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
    let (_dir, p) = project("");
    let mut db = MysqlExecutor::connect(&url).unwrap();
    let report = adopt(&p, &mut db, &AdoptOptions::default()).unwrap();
    let sdl = fs::read_to_string(p.schema_path()).unwrap();
    for expected in [
        "table accounts {",
        "id: bigserial primary key",
        "name: varchar(80) not null unique",
        "plan: varchar(20) not null default \"free\"",
        "credit: decimal(10,2) not null default 0.00",
        "active: bool not null default true",
        "created: timestamp_naive not null default now()",
        "invoices {",
        "account: bigint not null references accounts on delete cascade",
        "index by_account on invoices (account)",
        "constraint credit_ok on accounts using credit >= 0",
    ] {
        assert!(sdl.contains(expected), "expected `{expected}` in:\n{sdl}\nomissions: {:?}", report.omissions);
    }
    assert!(report.omissions.iter().any(|o| o.contains("big_invoices")), "the view is said to be left out: {:?}", report.omissions);
    // the baseline is recorded, and the database matches it
    let s = status(&p, &mut db).unwrap();
    assert_eq!((s.applied.len(), s.pending.len()), (1, 0));
    let d = drift::check(&p, &mut db).unwrap();
    assert!(d.in_sync(), "{:?}\n{:?}", d.items, d.notes);
}
