//! The runner on a real SQLite database file: apply, status, drift, and adopting
//! an existing database. No server needed, so these always run.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{adopt, apply, status, AdoptOptions, ApplyOptions, JournalContext, Project, RunnerError, SqliteExecutor};
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
    // a plain select of one table's columns is an SDL view now, not a note
    assert!(r.schema_sdl.contains("view v on customers (id)") && r.adopted.views == 1 && !all.contains("view v"), "{}
{all}", r.schema_sdl);

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

fn who() -> JournalContext {
    JournalContext { actor: "alice@build".into(), environment: Some("prod".into()), tool: "certo 9.9.9".into() }
}

fn text(e: &SqliteExecutor, sql: &str) -> String { e.connection().query_row(sql, [], |r| r.get::<_, String>(0)).unwrap() }

#[test]
fn the_journal_records_each_applied_migration_inside_its_transaction() {
    use certo_runner::Executor;
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    // off unless asked for: no table, no rows
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM sqlite_master WHERE name = '_certo_log'"), 0);

    // on: one row per migration, with who, where and what
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    fs::write(p.schema_path(), V2).unwrap();
    create(&p, "v2", None, true).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions { journal: Some(who()), ..Default::default() }).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM _certo_log"), 2);
    assert_eq!(text(&ex, "SELECT action || '|' || subject || '|' || actor || '|' || environment || '|' || tool FROM _certo_log WHERE id = 1"),
        "apply|0001_init|alice@build|prod|certo 9.9.9");
    assert_eq!(text(&ex, "SELECT subject FROM _certo_log WHERE id = 2"), "0002_v2");
    assert!(text(&ex, "SELECT detail FROM _certo_log WHERE id = 2").contains("\"checksum\""));
    assert_eq!(count(&ex, "SELECT count(*) FROM _certo_log WHERE at IS NOT NULL"), 2);

    // it is certo's own table: not drift, and not part of what import or adopt would see
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
    let imported = certo_runner::import_schema(&mut ex).unwrap();
    assert!(!imported.sdl.contains("_certo_log"), "{}", imported.sdl);
    assert_eq!(imported.counts.tables, 2);

    // a migration that rolls back leaves no row: a conflicting history row makes the migration fail inside the transaction
    fs::write(p.schema_path(), format!("{V2}
table extra {{ id: serial primary key }}")).unwrap();
    create(&p, "extra", None, false).unwrap();
    let m3 = certo_runner::migration::list(&p).unwrap().into_iter().find(|m| m.seq == 3).unwrap();
    ex.connection().execute("INSERT INTO _certo_migrations (seq, name, checksum, compiler_version) VALUES (3, 'squatter', 'x', 'x')", []).unwrap();
    ex.set_journal(Some(who()));
    assert!(ex.apply(&m3).is_err());
    assert_eq!(count(&ex, "SELECT count(*) FROM _certo_log"), 2, "no journal row for a change that did not happen");
}

#[test]
fn the_journal_records_an_adopted_baseline_with_the_record_itself() {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::init(dir.path(), "sqlite").unwrap();
    let db = dir.path().join("legacy.db").display().to_string();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    ex.connection().execute_batch(LEGACY).unwrap();
    adopt(&p, &mut ex, &AdoptOptions { journal: Some(who()), ..Default::default() }).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM _certo_log"), 1);
    assert_eq!(text(&ex, "SELECT action || '|' || subject FROM _certo_log"), "adopt|0001_baseline");
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
}

fn views_file(p: &Project, text: &str) { fs::write(p.views_path(), text).unwrap(); }

const VIEWS: &str = "view paid_orders { from orders o where o.paid select o.id, o.total }
    view big { from paid_orders p where p.total > 20 select p.id }";

const ORDERS: &str = "INSERT INTO customers (email) VALUES ('a@x.com');
    INSERT INTO orders (customer_id, total, status, paid) VALUES (1, 10, 'new', 1), (1, 50, 'paid', 1), (1, 5, 'new', 0);";

#[test]
fn views_are_created_recorded_left_alone_and_recreated_when_they_change() {
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    views_file(&p, VIEWS);
    let mut ex = SqliteExecutor::open(&db).unwrap();

    // a dry run shows what would happen to the views, last
    let dry = apply(&p, &mut ex, &ApplyOptions { dry_run: true, ..Default::default() }).unwrap();
    let (label, sql) = dry.scripts.last().unwrap();
    assert_eq!(label, "views");
    assert!(sql.contains("CREATE VIEW \"paid_orders\"") && sql.contains("CREATE VIEW \"big\""), "{sql}");
    assert_eq!(count(&ex, "SELECT count(*) FROM sqlite_master WHERE type = 'view'"), 0, "a dry run touches nothing");

    // created in dependency order, recorded, usable
    let r = apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(r.views, ["paid_orders", "big"]);
    ex.connection().execute_batch(ORDERS).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM paid_orders"), 2);
    assert_eq!(count(&ex, "SELECT count(*) FROM big"), 1);
    let st = status(&p, &mut ex).unwrap();
    assert!(st.views.in_sync && st.views.defined == ["paid_orders", "big"] && st.views.recorded == st.views.defined);
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());

    // nothing changed: the views are left alone
    assert!(apply(&p, &mut ex, &ApplyOptions::default()).unwrap().views.is_empty());

    // views.ql edited: the next apply recreates them with the new definitions, and until then drift says so
    views_file(&p, &VIEWS.replace("> 20", "> 5"));
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.items.iter().any(|i| i.text.contains("view big") && i.text.contains("changed")), "{:?}", d.items);
    assert!(!status(&p, &mut ex).unwrap().views.in_sync);
    assert_eq!(apply(&p, &mut ex, &ApplyOptions::default()).unwrap().views, ["paid_orders", "big"]);
    assert_eq!(count(&ex, "SELECT count(*) FROM big"), 2);
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());

    // a view dropped by hand is created again, and a view no longer in the file is dropped
    ex.connection().execute_batch("DROP VIEW big").unwrap();
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.items.iter().any(|i| i.text.contains("view big") && i.text.contains("not in the database")), "{:?}", d.items);
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM big"), 2);
    views_file(&p, "view paid_orders { from orders o where o.paid select o.id, o.total }");
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.items.iter().any(|i| i.text.contains("view big") && i.text.contains("no longer")), "{:?}", d.items);
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM sqlite_master WHERE type = 'view' AND name = 'big'"), 0);
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
}

#[test]
fn a_migration_can_change_a_table_a_view_reads() {
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    views_file(&p, VIEWS);
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    ex.connection().execute_batch(ORDERS).unwrap();

    // V2 rebuilds `orders`, which both views read: the views go first and come back after
    fs::write(p.schema_path(), V2).unwrap();
    create(&p, "v2", None, true).unwrap();
    let r = apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations, ["0002_v2"]);
    assert_eq!(r.views, ["paid_orders", "big"]);
    assert_eq!(count(&ex, "SELECT count(*) FROM paid_orders"), 2, "the data and the views both survived");
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
}

#[test]
fn a_mistake_in_views_ql_stops_everything_and_unmanaged_views_stay_noted() {
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    views_file(&p, "view v { from nope n select n.id }");
    let mut ex = SqliteExecutor::open(&db).unwrap();
    assert!(matches!(apply(&p, &mut ex, &ApplyOptions::default()), Err(RunnerError::Compile { .. })));
    // status and drift say what is wrong instead of failing, so the pending migration is still visible
    let st = status(&p, &mut ex).unwrap();
    assert_eq!(st.pending.len(), 1, "nothing was applied");
    assert!(st.views.error.as_deref().is_some_and(|e| e.contains("QL203")), "{:?}", st.views.error);
    assert!(drift::check(&p, &mut ex).unwrap().items.iter().any(|i| i.text.contains("views file has errors")));

    // a view somebody else made is still reported as not represented; one certo made is not
    views_file(&p, "view mine { from customers c select c.id }");
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    ex.connection().execute_batch("CREATE VIEW stray AS SELECT 1 AS x").unwrap();
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.notes.iter().any(|n| n.contains("view stray")), "{:?}", d.notes);
    assert!(!d.notes.iter().any(|n| n.contains("view mine")), "{:?}", d.notes);
    // certo's own tables are not part of the schema
    let imported = certo_runner::import_schema(&mut ex).unwrap();
    assert!(!imported.sdl.contains("_certo_views"));
}

#[test]
fn the_journal_notes_when_views_were_recreated() {
    let (_dir, p, db) = fixture(V1);
    create(&p, "init", None, false).unwrap();
    views_file(&p, VIEWS);
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions { journal: Some(who()), ..Default::default() }).unwrap();
    assert_eq!(text(&ex, "SELECT action || '|' || subject FROM _certo_log WHERE action = 'views'"), "views|2 view(s)");
    assert!(text(&ex, "SELECT detail FROM _certo_log WHERE action = 'views'").contains("paid_orders"));
}


const WITH_SDL_VIEW: &str = r#"
table people {
    id: serial primary key
    name: text not null
    age: int
}
view adults on people (id, name) where age >= 18
"#;

#[test]
fn sdl_views_are_part_of_the_migrations_and_the_drift_check() {
    let (_dir, p, db) = fixture(WITH_SDL_VIEW);
    create(&p, "init", None, false).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    let r = apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(r.migrations, ["0001_init"]);
    ex.connection().execute_batch("INSERT INTO people (name, age) VALUES ('a', 30), ('b', 5)").unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM adults"), 1);
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(d.in_sync(), "{:?}", d.items);
    assert!(d.notes.iter().all(|n| !n.contains("adults")), "a view the schema declares is not 'left out': {:?}", d.notes);

    // a view dropped by hand is missing
    ex.connection().execute_batch("DROP VIEW adults").unwrap();
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(!d.in_sync());
    assert!(d.items.iter().any(|i| i.text.contains("adults")), "{:?}", d.items);

    // changing the view, and the table under it, is one migration; the data stays
    ex.connection().execute_batch("CREATE VIEW adults AS SELECT id, name FROM people WHERE age >= 18").unwrap();
    fs::write(p.schema_path(), WITH_SDL_VIEW.replace("name: text not null", "name: varchar(50) not null").replace("age >= 18", "age >= 1")).unwrap();
    create(&p, "wider", None, true).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM adults"), 2);
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());

    // a QL view can read an SDL view
    views_file(&p, "view adult_names { from adults a select a.name }");
    let r = apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(r.views, ["adult_names"]);
    assert_eq!(count(&ex, "SELECT count(*) FROM adult_names"), 2);
    // ...and survives a migration that drops and recreates the SDL view under it
    fs::write(p.schema_path(), WITH_SDL_VIEW.replace("name: text not null", "name: varchar(50) not null").replace("age >= 18", "age >= 6")).unwrap();
    create(&p, "narrow", None, true).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();
    assert_eq!(count(&ex, "SELECT count(*) FROM adult_names"), 1);
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
}

#[test]
fn views_are_read_back_and_a_changed_definition_is_drift() {
    let (_dir, p, db) = fixture(WITH_SDL_VIEW);
    create(&p, "init", None, false).unwrap();
    let mut ex = SqliteExecutor::open(&db).unwrap();
    apply(&p, &mut ex, &ApplyOptions::default()).unwrap();

    // what certo made reads back as the view it was made from
    let prepared = certo_runner::import_schema(&mut ex).unwrap();
    assert_eq!(prepared.ir.views, certo_sdl::compile(WITH_SDL_VIEW).0.unwrap().views, "{}
{:?}", prepared.sdl, prepared.omissions);
    assert!(prepared.sdl.contains("view adults on people (id, name) where age >= 18"), "{}", prepared.sdl);
    assert!(prepared.omissions.iter().all(|o| !o.contains("adults")), "{:?}", prepared.omissions);
    assert_eq!(prepared.counts.views, 1);

    // a view that is more than a plain select stays a note, with the reason
    ex.connection().execute_batch("CREATE VIEW totals AS SELECT age, count(*) AS n FROM people GROUP BY age").unwrap();
    let prepared = certo_runner::import_schema(&mut ex).unwrap();
    assert_eq!(prepared.ir.views.len(), 1);
    assert!(prepared.omissions.iter().any(|o| o.starts_with("view totals is not represented in the schema:") && o.contains("group")), "{:?}", prepared.omissions);
    ex.connection().execute_batch("DROP VIEW totals").unwrap();

    // a definition changed by hand is drift, not just a view that is there
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());
    ex.connection().execute_batch("DROP VIEW adults; CREATE VIEW adults AS SELECT id, name FROM people WHERE age >= 21").unwrap();
    let d = drift::check(&p, &mut ex).unwrap();
    assert!(!d.in_sync());
    assert!(d.items.iter().any(|i| i.text.contains("view adults") && i.text.contains("differs")), "{:?}", d.items);
    // and the repair puts it right
    let sql = d.repair_sql(certo_sql::Dialect::Sqlite).unwrap();
    ex.connection().execute_batch(&sql).unwrap();
    assert!(drift::check(&p, &mut ex).unwrap().in_sync());

    // adopting a database with such a view makes it an SDL view of the new project
    drop(ex);
    let (dir2, p2, db2) = fixture("");
    {
        let legacy = SqliteExecutor::open(&db2).unwrap();
        legacy
            .connection()
            .execute_batch("CREATE TABLE people (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, age INTEGER); CREATE VIEW adults AS SELECT id, name FROM people WHERE age >= 18")
            .unwrap();
    }
    let _keep = &dir2;
    let mut ex2 = SqliteExecutor::open(&db2).unwrap();
    let report = adopt(&p2, &mut ex2, &AdoptOptions::default()).unwrap();
    assert_eq!(report.adopted.views, 1);
    assert!(fs::read_to_string(p2.schema_path()).unwrap().contains("view adults on people (id, name) where age >= 18"));
    assert!(drift::check(&p2, &mut ex2).unwrap().in_sync());
}
