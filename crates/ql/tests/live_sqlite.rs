//! QL compiled for SQLite and run on a real (in-memory) SQLite: queries and
//! mutations execute, parameters bind by `param_order`, and no column declared
//! NOT NULL ever contains NULL. (Type checks against the engine are PostgreSQL's
//! job; SQLite is dynamically typed.)

use certo_ql::{compile, Statement};
use certo_sdl::{compile as compile_sdl, SchemaIR};
use certo_sql::{render_with, Dialect, Schemas};
use rusqlite::{types::Value, Connection};

const SCHEMA: &str = r#"
enum Role { admin, user, guest }
table customers {
    id: serial primary key
    name: text not null
    email: varchar(100) unique
    role: Role default user
    balance: decimal(10,2)
}
table orders {
    id: serial primary key
    customer_id: int not null references customers
    total: decimal(10,2) not null
    status: text not null
    qty: smallint not null default 1
    paid: bool not null default false
}
"#;

struct Db {
    c: Connection,
    schema: SchemaIR,
}

fn db() -> Db {
    let (ir, d) = compile_sdl(SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let empty = SchemaIR::empty();
    let c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", "ON").unwrap();
    let plan = certo_mdl::diff(&empty, &schema);
    c.execute_batch(&render_with(&plan, Dialect::Sqlite, Schemas { old: &empty, new: &schema }).unwrap()).unwrap();
    c.execute_batch(
        "INSERT INTO customers (name, email, role, balance) VALUES
            ('Ann', 'a@x.com', 'admin', 100.5), ('Bob', NULL, 'user', NULL), ('Cy', 'c@x.com', 'guest', 0);
         INSERT INTO orders (customer_id, total, status, qty, paid) VALUES
            (1, 10, 'new', 2, 1), (1, 25.5, 'paid', 1, 0), (2, 5, 'new', 3, 0);",
    )
    .unwrap();
    Db { c, schema }
}

impl Db {
    fn stmt(&self, src: &str) -> Statement {
        let (s, d) = compile(&self.schema, src, Dialect::Sqlite);
        s.unwrap_or_else(|| panic!("{src}\n{d:?}")).remove(0)
    }

    /// Run a statement with parameters given by declared name; rows come back as strings.
    fn run(&mut self, s: &Statement, params: &[(&str, Value)]) -> Vec<Vec<String>> {
        let bound: Vec<Value> = s
            .param_order()
            .iter()
            .map(|n| params.iter().find(|(k, _)| k == n).unwrap_or_else(|| panic!("no value for {n}")).1.clone())
            .collect();
        let mut st = self.c.prepare(s.sql()).unwrap_or_else(|e| panic!("{e}\n{}", s.sql()));
        let n = st.column_count();
        let cols = s.columns().to_vec();
        assert_eq!(n, cols.len(), "{}: column count\n{}", s.name(), s.sql());
        let rows: Vec<Vec<Value>> = st
            .query_map(rusqlite::params_from_iter(bound.iter()), |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
            .unwrap_or_else(|e| panic!("{}: {e}\n{}", s.name(), s.sql()))
            .map(|r| r.unwrap())
            .collect();
        for row in &rows {
            for (v, c) in row.iter().zip(&cols) {
                assert!(c.nullable || *v != Value::Null, "{}: `{}` declared NOT NULL but is NULL", s.name(), c.name);
            }
        }
        rows.iter()
            .map(|r| {
                r.iter()
                    .map(|v| match v {
                        Value::Null => "NULL".into(),
                        Value::Integer(i) => i.to_string(),
                        Value::Real(f) => f.to_string(),
                        Value::Text(t) => t.clone(),
                        Value::Blob(_) => "<blob>".into(),
                    })
                    .collect()
            })
            .collect()
    }

    /// Execute a mutation without `returning`; the affected-row count.
    fn exec(&mut self, s: &Statement, params: &[(&str, Value)]) -> usize {
        let bound: Vec<Value> = s.param_order().iter().map(|n| params.iter().find(|(k, _)| k == n).unwrap().1.clone()).collect();
        self.c.execute(s.sql(), rusqlite::params_from_iter(bound.iter())).unwrap_or_else(|e| panic!("{e}\n{}", s.sql()))
    }
}

fn t(s: &str) -> Value { Value::Text(s.into()) }
fn i(n: i64) -> Value { Value::Integer(n) }

#[test]
fn queries_run_on_sqlite() {
    let mut db = db();

    let q = db.stmt("query big() { from orders o join customers c on o.customer_id == c.id
        where o.total >= 6 select c.name, o.total order by o.total desc limit 10 }");
    assert_eq!(db.run(&q, &[]), [["Ann", "25.5"], ["Ann", "10"]]);

    // a left join: the far side is honestly nullable
    let q = db.stmt("query all() { from customers c left join orders o on o.customer_id == c.id
        select c.name, o.id as order_id order by c.id, o.id }");
    let rows = db.run(&q, &[]);
    assert_eq!(rows.last().unwrap(), &["Cy", "NULL"]);

    // aggregates, grouping, having
    let q = db.stmt("query per() { from orders o join customers c on o.customer_id == c.id group by c.name
        having count(*) >= 1 select c.name, count(*) as n, sum(o.total) as total, max(o.status) as last
        order by total desc }");
    assert_eq!(db.run(&q, &[]), [["Ann", "2", "35.5", "paid"], ["Bob", "1", "5", "new"]]);
    let q = db.stmt("query none() { from orders o where o.total > 1000 select count(*) as n, sum(o.total) as s }");
    assert_eq!(db.run(&q, &[]), [["0", "NULL"]]);

    // enum literals, in, like, between, is null, case, coalesce
    let q = db.stmt("query roles() { from customers c where c.role in (\"admin\", \"guest\") select c.name order by c.name }");
    assert_eq!(db.run(&q, &[]), [["Ann"], ["Cy"]]);
    let q = db.stmt("query mid() { from orders o where o.total between 5 and 10.5 and o.status not in (\"paid\") select o.id order by o.id }");
    assert_eq!(db.run(&q, &[]), [["1"], ["3"]]);
    let q = db.stmt("query tiers() { from customers c
        select c.name, coalesce(c.email, \"none\") as email, case when c.balance > 50 then \"rich\" else \"poor\" end as tier
        order by c.id }");
    assert_eq!(db.run(&q, &[])[1], ["Bob", "none", "poor"]);
    let q = db.stmt("query when() { from orders o where o.id == 1 select now() as n, today() as d }");
    assert_eq!(db.run(&q, &[])[0].len(), 2);

    // parameters: numbered by first use, reused, bound by param_order
    let q = db.stmt("query by_qty(n: int, pat: text, lim: int) { from orders o join customers c on o.customer_id == c.id
        where o.qty >= :n and c.name like :pat and o.id >= :n select o.id, c.name order by o.id limit :lim }");
    assert_eq!(q.param_order(), ["n", "pat", "lim"]);
    assert!(q.sql().contains("CAST(?1 AS INTEGER)"), "{}", q.sql());
    assert_eq!(db.run(&q, &[("n", i(1)), ("pat", t("A%")), ("lim", i(10))]), [["1", "Ann"], ["2", "Ann"]]);
    assert_eq!(db.run(&q, &[("n", i(3)), ("pat", t("%")), ("lim", i(10))]), [["3", "Bob"]]);

    // OFFSET needs a LIMIT in SQLite
    let q = db.stmt("query second() { from orders o select o.id order by o.id offset 1 }");
    assert_eq!(db.run(&q, &[]), [["2"], ["3"]]);
    let q = db.stmt("query page() { from orders o select o.id order by o.id limit 1 offset 1 }");
    assert_eq!(db.run(&q, &[]), [["2"]]);
    let q = db.stmt("query statuses() { from orders o select distinct o.status order by o.status }");
    assert_eq!(db.run(&q, &[]), [["new"], ["paid"]]);
}

#[test]
fn mutations_run_on_sqlite() {
    let mut db = db();

    let add = db.stmt("insert add(name: text, email: varchar(100) null) {
        into customers set name = :name, email = :email returning id, name, role }");
    assert_eq!(db.run(&add, &[("name", t("Dee")), ("email", Value::Null)]), [["4", "Dee", "user"]]);
    assert!(db.c.execute(add.sql().replace("?1", "'x'").replace("?2", "'a@x.com'").as_str(), []).is_err(), "unique");

    let up = db.stmt("insert upsert(email: varchar(100), name: text) {
        into customers set email = :email, name = :name
        on conflict (email) do update set name = excluded.name returning id, name }");
    assert_eq!(db.run(&up, &[("email", t("a@x.com")), ("name", t("Ann2"))]), [["1", "Ann2"]]);
    let skip = db.stmt("insert once(email: varchar(100), name: text) {
        into customers set email = :email, name = :name on conflict (email) do nothing returning id }");
    assert!(db.run(&skip, &[("email", t("a@x.com")), ("name", t("No"))]).is_empty());

    let m = db.stmt("update rename(id: int, n: text) { customers c set name = :n where c.id == :id returning c.id, c.name }");
    assert_eq!(db.run(&m, &[("id", i(2)), ("n", t("Robert"))]), [["2", "Robert"]]);
    let m = db.stmt("update bump(id: int) { orders o set qty = o.qty + 1, paid = true where o.id == :id returning o.qty, o.paid }");
    assert_eq!(db.run(&m, &[("id", i(1))]), [["3", "1"]]);
    let m = db.stmt("update clear(id: int) { customers c set balance = null, role = \"guest\" where c.id == :id }");
    assert_eq!(db.exec(&m, &[("id", i(1))]), 1);

    let m = db.stmt("delete drop_orders(cid: int) { from orders o where o.customer_id == :cid returning o.id }");
    assert_eq!(db.run(&m, &[("cid", i(1))]), [["1"], ["2"]]);
    let m = db.stmt("delete wipe() { from orders o all rows }");
    assert_eq!(db.exec(&m, &[]), 1);
}
