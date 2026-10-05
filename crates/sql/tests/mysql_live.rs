//! MySQL lowering run on a real MySQL 8 server: a fresh schema, then schema evolution with data that must
//! survive, checked by comparing the evolved database's structure with a fresh one made from the new schema.
//!
//! Skipped unless `CERTO_TEST_MYSQL_URL` is set (for example `mysql://root@127.0.0.1:54398/certo_test`).
//! WARNING: it drops and recreates the databases `certo_evolved` and `certo_fresh` on that server.

use certo_mdl::{compile_migration, diff};
use certo_sdl::{compile, SchemaIR};
use certo_sql::{lower_batches_with, Dialect, Schemas};
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder};

fn ir(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    ir.unwrap_or_else(|| panic!("schema failed: {d:?}"))
}

fn conn_to(db: &str) -> Option<Conn> {
    let url = std::env::var("CERTO_TEST_MYSQL_URL").ok()?;
    let opts = OptsBuilder::from_opts(Opts::from_url(&url).expect("CERTO_TEST_MYSQL_URL")).db_name(Some(db.to_string()));
    Some(Conn::new(opts).expect("connect"))
}

/// A server connection with `db` recreated empty and selected.
fn fresh_db(db: &str) -> Option<Conn> {
    let mut c = conn_to("mysql")?;
    c.query_drop(format!("DROP DATABASE IF EXISTS {db}")).unwrap();
    c.query_drop(format!("CREATE DATABASE {db}")).unwrap();
    c.query_drop(format!("USE {db}")).unwrap();
    Some(c)
}

fn statements(old: &SchemaIR, new: &SchemaIR) -> Vec<String> {
    let plan = diff(old, new);
    lower_batches_with(&plan, Dialect::Mysql, Schemas { old, new })
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .inspect(|b| assert!(!b.transactional && b.statements.len() == 1, "MySQL batches are one DDL statement each"))
        .flat_map(|b| b.statements)
        .collect()
}

fn run(c: &mut Conn, sql: &[String]) {
    for s in sql {
        c.query_drop(s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
}

fn rows(c: &mut Conn, sql: &str) -> Vec<String> {
    let rs: Vec<mysql::Row> = c.query(sql).unwrap_or_else(|e| panic!("{e}\n{sql}"));
    rs.into_iter()
        .map(|r| {
            (0..r.len())
                .map(|i| match &r[i] {
                    mysql::Value::NULL => "NULL".to_string(),
                    mysql::Value::Bytes(b) => String::from_utf8_lossy(b).into(),
                    other => other.as_sql(true).trim_matches('\'').to_string(),
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect()
}

/// What must match between an evolved and a fresh database (column order aside: ADD COLUMN appends).
fn shape(c: &mut Conn, db: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows(
        c,
        &format!(
            "SELECT table_name, column_name, column_type, is_nullable, IFNULL(column_default, 'NULL'), extra, column_comment, collation_name
             FROM information_schema.columns WHERE table_schema = '{db}' ORDER BY table_name, column_name"
        ),
    ) {
        out.push(format!("col {r}"));
    }
    for r in rows(
        c,
        &format!(
            "SELECT k.table_name, k.column_name, k.referenced_table_name, k.referenced_column_name, r.update_rule, r.delete_rule
             FROM information_schema.key_column_usage k JOIN information_schema.referential_constraints r
               ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name
             WHERE k.table_schema = '{db}' ORDER BY k.table_name, k.column_name"
        ),
    ) {
        out.push(format!("fk {r}"));
    }
    for r in rows(
        c,
        &format!(
            "SELECT table_name, index_name, seq_in_index, column_name, non_unique FROM information_schema.statistics
             WHERE table_schema = '{db}' ORDER BY table_name, index_name, seq_in_index"
        ),
    ) {
        out.push(format!("idx {r}"));
    }
    for r in rows(
        c,
        &format!(
            "SELECT t.table_name, t.constraint_name, k.check_clause FROM information_schema.table_constraints t
             JOIN information_schema.check_constraints k ON k.constraint_schema = t.constraint_schema AND k.constraint_name = t.constraint_name
             WHERE t.table_schema = '{db}' AND t.constraint_type = 'CHECK' ORDER BY t.table_name, t.constraint_name"
        ),
    ) {
        out.push(format!("check {r}"));
    }
    out
}

const V1: &str = r#"
enum Status { new, paid }
table customers {
    id: serial primary key
    name: text not null
    legacy: text
}
table orders {
    id: serial primary key
    customer_id: int not null references customers
    status: Status not null default new
    note: text
    total: int
}
index by_status on orders (status)
"#;

const V2: &str = r#"
enum Status { new, paid, shipped }
table customers {
    id: serial primary key
    name: text not null
    email: varchar(100) unique
    joined: timestamp not null default now()
}
table orders {
    id: serial primary key
    customer_id: int not null references customers on delete cascade
    status: Status not null default new
    note: text not null default "none"
    total: bigint
    paid: bool not null default false
}
constraint total_positive on orders using total >= 0
index by_status on orders (status)
index by_customer on orders (customer_id)
"#;

#[test]
fn a_fresh_schema_runs_and_behaves() {
    let Some(mut c) = fresh_db("certo_behaves") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let schema = ir(V2);
    run(&mut c, &statements(&SchemaIR::empty(), &schema));
    c.query_drop("INSERT INTO customers (name, email) VALUES ('Ann', 'a@x.com')").unwrap();
    c.query_drop("INSERT INTO orders (customer_id, total) VALUES (1, 5)").unwrap();

    // defaults: the enum's, a false boolean, and now()
    assert_eq!(rows(&mut c, "SELECT status, note, paid FROM orders"), ["new|none|0"]);
    assert_eq!(rows(&mut c, "SELECT joined IS NOT NULL FROM customers"), ["1"]);
    // text is compared exactly: 'a@x.com' and 'A@x.com' are different emails, as on PostgreSQL and SQLite
    c.query_drop("INSERT INTO customers (name, email) VALUES ('Bob', 'A@x.com')").unwrap();
    assert!(c.query_drop("INSERT INTO customers (name, email) VALUES ('Cy', 'a@x.com')").is_err(), "unique holds");
    // the enum, the CHECK and the foreign key are enforced
    assert!(c.query_drop("INSERT INTO orders (customer_id, status, total) VALUES (1, 'bogus', 1)").is_err());
    assert!(c.query_drop("INSERT INTO orders (customer_id, total) VALUES (1, -1)").is_err());
    assert!(c.query_drop("INSERT INTO orders (customer_id, total) VALUES (99, 1)").is_err());
    // on delete cascade
    c.query_drop("DELETE FROM customers WHERE id = 1").unwrap();
    assert_eq!(rows(&mut c, "SELECT count(*) FROM orders"), ["0"]);
}

#[test]
fn evolving_keeps_the_data_and_matches_a_fresh_database() {
    let Some(mut evolved) = fresh_db("certo_evolved") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let (v1, v2) = (ir(V1), ir(V2));
    run(&mut evolved, &statements(&SchemaIR::empty(), &v1));
    evolved.query_drop("INSERT INTO customers (name, legacy) VALUES ('Ann', 'x'), ('Bob', NULL)").unwrap();
    evolved.query_drop("INSERT INTO orders (customer_id, status, note, total) VALUES (1, 'paid', NULL, 7), (2, 'new', 'hi', NULL)").unwrap();

    run(&mut evolved, &statements(&v1, &v2));
    assert_eq!(rows(&mut evolved, "SELECT id, name FROM customers ORDER BY id"), ["1|Ann", "2|Bob"]);
    // the NULL note was filled with the new default; the rest kept
    assert_eq!(rows(&mut evolved, "SELECT id, status, note, total, paid FROM orders ORDER BY id"), ["1|paid|none|7|0", "2|new|hi|NULL|0"]);
    evolved.query_drop("INSERT INTO orders (customer_id, status, total) VALUES (1, 'shipped', 1)").unwrap();

    let mut f = fresh_db("certo_fresh").unwrap();
    run(&mut f, &statements(&SchemaIR::empty(), &v2));
    assert_eq!(shape(&mut evolved, "certo_evolved"), shape(&mut f, "certo_fresh"));

    // and back again (a removed enum value must not be in use: MySQL refuses to truncate it)
    evolved.query_drop("DELETE FROM orders WHERE status = 'shipped'").unwrap();
    run(&mut evolved, &statements(&v2, &v1));
    let mut f1 = fresh_db("certo_fresh").unwrap();
    run(&mut f1, &statements(&SchemaIR::empty(), &v1));
    assert_eq!(shape(&mut evolved, "certo_evolved"), shape(&mut f1, "certo_fresh"));
}

#[test]
fn views_renames_and_enum_remaps() {
    let Some(mut c) = fresh_db("certo_views") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let before = ir("enum Role { admin, user, guest }
        table people { id: serial primary key  name: varchar(50) not null  age: int  role: Role not null default user }
        table pets { id: serial primary key  owner: int not null references people  nick: varchar(50) unique }
        view adults on people (id, name) where age >= 18");
    run(&mut c, &statements(&SchemaIR::empty(), &before));
    c.query_drop("INSERT INTO people (name, age, role) VALUES ('a', 30, 'guest'), ('b', 5, 'user')").unwrap();
    c.query_drop("INSERT INTO pets (owner, nick) VALUES (1, 'rex')").unwrap();
    assert_eq!(rows(&mut c, "SELECT name FROM adults"), ["a"]);

    // rename a table and a column, remove an enum value (rows move to another), change a column the view reads
    let after = ir("enum Role { admin, user }
        table humans { id: serial primary key  name: varchar(80) not null  age: int  role: Role not null default user }
        table pets { id: serial primary key  owner: int not null references humans  label: varchar(50) unique }
        view adults on humans (id, name) where age >= 21");
    let (plan, d) = compile_migration(
        &before,
        &after,
        "rename table people -> humans\nrename column pets.nick -> label\nremap Role.guest -> user",
    );
    let plan = plan.unwrap_or_else(|| panic!("{d:?}"));
    let batches = lower_batches_with(&plan, Dialect::Mysql, Schemas { old: &before, new: &after }).unwrap_or_else(|e| panic!("{e}"));
    run(&mut c, &batches.into_iter().flat_map(|b| b.statements).collect::<Vec<_>>());
    assert_eq!(rows(&mut c, "SELECT name, role FROM humans ORDER BY id"), ["a|user", "b|user"]);
    assert_eq!(rows(&mut c, "SELECT label FROM pets"), ["rex"]);
    assert_eq!(rows(&mut c, "SELECT name FROM adults"), ["a"]);
    assert!(c.query_drop("INSERT INTO humans (name, role) VALUES ('c', 'guest')").is_err(), "the value is gone");
    // the renamed unique and foreign key still work, and later drops find them by their new names
    assert!(c.query_drop("INSERT INTO pets (owner, label) VALUES (1, 'rex')").is_err());
    assert!(c.query_drop("INSERT INTO pets (owner, label) VALUES (99, 'zed')").is_err());
    let gone = ir("enum Role { admin, user }
        table humans { id: serial primary key  name: varchar(80) not null  age: int  role: Role not null default user }
        table pets { id: serial primary key  owner: int not null  label: varchar(50) }");
    run(&mut c, &statements(&after, &gone));
    c.query_drop("INSERT INTO pets (owner, label) VALUES (99, 'rex')").unwrap();
}

#[test]
fn what_mysql_cannot_do_is_refused_with_a_reason() {
    let refused = |old: &str, new: &str| -> String {
        let (old, new) = (if old.is_empty() { SchemaIR::empty() } else { ir(old) }, ir(new));
        lower_batches_with(&diff(&old, &new), Dialect::Mysql, Schemas { old: &old, new: &new }).unwrap_err().to_string()
    };
    assert!(refused("", "table t { id: int primary key  e: text unique }").contains("varchar(n)"));
    assert!(refused("", "table t { id: int primary key  j: json }\nindex i on t (j)").contains("json"));
    assert!(refused("", "table t { id: int generated always }").contains("generated always"));
    assert!(refused("", "sequence s\ntable t { id: int primary key }").contains("sequences"));
    assert!(refused("", "type A { x: int }\ntable t { id: int primary key }").contains("composite"));
}
