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
table categories {
    id: serial primary key
    parent_id: int references categories
    name: text not null
    note: text
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
            (1, 10, 'new', 2, 1), (1, 25.5, 'paid', 1, 0), (2, 5, 'new', 3, 0);
         INSERT INTO categories (parent_id, name, note) VALUES (NULL, 'root', NULL), (1, 'a', 'n'), (1, 'b', NULL), (2, 'a1', NULL);",
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

#[test]
fn subqueries_and_multi_row_inserts_run_on_sqlite() {
    let mut db = db();

    // in / not in / exists / not exists, correlated and not
    let q = db.stmt("query with_orders() { from customers c where c.id in (from orders o select o.customer_id) select c.name order by c.name }");
    assert_eq!(db.run(&q, &[]), [["Ann"], ["Bob"]]);
    let q = db.stmt("query without() { from customers c where c.id not in (from orders o select o.customer_id) select c.name }");
    assert_eq!(db.run(&q, &[]), [["Cy"]]);
    let q = db.stmt("query big(min: decimal(10,2)) { from customers c
        where exists (from orders o where o.customer_id == c.id and o.total >= :min select 1) select c.name order by c.name }");
    assert_eq!(db.run(&q, &[("min", Value::Real(20.0))]), [["Ann"]]);
    let q = db.stmt("query idle() { from customers c where not exists (from orders o where o.customer_id == c.id select o.id) select c.name }");
    assert_eq!(db.run(&q, &[]), [["Cy"]]);

    // scalar subqueries: an aggregate is one row even when nothing matches; limit 1 may find nothing
    let q = db.stmt("query counts() { from customers c select c.name,
        (from orders o where o.customer_id == c.id select count(*)) as n,
        (from orders o where o.customer_id == c.id select max(o.total)) as biggest,
        (from orders o where o.customer_id == c.id select o.status order by o.id limit 1) as first_status
        order by c.id }");
    assert_eq!(db.run(&q, &[]), [["Ann", "2", "25.5", "new"], ["Bob", "1", "5", "new"], ["Cy", "0", "NULL", "NULL"]]);
    let q = db.stmt("query above_avg() { from orders o where o.total > (from orders p select avg(p.total)) select o.id }");
    assert_eq!(db.run(&q, &[]), [["2"]]);

    // a grouped query using its grouped column inside a subquery
    let q = db.stmt("query per() { from customers c group by c.id, c.name
        select c.name, (from orders o where o.customer_id == c.id select count(*)) as n order by c.name }");
    assert_eq!(db.run(&q, &[]), [["Ann", "2"], ["Bob", "1"], ["Cy", "0"]]);

    // mutations with subqueries
    let m = db.stmt("update pay_admins() { orders o set paid = true where o.customer_id in (from customers c where c.role == \"admin\" select c.id) }");
    assert_eq!(db.exec(&m, &[]), 2);
    let m = db.stmt("delete drop_idle() { from customers c where not exists (from orders o where o.customer_id == c.id select 1) returning c.name }");
    assert_eq!(db.run(&m, &[]), [["Cy"]]);

    // multi-row insert with returning, and an upsert across rows
    let m = db.stmt("insert many(a: text, b: text) { into customers (name, email) values (:a, \"a2@x.com\"), (:b, \"b2@x.com\") returning id, name }");
    let rows = db.run(&m, &[("a", t("Dee")), ("b", t("Eve"))]);
    assert_eq!(rows.iter().map(|r| r[1].as_str()).collect::<Vec<_>>(), ["Dee", "Eve"]);
    let m = db.stmt("insert upsert(a: text, b: text) { into customers (name, email) values (:a, \"a2@x.com\"), (:b, \"z@x.com\")
        on conflict (email) do update set name = excluded.name returning name }");
    let rows = db.run(&m, &[("a", t("Dee2")), ("b", t("Zed"))]);
    assert_eq!(rows.len(), 2);
    assert_eq!(db.c.query_row("SELECT name FROM customers WHERE email = 'a2@x.com'", [], |r| r.get::<_, String>(0)).unwrap(), "Dee2");

    // insert ... select, plain and with an upsert (SQLite needs the WHERE to parse it)
    db.c.execute_batch("CREATE TABLE archive (id INTEGER PRIMARY KEY, total NUMERIC NOT NULL);").unwrap();
    let (ir, _) = compile_sdl("table archive { id: int primary key  total: decimal not null }
table orders { id: serial primary key  customer_id: int not null  total: decimal(10,2) not null  status: text not null  paid: bool not null default false }");
    let mut db2 = Db { c: db.c, schema: ir.unwrap() };
    let m = db2.stmt("insert snapshot() { into archive (id, total) from orders o where o.total > 6 select o.id, o.total returning id }");
    assert_eq!(db2.run(&m, &[]), [["1"], ["2"]]);
    let m = db2.stmt("insert again() { into archive (id, total) from orders o select o.id, o.total on conflict (id) do nothing }");
    assert!(m.sql().contains("WHERE TRUE"), "{}", m.sql());
    db2.exec(&m, &[]);
    assert_eq!(db2.c.query_row("SELECT count(*) FROM archive", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
}

#[test]
fn window_functions_run_on_sqlite() {
    let mut db = db();
    let q = db.stmt("query ranked() { from orders o select o.id,
        row_number() over (partition by o.customer_id order by o.total desc) as rn,
        sum(o.total) over (partition by o.customer_id) as cust_total,
        lag(o.total) over (order by o.id) as prev,
        count(*) over () as n
        order by o.id }");
    assert_eq!(
        db.run(&q, &[]),
        [["1", "2", "35.5", "NULL", "3"], ["2", "1", "35.5", "10", "3"], ["3", "1", "5", "25.5", "3"]]
    );
    // ranking groups by an aggregate
    let q = db.stmt("query top() { from orders o group by o.customer_id
        select o.customer_id, sum(o.total) as total, rank() over (order by sum(o.total) desc) as r order by r }");
    assert_eq!(db.run(&q, &[]), [["1", "35.5", "1"], ["2", "5", "2"]]);
    let q = db.stmt("query buckets(n: int) { from orders o select o.id, ntile(:n) over (order by o.id) as b order by o.id }");
    assert_eq!(db.run(&q, &[("n", i(3))]), [["1", "1"], ["2", "2"], ["3", "3"]]);
    let q = db.stmt("query firsts() { from orders o select o.id, first_value(o.status) over (partition by o.customer_id order by o.id) as f order by o.id }");
    assert_eq!(db.run(&q, &[]), [["1", "new"], ["2", "new"], ["3", "new"]]);
}

#[test]
fn set_operations_run_on_sqlite() {
    let mut db = db();
    let q = db.stmt("query both() { from customers c select c.id, c.name union all from orders o select o.id, o.status order by id, name }");
    assert_eq!(
        db.run(&q, &[]),
        [["1", "Ann"], ["1", "new"], ["2", "Bob"], ["2", "paid"], ["3", "Cy"], ["3", "new"]]
    );
    let q = db.stmt("query statuses() { from orders o select o.status union from orders p select p.status order by status }");
    assert_eq!(db.run(&q, &[]), [["new"], ["paid"]], "union removes duplicates");
    let q = db.stmt("query have_orders() { from customers c select c.id intersect from orders o select o.customer_id order by id }");
    assert_eq!(db.run(&q, &[]), [["1"], ["2"]]);
    let q = db.stmt("query idle() { from customers c select c.id except from orders o select o.customer_id }");
    assert_eq!(db.run(&q, &[]), [["3"]]);
    // parameters across branches, with ordering and paging over the whole result
    let q = db.stmt("query paged(a: int, b: int, lim: int, off: int) {
        from customers c where c.id >= :a select c.id union all from orders o where o.id >= :b select o.id
        order by id desc limit :lim offset :off }");
    assert_eq!(db.run(&q, &[("a", i(2)), ("b", i(2)), ("lim", i(3)), ("off", i(1))]), [["3"], ["2"], ["2"]]);
    // inside a subquery
    let q = db.stmt("query ids() { from customers c where c.id in (from orders o select o.customer_id union from orders p select p.id) select c.name order by c.name }");
    assert_eq!(db.run(&q, &[]), [["Ann"], ["Bob"], ["Cy"]]);
}

#[test]
fn with_queries_run_on_sqlite() {
    let mut db = db();
    let q = db.stmt("query spenders(min: decimal(10,2)) {
        with spend as (from orders o group by o.customer_id select o.customer_id, sum(o.total) as total)
        from customers c join spend s on s.customer_id == c.id where s.total >= :min
        select c.name, s.total order by s.total desc }");
    assert_eq!(db.run(&q, &[("min", Value::Real(10.0))]), [["Ann", "35.5"]]);
    assert_eq!(db.run(&q, &[("min", Value::Real(1.0))]), [["Ann", "35.5"], ["Bob", "5"]]);

    // chained, reused, and combined with a window function and a union
    let q = db.stmt("query chain() {
        with a as (from orders o select o.id, o.customer_id, row_number() over (partition by o.customer_id order by o.id) as rn),
             b as (from a where a.rn == 1 select a.id)
        from b select b.id order by b.id }");
    assert_eq!(db.run(&q, &[]), [["1"], ["3"]]);
    let q = db.stmt("query ids() { with ids as (from customers c select c.id union from orders o select o.id) from ids i select i.id order by i.id }");
    assert_eq!(db.run(&q, &[]), [["1"], ["2"], ["3"]]);

    // mutations
    let m = db.stmt("update tag() { with paid_ids as (from orders o where o.paid select o.id)
        orders x set status = \"tagged\" where x.id in (from paid_ids p select p.id) }");
    assert_eq!(db.exec(&m, &[]), 1);
    let m = db.stmt("delete gone() { with idle as (from customers c where not exists (from orders o where o.customer_id == c.id select 1) select c.id)
        from customers d where d.id in (from idle i select i.id) returning d.name }");
    assert_eq!(db.run(&m, &[]), [["Cy"]]);
}

#[test]
fn window_frames_run_on_sqlite() {
    let mut db = db();
    // orders: id 1 (cust 1) 10, id 2 (cust 1) 25.5, id 3 (cust 2) 5
    let q = db.stmt("query running() { from orders o select o.id,
        sum(o.total) over (order by o.id rows between unbounded preceding and current row) as running,
        sum(o.total) over (partition by o.customer_id order by o.id rows between unbounded preceding and current row) as per_customer,
        count(*) over (order by o.id rows between 1 preceding and current row) as last_two,
        sum(o.total) over (order by o.id rows between current row and unbounded following) as remaining
        order by o.id }");
    assert_eq!(
        db.run(&q, &[]),
        [["1", "10", "10", "1", "40.5"], ["2", "35.5", "35.5", "2", "30.5"], ["3", "40.5", "5", "2", "5"]]
    );
    // a parameter as the offset
    let q = db.stmt("query recent(n: int) { from orders o select o.id,
        count(*) over (order by o.id rows between :n preceding and current row) as seen order by o.id }");
    assert_eq!(db.run(&q, &[("n", i(0))]), [["1", "1"], ["2", "1"], ["3", "1"]]);
    assert_eq!(db.run(&q, &[("n", i(5))]), [["1", "1"], ["2", "2"], ["3", "3"]]);
    // a frame of only later rows is empty on the last row
    let q = db.stmt("query next_id() { from orders o select o.id,
        first_value(o.id) over (order by o.id rows between 1 following and 2 following) as nxt order by o.id }");
    assert_eq!(db.run(&q, &[]), [["1", "2"], ["2", "3"], ["3", "NULL"]]);
    assert!(q.columns()[1].nullable);
    // range by value, and groups of equal keys
    let q = db.stmt("query near() { from orders o select o.id,
        count(*) over (order by o.total range between 10 preceding and current row) as within_ten order by o.id }");
    assert_eq!(db.run(&q, &[]), [["1", "2"], ["2", "1"], ["3", "1"]]);
    let q = db.stmt("query by_group() { from orders o select o.id,
        count(*) over (order by o.status groups between current row and current row) as peers order by o.id }");
    assert_eq!(db.run(&q, &[]), [["1", "2"], ["2", "1"], ["3", "2"]]);
}

#[test]
fn recursive_queries_run_on_sqlite() {
    let mut db = db();
    // the whole tree below the root, with depths
    let q = db.stmt("query tree() { with recursive tree as (
            from categories c where c.parent_id is null select c.id, c.name, 0 as depth
            union all
            from categories c join tree t on c.parent_id == t.id select c.id, c.name, t.depth + 1 as depth)
        from tree t select t.name, t.depth order by t.depth, t.name }");
    assert_eq!(db.run(&q, &[]), [["root", "0"], ["a", "1"], ["b", "1"], ["a1", "2"]]);
    // from a parameter: the subtree under a given category, and counting it
    let q = db.stmt("query below(root: int) { with recursive sub as (
            from categories c where c.id == :root select c.id
            union all
            from categories c join sub s on c.parent_id == s.id select c.id)
        from sub select count(*) as n }");
    assert_eq!(db.run(&q, &[("root", i(1))]), [["4"]]);
    assert_eq!(db.run(&q, &[("root", i(2))]), [["2"]]);
    // walking up: the ancestors of a node
    let q = db.stmt("query ancestors(leaf: int) { with recursive up as (
            from categories c where c.id == :leaf select c.id, c.parent_id, c.name
            union
            from categories c join up u on c.id == u.parent_id select c.id, c.parent_id, c.name)
        from up u select u.name order by u.id }");
    assert_eq!(db.run(&q, &[("leaf", i(4))]), [["root"], ["a"], ["a1"]]);
    // a nullable column that only a step can make NULL is declared nullable, and is
    let q = db.stmt("query labels() { with recursive t as (
            from categories c where c.parent_id is null select c.id, c.name as label
            union all
            from categories c join t on c.parent_id == t.id select c.id, c.note as label)
        from t select t.id, t.label order by t.id }");
    assert!(q.columns()[1].nullable);
    assert_eq!(db.run(&q, &[]), [["1", "root"], ["2", "n"], ["3", "NULL"], ["4", "NULL"]]);
    // in a mutation
    let m = db.stmt("delete prune(root: int) { with recursive sub as (
            from categories c where c.id == :root select c.id
            union all from categories c join sub s on c.parent_id == s.id select c.id)
        from categories x where x.id in (from sub s select s.id) and x.id <> :root returning x.name }");
    let mut gone: Vec<String> = db.run(&m, &[("root", i(2))]).into_iter().map(|r| r[0].clone()).collect();
    gone.sort();
    assert_eq!(gone, ["a1"]);
}

#[test]
fn concatenation_runs_on_sqlite() {
    let mut db = db();
    // customers: (1, Ann, a@x.com, admin), (2, Bob, NULL, user), (3, Cy, c@x.com, guest)
    let q = db.stmt("query who() { from customers c select c.id,
        c.name || \" <\" || c.email || \">\" as who,
        c.name || \"#\" || c.id as tag,
        c.name || \":\" || c.role as role_tag order by c.id }");
    let rows = db.run(&q, &[]);
    assert_eq!(rows[0], ["1", "Ann <a@x.com>", "Ann#1", "Ann:admin"]);
    assert_eq!(rows[1], ["2", "NULL", "Bob#2", "Bob:user"], "NULL if any part is: Bob has no email");
    assert_eq!(rows[2], ["3", "Cy <c@x.com>", "Cy#3", "Cy:guest"]);
    assert!(q.columns()[1].nullable && !q.columns()[2].nullable);
    // a parameter, and in a filter
    let q = db.stmt("query greet(who: text) { from customers c where c.name || \"!\" == :who select c.id }");
    assert_eq!(db.run(&q, &[("who", t("Bob!"))]), [["2"]]);
    // building paths through a hierarchy
    let q = db.stmt("query paths() { with recursive t as (
            from categories c where c.parent_id is null select c.id, c.name as path
            union all
            from categories c join t on c.parent_id == t.id select c.id, t.path || \"/\" || c.name as path)
        from t select t.path order by t.path }");
    assert_eq!(db.run(&q, &[]), [["root"], ["root/a"], ["root/a/a1"], ["root/b"]]);
}
