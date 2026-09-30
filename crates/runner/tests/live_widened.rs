//! Live PostgreSQL test for the widened SDL: every new type, literal,
//! operator and function is written in SDL, lowered to SQL, applied to a real
//! server, read back, and required to show NO drift, then exercised.
//!
//! Skipped unless `CERTO_TEST_PG_URL` is set. WARNING: it DROPS and recreates
//! the `public` schema of that database; use a throwaway database only.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{apply, ApplyOptions, Executor, PgExecutor, Project};
use postgres::{Client, NoTls};

const V1: &str = r#"
enum Role { admin, user, guest }

table items {
    id: int primary key
    sm: smallint default -3
    rl: real default 0.5
    naive: timestamp_naive default now()
    code: varchar(20) not null default "x"
    fixed: char(3)
    price: decimal(10,2) default -19.99
    qty: decimal(8) default 0
    born: date default today()
    role: Role default guest
    n: int
    label: text
}

constraint c_not on items using not n == 1 or n is null
constraint c_in on items using role in (admin, user) or n in (1, 2, -3) or label not in ("a", "b")
constraint c_isnn on items using label is not null or n is not null
constraint c_fn on items using length(trim(code)) > 0 and round(price) >= -100 and (nullif(n, 0) is not null or n is null)
constraint c_group on items using not (n > 100 or n < -100)
"#;

fn url() -> Option<String> { std::env::var("CERTO_TEST_PG_URL").ok().filter(|u| !u.is_empty()) }

fn texts(d: &drift::Drift) -> Vec<String> {
    d.items.iter().map(|i| format!("{:?}: {}", i.kind, i.text)).collect()
}

#[test]
fn the_widened_language_lowers_applies_and_reads_back_exactly() {
    let Some(url) = url() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut raw = Client::connect(&url, NoTls).expect("connect");
    raw.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let project = Project::init(dir.path(), "postgres").unwrap();
    std::fs::write(project.schema_path(), V1).unwrap();
    let c = create(&project, "init", None, false).unwrap();
    assert!(!c.destructive);
    let mut db = PgExecutor::connect(&url).unwrap();
    apply(&project, &mut db, &ApplyOptions::default()).expect("the generated SQL is valid PostgreSQL");

    // ---- no false drift for any new construct -------------------------------------
    let d = drift::check(&project, &mut db).unwrap();
    assert!(d.in_sync(), "introspecting what was just applied must match exactly, got:\n{}", texts(&d).join("\n"));
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    let live = db.introspect().unwrap().ir;
    let items = live.table("items").unwrap();
    use certo_sdl::{Builtin, TypeIR};
    let ty = |c: &str| items.column(c).unwrap().ty.clone();
    assert_eq!(ty("sm"), TypeIR::Builtin(Builtin::SmallInt));
    assert_eq!(ty("rl"), TypeIR::Builtin(Builtin::Real));
    assert_eq!(ty("naive"), TypeIR::Builtin(Builtin::TimestampNaive));
    assert_eq!(ty("code"), TypeIR::Builtin(Builtin::Varchar(20)));
    assert_eq!(ty("fixed"), TypeIR::Builtin(Builtin::Char(3)));
    assert_eq!(ty("price"), TypeIR::Builtin(Builtin::Numeric(10, 2)));
    assert_eq!(ty("qty"), TypeIR::Builtin(Builtin::Numeric(8, 0)));
    assert_eq!(items.constraints.len(), 5);

    // ---- the database really behaves as the SDL says --------------------------------
    raw.batch_execute("INSERT INTO items (id, n, label) VALUES (1, 2, 'x')").unwrap();
    let r = raw.query_one("SELECT sm, rl, price::text, qty::text, code, role::text, born = CURRENT_DATE FROM items WHERE id = 1", &[]).unwrap();
    assert_eq!(r.get::<_, i16>(0), -3, "negative smallint default");
    assert_eq!(r.get::<_, f32>(1), 0.5, "real default");
    assert_eq!(r.get::<_, String>(2), "-19.99", "negative decimal default, exact");
    assert_eq!(r.get::<_, String>(3), "0");
    assert_eq!(r.get::<_, String>(4), "x");
    assert_eq!(r.get::<_, String>(5), "guest");
    assert!(r.get::<_, bool>(6), "today() default");

    let rejects = |sql: &str, constraint: &str| {
        let mut c = Client::connect(&url, NoTls).unwrap();
        let e = c.batch_execute(sql).expect_err(sql);
        let msg = e.as_db_error().map(|d| d.message().to_string()).unwrap_or_default();
        assert!(msg.contains(constraint), "`{sql}` should violate {constraint}, got: {msg}");
    };
    rejects("INSERT INTO items (id, n, label) VALUES (2, 1, 'x')", "c_not"); // not n == 1 or n is null
    rejects("INSERT INTO items (id, n, label) VALUES (3, 200, 'x')", "c_group"); // not (n > 100 or ...)
    rejects("INSERT INTO items (id, n, label) VALUES (4, NULL, NULL)", "c_isnn"); // label is not null or n is not null
    rejects("INSERT INTO items (id, code, n, label) VALUES (5, '   ', 2, 'x')", "c_fn"); // length(trim(code)) > 0
    // c_in: role in (admin, user) or n in (1,2,-3) or label not in (a, b)
    raw.batch_execute("INSERT INTO items (id, n, label, role) VALUES (6, 2, 'a', 'guest')").unwrap(); // n in list satisfies it
    rejects("INSERT INTO items (id, n, label, role) VALUES (7, 3, 'a', 'guest')", "c_in");
    raw.batch_execute("INSERT INTO items (id, n, label, role) VALUES (8, 3, 'zzz', 'guest')").unwrap(); // label not in (a, b)

    // ---- evolving a parameterised type and a widened constraint ----------------------
    let v2 = V1
        .replace("code: varchar(20)", "code: varchar(40)")
        .replace("price: decimal(10,2)", "price: decimal(12,2)")
        .replace("n in (1, 2, -3)", "n in (1, 2, -3, -4)");
    std::fs::write(project.schema_path(), &v2).unwrap();
    let c = create(&project, "widen", None, false).unwrap();
    assert!(c.summary.iter().any(|s| s.contains("column items.code")) && c.summary.iter().any(|s| s.contains("check constraint") || s.contains("constraint c_in")), "{:?}", c.summary);
    apply(&project, &mut db, &ApplyOptions::default()).expect("altering parameterised types works");
    let d = drift::check(&project, &mut db).unwrap();
    assert!(d.in_sync(), "after the change:\n{}", texts(&d).join("\n"));
    let r = raw.query_one("SELECT count(*) FROM items", &[]).unwrap();
    assert_eq!(r.get::<_, i64>(0), 3, "existing rows survived the type change");

    // ---- and a hand edit of a widened construct is detected ---------------------------
    raw.batch_execute("ALTER TABLE items DROP CONSTRAINT c_isnn; ALTER TABLE items ADD CONSTRAINT c_isnn CHECK (label IS NULL OR n IS NOT NULL)").unwrap();
    let d = drift::check(&project, &mut db).unwrap();
    assert!(texts(&d).iter().any(|t| t.contains("check constraint c_isnn")), "{:?}", texts(&d));
}
