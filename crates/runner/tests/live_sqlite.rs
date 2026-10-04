//! The runner on a real SQLite database file: apply, status, drift, and adopting
//! an existing database. No server needed, so these always run.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{adopt, apply, status, AdoptOptions, ApplyOptions, Project, RunnerError, SqliteExecutor};
use std::fs;
use tempfile::TempDir;

const V1: &str = r#"
enum Status { new, paid }
enum Unused { a, b }
table customers {
    id: serial primary key
    email: varchar(100) not null unique
    joined: timestamp not null default now()
    token: uuid default gen_uuid()
    meta: json
}
table orders {
    id: serial primary key
    customer_id: int not null references customers on delete cascade
    status: Status not null default new
    total: decimal(10,2) not null
    note: text
    paid: bool not null default false
    shipped: date
}
index orders_status on orders (status)
constraint total_positive on orders using total >= 0
"#;

const V2: &str = r#"
enum Status { new, paid, shipped }
enum Unused { a, b }
table customers {
    id: serial primary key
    email: varchar(100) not null unique
    joined: timestamp not null default now()
    token: uuid default gen_uuid()
    meta: json
    nickname: text
}
table orders {
    id: serial primary key
    customer_id: int not null references customers on delete cascade
    status: Status not null default new
    total: decimal(10,2) not null
    note: text not null default "none"
    paid: bool not null default false
    shipped: date
}
index orders_status on orders (status)
index orders_customer on orders (customer_id)
constraint total_positive on orders using total >= 0
"#;

fn fixture(schema: &str) -> (TempDir, Project, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "sqlite").unwrap();
    fs::write(p.schema_path(), schema).unwrap();
    let db = dir.path().join("app.db").display().to_string();
    (dir, p, db)
}

fn count(e: &SqliteExecutor, sql: &str) -> i64 { e.connection().query_row(sql, [], |r| r.get(0)).unwrap() }

#[test]
fn apply_status_and_drift_on_a_database_file() {
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();

    let mut ex = SqliteExecutor::open(&db).unwrap();
    assert_eq!(status(&p, &mut ex).unwrap().pending.len(), 1);
    let r = apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations.len(), 1);
    assert!(status(&p, &mut ex).unwrap().pending.is_empty());

    // what the adapter wrote reads back as exactly the schema that was asked for
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.in_sync(), "{:?}", d.items);

    // evolve with data: rebuilds keep rows, the new rules apply, drift stays clean
    ex.connection()
        .execute_batch(
            "INSERT INTO customers (email) VALUES ('a@x.com'), ('b@x.com');
             INSERT INTO orders (customer_id, status, total, note) VALUES (1, 'paid', 10, NULL), (2, 'new', 5, 'hi');",
        )
        .unwrap();
    fs::write(p.schema_path(), V2).unwrap();
    create(&p, "evolve", None, true).unwrap(); // NULL notes take the new default: flagged as lossy, which is expected
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM orders WHERE note = 'none'"), 1, "NULL note took its default");
    assert_eq!(count(&ex, "SELECT count(*) FROM customers"), 2);
    ex.connection().execute("INSERT INTO orders (customer_id, status, total) VALUES (1, 'shipped', 1)", []).unwrap();
    assert_eq!(count(&ex, "PRAGMA foreign_keys"), 1, "foreign keys are back on after a rebuild");
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.in_sync(), "{:?}", d.items);

    // drift: someone changes the database by hand
    ex.connection().execute_batch("ALTER TABLE customers ADD COLUMN sneaky INTEGER; CREATE INDEX stray ON orders (total);").unwrap();
    let d = drift::check(&p, &mut ex).unwrap();
    let text: Vec<String> = d.items.iter().map(|i| i.text.clone()).collect();
    assert!(text.iter().any(|t| t.contains("sneaky")) && text.iter().any(|t| t.contains("stray")), "{text:?}");

    // the repair script is real SQL for SQLite (it needs both schemas, which the report carries) and fixes it
    let repair = d.repair_sql(certo_sql::Dialect::Sqlite).unwrap_or_else(|e| panic!("{e}"));
    ex.connection().execute_batch(&repair).unwrap_or_else(|e| panic!("{e}
{repair}"));
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.in_sync(), "after the repair: {:?}", d.items);
}

#[test]
fn a_failing_migration_rolls_back_and_is_not_recorded() {
    let (_dir, p, db) = fixture("table t { id: serial primary key }");
    create(&p, "init", None, false).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();

    fs::write(p.schema_path(), "table t { id: serial primary key  n: int not null default 0 }\ntable u { id: serial primary key }").unwrap();
    let mdl = "after { sql sqlite \"INSERT INTO nope VALUES (1)\" }";
    create(&p, "bad", Some(("bad.mdl", mdl)), true).unwrap(); // raw SQL always counts as destructive
    let err = apply(&p, &mut ex, &ApplyOptions::default()).unwrap_err();
    assert!(matches!(err, RunnerError::Database { .. }), "{err}");
    assert_eq!(count(&ex, "SELECT count(*) FROM sqlite_master WHERE name = 'u'"), 0, "rolled back");
    assert_eq!(status(&p, &mut ex).unwrap().applied.len(), 1);
}

const LEGACY: &str = r#"
CREATE TABLE customers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email VARCHAR(255) NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT 'anon',
    age INTEGER DEFAULT 18,
    balance NUMERIC(10,2) DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT 1,
    created DATETIME DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT c_age CHECK (age >= 0)
);
CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    status TEXT DEFAULT 'new' CHECK (status IN ('new', 'paid')),
    total INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX orders_customer ON orders (customer_id);
CREATE UNIQUE INDEX customers_name_uq ON customers (name);
CREATE TABLE audit (a INTEGER, b INTEGER, PRIMARY KEY (a, b));
CREATE VIEW v AS SELECT id FROM customers;
"#;

#[test]
fn an_existing_sqlite_database_can_be_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "sqlite").unwrap();
    let db = dir.path().join("legacy.db").display().to_string();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    ex.connection().execute_batch(LEGACY).unwrap();
    ex.connection().execute("INSERT INTO customers (email) VALUES ('a@x.com')", []).unwrap();

    let r = adopt(&p, &mut ex, &AdoptOptions::default()).unwrap();
    assert_eq!(r.adopted.tables, 3);
    assert!(r.schema_sdl.contains("table customers") && r.schema_sdl.contains("varchar(255)"), "{}", r.schema_sdl);
    let all = r.omissions.join("\n");
    // what SDL has no word for is reported, not guessed
    assert!(all.contains("DATETIME"), "unknown column type reported:\n{all}");
    assert!(all.contains("customers_name_uq"), "unique index reported:\n{all}");
    assert!(all.contains("view v"), "{all}");

    // the baseline is recorded, not run, and what remains differs only by what was left out
    assert_eq!(status(&p, &mut ex).unwrap().applied.len(), 1);
    assert_eq!(count(&ex, "SELECT count(*) FROM customers"), 1, "data untouched");
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.in_sync(), "adopted schema should match the database: {:?}", d.items);

    // and the project carries on from there
    fs::write(p.schema_path(), format!("{}\n", fs::read_to_string(p.schema_path()).unwrap()).replace("table audit", "table audit")).unwrap();
}

#[test]
fn a_second_runner_on_the_same_database_is_refused_until_the_first_is_done() {
    use certo_runner::RunnerError;
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();

    let mut first = SqliteExecutor::open(&db).unwrap();
    assert!(std::path::Path::new(&format!("{db}.certo-lock")).is_file(), "the lock lives beside the database");
    // while the first holds the lock, the second cannot even connect
    match SqliteExecutor::open(&db) {
        Err(RunnerError::Connection(m)) => assert!(m.contains("holds the lock"), "{m}"),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("a second runner got in"),
    }
    // the first is unaffected
    apply(&p, &mut first, &ApplyOptions::default()).unwrap();

    // dropping it releases the lock; a runner can start again
    drop(first);
    let mut second = SqliteExecutor::open(&db).expect("the lock was released");
    assert!(status(&p, &mut second).unwrap().pending.is_empty());
    drop(second);

    // an in-memory database has nothing to lock, so any number of them can be open
    let (a, b) = (SqliteExecutor::open(":memory:").unwrap(), SqliteExecutor::open(":memory:").unwrap());
    drop((a, b));
    // and `sqlite:` URLs lock the same file as the plain path
    let held = SqliteExecutor::open(&format!("sqlite:{db}")).unwrap();
    assert!(SqliteExecutor::open(&db).is_err());
    drop(held);
}

#[test]
fn a_table_rebuild_is_recorded_in_the_same_transaction_as_the_change() {
    use certo_runner::Executor;
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();

    // V2 makes `orders.note` NOT NULL, which SQLite cannot do in place: the table is rebuilt, and the script
    // ends with `PRAGMA foreign_keys = ON`, which cannot be part of the transaction
    fs::write(p.schema_path(), V2).unwrap();
    create(&p, "v2", None, true).unwrap(); // narrowing a column is a destructive change
    let migrations = certo_runner::migration::list(&p).unwrap();
    let m2 = migrations.iter().find(|m| m.seq == 2).unwrap();
    assert!(m2.script.batches.last().is_some_and(|b| !b.transactional), "the script ends outside the transaction");
    let note_not_null = |ex: &SqliteExecutor| count(ex, "SELECT \"notnull\" FROM pragma_table_info('orders') WHERE name = 'note'");
    assert_eq!(note_not_null(&ex), 0);

    // make recording fail: a row for seq 2 is already there
    ex.connection()
        .execute("INSERT INTO _certo_migrations (seq, name, checksum, compiler_version) VALUES (2, 'squatter', 'x', 'x')", [])
        .unwrap();
    let tables = count(&ex, "SELECT count(*) FROM sqlite_master WHERE type = 'table'");
    assert!(ex.apply(m2).is_err(), "the history insert fails");
    // the history row is part of the transaction, so its failure took the rebuild with it: no change without a record
    assert_eq!(note_not_null(&ex), 0, "the table is as it was");
    assert_eq!(count(&ex, "SELECT count(*) FROM sqlite_master WHERE type = 'table'"), tables, "no half-built table is left behind");
    // and foreign keys are enforced again
    assert_eq!(count(&ex, "PRAGMA foreign_keys"), 1);

    // without the squatter the migration applies, changes the table and is recorded
    ex.connection().execute("DELETE FROM _certo_migrations WHERE seq = 2", []).unwrap();
    ex.apply(m2).unwrap();
    assert_eq!(note_not_null(&ex), 1);
    assert_eq!(count(&ex, "SELECT count(*) FROM _certo_migrations WHERE seq = 2 AND name = 'v2'"), 1);
    assert_eq!(count(&ex, "PRAGMA foreign_keys"), 1);
}

