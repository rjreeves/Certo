//! QL compiled for MySQL and run on a real MySQL 8 server, next to SQLite: the same queries over the same data must give
//! the same rows on both (numbers compared as numbers), mutations bind and count correctly, and what MySQL cannot do the
//! same way is refused with a reason.
//!
//! Skipped unless `CERTO_TEST_MYSQL_URL` is set (for example `mysql://root@127.0.0.1:54398/certo_test`).
//! WARNING: it drops and recreates the databases `certo_ql_parity` and `certo_ql_mutations` on that server.

use certo_ql::{compile, Statement};
use certo_sdl::{compile as compile_sdl, SchemaIR};
use certo_sql::{render_with, Dialect, Schemas};
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder};

const SCHEMA: &str = r#"
enum Role { admin, user, guest }
table customers {
    id: serial primary key
    name: varchar(50) not null
    email: varchar(100) unique
    role: Role default user
    balance: decimal(10,2)
    born: date
    meta: json
}
table orders {
    id: serial primary key
    customer_id: int not null references customers
    total: decimal(10,2) not null
    status: varchar(20) not null
    qty: smallint not null default 1
    paid: bool not null default false
    shipped: timestamp
}
table categories {
    id: serial primary key
    parent_id: int references categories
    name: varchar(50) not null
    note: varchar(100)
}
"#;

const DATA: &str = r#"
INSERT INTO customers (name, email, role, balance, born, meta) VALUES
    ('Ann', 'a@x.com', 'admin', 100.5, '1990-01-15', '{"plan":"pro","seats":5,"trial":false,"tags":["a","b"]}'),
    ('Bob', NULL, 'user', NULL, NULL, '{"plan":3,"seats":"many","trial":true}'),
    ('Cy',  'c@x.com', 'guest', 0, '2000-06-30', NULL),
    ('Di',  'd@x.com', 'user', 42.25, '1985-12-01', '{"seats":12345678901,"tags":[]}');
INSERT INTO orders (customer_id, total, status, qty, paid, shipped) VALUES
    (1, 10, 'new', 2, 1, '2026-01-02 03:04:05'), (1, 25.5, 'paid', 1, 0, NULL), (2, 5, 'new', 3, 0, NULL),
    (4, 7.75, 'paid', 4, 1, '2026-02-03 00:00:00'), (4, 12, 'new', 1, 1, NULL);
INSERT INTO categories (parent_id, name, note) VALUES (NULL, 'root', NULL), (1, 'a', 'n'), (1, 'b', NULL), (2, 'a1', NULL);
"#;

fn schema() -> SchemaIR {
    let (ir, d) = compile_sdl(SCHEMA);
    ir.unwrap_or_else(|| panic!("{d:?}"))
}

fn mysql_conn(db: &str) -> Option<Conn> {
    let url = std::env::var("CERTO_TEST_MYSQL_URL").ok()?;
    let mut c = Conn::new(OptsBuilder::from_opts(Opts::from_url(&url).expect("url")).db_name(Some("mysql".to_string()))).expect("connect");
    c.query_drop(format!("DROP DATABASE IF EXISTS {db}")).unwrap();
    c.query_drop(format!("CREATE DATABASE {db}")).unwrap();
    c.query_drop(format!("USE {db}")).unwrap();
    let schema = schema();
    let empty = SchemaIR::empty();
    let plan = certo_mdl::diff(&empty, &schema);
    for s in certo_sql::lower_with(&plan, Dialect::Mysql, Schemas { old: &empty, new: &schema }).unwrap_or_else(|e| panic!("{e}")) {
        c.query_drop(&s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
    for s in DATA.split(';').filter(|s| !s.trim().is_empty()) {
        c.query_drop(s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
    Some(c)
}

fn sqlite_conn() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", "ON").unwrap();
    let schema = schema();
    let empty = SchemaIR::empty();
    let plan = certo_mdl::diff(&empty, &schema);
    c.execute_batch(&render_with(&plan, Dialect::Sqlite, Schemas { old: &empty, new: &schema }).unwrap()).unwrap();
    c.execute_batch(DATA).unwrap();
    c
}

/// A parameter value both drivers can bind.
#[derive(Clone)]
enum P {
    Int(i64),
    Text(&'static str),
    Null,
}

fn statement(src: &str, dialect: Dialect) -> Statement {
    let (s, d) = compile(&schema(), src, dialect);
    s.unwrap_or_else(|| panic!("{src}\n{d:?}")).remove(0)
}

fn norm(cell: String) -> String {
    match cell.parse::<f64>() {
        Ok(f) if cell != "NULL" => format!("{f}"),
        _ => cell,
    }
}

fn run_mysql(c: &mut Conn, src: &str, params: &[(&str, P)]) -> Vec<Vec<String>> {
    let s = statement(src, Dialect::Mysql);
    let bound: Vec<mysql::Value> = s
        .param_order()
        .iter()
        .map(|n| match &params.iter().find(|(k, _)| k == n).unwrap_or_else(|| panic!("no value for {n}")).1 {
            P::Int(i) => mysql::Value::Int(*i),
            P::Text(t) => mysql::Value::Bytes(t.as_bytes().to_vec()),
            P::Null => mysql::Value::NULL,
        })
        .collect();
    let rows: Vec<mysql::Row> = c.exec(s.sql(), bound).unwrap_or_else(|e| panic!("{e}\n{src}\n{}", s.sql()));
    assert!(rows.iter().all(|r| r.len() == s.columns().len()), "{}: column count\n{}", s.name(), s.sql());
    rows.into_iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    norm(match &r[i] {
                        mysql::Value::NULL => "NULL".to_string(),
                        mysql::Value::Bytes(b) => String::from_utf8_lossy(b).into(),
                        other => other.as_sql(true).trim_matches('\'').to_string(),
                    })
                })
                .collect()
        })
        .collect()
}

fn run_sqlite(c: &rusqlite::Connection, src: &str, params: &[(&str, P)]) -> Vec<Vec<String>> {
    use rusqlite::types::Value;
    let s = statement(src, Dialect::Sqlite);
    let bound: Vec<Value> = s
        .param_order()
        .iter()
        .map(|n| match &params.iter().find(|(k, _)| k == n).unwrap().1 {
            P::Int(i) => Value::Integer(*i),
            P::Text(t) => Value::Text((*t).into()),
            P::Null => Value::Null,
        })
        .collect();
    let mut st = c.prepare(s.sql()).unwrap_or_else(|e| panic!("{e}\n{}", s.sql()));
    let n = st.column_count();
    let rows: Vec<Vec<Value>> = st
        .query_map(rusqlite::params_from_iter(bound.iter()), |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap_or_else(|e| panic!("{e}\n{src}\n{}", s.sql()))
        .map(|r| r.unwrap())
        .collect();
    rows.into_iter()
        .map(|r| {
            r.into_iter()
                .map(|v| {
                    norm(match v {
                        Value::Null => "NULL".into(),
                        Value::Integer(i) => i.to_string(),
                        Value::Real(f) => f.to_string(),
                        Value::Text(t) => t,
                        Value::Blob(_) => "<blob>".into(),
                    })
                })
                .collect()
        })
        .collect()
}

#[test]
fn mysql_gives_the_rows_sqlite_gives() {
    let Some(mut my) = mysql_conn("certo_ql_parity") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let lite = sqlite_conn();
    let queries: Vec<(&str, Vec<(&str, P)>)> = vec![
        // joins, left joins, ordering, limits
        ("query q() { from orders o join customers c on o.customer_id == c.id where o.total >= 6 select c.name, o.total order by o.total desc, o.id limit 10 }", vec![]),
        ("query q() { from customers c left join orders o on o.customer_id == c.id select c.name, o.id as order_id order by c.id, o.id }", vec![]),
        ("query q(n: int) { from orders o select o.id order by o.id limit :n offset 1 }", vec![("n", P::Int(2))]),
        ("query q() { from orders o select o.id order by o.id offset 3 }", vec![]),
        ("query q() { from orders o select distinct o.status order by o.status }", vec![]),
        // aggregates
        ("query q() { from orders o join customers c on o.customer_id == c.id group by c.name having count(*) >= 1 select c.name, count(*) as n, sum(o.total) as total, max(o.status) as last order by total desc, c.name }", vec![]),
        ("query q() { from orders o where o.total > 1000 select count(*) as n, sum(o.total) as s }", vec![]),
        ("query q() { from orders o select count(*) filter (where o.paid) as paid_n, sum(o.qty) filter (where o.status == \"new\") as new_qty, count(*) as n }", vec![]),
        ("query q() { from orders o group by o.status select o.status, string_agg(o.status, \",\" order by o.id) as ids order by o.status }", vec![]),
        ("query q() { from orders o group by o.status select o.status, sum(o.qty) as qty, min(o.total) as lo order by o.status }", vec![]),
        // predicates and expressions
        ("query q() { from customers c where c.role in (\"admin\", \"guest\") select c.name order by c.name }", vec![]),
        ("query q(r: Role null) { from customers c where :r is null or c.role == :r select c.name order by c.name }", vec![("r", P::Null)]),
        ("query q(r: Role null) { from customers c where :r is null or c.role == :r select c.name order by c.name }", vec![("r", P::Text("user"))]),
        ("query q() { from orders o where o.total between 5 and 10.5 and o.status not in (\"paid\") select o.id order by o.id }", vec![]),
        ("query q() { from customers c where c.name like \"%y\" or c.email is null select c.name order by c.name }", vec![]),
        ("query q() { from customers c select c.name, coalesce(c.email, \"none\") as email, case when c.balance is null then \"?\" when c.balance > 50 then \"big\" else \"small\" end as size order by c.id }", vec![]),
        ("query q(k: int) { from orders o select o.id, o.qty * :k + o.id - :k as x, o.qty / 2 as half order by o.id }", vec![("k", P::Int(3))]),
        // text
        ("query q() { from customers c select c.name || \"-\" || c.name as twice, lower(c.name) as lo, upper(c.name) as up, length(c.name) as n, left(c.name, 1) as l, right(c.name, 1) as r order by c.id }", vec![]),
        ("query q() { from customers c select position(c.name, \"y\") as p, starts_with(c.name, \"C\") as s, trim(c.name) as t order by c.id }", vec![]),
        // dates
        ("query q() { from customers c where c.born is not null select c.name, date_part(\"year\", c.born) as y, date_part(\"month\", c.born) as m, date_part(\"day\", c.born) as d order by c.id }", vec![]),
        ("query q(d: date) { from customers c where c.born is not null select c.name, days_between(c.born, :d) as days, add_days(c.born, 10) as later order by c.id }", vec![("d", P::Text("2001-01-01"))]),
        ("query q() { from orders o where o.shipped is not null select o.id, date_part(\"hour\", o.shipped) as h order by o.id }", vec![]),
        // windows
        ("query q() { from orders o select o.id, row_number() over (order by o.id) as rn, sum(o.total) over (partition by o.status order by o.id) as running, rank() over (order by o.status) as rk order by o.id }", vec![]),
        ("query q() { from orders o select o.id, sum(o.qty) over (order by o.id rows between 1 preceding and current row) as w order by o.id }", vec![]),
        // set operations, subqueries, with
        ("query q() { from customers c select c.name union from categories g select g.name order by name }", vec![]),
        ("query q() { from customers c where exists (from orders o where o.customer_id == c.id and o.paid select o.id) select c.name order by c.name }", vec![]),
        ("query q() { from customers c where c.id in (from orders o where o.total > 8 select o.customer_id) select c.name order by c.name }", vec![]),
        ("query q() { from customers c select c.name, (from orders o where o.customer_id == c.id select count(*)) as n order by c.id }", vec![]),
        ("query q() { with big as (from orders o where o.total > 8 select o.id, o.customer_id) from big b join customers c on b.customer_id == c.id select c.name, b.id order by b.id }", vec![]),
        // json
        ("query q() { from customers c select c.name, json_text(c.meta, \"plan\") as plan, json_int(c.meta, \"seats\") as seats, json_bool(c.meta, \"trial\") as trial, json_has(c.meta, \"tags\") as has order by c.id }", vec![]),
        ("query q() { from customers c select c.name, json_text(c.meta, \"tags\", 1) as second, json_has(c.meta, \"tags\", 0) as first order by c.id }", vec![]),
        // a parameter used more than once
        ("query q(n: int) { from orders o where o.qty >= :n and o.id >= :n select o.id, :n + 1 as next order by o.id }", vec![("n", P::Int(2))]),
    ];
    for (src, params) in &queries {
        let (a, b) = (run_mysql(&mut my, src, params), run_sqlite(&lite, src, params));
        assert_eq!(a, b, "MySQL and SQLite disagree on:\n{src}\nMySQL SQL:\n{}", statement(src, Dialect::Mysql).sql());
    }
}

fn exec_mysql(c: &mut Conn, src: &str, params: &[(&str, P)]) -> u64 {
    let s = statement(src, Dialect::Mysql);
    let bound: Vec<mysql::Value> = s
        .param_order()
        .iter()
        .map(|n| match &params.iter().find(|(k, _)| k == n).unwrap().1 {
            P::Int(i) => mysql::Value::Int(*i),
            P::Text(t) => mysql::Value::Bytes(t.as_bytes().to_vec()),
            P::Null => mysql::Value::NULL,
        })
        .collect();
    c.exec_drop(s.sql(), bound).unwrap_or_else(|e| panic!("{e}\n{src}\n{}", s.sql()));
    c.affected_rows()
}

#[test]
fn mutations_run_on_mysql() {
    let Some(mut c) = mysql_conn("certo_ql_mutations") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    // insert: set, values, select; each reports the rows it added
    assert_eq!(exec_mysql(&mut c, "insert add(n: text, e: text null) { into customers set name = :n, email = :e }", &[("n", P::Text("Ed")), ("e", P::Null)]), 1);
    assert_eq!(exec_mysql(&mut c, "insert add2() { into categories (parent_id, name) values (null, \"x\"), (null, \"y\") }", &[]), 2);
    assert_eq!(exec_mysql(&mut c, "insert copy() { into categories (parent_id, name) from categories g where g.parent_id is null select g.id, g.name }", &[]), 3);
    assert_eq!(exec_mysql(&mut c, "insert bare() { into categories set name = \"z\" }", &[]), 1);
    // update and delete
    assert_eq!(exec_mysql(&mut c, "update bump(n: text, k: int) { customers c set name = :n where c.id == :k }", &[("n", P::Text("Annie")), ("k", P::Int(1))]), 1);
    assert_eq!(exec_mysql(&mut c, "update every() { orders o set qty = o.qty + 1 all rows }", &[]), 5);
    assert_eq!(exec_mysql(&mut c, "delete gone(k: int) { from categories g where g.id >= :k }", &[("k", P::Int(7))]), 4);
    assert_eq!(run_mysql(&mut c, "query q() { from customers c select c.name order by c.id limit 1 }", &[]), [["Annie"]]);
    assert_eq!(run_mysql(&mut c, "query q() { from orders o select sum(o.qty) as q }", &[]), [["16"]]);

    // upsert on the table's only unique key: do nothing, or update from the incoming row
    assert_eq!(exec_mysql(&mut c, "insert up(i: int, n: text) { into categories set id = :i, name = :n on conflict (id) do nothing }", &[("i", P::Int(1)), ("n", P::Text("changed"))]), 0);
    assert_eq!(run_mysql(&mut c, "query q() { from categories g where g.id == 1 select g.name }", &[]), [["root"]]);
    exec_mysql(
        &mut c,
        "insert up(i: int, n: text) { into categories set id = :i, name = :n on conflict (id) do update set name = excluded.name || categories.name }",
        &[("i", P::Int(1)), ("n", P::Text("new-"))],
    );
    assert_eq!(run_mysql(&mut c, "query q() { from categories g where g.id == 1 select g.name }", &[]), [["new-root"]]);
}

#[test]
fn what_mysql_cannot_do_the_same_way_is_refused() {
    let refused = |src: &str| -> Vec<String> {
        let (s, d) = compile(&schema(), src, Dialect::Mysql);
        assert!(s.is_none(), "expected a refusal: {src}");
        d.iter().map(|x| x.code.clone()).collect()
    };
    assert_eq!(refused("insert add(n: text) { into customers set name = :n returning id }"), ["QL270"]);
    assert_eq!(refused("query q() { from orders o select sum(o.qty) over (order by o.id groups between 1 preceding and current row) as w }"), ["QL271"]);
    // customers has two unique keys (id and email), so a conflict on one is not the only way to conflict
    assert_eq!(refused("insert up(i: int) { into customers set id = :i, name = \"x\" on conflict (id) do nothing }"), ["QL272"]);
    assert_eq!(refused("insert up() { into categories (id, name) from categories g select g.id, g.name on conflict (id) do nothing }"), ["QL272"]);
    assert_eq!(refused("delete d() { from categories g where g.id in (from categories h where h.note is null select h.id) }"), ["QL273"]);
    // and the same statements are fine where they can be
    assert!(compile(&schema(), "insert add(n: text) { into customers set name = :n returning id }", Dialect::Postgres).0.is_some());
}
