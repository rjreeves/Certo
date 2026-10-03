//! Live PostgreSQL test for QL: compiled queries are prepared and executed on
//! a real server, and the typed contract QL declares (result column types,
//! parameter types, nullability) is checked against what PostgreSQL reports and
//! what the rows actually contain.
//!
//! Skipped unless `CERTO_TEST_PG_URL` is set. WARNING: it DROPS and recreates
//! the `public` schema of that database; use a throwaway database only.

use certo_ql::{compile, CompiledMutation, CompiledQuery};
use certo_sdl::{compile as compile_sdl, Builtin, SchemaIR, TypeIR};
use certo_sql::Dialect;
use postgres::types::{ToSql, Type};
use postgres::{Client, NoTls};
use serde_json::{json, Value};

const SCHEMA: &str = r#"
enum Role { admin, user, guest }
table customers {
    id: serial primary key
    name: text not null
    email: varchar(100) unique
    role: Role default user
    balance: decimal(10,2)
    born: date
    token: uuid
}
table orders {
    id: serial primary key
    customer_id: int not null references customers
    total: decimal(10,2) not null
    status: text not null
    created: timestamp not null default now()
    shipped: timestamp
    qty: smallint not null default 1
    paid: bool not null default false
}
table items {
    id: bigserial primary key
    order_id: int not null references orders
    sku: text not null
    price: decimal(10,2) not null
    qty: int not null
}
table categories {
    id: serial primary key
    parent_id: int references categories
    name: text not null
    note: text
}
"#;

const DATA: &str = r#"
INSERT INTO customers (name, email, role, balance, born) VALUES
    ('Ann', 'a@x.com', 'admin', 100.50, '1990-01-01'),
    ('Bob', NULL,      'user',  NULL,   NULL),
    ('Cy',  'c@x.com', 'guest', 0,      '2000-06-15');
INSERT INTO orders (customer_id, total, status, qty, paid, shipped) VALUES
    (1, 10.00, 'new',  2, true,  '2026-01-02 00:00:00'),
    (1, 25.50, 'paid', 1, false, NULL),
    (2,  5.00, 'new',  3, false, NULL);
INSERT INTO items (order_id, sku, price, qty) VALUES
    (1, 'A',  4.00, 1), (1, 'B', 3.00, 2), (2, 'A', 25.50, 1);
INSERT INTO categories (parent_id, name, note) VALUES (NULL, 'root', NULL), (1, 'a', 'n'), (1, 'b', NULL), (2, 'a1', NULL);
"#;

fn url() -> Option<String> { std::env::var("CERTO_TEST_PG_URL").ok().filter(|u| !u.is_empty()) }

/// The QL-declared type as the family PostgreSQL reports it.
fn declared(t: &TypeIR) -> String {
    match t {
        TypeIR::Builtin(Builtin::Text | Builtin::Varchar(_) | Builtin::Char(_)) => "text".into(),
        TypeIR::Builtin(Builtin::Decimal | Builtin::Numeric(..)) => "decimal".into(),
        TypeIR::Builtin(b) => b.sdl_name(),
        TypeIR::Enum(n) | TypeIR::Composite(n) => n.clone(),
    }
}

fn reported(t: &Type) -> String {
    match *t {
        Type::INT2 => "smallint".into(),
        Type::INT4 => "int".into(),
        Type::INT8 => "bigint".into(),
        Type::NUMERIC => "decimal".into(),
        Type::FLOAT4 => "real".into(),
        Type::FLOAT8 => "float".into(),
        Type::BOOL => "bool".into(),
        Type::UUID => "uuid".into(),
        Type::TIMESTAMPTZ => "timestamp".into(),
        Type::TIMESTAMP => "timestamp_naive".into(),
        Type::DATE => "date".into(),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR => "text".into(),
        _ => t.name().to_string(), // enums report their own name
    }
}

struct Live {
    client: Client,
    schema: SchemaIR,
}

impl Live {
    fn query(&self, src: &str) -> CompiledQuery {
        let (q, d) = compile(&self.schema, src, Dialect::Postgres);
        q.unwrap_or_else(|| panic!("{src}\n{d:?}")).remove(0).as_query().expect("a query").clone()
    }

    fn mutation(&self, src: &str) -> CompiledMutation {
        let (q, d) = compile(&self.schema, src, Dialect::Postgres);
        q.unwrap_or_else(|| panic!("{src}\n{d:?}")).remove(0).as_mutation().expect("a mutation").clone()
    }

    /// Prepare a mutation and check its declared parameter and `returning` types
    /// against PostgreSQL's.
    fn check_mutation(&mut self, m: &CompiledMutation) {
        let stmt = self.client.prepare(&m.sql).unwrap_or_else(|e| panic!("`{}` is not valid SQL: {e}\n{}", m.ir.name, m.sql));
        assert_eq!(stmt.columns().len(), m.ir.returning.len(), "{}: returning count", m.ir.name);
        for (c, d) in stmt.columns().iter().zip(&m.ir.returning) {
            assert_eq!(c.name(), d.name);
            assert_eq!(reported(c.type_()), declared(&d.ty), "{}: type of returning `{}`", m.ir.name, d.name);
        }
        assert_eq!(stmt.params().len(), m.param_order.len(), "{}: parameter count", m.ir.name);
        for (t, name) in stmt.params().iter().zip(&m.param_order) {
            let p = m.ir.params.iter().find(|p| &p.name == name).unwrap();
            assert_eq!(reported(t), declared(&p.ty), "{}: type of parameter `{name}`", m.ir.name);
        }
    }

    /// Run a mutation and return the affected-row count plus, for `returning`,
    /// the rows as JSON (their declared nullability is checked against the data).
    fn run(&mut self, m: &CompiledMutation, params: &[&(dyn ToSql + Sync)]) -> (u64, Vec<Value>) {
        self.check_mutation(m);
        if m.ir.returning.is_empty() {
            let n = self.client.execute(&m.sql, params).unwrap_or_else(|e| panic!("{}: {e}", m.ir.name));
            return (n, vec![]);
        }
        let wrapped = format!("WITH q AS ({}) SELECT to_jsonb(q)::text FROM q", m.sql);
        let rows: Vec<Value> = self
            .client
            .query(&wrapped, params)
            .unwrap_or_else(|e| panic!("{}: {e}", m.ir.name))
            .iter()
            .map(|r| serde_json::from_str(&r.get::<_, String>(0)).unwrap())
            .collect();
        for row in &rows {
            for d in &m.ir.returning {
                assert!(d.nullable || !row[&d.name].is_null(), "{}: `{}` declared NOT NULL but is NULL", m.ir.name, d.name);
            }
        }
        (rows.len() as u64, rows)
    }

    fn count(&mut self, sql: &str) -> i64 { self.client.query_one(sql, &[]).unwrap().get(0) }

    /// Prepare (validates the SQL) and check the declared contract against PostgreSQL's.
    fn check_contract(&mut self, q: &CompiledQuery) {
        let stmt = self.client.prepare(&q.sql).unwrap_or_else(|e| panic!("`{}` is not valid SQL: {e}\n{}", q.ir.name, q.sql));
        let cols = stmt.columns();
        assert_eq!(cols.len(), q.ir.select.len(), "{}: column count", q.ir.name);
        for (c, d) in cols.iter().zip(&q.ir.select) {
            assert_eq!(c.name(), d.name, "{}: column name", q.ir.name);
            assert_eq!(reported(c.type_()), declared(&d.ty), "{}: type of column `{}` (sql: {})", q.ir.name, d.name, q.sql);
        }
        assert_eq!(stmt.params().len(), q.param_order.len(), "{}: parameter count", q.ir.name);
        for (t, name) in stmt.params().iter().zip(&q.param_order) {
            let p = q.ir.params.iter().find(|p| &p.name == name).unwrap();
            assert_eq!(reported(t), declared(&p.ty), "{}: type of parameter `{name}`", q.ir.name);
        }
    }

    /// Run the query (as JSON rows, so any column type can be compared) and
    /// check the declared nullability against the data.
    fn rows(&mut self, q: &CompiledQuery, params: &[&(dyn ToSql + Sync)]) -> Vec<Value> {
        self.check_contract(q);
        let wrapped = format!("SELECT to_jsonb(q)::text FROM ({}) AS q", q.sql);
        let rows: Vec<Value> = self
            .client
            .query(&wrapped, params)
            .unwrap_or_else(|e| panic!("{}: {e}", q.ir.name))
            .iter()
            .map(|r| serde_json::from_str(&r.get::<_, String>(0)).unwrap())
            .collect();
        for row in &rows {
            for d in &q.ir.select {
                if !d.nullable {
                    assert!(!row[&d.name].is_null(), "{}: column `{}` is declared NOT NULL but a row has NULL: {row}", q.ir.name, d.name);
                }
            }
        }
        rows
    }
}

fn live() -> Option<Live> {
    let url = url()?;
    let mut client = Client::connect(&url, NoTls).expect("connect");
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").unwrap();
    let (ir, d) = compile_sdl(SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let plan = certo_mdl::diff(&SchemaIR::empty(), &schema);
    client.batch_execute(&certo_sql::render(&plan, Dialect::Postgres).unwrap()).unwrap();
    client.batch_execute(DATA).unwrap();
    Some(Live { client, schema })
}

#[test]
fn compiled_queries_run_and_their_declared_contract_is_true() {
    let Some(mut db) = live() else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };

    // ---- join, filter, order, limit ------------------------------------------
    let q = db.query("query big_orders() {
        from orders o join customers c on o.customer_id == c.id
        where o.total >= 6
        select c.name, o.total order by o.total desc limit 10
    }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Ann", "total": 25.5}), json!({"name": "Ann", "total": 10.0})]);

    // ---- a left join: the far side is honestly nullable ----------------------------
    let q = db.query("query all_customers() {
        from customers c left join orders o on o.customer_id == c.id
        select c.name, o.id as order_id, o.total as total order by c.id, o.id
    }");
    assert!(q.ir.select[1].nullable && q.ir.select[2].nullable && !q.ir.select[0].nullable);
    let rows = db.rows(&q, &[]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3], json!({"name": "Cy", "order_id": null, "total": null}), "Cy has no orders");
    assert!(rows[..3].iter().all(|r| !r["order_id"].is_null()));

    // ---- aggregates and grouping: types match PostgreSQL's exactly ---------------------
    let q = db.query("query per_customer() {
        from orders o join customers c on o.customer_id == c.id
        group by c.name
        having count(*) >= 1
        select c.name, count(*) as n, sum(o.total) as total, sum(o.qty) as qty, avg(o.qty) as avg_qty,
               min(o.created) as first, max(o.status) as last_status, count(distinct o.status) as statuses
        order by total desc
    }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows[0]["name"], "Ann");
    assert_eq!((rows[0]["n"].clone(), rows[0]["total"].clone(), rows[0]["qty"].clone()), (json!(2), json!(35.5), json!(3)));
    assert_eq!(rows[0]["statuses"], 2);
    assert_eq!(rows[1]["name"], "Bob");
    assert_eq!(rows[1]["avg_qty"].as_f64().unwrap(), 3.0);
    // a whole-table aggregate over no rows: COUNT is 0 (never NULL), SUM is NULL (declared nullable)
    let q = db.query("query none() { from orders o where o.total > 1000 select count(*) as n, sum(o.total) as s }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows, [json!({"n": 0, "s": null})]);
    assert!(!q.ir.select[0].nullable && q.ir.select[1].nullable);
    // sum(bigint) is numeric
    let q = db.query("query s() { from items i select sum(i.id) as x }");
    db.rows(&q, &[]);

    // ---- case / coalesce / functions ---------------------------------------------------
    let q = db.query("query tiers() {
        from customers c
        select c.name,
               coalesce(c.email, \"none\") as email,
               case when c.balance > 50 then \"rich\" else \"poor\" end as tier,
               upper(c.name) as shout, length(c.name) as len, lower(c.email) as lower_email
        order by c.id
    }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows[1], json!({"name": "Bob", "email": "none", "tier": "poor", "shout": "BOB", "len": 3, "lower_email": null}));
    assert_eq!(rows[0]["tier"], "rich");
    assert!(!q.ir.select[1].nullable && !q.ir.select[2].nullable && q.ir.select[5].nullable);

    // ---- arithmetic and numeric functions ------------------------------------------------
    let q = db.query("query math() {
        from orders o select o.id, o.total * 2 as double_total, o.qty + 1 as q1, abs(o.total - 20) as away, round(o.total) as rounded
        order by o.id
    }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows[1]["double_total"], 51.0);
    assert_eq!(rows[1]["rounded"], 26.0); // 25.50 rounds half away from zero
    assert_eq!(rows[0]["away"], 10.0);

    // ---- predicates: enum literals, in, like, between, is null, not -------------------
    let q = db.query("query admins() { from customers c where c.role == \"admin\" select c.name }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Ann"})]);
    let q = db.query("query some_roles() { from customers c where c.role in (\"admin\", \"guest\") select c.name order by c.name }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Ann"}), json!({"name": "Cy"})]);
    let q = db.query("query no_email() { from customers c where c.email is null select c.name }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Bob"})]);
    let q = db.query("query has_email() { from customers c where c.email is not null and not c.name like \"C%\" select c.name }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Ann"})]);
    let q = db.query("query mid() { from orders o where o.total between 5 and 10.5 and o.status not in (\"paid\") select o.id order by o.id }");
    assert_eq!(db.rows(&q, &[]), [json!({"id": 1}), json!({"id": 3})]);
    let q = db.query("query born() { from customers c where c.born > \"1995-01-01\" select c.name }");
    assert_eq!(db.rows(&q, &[]), [json!({"name": "Cy"})]);

    // ---- parameters: numbered by first use, reused, cast to their declared types -----------
    let q = db.query("query by_qty(n: int, pat: text, lim: int) {
        from orders o join customers c on o.customer_id == c.id
        where o.qty >= :n and c.name like :pat and o.id >= :n
        select o.id, c.name order by o.id limit :lim
    }");
    assert_eq!(q.param_order, ["n", "pat", "lim"]);
    // n = 1 matches both of Ann's orders; the pattern excludes Bob's
    let rows = db.rows(&q, &[&1i32, &"A%", &10i32]);
    assert_eq!(rows, [json!({"id": 1, "name": "Ann"}), json!({"id": 2, "name": "Ann"})]);
    // `n` is used twice but bound once
    assert_eq!(db.rows(&q, &[&3i32, &"%", &10i32]), [json!({"id": 3, "name": "Bob"})]);
    assert_eq!(db.rows(&q, &[&1i32, &"%", &1i32]).len(), 1, "limit as a parameter");
    // enum, decimal and nullable parameters: prepare checks their declared types against PostgreSQL's
    let q = db.query("query typed(r: Role, min: decimal(10,2), since: timestamp null, d: date, who: uuid, s: varchar(20)) {
        from customers c join orders o on o.customer_id == c.id
        where c.role == :r and o.total >= :min and o.created > :since and c.born < :d and c.token == :who and c.name == :s
        select c.id
    }");
    db.check_contract(&q);

    // ---- distinct / offset / having ------------------------------------------------------
    let q = db.query("query statuses() { from orders o select distinct o.status order by o.status }");
    assert_eq!(db.rows(&q, &[]), [json!({"status": "new"}), json!({"status": "paid"})]);
    let q = db.query("query second() { from orders o select o.id order by o.id limit 1 offset 1 }");
    assert_eq!(db.rows(&q, &[]), [json!({"id": 2})]);
    let q = db.query("query busy() { from orders o group by o.customer_id having count(*) > 1 select o.customer_id, count(*) as n }");
    assert_eq!(db.rows(&q, &[]), [json!({"customer_id": 1, "n": 2})]);
    let q = db.query("query when() { from orders o where o.id == 1 select now() as n, today() as d }");
    db.rows(&q, &[]);

    // ==== mutations ================================================================
    // insert, with defaults and a nullable parameter; the returning contract is true
    let m = db.mutation("insert add(name: text, email: varchar(100) null) {
        into customers set name = :name, email = :email returning id, name, role, email }");
    let (_, rows) = db.run(&m, &[&"Dee", &None::<String>]);
    assert_eq!(rows, [json!({"id": 4, "name": "Dee", "role": "user", "email": null})]);
    // ...and unique violations are PostgreSQL's to report
    assert!(db.client.execute(&m.sql, &[&"Dee2", &Some("a@x.com".to_string())]).is_err());

    // upsert: do update reads `excluded`, do nothing yields no row
    let up = db.mutation("insert upsert(email: varchar(100), name: text) {
        into customers set email = :email, name = :name
        on conflict (email) do update set name = excluded.name returning id, name }");
    let (_, rows) = db.run(&up, &[&"a@x.com", &"Ann2"]);
    assert_eq!(rows, [json!({"id": 1, "name": "Ann2"})]);
    let (_, rows) = db.run(&up, &[&"new@x.com", &"Eve"]);
    assert!(rows[0]["id"].as_i64().unwrap() > 4, "sequence values are burned by conflicts, so only order is certain");
    let skip = db.mutation("insert once(email: varchar(100), name: text) {
        into customers set email = :email, name = :name on conflict (email) do nothing returning id }");
    assert_eq!(db.run(&skip, &[&"a@x.com", &"Nope"]).1, Vec::<Value>::new());
    assert_eq!(db.count("SELECT count(*) FROM customers"), 5);

    // update: uses the checker (typed params, the row's own columns), returns typed rows
    let m = db.mutation("update rename(id: int, n: text) { customers c set name = :n where c.id == :id returning c.id, c.name }");
    assert_eq!(db.run(&m, &[&"Robert", &2i32]).1, [json!({"id": 2, "name": "Robert"})]);
    let m = db.mutation("update bump(id: int) { orders o set qty = o.qty + 1, paid = true where o.id == :id returning o.qty, o.paid }");
    assert_eq!(db.run(&m, &[&1i32]).1, [json!({"qty": 3, "paid": true})]);
    // a nullable value into a nullable column, and a literal into an enum column
    let m = db.mutation("update clear(id: int) { customers c set balance = null, role = \"guest\", born = \"2001-02-03\" where c.id == :id }");
    assert_eq!(db.run(&m, &[&1i32]).0, 1);
    assert_eq!(db.count("SELECT count(*) FROM customers WHERE balance IS NULL AND role = 'guest' AND id = 1"), 1);

    // delete: a filtered delete, then `all rows`
    let m = db.mutation("delete drop_items(oid: int) { from items i where i.order_id == :oid returning i.id, i.sku }");
    let (n, rows) = db.run(&m, &[&1i32]);
    assert_eq!((n, rows.len()), (2, 2));
    assert!(rows[0]["id"].is_i64());
    let m = db.mutation("delete wipe() { from items i all rows }");
    assert_eq!(db.run(&m, &[]).0, 1);
    assert_eq!(db.count("SELECT count(*) FROM items"), 0);

    // rejected before any SQL is written
    let bad = |db: &Live, src: &str| {
        let (s, d) = compile(&db.schema, src, Dialect::Postgres);
        assert!(s.is_none(), "{src}");
        d.iter().map(|x| x.code.clone()).collect::<Vec<_>>()
    };
    assert_eq!(bad(&db, "insert i(n: text null) { into customers set name = :n }"), ["QL232"]);
    assert_eq!(bad(&db, "update u(q: int) { orders o set qty = :q all rows }"), ["QL211"]);
    assert_eq!(bad(&db, "insert i() { into orders set qty = 1 }"), ["QL233"]);

    // ==== subqueries and multi-row inserts ===========================================
    let q = db.query("query with_orders() { from customers c where c.id in (from orders o select o.customer_id) select c.name order by c.name }");
    assert!(!db.rows(&q, &[]).is_empty());
    let q = db.query("query idle() { from customers c where not exists (from orders o where o.customer_id == c.id select 1) select c.id order by c.id }");
    db.rows(&q, &[]);
    let q = db.query("query counts() { from customers c select c.id,
        (from orders o where o.customer_id == c.id select count(*)) as n,
        (from orders o where o.customer_id == c.id select max(o.total)) as biggest,
        (from orders o where o.customer_id == c.id select o.status order by o.id limit 1) as first_status
        order by c.id }");
    let rows = db.rows(&q, &[]);
    assert!(rows.iter().all(|r| !r["n"].is_null()), "a count is never NULL, even with no rows");
    assert!(rows.iter().any(|r| r["biggest"].is_null()), "max over no rows is NULL (declared nullable)");
    let q = db.query("query above_avg(min: int) { from orders o
        where o.total > (from orders p select avg(p.total)) or o.qty > :min select o.id order by o.id }");
    db.check_contract(&q);
    let q = db.query("query grouped() { from customers c group by c.id
        select c.id, (from orders o where o.customer_id == c.id select count(*)) as n order by c.id }");
    db.rows(&q, &[]);

    let m = db.mutation("update pay(min: int) { orders o set paid = true where o.customer_id in (from customers c where c.id >= :min select c.id) }");
    db.check_mutation(&m);
    let m = db.mutation("delete drop_idle() { from customers c where not exists (from orders o where o.customer_id == c.id select 1) returning c.id }");
    db.run(&m, &[]);

    let m = db.mutation("insert many(a: text, b: text) { into customers (name, email) values (:a, \"m1@x.com\"), (:b, \"m2@x.com\") returning id, name }");
    let (n, rows) = db.run(&m, &[&"Fay", &"Gus"]);
    assert_eq!(n, 2);
    assert_eq!(rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Fay", "Gus"]);
    let m = db.mutation("insert upsert_many(a: text, b: text) { into customers (name, email) values (:a, \"m1@x.com\"), (:b, \"m3@x.com\")
        on conflict (email) do update set name = excluded.name returning name }");
    let (n, _) = db.run(&m, &[&"Fay2", &"Hal"]);
    assert_eq!(n, 2);
    assert_eq!(db.count("SELECT count(*) FROM customers WHERE name = 'Fay2'"), 1);
    let before = db.count("SELECT count(*) FROM items");
    let m = db.mutation("insert snapshot() { into items (order_id, sku, price, qty) from orders o where o.total > 6 select o.id, \"snap\", o.total, 1 returning id, sku }");
    let (n, rows) = db.run(&m, &[]);
    assert!(n >= 1 && rows.iter().all(|r| r["sku"] == "snap"));
    assert_eq!(db.count("SELECT count(*) FROM items"), before + n as i64);

    // ==== window functions ===========================================================
    let q = db.query("query ranked() { from orders o select o.id,
        row_number() over (partition by o.customer_id order by o.total desc) as rn,
        rank() over (order by o.total) as r,
        sum(o.total) over (partition by o.customer_id) as cust_total,
        avg(o.qty) over () as avg_qty,
        count(*) over () as n,
        lag(o.total) over (order by o.id) as prev,
        lead(o.total, 1, 0) over (order by o.id) as next,
        first_value(o.status) over (partition by o.customer_id order by o.id) as first_status,
        ntile(2) over (order by o.id) as half,
        percent_rank() over (order by o.id) as pr
        order by o.id }");
    let rows = db.rows(&q, &[]);
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| !r["rn"].is_null() && !r["n"].is_null()), "ranking functions and count are never NULL");
    assert!(rows[0]["prev"].is_null(), "lag has no row before the first");
    let q = db.query("query top() { from orders o group by o.customer_id
        select o.customer_id, sum(o.total) as total, rank() over (order by sum(o.total) desc) as r order by r }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows[0]["r"], 1);

    // ==== union / intersect / except ==================================================
    let q = db.query("query both() { from customers c select c.id, c.email as contact union all from items i select i.id, i.sku order by id, contact }");
    let rows = db.rows(&q, &[]);
    assert!(!rows.is_empty());
    assert!(q.ir.select[1].nullable, "the nullable email makes the combined column nullable");
    let q = db.query("query roles() { from customers c select c.role union from customers d select d.role }");
    db.check_contract(&q); // the combined column is still the enum
    let q = db.query("query idle() { from customers c select c.id except from orders o select o.customer_id }");
    db.rows(&q, &[]);
    let q = db.query("query paged(a: int, lim: int) { from customers c where c.id >= :a select c.id union all from orders o where o.id >= :a select o.id order by id desc limit :lim }");
    db.check_contract(&q);

    // ==== with ========================================================================
    let q = db.query("query spenders(min: int) {
        with spend as (from orders o group by o.customer_id select o.customer_id, sum(o.total) as total, count(*) as n)
        from customers c left join spend s on s.customer_id == c.id where s.total >= :min or s.total is null
        select c.id, s.total, s.n order by c.id }");
    let rows = db.rows(&q, &[&1i32]);
    assert!(!rows.is_empty());
    assert!(q.ir.select[1].nullable && q.ir.select[2].nullable, "a left-joined with query's columns are nullable");
    let q = db.query("query chain() {
        with a as (from orders o select o.id, o.customer_id, row_number() over (partition by o.customer_id order by o.id) as rn),
             b as (from a where a.rn == 1 select a.id)
        from b select b.id order by b.id }");
    assert!(!db.rows(&q, &[]).is_empty());
    let q = db.query("query ids() { with ids as (from customers c select c.id union from orders o select o.id) from ids i select i.id order by i.id }");
    assert!(!db.rows(&q, &[]).is_empty());
    let m = db.mutation("update tag() { with paid_ids as (from orders o where o.paid select o.id)
        orders x set status = \"tagged\" where x.id in (from paid_ids p select p.id) }");
    db.check_mutation(&m);

    // ==== window frames ===============================================================
    let q = db.query("query running(n: int) { from orders o select o.id,
        sum(o.total) over (order by o.id rows between unbounded preceding and current row) as running,
        count(*) over (order by o.id rows between :n preceding and current row) as seen,
        first_value(o.id) over (order by o.id rows between 1 following and 2 following) as nxt,
        sum(o.total) over (order by o.total range between 10 preceding and current row) as near,
        count(*) over (order by o.status groups between current row and current row) as peers
        order by o.id }");
    let rows = db.rows(&q, &[&2i32]);
    assert!(!rows.is_empty());
    assert!(rows.last().unwrap()["nxt"].is_null(), "no row follows the last one");
    assert!(rows.iter().all(|r| !r["seen"].is_null()), "count is never NULL");

    // ==== with recursive ==============================================================
    let q = db.query("query tree() { with recursive tree as (
            from categories c where c.parent_id is null select c.id, c.name, 0 as depth
            union all
            from categories c join tree t on c.parent_id == t.id select c.id, c.name, t.depth + 1 as depth)
        from tree t select t.name, t.depth order by t.depth, t.name }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows.iter().map(|r| r["depth"].as_i64().unwrap()).collect::<Vec<_>>(), [0, 1, 1, 2]);
    let q = db.query("query below(root: int) { with recursive sub as (
            from categories c where c.id == :root select c.id
            union all
            from categories c join sub s on c.parent_id == s.id select c.id)
        from sub select count(*) as n }");
    assert_eq!(db.rows(&q, &[&1i32]), [json!({"n": 4})]);
    let q = db.query("query labels() { with recursive t as (
            from categories c where c.parent_id is null select c.id, c.name as label
            union all
            from categories c join t on c.parent_id == t.id select c.id, c.note as label)
        from t select t.id, t.label order by t.id }");
    assert!(q.ir.select[1].nullable);
    db.rows(&q, &[]);
    let m = db.mutation("delete prune(root: int) { with recursive sub as (
            from categories c where c.id == :root select c.id
            union all from categories c join sub s on c.parent_id == s.id select c.id)
        from categories x where x.id in (from sub s select s.id) and x.id <> :root returning x.name }");
    db.check_mutation(&m);

    // ==== || ============================================================================
    let q = db.query("query who() { from customers c select c.id,
        c.name || \" <\" || c.email || \">\" as who,
        c.name || \"#\" || c.id as tag,
        c.name || \":\" || c.role as role_tag order by c.id }");
    let rows = db.rows(&q, &[]);
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|r| r["who"].is_null()), "NULL if any part is (Bob has no email)");
    assert!(rows.iter().all(|r| !r["tag"].is_null()));
    let q = db.query("query paths() { with recursive t as (
            from categories c where c.parent_id is null select c.id, c.name as path
            union all
            from categories c join t on c.parent_id == t.id select c.id, t.path || \"/\" || c.name as path)
        from t select t.path order by t.path }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows.iter().map(|r| r["path"].as_str().unwrap()).collect::<Vec<_>>(), ["root", "root/a", "root/a/a1", "root/b"]);

    // ==== text and date functions =========================================================
    let q = db.query("query f() { from customers c select c.id, c.name, substr(c.name, 2) as tail, substr(c.name, 1, 2) as head,
        replace(c.name, \"n\", \"N\") as rep, position(c.name, \"n\") as pos,
        date_part(\"year\", c.born) as y, date_part(\"month\", c.born) as m, date_part(\"day\", c.born) as d order by c.id }");
    let rows = db.rows(&q, &[]);
    // earlier mutations may have renamed rows, so compare with the name itself
    for r in &rows {
        let name = r["name"].as_str().unwrap();
        assert_eq!(r["tail"], name.chars().skip(1).collect::<String>());
        assert_eq!(r["head"], name.chars().take(2).collect::<String>());
        assert_eq!(r["rep"], name.replace('n', "N"));
        assert_eq!(r["pos"], name.find('n').map_or(0, |i| i + 1));
    }
    assert!(rows.iter().any(|r| r["y"].as_i64().is_some_and(|y| (1900..2100).contains(&y))));
    assert!(rows.iter().all(|r| r["m"].is_null() || (1..=12).contains(&r["m"].as_i64().unwrap())));
    let q = db.query("query t() { from orders o where o.shipped is not null select date_part(\"hour\", o.shipped) as h, date_part(\"minute\", o.shipped) as mi, date_part(\"year\", o.created) as y }");
    db.rows(&q, &[]);

    // ==== more text and date functions, named windows ====================================
    let q = db.query("query f() { from customers c select c.id, c.name, left(c.name, 2) as l, right(c.name, 2) as r, starts_with(c.name, \"A\") as sa,
        add_days(c.born, 7) as plus, days_between(c.born, add_days(c.born, 40)) as span order by c.id }");
    let rows = db.rows(&q, &[]);
    for r in &rows {
        let name = r["name"].as_str().unwrap();
        assert_eq!(r["l"], name.chars().take(2).collect::<String>());
        let n = name.chars().count();
        assert_eq!(r["r"], name.chars().skip(n.saturating_sub(2)).collect::<String>());
        assert_eq!(r["sa"], name.starts_with('A'));
        assert!(r["span"].is_null() || r["span"] == 40);
    }
    assert!(rows.iter().any(|r| r["span"] == 40));
    let q = db.query("query w() { from customers c window w as (order by c.id)
        select c.id, row_number() over w as n, sum(c.id) over w as running order by c.id }");
    let rows = db.rows(&q, &[]);
    assert_eq!(rows[0]["n"], 1);
    assert_eq!(rows[0]["running"], rows[0]["id"]);

    // a timestamp with a time zone is read in UTC, whatever the session's zone is
    let q = db.query("query h() { from orders o where o.shipped is not null select date_part(\"hour\", o.shipped) as h, date_part(\"day\", o.shipped) as d }");
    let utc = db.rows(&q, &[]);
    db.client.batch_execute("SET TIME ZONE 'Pacific/Auckland'").unwrap();
    let auckland = db.rows(&q, &[]);
    db.client.batch_execute("RESET TIME ZONE").unwrap();
    assert_eq!(utc, auckland);

    // ==== filtered aggregates and string_agg ===============================================
    let q = db.query("query f() { from orders o group by o.customer_id select o.customer_id,
        count(*) filter (where o.paid) as paid_n, sum(o.total) filter (where o.status == \"new\") as new_total,
        string_agg(o.status, \",\" order by o.id) as asc_, string_agg(o.status, \",\" order by o.id desc) as desc_ order by o.customer_id }");
    let rows = db.rows(&q, &[]);
    assert!(!rows.is_empty());
    for r in &rows {
        let asc = r["asc_"].as_str().unwrap().split(',').collect::<Vec<_>>();
        let mut desc = r["desc_"].as_str().unwrap().split(',').collect::<Vec<_>>();
        desc.reverse();
        assert_eq!(asc, desc, "the two orders are mirror images");
        assert!(r["paid_n"].as_i64().unwrap() >= 0);
    }
    let q = db.query("query e() { from customers c where c.id > 999999 select string_agg(c.name, \",\") as s, count(*) filter (where c.id > 0) as n }");
    let rows = db.rows(&q, &[]);
    assert!(rows[0]["s"].is_null());
    assert_eq!(rows[0]["n"], 0);
    let q = db.query("query m() { from customers c select string_agg(c.email, \";\" order by c.id) as emails }");
    let rows = db.rows(&q, &[]);
    assert!(!rows[0]["emails"].as_str().unwrap().contains("null"), "NULL values are skipped");
    let q = db.query("query w(min: decimal(10,2)) { from orders o select o.id, count(*) filter (where o.total > :min) over () as big order by o.id }");
    db.check_contract(&q);

    // ==== fragments =========================================================================
    let file = "fragment paid_orders() { from orders o where o.paid select o.id, o.customer_id, o.total }
        fragment paying() { from customers c where c.id in (from paid_orders p select p.customer_id) select c.id, c.name }
        query q() { from paid_orders p join customers c on p.customer_id == c.id select c.name, p.total order by p.id }
        query names() { from paying x select x.name order by x.name }
        query unpaid() { from customers c left join paid_orders p on p.customer_id == c.id where p.id is null select c.name order by c.name }
        delete gone() { from orders o where o.id in (from paid_orders p select p.id) }";
    let (stmts, d) = compile(&db.schema, file, Dialect::Postgres);
    let stmts = stmts.unwrap_or_else(|| panic!("{d:?}"));
    for s in &stmts {
        match s {
            certo_ql::Statement::Query(q) => { db.rows(q, &[]); }
            certo_ql::Statement::Mutation(m) => db.check_mutation(m),
        }
    }
}
