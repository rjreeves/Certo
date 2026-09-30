//! Live PostgreSQL test for adopting an existing database.
//!
//! Skipped unless `CERTO_TEST_PG_URL` is set. WARNING: it DROPS and recreates
//! the `public` schema (and a `replay` schema) of that database; use a
//! throwaway database only.

use certo_runner::drift;
use certo_runner::migration::create;
use certo_runner::{adopt, apply, AdoptOptions, ApplyOptions, Executor, PgExecutor, Project, RunnerError};
use postgres::{Client, NoTls};

const LEGACY: &str = r#"
CREATE TYPE mood AS ENUM ('happy', 'sad');
CREATE TYPE "Bad Enum" AS ENUM ('a', 'b c');

CREATE TABLE customers (
    id serial PRIMARY KEY,
    email varchar(255) NOT NULL UNIQUE,
    name text NOT NULL DEFAULT 'anon',
    age integer DEFAULT 18 CHECK (age >= 0),
    balance numeric(10,2) DEFAULT 0,
    balance2 numeric DEFAULT 0,
    mood mood DEFAULT 'happy',
    score double precision DEFAULT -1,
    created timestamptz NOT NULL DEFAULT now(),
    updated timestamp DEFAULT now(),
    CONSTRAINT c_name_len CHECK (length(name) > 0),
    CONSTRAINT c_has_email CHECK (email IS NOT NULL)
);

CREATE TABLE orders (
    id bigserial PRIMARY KEY,
    customer_id integer NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    status text DEFAULT 'new' CHECK (status IN ('new', 'paid')),
    total integer NOT NULL DEFAULT 0
);
CREATE INDEX orders_customer ON orders (customer_id);
CREATE INDEX orders_paid ON orders (total) WHERE status = 'paid';
CREATE UNIQUE INDEX customers_name_uq ON customers (name);

CREATE SEQUENCE ticket_seq START 5000 INCREMENT 10 CACHE 5;
CREATE TABLE tickets (
    id bigint PRIMARY KEY DEFAULT nextval('ticket_seq'),
    seq integer GENERATED ALWAYS AS IDENTITY,
    note text
);
CREATE SEQUENCE "Odd Seq" START 7;
CREATE TABLE odd_users (id integer PRIMARY KEY DEFAULT nextval('"Odd Seq"'));

CREATE TABLE "Legacy Table" (id integer PRIMARY KEY);
CREATE TABLE audit (a integer, b integer, PRIMARY KEY (a, b), UNIQUE (b, a));
CREATE TABLE refs (
    id integer PRIMARY KEY,
    ra integer, rb integer,
    FOREIGN KEY (ra, rb) REFERENCES audit (a, b),
    lg integer REFERENCES "Legacy Table" (id)
);
"#;

fn url() -> Option<String> { std::env::var("CERTO_TEST_PG_URL").ok().filter(|u| !u.is_empty()) }

fn texts(d: &drift::Drift) -> Vec<String> {
    d.items.iter().map(|i| format!("{:?}: {}", i.kind, i.text)).collect()
}

#[test]
fn adopting_a_legacy_database_is_faithful_reported_and_replayable() {
    let Some(url) = url() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut raw = Client::connect(&url, NoTls).expect("connect");
    raw.batch_execute("DROP SCHEMA IF EXISTS replay CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE SCHEMA replay;")
        .unwrap();
    raw.batch_execute(LEGACY).unwrap();
    raw.batch_execute(
        "INSERT INTO customers (email, name) VALUES ('a@x.com', 'Ann'), ('b@x.com', 'Bob');
         INSERT INTO orders (customer_id) SELECT id FROM customers",
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let project = Project::init(dir.path(), "postgres").unwrap();
    let mut db = PgExecutor::connect(&url).unwrap();

    // ---- dry run: reports, changes nothing ------------------------------------
    let dry = adopt(&project, &mut db, &AdoptOptions { dry_run: true, force: false }).unwrap();
    assert!(dry.dry_run && dry.migration.is_none());
    assert!(certo_sdl::compile(&dry.schema_sdl).0.is_some(), "the SDL it would write must compile");
    assert!(project.state_ir().unwrap().tables.is_empty());
    assert!(certo_runner::migration::list(&project).unwrap().is_empty());
    assert_eq!(db.introspect().unwrap().ir.tables.len(), 7, "dry run touched nothing");

    // ---- adopt for real ------------------------------------------------------------
    let r = adopt(&project, &mut db, &AdoptOptions::default()).unwrap();
    assert_eq!(r.migration.as_deref(), Some("0001_baseline"));
    let sdl = std::fs::read_to_string(project.schema_path()).unwrap();
    assert_eq!(sdl, r.schema_sdl);
    let ir = certo_sdl::compile(&sdl).0.expect("adopted schema compiles");
    assert_eq!(project.state_ir().unwrap(), ir, "IR.json is exactly what schema.sdl compiles to");

    // what was adopted, translated correctly
    let customers = ir.table("customers").unwrap();
    assert!(customers.column("id").unwrap().primary_key);
    assert_eq!(customers.column("name").unwrap().default, Some(certo_sdl::ExprIR::String { value: "anon".into() }));
    assert_eq!(customers.column("age").unwrap().default, Some(certo_sdl::ExprIR::Number { value: 18 }));
    assert!(matches!(customers.column("mood").unwrap().default, Some(certo_sdl::ExprIR::EnumVariant { ref variant, .. }) if variant == "happy"));
    assert!(matches!(customers.column("created").unwrap().default, Some(certo_sdl::ExprIR::Call { ref func, .. }) if func == "now"));
    assert!(customers.column("balance2").is_some());
    // formerly left out, adopted now that SDL has the vocabulary
    use certo_sdl::{Builtin, ExprIR, TypeIR};
    let email = customers.column("email").expect("varchar(255) is adopted");
    assert_eq!(email.ty, TypeIR::Builtin(Builtin::Varchar(255)));
    assert!(email.unique && !email.nullable);
    assert_eq!(customers.column("balance").unwrap().ty, TypeIR::Builtin(Builtin::Numeric(10, 2)));
    assert_eq!(customers.column("balance").unwrap().default, Some(ExprIR::Number { value: 0 }));
    let updated = customers.column("updated").expect("timestamp without time zone is adopted");
    assert_eq!(updated.ty, TypeIR::Builtin(Builtin::TimestampNaive));
    assert!(matches!(updated.default, Some(ExprIR::Call { ref func, .. }) if func == "now"));
    assert_eq!(customers.column("score").unwrap().default, Some(ExprIR::Number { value: -1 }), "a negative default");

    // serial, identity and sequences (formerly the biggest gap)
    use certo_sdl::Generation;
    assert_eq!(customers.column("id").unwrap().generated, Some(Generation::Serial), "serial key");
    assert!(customers.column("id").unwrap().default.is_none(), "the nextval default is implied by serial");
    assert_eq!(ir.table("orders").unwrap().column("id").unwrap().ty, TypeIR::Builtin(Builtin::BigInt));
    assert_eq!(ir.table("orders").unwrap().column("id").unwrap().generated, Some(Generation::Serial), "bigserial key");
    let tickets = ir.table("tickets").unwrap();
    assert_eq!(tickets.column("seq").unwrap().generated, Some(Generation::Always), "identity column");
    assert_eq!(tickets.column("id").unwrap().default, Some(ExprIR::NextVal { sequence: "ticket_seq".into() }), "shared sequence default");
    assert_eq!(ir.sequences.len(), 1, "only ticket_seq; the ones serial owns belong to their columns");
    let s = &ir.sequences[0];
    assert_eq!((s.name.as_str(), s.start, s.increment, s.cache), ("ticket_seq", 5000, 10, 5));
    // a foreign key to a serial key is an ordinary int column
    assert_eq!(ir.table("orders").unwrap().column("customer_id").unwrap().ty, TypeIR::Builtin(Builtin::Int));
    let names: Vec<_> = customers.constraints.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"c_name_len") && names.contains(&"customers_age_check"), "{names:?}");
    assert!(names.contains(&"c_has_email"), "IS NOT NULL is adopted: {names:?}");
    let status_check = ir.table("orders").unwrap().constraints.iter().find(|c| c.name == "orders_status_check").expect("IN (...) is adopted");
    assert!(matches!(status_check.expr, ExprIR::In { ref list, negated: false, .. } if list.len() == 2));
    let fk = ir.table("orders").unwrap().column("customer_id").unwrap().references.clone().unwrap();
    assert_eq!((fk.table.as_str(), fk.on_delete), ("customers", certo_sdl::ReferentialAction::Cascade));
    assert_eq!(ir.table("audit").unwrap().columns.iter().filter(|c| c.primary_key).count(), 2, "composite key");
    assert_eq!(ir.table("orders").unwrap().indexes[0].name, "orders_customer");

    // what was left out, each with a reason
    let om = r.omissions.join("\n");
    for needle in [
        "orders_paid", "customers_name_uq",
        "Odd Seq",
        "Bad Enum", "Legacy Table",
        "refs_ra_rb_fkey", "audit_b_a_key",
        "foreign key",
    ] {
        assert!(om.contains(needle), "omissions should mention `{needle}`:\n{om}");
    }
    for gone in ["default of customers.id", "default of orders.id", "default of tickets.id", "ticket_seq"] {
        assert!(!om.contains(gone), "`{gone}` is adopted now:\n{om}");
    }
    // the table that used the unspellable sequence keeps its column, minus the default
    assert!(ir.table("odd_users").unwrap().column("id").unwrap().default.is_none());
    assert!(om.contains("default of odd_users.id"), "{om}");
    for adopted in ["customers.email", "customers.balance ", "customers.updated", "customers.score", "c_has_email", "orders_status_check"] {
        assert!(!om.contains(adopted), "`{adopted}` should be adopted now, not omitted:\n{om}");
    }
    assert!(ir.table("Legacy Table").is_none());
    assert!(ir.table("refs").unwrap().column("lg").unwrap().references.is_none(), "key to an unadopted table is dropped");

    // ---- what still differs is exactly what was left out -----------------------------
    let known = r.known_drift.iter().map(|i| i.text.clone()).collect::<Vec<_>>().join("\n");
    for needle in ["table Legacy Table", "enum Bad Enum", "sequence Odd Seq", "column odd_users.id: default"] {
        assert!(known.contains(needle), "known drift should mention `{needle}`:\n{known}");
    }
    for adopted in [
        "customers.name", "customers.age", "customers.mood", "customers.created", "orders.customer_id", "orders.total", "c_name_len",
        "customers.email", "customers.balance", "customers.updated", "customers.score", "c_has_email", "orders_status_check",
        "customers.id", "orders.id", "tickets", "ticket_seq",
    ] {
        assert!(!known.contains(adopted), "`{adopted}` was adopted faithfully and must not show as drift:\n{known}");
    }

    // ---- the data is untouched and the baseline did not run ----------------------------
    let rows = raw.query_one("SELECT count(*) FROM customers", &[]).unwrap().get::<_, i64>(0);
    assert_eq!(rows, 2);
    let hist = raw.query("SELECT seq, name FROM _certo_migrations", &[]).unwrap();
    assert_eq!((hist.len(), hist[0].get::<_, String>(1)), (1, "baseline".to_string()));

    // ---- normal workflow continues on top of the adoption --------------------------------
    std::fs::write(project.schema_path(), format!("{sdl}\ntable notes {{ id: int primary key  body: text }}\n")).unwrap();
    let c = create(&project, "add notes", None, false).unwrap();
    assert_eq!(c.seq, 2);
    let ap = apply(&project, &mut db, &ApplyOptions::default()).unwrap();
    assert_eq!(ap.migrations, ["0002_add_notes"]);
    assert!(raw.query_one("SELECT to_regclass('notes') IS NOT NULL", &[]).unwrap().get::<_, bool>(0));

    // ---- the strongest fidelity check: replay the history on an EMPTY schema -----------
    // The baseline holds real CREATE statements, so migrations alone must rebuild the
    // adopted schema, and introspecting the result must match it exactly.
    drop(db); // the runner lock is per database, so release it before a second runner connects
    let replay_url = format!("{url}?options=-c%20search_path%3Dreplay");
    let mut replay = PgExecutor::connect(&replay_url).unwrap();
    let ap = apply(&project, &mut replay, &ApplyOptions::default()).unwrap();
    assert_eq!(ap.migrations, ["0001_baseline", "0002_add_notes"]);
    let d = drift::check(&project, &mut replay).unwrap();
    assert!(
        d.in_sync(),
        "replaying the migrations on an empty schema must reproduce the adopted schema exactly, got:\n{}",
        texts(&d).join("\n")
    );

    // ---- adopting again is refused -----------------------------------------------------
    drop(replay);
    let mut db = PgExecutor::connect(&url).unwrap();
    let again = adopt(&project, &mut db, &AdoptOptions::default());
    assert!(matches!(again, Err(RunnerError::Project(_))), "{again:?}");
}
