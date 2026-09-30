//! Runs SQLite-lowered scripts on a real (in-memory) SQLite: a fresh schema,
//! then schema evolution with data that must survive, checked by comparing the
//! evolved database's structure with a fresh database created from the new schema.

use certo_mdl::diff;
use certo_sdl::{compile, SchemaIR};
use certo_sql::{lower_batches_with, render_with, Dialect, Schemas};
use rusqlite::Connection;

fn ir(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    ir.unwrap_or_else(|| panic!("schema failed: {d:?}"))
}

fn script(old: &SchemaIR, new: &SchemaIR) -> String {
    let plan = diff(old, new);
    render_with(&plan, Dialect::Sqlite, Schemas { old, new }).unwrap_or_else(|e| panic!("{e}"))
}

fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", "ON").unwrap();
    c
}

/// Apply like a runner would: batches in order, transactional ones atomically.
fn apply(c: &mut Connection, old: &SchemaIR, new: &SchemaIR) {
    let plan = diff(old, new);
    for b in lower_batches_with(&plan, Dialect::Sqlite, Schemas { old, new }).unwrap_or_else(|e| panic!("{e}")) {
        let sql = b.statements.join("\n");
        if b.transactional {
            let tx = c.transaction().unwrap();
            tx.execute_batch(&sql).unwrap_or_else(|e| panic!("{e}\n{sql}"));
            tx.commit().unwrap();
        } else {
            c.execute_batch(&sql).unwrap_or_else(|e| panic!("{e}\n{sql}"));
        }
    }
}

fn q(c: &Connection, sql: &str) -> Vec<String> {
    let mut st = c.prepare(sql).unwrap();
    let n = st.column_count();
    st.query_map([], |r| {
        Ok((0..n)
            .map(|i| match r.get_ref(i).unwrap() {
                rusqlite::types::ValueRef::Null => "NULL".to_string(),
                rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                rusqlite::types::ValueRef::Real(v) => v.to_string(),
                rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
                rusqlite::types::ValueRef::Blob(_) => "<blob>".into(),
            })
            .collect::<Vec<_>>()
            .join("|"))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

/// Structure that must match between an evolved and a fresh database.
fn shape(c: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for t in q(c, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name") {
        for r in q(c, &format!("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('{t}') ORDER BY cid")) {
            out.push(format!("{t}: col {r}"));
        }
        for r in q(c, &format!("SELECT \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list('{t}') ORDER BY \"from\"")) {
            out.push(format!("{t}: fk {r}"));
        }
        for r in q(c, &format!("SELECT name, \"unique\", origin FROM pragma_index_list('{t}') WHERE origin <> 'pk' ORDER BY name")) {
            // auto-index names differ in numbering only when unnamed; ours are named
            out.push(format!("{t}: idx {r}"));
        }
    }
    out
}

fn assert_healthy(c: &Connection) {
    assert_eq!(q(c, "PRAGMA integrity_check"), ["ok"]);
    assert!(q(c, "PRAGMA foreign_key_check").is_empty(), "foreign key violations after migration");
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
fn a_fresh_schema_runs_and_enforces_its_rules() {
    let (old, new) = (SchemaIR::empty(), ir(V2));
    let c = db();
    c.execute_batch(&script(&old, &new)).unwrap();
    assert_healthy(&c);

    c.execute("INSERT INTO customers (name, email) VALUES ('Ann', 'a@x.com')", []).unwrap();
    c.execute("INSERT INTO orders (customer_id, total) VALUES (1, 5)", []).unwrap();
    assert_eq!(q(&c, "SELECT id, status, note, paid FROM orders"), ["1|new|none|0"]);
    assert_eq!(q(&c, "SELECT length(joined) FROM customers"), ["19"], "now() default");

    let bad = |sql: &str| c.execute(sql, []).expect_err(sql).to_string();
    assert!(bad("INSERT INTO orders (customer_id, status) VALUES (1, 'bogus')").contains("CHECK"), "enum values are checked");
    assert!(bad("INSERT INTO orders (customer_id, total) VALUES (1, -1)").contains("total_positive"));
    assert!(bad("INSERT INTO orders (customer_id) VALUES (99)").contains("FOREIGN KEY"));
    assert!(bad("INSERT INTO customers (name, email) VALUES ('Bo', 'a@x.com')").contains("UNIQUE"));
    // on delete cascade came through
    c.execute("DELETE FROM customers WHERE id = 1", []).unwrap();
    assert_eq!(q(&c, "SELECT count(*) FROM orders"), ["0"]);
}

#[test]
fn evolving_a_schema_keeps_the_rows_and_ends_in_the_same_shape_as_a_fresh_one() {
    let (v1, v2) = (ir(V1), ir(V2));
    let mut c = db();
    apply(&mut c, &SchemaIR::empty(), &v1);
    c.execute_batch(
        "INSERT INTO customers (name, legacy) VALUES ('Ann', 'x'), ('Bob', NULL);
         INSERT INTO orders (customer_id, status, note, total) VALUES (1, 'paid', NULL, 10), (2, 'new', 'hi', NULL);",
    )
    .unwrap();

    apply(&mut c, &v1, &v2);
    assert_healthy(&c);

    // rows survived; new NOT NULL column took its default; NULL note took its default
    assert_eq!(q(&c, "SELECT id, name, email FROM customers ORDER BY id"), ["1|Ann|NULL", "2|Bob|NULL"]);
    assert_eq!(
        q(&c, "SELECT id, customer_id, status, note, total, paid FROM orders ORDER BY id"),
        ["1|1|paid|none|10|0", "2|2|new|hi|NULL|0"]
    );
    assert_eq!(q(&c, "SELECT count(*) FROM customers WHERE joined IS NOT NULL"), ["2"]);
    // the new rules are live
    c.execute("INSERT INTO orders (customer_id, status) VALUES (1, 'shipped')", []).expect("new enum value");
    assert!(c.execute("INSERT INTO orders (customer_id, total) VALUES (1, -5)", []).is_err());
    c.execute("DELETE FROM customers WHERE id = 2", []).unwrap();
    assert_eq!(q(&c, "SELECT count(*) FROM orders WHERE customer_id = 2"), ["0"], "on delete cascade");
    // the dropped column is gone; ids keep counting
    assert!(c.prepare("SELECT legacy FROM customers").is_err());
    c.execute("INSERT INTO customers (name) VALUES ('Cy')", []).unwrap();
    assert_eq!(q(&c, "SELECT max(id) FROM customers"), ["3"]);

    let fresh = db();
    fresh.execute_batch(&script(&SchemaIR::empty(), &v2)).unwrap();
    assert_eq!(shape(&c), shape(&fresh));
}

#[test]
fn small_changes_use_alter_table_and_only_hard_ones_rebuild() {
    let v1 = ir("table t { id: serial primary key  a: text }");
    let v2 = ir("table t { id: serial primary key  a: text  b: int default 0  c: text }");
    let s = script(&v1, &v2);
    assert!(s.contains("ALTER TABLE \"t\" ADD COLUMN \"b\" INTEGER DEFAULT 0"), "{s}");
    assert!(!s.contains("__certo_new_"), "no rebuild needed:\n{s}");
    assert!(!s.contains("foreign_keys"));

    let v3 = ir("table t { id: serial primary key  a: text not null }");
    let s = script(&v1, &v3);
    assert!(s.contains("__certo_new_t"), "{s}");
    assert!(s.starts_with("PRAGMA foreign_keys = OFF;\nBEGIN;"), "{s}");
    assert!(s.ends_with("COMMIT;\nPRAGMA foreign_keys = ON;"), "{s}");
}

#[test]
fn removing_an_enum_value_rebuilds_and_refuses_rows_that_still_use_it() {
    let (a, b) = (ir("enum E { x, y, z }\ntable t { id: serial primary key  e: E }"), ir("enum E { x, y }\ntable t { id: serial primary key  e: E }"));
    let mut c = db();
    apply(&mut c, &SchemaIR::empty(), &a);
    c.execute("INSERT INTO t (e) VALUES ('x')", []).unwrap();
    apply(&mut c, &a, &b);
    assert!(c.execute("INSERT INTO t (e) VALUES ('z')", []).is_err());
    assert_eq!(q(&c, "SELECT e FROM t"), ["x"]);

    // a row still using the removed value stops the migration, and the transaction rolls it back
    let mut c = db();
    apply(&mut c, &SchemaIR::empty(), &a);
    c.execute("INSERT INTO t (e) VALUES ('z')", []).unwrap();
    let plan = diff(&a, &b);
    let batches = lower_batches_with(&plan, Dialect::Sqlite, Schemas { old: &a, new: &b }).unwrap();
    c.execute_batch(&batches[0].statements.join("\n")).unwrap();
    let tx = c.transaction().unwrap();
    assert!(tx.execute_batch(&batches[1].statements.join("\n")).is_err());
    drop(tx);
    assert_eq!(q(&c, "SELECT e FROM t"), ["z"], "rolled back, data intact");
}

#[test]
fn what_sqlite_cannot_express_is_refused_with_a_reason() {
    let refuse = |src: &str| {
        let new = ir(src);
        let old = SchemaIR::empty();
        render_with(&diff(&old, &new), Dialect::Sqlite, Schemas { old: &old, new: &new }).unwrap_err().to_string()
    };
    assert!(refuse("sequence s").contains("no sequences"), "{}", refuse("sequence s"));
    assert!(refuse("type A { x: text }\ntable t { id: int primary key  a: A }").contains("composite"));
    assert!(refuse("table t { id: int primary key  n: serial }").contains("single-column integer primary key"));
    // the schema-free entry points say what is missing rather than guessing
    let plan = diff(&SchemaIR::empty(), &ir("table t { id: int primary key }"));
    assert!(certo_sql::render(&plan, Dialect::Sqlite).unwrap_err().to_string().contains("schemas"));
}

#[test]
fn defaults_and_types_map_to_sqlite() {
    let new = ir("table t {
        id: uuid primary key default gen_uuid()
        b: bool default true
        d: date default today()
        j: json
        x: bytes
        n: decimal(10,2) default 1.50
        v: varchar(20) default \"hi\"
    }");
    let s = script(&SchemaIR::empty(), &new);
    let c = db();
    c.execute_batch(&s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    c.execute("INSERT INTO t DEFAULT VALUES", []).unwrap();
    assert_eq!(q(&c, "SELECT length(id), b, length(d), n, v FROM t"), ["36|1|10|1.5|hi"]);
    assert_eq!(q(&c, "SELECT substr(id, 15, 1) FROM t"), ["4"], "version-4 uuid");
}
