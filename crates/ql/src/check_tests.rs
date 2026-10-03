use crate::*;
use certo_diagnostics::Severity;
use certo_sdl::{compile as compile_sdl, Builtin, SchemaIR, TypeIR};
use certo_sql::Dialect;

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
    note: text
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
"#;

fn schema() -> SchemaIR {
    let (ir, d) = compile_sdl(SCHEMA);
    ir.unwrap_or_else(|| panic!("fixture schema failed: {d:?}"))
}

fn ok(src: &str) -> Vec<CompiledQuery> {
    let (q, d) = compile(&schema(), src, Dialect::Postgres);
    q.unwrap_or_else(|| panic!("query failed: {d:?}"))
        .into_iter()
        .map(|s| s.as_query().expect("a query").clone())
        .collect()
}

fn diags(src: &str) -> Vec<certo_diagnostics::Diagnostic> { compile(&schema(), src, Dialect::Postgres).1 }

fn errors(src: &str) -> Vec<String> {
    let (q, d) = compile(&schema(), src, Dialect::Postgres);
    assert!(q.is_none(), "expected failure for: {src}");
    d.iter().filter(|x| x.severity == Severity::Error).map(|x| x.code.clone()).collect()
}

/// The type and nullability of one expression, evaluated with `o` inner and `c` left-joined.
fn col(expr: &str) -> (TypeIR, bool) {
    let q = ok(&format!("query t() {{ from orders o left join customers c on o.customer_id == c.id select {expr} as x }}"));
    let c = &q[0].ir.select[0];
    (c.ty.clone(), c.nullable)
}

fn b(x: Builtin) -> TypeIR { TypeIR::Builtin(x) }

// ---- result types and nullability -------------------------------------------- //

#[test]
fn column_types_and_nullability_follow_the_schema_and_joins() {
    assert_eq!(col("o.id"), (b(Builtin::Int), false));
    assert_eq!(col("o.total"), (b(Builtin::Numeric(10, 2)), false));
    assert_eq!(col("o.shipped"), (b(Builtin::Timestamp), true)); // nullable column
    assert_eq!(col("o.qty"), (b(Builtin::SmallInt), false));
    // the left-joined side is nullable even for NOT NULL columns
    assert_eq!(col("c.name"), (b(Builtin::Text), true));
    assert_eq!(col("c.id"), (b(Builtin::Int), true));
    assert_eq!(col("c.email"), (b(Builtin::Varchar(100)), true));
    assert_eq!(col("c.role"), (TypeIR::Enum("Role".into()), true));
    // an inner join keeps NOT NULL columns non-null
    let q = ok("query t() { from orders o join customers c on o.customer_id == c.id select c.name as n, c.email as e }");
    assert!(!q[0].ir.select[0].nullable && q[0].ir.select[1].nullable);
}

#[test]
fn expression_types_and_nullability() {
    assert_eq!(col("o.total + 1"), (b(Builtin::Decimal), false));
    assert_eq!(col("o.qty * 2"), (b(Builtin::Int), false)); // smallint * int -> int
    assert_eq!(col("o.total > 100"), (b(Builtin::Bool), false));
    assert_eq!(col("c.id > 1"), (b(Builtin::Bool), true)); // NULL operand -> NULL result
    assert_eq!(col("o.shipped is null"), (b(Builtin::Bool), false)); // IS NULL is never NULL
    assert_eq!(col("c.name is not null"), (b(Builtin::Bool), false));
    assert_eq!(col("lower(o.status)"), (b(Builtin::Text), false));
    assert_eq!(col("upper(c.name)"), (b(Builtin::Text), true));
    assert_eq!(col("length(o.status)"), (b(Builtin::Int), false));
    assert_eq!(col("abs(o.total)"), (b(Builtin::Numeric(10, 2)), false));
    assert_eq!(col("round(o.total)"), (b(Builtin::Numeric(10, 2)), false));
    assert_eq!(col("now()"), (b(Builtin::Timestamp), false));
    assert_eq!(col("today()"), (b(Builtin::Date), false));
    assert_eq!(col("nullif(o.qty, 0)"), (b(Builtin::SmallInt), true));
    // coalesce is non-null as soon as one argument is
    assert_eq!(col("coalesce(c.name, \"anon\")"), (b(Builtin::Text), false));
    assert_eq!(col("coalesce(c.name, c.note)"), (b(Builtin::Text), true));
    assert_eq!(col("coalesce(c.id, 0)"), (b(Builtin::Int), false));
    // case: NULL when no branch can match
    assert_eq!(col("case when o.total > 100 then \"big\" else \"small\" end"), (b(Builtin::Text), false));
    assert_eq!(col("case when o.total > 100 then \"big\" end"), (b(Builtin::Text), true));
    assert_eq!(col("case when o.paid then 1 when o.qty > 1 then 2 else 3 end"), (b(Builtin::Int), false));
    assert_eq!(col("case when o.paid then 1 else c.id end"), (b(Builtin::Int), true));
    assert_eq!(col("o.total between 1 and 10"), (b(Builtin::Bool), false));
    assert_eq!(col("o.status like \"a%\""), (b(Builtin::Bool), false));
    assert_eq!(col("o.id in (1, 2)"), (b(Builtin::Bool), false));
    assert_eq!(col("not o.paid"), (b(Builtin::Bool), false));
}

#[test]
fn aggregate_types_follow_postgres() {
    let agg = |e: &str| {
        let q = ok(&format!("query t() {{ from orders o select {e} as x }}"));
        (q[0].ir.select[0].ty.clone(), q[0].ir.select[0].nullable)
    };
    assert_eq!(agg("count(*)"), (b(Builtin::BigInt), false));
    assert_eq!(agg("count(o.shipped)"), (b(Builtin::BigInt), false));
    assert_eq!(agg("count(distinct o.status)"), (b(Builtin::BigInt), false));
    assert_eq!(agg("sum(o.qty)"), (b(Builtin::BigInt), true)); // sum(smallint) is bigint; NULL over no rows
    assert_eq!(agg("sum(o.id)"), (b(Builtin::BigInt), true));
    assert_eq!(agg("sum(o.total)"), (b(Builtin::Decimal), true));
    assert_eq!(agg("avg(o.qty)"), (b(Builtin::Decimal), true));
    assert_eq!(agg("avg(o.total)"), (b(Builtin::Decimal), true));
    assert_eq!(agg("min(o.created)"), (b(Builtin::Timestamp), true));
    assert_eq!(agg("max(o.status)"), (b(Builtin::Text), true));
    assert_eq!(agg("max(o.total)"), (b(Builtin::Numeric(10, 2)), true));
    // sum of a bigint is numeric
    let q = ok("query t() { from items i select sum(i.id) as s }");
    assert_eq!(q[0].ir.select[0].ty, b(Builtin::Decimal));
}

#[test]
fn star_expansion() {
    let q = ok("query t() { from customers select * }");
    let names: Vec<_> = q[0].ir.select.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "email", "role", "balance", "born", "token", "note"]);
    let q = ok("query t() { from orders o join customers c on o.customer_id == c.id select o.*, c.name as customer }");
    assert_eq!(q[0].ir.select.len(), 9);
    assert_eq!(q[0].ir.select[8].name, "customer");
    // a left join makes the expanded columns of the far side nullable
    let q = ok("query t() { from orders o left join customers c on o.customer_id == c.id select c.* }");
    assert!(q[0].ir.select.iter().all(|c| c.nullable));
    // both sources have `id`: needs explicit names
    assert_eq!(errors("query t() { from orders o join customers c on o.customer_id == c.id select * }"), ["QL216"]);
}

// ---- name resolution --------------------------------------------------------- //

#[test]
fn name_errors() {
    assert_eq!(errors("query t() { from ghost select ghost.x }"), ["QL203"]);
    assert_eq!(errors("query t() { from orders o select p.id }"), ["QL205"]);
    assert_eq!(errors("query t() { from orders o select o.ghost }"), ["QL206"]);
    assert_eq!(errors("query t() { from orders o select ghost }"), ["QL206"]);
    assert_eq!(errors("query t() { from orders o join customers c on o.customer_id == c.id select id }"), ["QL207"]); // ambiguous
    assert_eq!(errors("query t() { from orders o join customers o on true select o.id }"), ["QL204"]);
    assert_eq!(errors("query t() { from orders o select z.* }"), ["QL205"]);
    // a join's ON sees the join's own table and the earlier ones, not later ones
    ok("query t() { from orders o join customers c on o.customer_id == c.id join items i on i.order_id == o.id select o.id }");
    assert_eq!(errors("query t() { from orders o join customers c on o.customer_id == i.id join items i on i.order_id == o.id select o.id }"), ["QL205"]);
    // an unqualified name that exists in only one source is fine
    ok("query t() { from orders o join customers c on o.customer_id == c.id select total, name }");
}

#[test]
fn duplicate_queries_and_unrelated_errors_do_not_hide_each_other() {
    assert_eq!(errors("query a() { from orders select orders.id } query a() { from orders select orders.id }"), ["QL201"]);
    let (q, d) = compile(&schema(), "query good() { from orders select orders.id } query bad() { from ghost select ghost.x }", Dialect::Postgres);
    assert!(q.is_none());
    assert_eq!(d.iter().filter(|x| x.severity == Severity::Error).count(), 1);
}

// ---- parameters -------------------------------------------------------------- //

#[test]
fn parameters() {
    let q = ok("query t(min: decimal(10,2), since: timestamp null, r: Role, s: varchar(20), n: int) {
        from orders o join customers c on o.customer_id == c.id
        where o.total >= :min and o.created > :since and c.role == :r and c.name == :s and o.qty > :n
        select o.id
    }");
    let p = &q[0].ir.params;
    assert_eq!(p[0].ty, b(Builtin::Numeric(10, 2)));
    assert!(!p[0].nullable && p[1].nullable);
    assert_eq!(p[2].ty, TypeIR::Enum("Role".into()));
    assert_eq!(p[3].ty, b(Builtin::Varchar(20)));
    // a nullable parameter makes the comparison nullable, not the query invalid
    assert_eq!(errors("query t() { from orders o where o.id == :ghost select o.id }"), ["QL208"]);
    assert_eq!(errors("query t(a: int, a: text) { from orders o select o.id }"), ["QL202"]);
    assert_eq!(errors("query t(a: nope) { from orders o select o.id }"), ["QL219"]);
    assert_eq!(errors("query t(a: varchar) { from orders o select o.id }"), ["QL219"]);
    assert_eq!(errors("query t(a: serial) { from orders o select o.id }"), ["QL219"]);
    // an unused parameter is a warning, not an error
    let d = diags("query t(a: int) { from orders o select o.id }");
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].code.as_str(), d[0].severity), ("QL290", Severity::Warning));
    assert!(compile(&schema(), "query t(a: int) { from orders o select o.id }", Dialect::Postgres).0.is_some());
    // types must match what they are compared with
    assert_eq!(errors("query t(a: text) { from orders o where o.id == :a select o.id }"), ["QL211"]);
}

// ---- enums and literal coercion --------------------------------------------- //

#[test]
fn string_literals_are_checked_against_enums() {
    ok("query t() { from customers c where c.role == \"admin\" select c.id }");
    ok("query t() { from customers c where \"admin\" == c.role select c.id }");
    ok("query t() { from customers c where c.role != \"guest\" and c.role in (\"admin\", \"user\") select c.id }");
    let d = diags("query t() { from customers c where c.role == \"root\" select c.id }");
    let e = d.iter().find(|x| x.code == "QL218").expect("QL218");
    assert!(e.message.contains("admin, user, guest"), "{}", e.message);
    assert_eq!(errors("query t() { from customers c where c.role in (\"admin\", \"root\") select c.id }"), ["QL218"]);
    assert_eq!(errors("query t() { from customers c where c.role == 5 select c.id }"), ["QL211"]);
    // other typed columns accept a string literal too (uuid, dates, timestamps)
    ok("query t() { from customers c where c.token == \"00000000-0000-0000-0000-000000000000\" and c.born > \"2000-01-01\" select c.id }");
    ok("query t() { from orders o where o.created > \"2026-01-01T00:00:00\" select o.id }");
    // ...but not a plain number column
    assert_eq!(errors("query t() { from orders o where o.qty == \"5\" select o.id }"), ["QL211"]);
}

// ---- typing errors ---------------------------------------------------------- //

#[test]
fn type_errors() {
    assert_eq!(errors("query t() { from orders o where o.status == 1 select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o where o.paid > true select o.id }"), ["QL211"]); // booleans are unordered
    assert_eq!(errors("query t() { from orders o where o.status + 1 > 0 select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o where o.id like \"1%\" select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o where o.id select o.id }"), ["QL212"]);
    assert_eq!(errors("query t() { from orders o where not o.id select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o where o.id > 1 and o.status select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o join customers c on o.customer_id select o.id }"), ["QL212"]);
    assert_eq!(errors("query t() { from orders o where o.id in (\"a\") select o.id }"), ["QL211"]);
    assert_eq!(errors("query t() { from orders o select case when o.id then 1 end as x }"), ["QL212"]);
    assert_eq!(errors("query t() { from orders o select case when o.paid then 1 else \"a\" end as x }"), ["QL211"]);
    // timestamps, dates and text compare within their families
    ok("query t() { from orders o join customers c on o.customer_id == c.id where o.created > c.born and o.status == c.name select o.id }");
}

#[test]
fn functions() {
    assert_eq!(errors("query t() { from orders o select ghost(o.id) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select lower(o.id) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select lower(o.status, o.status) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select abs(o.status) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select now(1) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select coalesce() as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select coalesce(o.id, o.status) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select nullif(o.id) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select lower(*) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select sum(*) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select sum(o.status) as x }"), ["QL209"]);
    assert_eq!(errors("query t() { from orders o select max(o.paid) as x }"), ["QL209"]);
}

// ---- aggregates and grouping ------------------------------------------------- //

#[test]
fn aggregates_and_group_by() {
    ok("query t() { from orders o select count(*) as n, sum(o.total) as s }"); // whole-table aggregate
    ok("query t() { from orders o group by o.status select o.status, count(*) as n order by n desc }");
    ok("query t() { from orders o group by o.status select upper(o.status) as s, count(*) as n }"); // an expression of a grouped column
}

#[test]
fn grouping_rules() {
    // a column that is neither grouped nor aggregated
    assert_eq!(errors("query t() { from orders o group by o.status select o.status, o.total }"), ["QL214"]);
    assert_eq!(errors("query t() { from orders o select o.id, count(*) as n }"), ["QL214"]);
    assert_eq!(errors("query t() { from orders o group by o.status having o.total > 1 select o.status }"), ["QL214"]);
    assert_eq!(errors("query t() { from orders o group by o.status select o.status, count(*) as n order by o.total }"), ["QL214"]);
    // aggregates are for select / having / order by only
    assert_eq!(errors("query t() { from orders o where count(*) > 1 select o.id }"), ["QL213"]);
    assert_eq!(errors("query t() { from orders o group by count(*) select o.id }"), ["QL213"]);
    assert_eq!(errors("query t() { from orders o join customers c on count(*) > 1 select o.id }"), ["QL213"]);
    assert_eq!(errors("query t() { from orders o select sum(count(*)) as x }"), ["QL213"]);
    // having may use aggregates and grouped columns
    ok("query t() { from orders o group by o.status having count(*) > 1 and o.status != \"x\" select o.status, sum(o.total) as s }");
    // an expression built from grouped columns and constants is fine
    ok("query t() { from orders o group by o.status select o.status, count(*) + 1 as n }");
    ok("query t() { from orders o group by o.status, o.paid select case when o.paid then o.status else \"n/a\" end as s }");
}

#[test]
fn naming_and_ordering() {
    assert_eq!(errors("query t() { from orders o select o.id + 1 }"), ["QL215"]);
    assert_eq!(errors("query t() { from orders o select count(*) }"), ["QL215"]);
    assert_eq!(errors("query t() { from orders o select o.id, o.id }"), ["QL216"]);
    assert_eq!(errors("query t() { from orders o select o.id as x, o.status as x }"), ["QL216"]);
    assert_eq!(errors("query t() { from orders o select null as x }"), ["QL220"]);
    // order by may name an output column, which wins over a source column of the same name
    let q = ok("query t() { from orders o select o.total as amount order by amount desc, o.id }");
    assert_eq!(q[0].ir.order_by[0].expr, QExpr::Column { source: "o".into(), column: "total".into() });
    assert!(q[0].ir.order_by[0].desc && !q[0].ir.order_by[1].desc);
    // select distinct: order by must be in the list
    ok("query t() { from orders o select distinct o.status order by o.status }");
    assert_eq!(errors("query t() { from orders o select distinct o.status order by o.id }"), ["QL221"]);
}

#[test]
fn limit_and_offset() {
    ok("query t(n: int, k: bigint null) { from orders o select o.id limit :n offset :k }");
    ok("query t() { from orders o select o.id limit 10 offset 20 }");
    assert_eq!(errors("query t() { from orders o select o.id limit -1 }"), ["QL217"]);
    assert_eq!(errors("query t(n: text) { from orders o select o.id limit :n }"), ["QL217"]);
    assert_eq!(errors("query t() { from orders o select o.id limit o.id }"), ["QL217"]);
    assert_eq!(errors("query t() { from orders o select o.id limit 1 + 1 }"), ["QL217"]);
    assert_eq!(errors("query t() { from orders o select o.id offset \"x\" }"), ["QL217"]);
}

// ---- the compiled contract and its SQL --------------------------------------- //

#[test]
fn the_typed_contract_serialises_for_a_host() {
    let q = ok("query recent(min: decimal(10,2), since: timestamp null) {
        from orders o left join customers c on o.customer_id == c.id
        where o.total >= :min select o.id, c.name as customer, count(*) as n group by o.id
    }".replace("count(*) as n group by o.id", "o.total").as_str());
    let v: serde_json::Value = serde_json::to_value(&q[0].ir).unwrap();
    assert_eq!(v["name"], "recent");
    assert_eq!(v["params"][0]["type"]["name"], serde_json::json!({"numeric": [10, 2]}));
    assert_eq!(v["params"][1]["nullable"], true);
    assert_eq!(v["select"][1]["name"], "customer");
    assert_eq!(v["select"][1]["nullable"], true); // left-joined
    assert_eq!(v["select"][0]["nullable"], false);
    assert_eq!(v["sources"][1]["join"], "left");
    assert_eq!(serde_json::from_value::<QueryIR>(v).unwrap(), q[0].ir);
}

#[test]
fn lowering() {
    let q = ok("query recent(min_total: decimal(10,2), since: timestamp null) {
        from orders o
        join customers c on o.customer_id == c.id
        where o.total >= :min_total and o.created > :since
        select o.id, c.name as customer, o.total
        order by o.total desc
        limit 50
    }");
    assert_eq!(q[0].sql, "SELECT \"o\".\"id\" AS \"id\", \"c\".\"name\" AS \"customer\", \"o\".\"total\" AS \"total\"\n\
FROM \"orders\" AS \"o\"\n\
INNER JOIN \"customers\" AS \"c\" ON (\"o\".\"customer_id\" = \"c\".\"id\")\n\
WHERE ((\"o\".\"total\" >= ($1::numeric(10,2))) AND (\"o\".\"created\" > ($2::timestamptz)))\n\
ORDER BY \"o\".\"total\" DESC\n\
LIMIT 50");
    assert_eq!(q[0].param_order, ["min_total", "since"]);

    // placeholders are numbered by first use, reused, and follow use order, not declaration order
    let q = ok("query t(a: int, b: text) { from orders o where o.status == :b and o.id > :a and o.qty < :a select o.id }");
    assert_eq!(q[0].param_order, ["b", "a"]);
    assert!(q[0].sql.contains("($1::text)") && q[0].sql.contains("($2::integer)"), "{}", q[0].sql);
    assert_eq!(q[0].sql.matches("$2").count(), 2, "a reused parameter reuses its placeholder: {}", q[0].sql);
}

#[test]
fn lowering_covers_every_form() {
    let q = ok("query t(r: Role, lim: int) {
        from customers c
        left join orders o on o.customer_id = c.id and o.total <> 0
        where c.role == \"admin\" and c.role != :r and not (c.id in (1, 2, -3))
          and c.name like \"a%\" and c.email is not null and o.total between 1.5 and 10 and c.name not like \"z%\"
        group by c.name
        having count(distinct o.id) > 1 and sum(o.total) >= -2.5
        select distinct
            upper(c.name) as name_upper,
            trim(c.name) as name_trim,
            case when count(*) > 1 then \"many\" else \"few\" end as bucket,
            coalesce(max(o.total), 0) as top,
            today() as day
        order by top desc
        limit :lim offset 5
    }");
    let sql = &q[0].sql;
    for expected in [
        "SELECT DISTINCT ",
        "LEFT JOIN \"orders\" AS \"o\" ON ((\"o\".\"customer_id\" = \"c\".\"id\") AND (\"o\".\"total\" <> 0))",
        "(\"c\".\"role\" = 'admin')",
        "(\"c\".\"role\" <> ($1::\"Role\"))",
        "(NOT (\"c\".\"id\" IN (1, 2, (-3))))",
        "(\"c\".\"name\" LIKE 'a%')",
        "(\"c\".\"name\" NOT LIKE 'z%')",
        "(\"c\".\"email\" IS NOT NULL)",
        "((\"o\".\"total\" >= 1.5) AND (\"o\".\"total\" <= 10))",
        "GROUP BY \"c\".\"name\"",
        "HAVING ((count(DISTINCT \"o\".\"id\") > 1) AND (sum(\"o\".\"total\") >= (-2.5)))",
        "upper(\"c\".\"name\") AS \"name_upper\"",
        "btrim(\"c\".\"name\") AS \"name_trim\"",
        "CASE WHEN (count(*) > 1) THEN 'many' ELSE 'few' END AS \"bucket\"",
        "coalesce(max(\"o\".\"total\"), 0) AS \"top\"",
        "CURRENT_DATE AS \"day\"",
        "LIMIT ($2::integer)\nOFFSET 5",
    ] {
        assert!(sql.contains(expected), "expected `{expected}` in:\n{sql}");
    }
    assert!(sql.contains("ORDER BY coalesce(max(\"o\".\"total\"), 0) DESC"), "{sql}");
}

#[test]
fn independent_errors_are_all_reported_in_one_pass() {
    let (q, d) = compile(&schema(), "query t(n: int) {
        from orders o join customers c on o.customer_id == c.id
        where c.role == \"root\" and o.totl > :n
        select c.nme, o.status like 5
        order by o.ghost
        limit -1
    }", Dialect::Postgres);
    assert!(q.is_none());
    let mut codes: Vec<_> = d.iter().filter(|x| x.severity == Severity::Error).map(|x| x.code.as_str()).collect();
    codes.sort();
    // the enum value, both typos on either side of the `and`, the select items, the order-by and the limit
    assert_eq!(codes, ["QL206", "QL206", "QL206", "QL211", "QL217", "QL218"], "{d:?}");
    // an error in one query does not stop the next from being checked
    let (_, d) = compile(&schema(), "query a() { from orders o select o.ghost } query b() { from orders o select o.ghost2 }", Dialect::Postgres);
    assert_eq!(d.iter().filter(|x| x.code == "QL206").count(), 2);
    // grouping is only judged once the rest of the query is sound (no noise from earlier mistakes)
    assert_eq!(errors("query t() { from orders o group by o.status select o.status, o.ghost }"), ["QL206"]);
    assert_eq!(errors("query t() { from orders o group by o.status select o.status, o.total }"), ["QL214"]);
}

// ---- mutations ------------------------------------------------------------------ //

fn mutation(src: &str) -> CompiledMutation {
    let (s, d) = compile(&schema(), src, Dialect::Postgres);
    s.unwrap_or_else(|| panic!("mutation failed: {d:?}")).remove(0).as_mutation().expect("a mutation").clone()
}

#[test]
fn insert_lowers_with_typed_parameters_and_returning() {
    let m = mutation(
        "insert add(name: text, email: varchar(100) null) { into customers set name = :name, email = :email returning id, role }",
    );
    assert_eq!(
        m.sql,
        "INSERT INTO \"customers\" AS \"customers\" (\"name\", \"email\")\nVALUES (($1::text), ($2::character varying(100)))\nRETURNING \"customers\".\"id\" AS \"id\", \"customers\".\"role\" AS \"role\""
    );
    assert_eq!(m.param_order, ["name", "email"]);
    assert_eq!(m.ir.returning.len(), 2);
    assert_eq!(m.ir.returning[0].ty, b(Builtin::Int));
    assert!(!m.ir.returning[0].nullable);
}

#[test]
fn update_and_delete_lower_and_reuse_the_checker() {
    let m = mutation("update rename(id: int, n: text) { customers c set name = :n where c.id == :id returning c.id, c.name }");
    assert_eq!(
        m.sql,
        "UPDATE \"customers\" AS \"c\"\nSET \"name\" = ($1::text)\nWHERE (\"c\".\"id\" = ($2::integer))\nRETURNING \"c\".\"id\" AS \"id\", \"c\".\"name\" AS \"name\""
    );
    let d = mutation("delete purge(before: timestamp) { from orders o where o.created < :before }");
    assert_eq!(d.sql, "DELETE FROM \"orders\" AS \"o\"\nWHERE (\"o\".\"created\" < ($1::timestamptz))");
    assert!(d.ir.returning.is_empty());
    let all = mutation("delete wipe() { from items i all rows }");
    assert_eq!(all.sql, "DELETE FROM \"items\" AS \"i\"");
    assert!(all.ir.all_rows);
}

#[test]
fn upserts_lower_and_may_read_excluded() {
    let m = mutation(
        "insert up(e: varchar(100), n: text) { into customers set email = :e, name = :n
           on conflict (email) do update set name = excluded.name returning id }",
    );
    assert!(m.sql.contains("ON CONFLICT (\"email\") DO UPDATE SET \"name\" = \"excluded\".\"name\""), "{}", m.sql);
    let n = mutation("insert up(e: varchar(100), n: text) { into customers set email = :e, name = :n on conflict (id) do nothing }");
    assert!(n.sql.contains("ON CONFLICT (\"id\") DO NOTHING"));
}

#[test]
fn an_update_may_read_the_row_it_changes() {
    let m = mutation("update bump(id: int) { orders o set qty = o.qty + 1, paid = true where o.id == :id }");
    assert!(m.sql.contains("SET \"qty\" = (\"o\".\"qty\" + 1), \"paid\" = TRUE"), "{}", m.sql);
}

#[test]
fn mutation_rules_are_enforced() {
    // a value that may be NULL never goes into a NOT NULL column
    assert_eq!(errors("insert i(n: text null) { into customers set name = :n }"), ["QL232"]);
    assert_eq!(errors("insert i() { into customers set name = null }"), ["QL232"]);
    // required columns must be supplied (orders needs customer_id, total, status)
    assert_eq!(errors("insert i() { into orders set qty = 1 }"), ["QL233"]);
    // no assigning twice, unknown columns/tables, unknown enum values
    assert_eq!(errors("insert i() { into customers set name = \"a\", name = \"b\" }"), ["QL235"]);
    assert_eq!(errors("insert i() { into customers set name = \"a\", nope = 1 }"), ["QL206"]);
    assert_eq!(errors("insert i() { into nope set a = 1 }"), ["QL203"]);
    assert_eq!(errors("insert i() { into customers set name = \"a\", role = \"boss\" }"), ["QL218"]);
    // no narrowing: an int parameter does not fit a smallint column; a literal does
    assert_eq!(errors("update u(q: int) { orders o set qty = :q all rows }"), ["QL211"]);
    mutation("update u() { orders o set qty = 3 all rows }");
    // a fractional literal needs a column that can hold one
    assert_eq!(errors("update u() { orders o set qty = 1.5 all rows }"), ["QL211"]);
    // an insert's values cannot read the row being created
    assert_eq!(errors("insert i() { into customers set name = customers.name }"), ["QL205"]);
    // an upsert must name a real unique key
    assert_eq!(errors("insert i(n: text) { into customers set name = :n on conflict (name) do nothing }"), ["QL236"]);
    // strict about text ← number and friends
    assert_eq!(errors("update u() { customers c set name = 5 all rows }"), ["QL211"]);
}

#[test]
fn generated_always_columns_cannot_be_set() {
    let (ir, d) = certo_sdl::compile("table t { id: int generated always primary key  n: int }");
    let ir = ir.unwrap_or_else(|| panic!("{d:?}"));
    let (s, d) = compile(&ir, "insert i() { into t set id = 1, n = 2 }", Dialect::Postgres);
    assert!(s.is_none());
    assert!(d.iter().any(|x| x.code == "QL234"), "{d:?}");
}

#[test]
fn statement_names_are_unique_across_queries_and_mutations() {
    assert_eq!(
        errors("query a() { from orders o select o.id } delete a() { from items i all rows }"),
        ["QL201"]
    );
}

#[test]
fn statements_serialise_with_their_kind() {
    let (s, _) = compile(
        &schema(),
        "query q() { from orders o select o.id }
         insert add(n: text) { into customers set name = :n returning id }
         update u(id: int) { customers c set note = null where c.id == :id }
         delete d() { from items i all rows }",
        Dialect::Postgres,
    );
    let j = to_json(&s.unwrap());
    let kinds: Vec<_> = j.as_array().unwrap().iter().map(|x| x["kind"].as_str().unwrap().to_string()).collect();
    assert_eq!(kinds, ["query", "insert", "update", "delete"]);
    assert_eq!(j[1]["columns"][0]["name"], "id");
    assert_eq!(j[3]["columns"], serde_json::json!([]));
}

// ---- subqueries ------------------------------------------------------------------ //

fn sql_of(src: &str) -> String { ok(src).remove(0).sql }

#[test]
fn in_and_exists_subqueries_lower_and_are_typed() {
    let s = sql_of(
        "query q() { from customers c where c.id in (from orders o where o.total > 5 select o.customer_id) select c.name }",
    );
    assert!(
        s.contains("(\"c\".\"id\" IN (SELECT \"o\".\"customer_id\" AS \"customer_id\" FROM \"orders\" AS \"o\" WHERE (\"o\".\"total\" > 5)))"),
        "{s}"
    );
    let s = sql_of("query q() { from customers c where c.id not in (from orders o select o.customer_id) select c.id }");
    assert!(s.contains("NOT IN (SELECT"), "{s}");

    // correlated: the inner query reads the outer `c`
    let s = sql_of(
        "query q() { from customers c where exists (from orders o where o.customer_id == c.id select 1) select c.id }",
    );
    assert!(s.contains("EXISTS (SELECT 1 AS \"column1\" FROM \"orders\" AS \"o\" WHERE (\"o\".\"customer_id\" = \"c\".\"id\"))"), "{s}");
    let s = sql_of(
        "query q() { from customers c where not exists (from orders o where o.customer_id == c.id select o.id) select c.id }",
    );
    assert!(s.contains("(NOT EXISTS ("), "{s}");
    // parameters are shared with the outer query
    let q = ok("query q(min: decimal(10,2)) { from customers c where exists (from orders o where o.customer_id == c.id and o.total >= :min select 1) select c.id }");
    assert_eq!(q[0].param_order, ["min"]);
}

#[test]
fn scalar_subqueries_are_typed_and_must_yield_one_row() {
    let q = &ok(
        "query q() { from customers c select c.name,
            (from orders o where o.customer_id == c.id select count(*)) as n,
            (from orders o where o.customer_id == c.id select max(o.total)) as biggest,
            (from orders o where o.customer_id == c.id select o.status order by o.id limit 1) as first_status }",
    )[0];
    let cols = &q.ir.select;
    assert_eq!((cols[1].ty.clone(), cols[1].nullable), (b(Builtin::BigInt), false), "count is never null");
    assert_eq!(cols[2].ty, TypeIR::Builtin(Builtin::Numeric(10, 2)));
    assert!(cols[2].nullable, "max over no rows");
    assert!(cols[3].nullable, "limit 1 may find no row");
    // more than one row, or more than one column, is refused
    assert_eq!(errors("query q() { from customers c select (from orders o select o.id) as x }"), ["QL241"]);
    assert_eq!(errors("query q() { from customers c select (from orders o select count(*), max(o.id)) as x }"), ["QL240"]);
    assert_eq!(errors("query q() { from customers c where c.id in (from orders o select o.id, o.total) select c.id }"), ["QL240"]);
    // the subquery's value is usable like any other
    let q = ok("query q() { from orders o where o.total > (from orders p select avg(p.total)) select o.id }");
    assert!(q[0].sql.contains("(\"o\".\"total\" > (SELECT avg(\"p\".\"total\") AS \"column1\" FROM \"orders\" AS \"p\"))"), "{}", q[0].sql);
}

#[test]
fn subquery_scopes_resolve_innermost_first() {
    // `name` exists only in customers, so it means the outer `c`; `id` is in orders, so it means `o`
    ok("query q() { from customers c where exists (from orders o where o.status == name and id > 0 select 1) select c.id }");
    // the same alias inside a subquery hides the outer one
    let s = sql_of("query q() { from customers c where exists (from customers c where c.id == 1 select 1) select c.id }");
    assert!(s.contains("FROM \"customers\" AS \"c\" WHERE"), "{s}");
    // an alias that belongs to no open query
    assert_eq!(errors("query q() { from customers c where exists (from orders o where x.id == 1 select 1) select c.id }"), ["QL205"]);
    // a subquery's own tables are not visible outside it
    assert_eq!(errors("query q() { from customers c where exists (from orders o select 1) and o.id == 1 select c.id }"), ["QL205"]);
    // ambiguity is judged within the innermost scope that has the name
    assert_eq!(
        errors("query q() { from customers c where exists (from orders o join items i on i.order_id == o.id where qty > 0 select 1) select c.id }"),
        ["QL207"]
    );
}

#[test]
fn grouping_rules_see_through_subqueries() {
    // a grouped query may use its grouped columns inside a subquery...
    ok("query q() { from customers c group by c.id select c.id, (from orders o where o.customer_id == c.id select count(*)) as n }");
    // ...but not one it has not grouped
    assert_eq!(
        errors("query q() { from customers c group by c.id select c.id, (from orders o where o.status == c.name select count(*)) as n }")
            .len(),
        1
    );
    // inside a grouped subquery, the outer query's columns are constants
    ok("query q() { from customers c where exists (from orders o where o.customer_id == c.id group by o.status select o.status, c.id) select c.id }");
    // a subquery's aggregate does not make the outer query grouped
    ok("query q() { from customers c select c.id, (from orders o select count(*)) as n }");
    // aggregates are still refused where the clause forbids them
    assert_eq!(errors("query q() { from customers c where (from orders o select count(*)) > count(*) select c.id }"), ["QL213"]);
}

#[test]
fn mutations_take_subqueries() {
    let m = mutation("update u() { orders o set paid = true where o.customer_id in (from customers c where c.role == \"admin\" select c.id) }");
    assert!(m.sql.contains("WHERE (\"o\".\"customer_id\" IN (SELECT \"c\".\"id\" AS \"id\" FROM \"customers\" AS \"c\""), "{}", m.sql);
    let m = mutation("delete d() { from orders o where not exists (from items i where i.order_id == o.id select 1) }");
    assert!(m.sql.contains("NOT EXISTS (SELECT 1"), "{}", m.sql);
    // a value that may be NULL still cannot go into a NOT NULL column, subquery or not
    assert_eq!(
        errors("insert i() { into items set order_id = 1, sku = \"x\", qty = 1, price = (from orders o select max(o.total)) }"),
        ["QL232"]
    );
    mutation("insert i() { into items set order_id = 1, sku = \"x\", qty = 1, price = coalesce((from orders o select max(o.total)), 0) }");
}

// ---- multi-row insert and insert ... select ------------------------------------------ //

#[test]
fn multi_row_insert() {
    let m = mutation(
        "insert many(a: text, b: text) { into customers (name, email) values (:a, \"a@x.com\"), (:b, \"b@x.com\") returning id, name }",
    );
    assert_eq!(
        m.sql,
        "INSERT INTO \"customers\" AS \"customers\" (\"name\", \"email\")\nVALUES (($1::text), 'a@x.com'), (($2::text), 'b@x.com')\nRETURNING \"customers\".\"id\" AS \"id\", \"customers\".\"name\" AS \"name\""
    );
    assert_eq!(m.param_order, ["a", "b"]);
    assert_eq!(m.ir.rows.len(), 2);
    assert_eq!(m.ir.insert_columns, ["name", "email"]);

    // one row in the tabular form is fine too, and the same rules apply as for `set`
    mutation("insert one(a: text) { into customers (name) values (:a) }");
    assert_eq!(errors("insert i(a: text null) { into customers (name) values (:a) }"), ["QL232"]);
    assert_eq!(errors("insert i() { into customers (name, email) values (\"a\") }"), ["QL242"]);
    assert_eq!(errors("insert i() { into customers (name) values (\"a\"), (\"b\", \"c\") }"), ["QL242"]);
    assert_eq!(errors("insert i() { into customers (name, name) values (\"a\", \"b\") }"), ["QL235"]);
    assert_eq!(errors("insert i() { into customers (name, nope) values (\"a\", 1) }"), ["QL206"]);
    assert_eq!(errors("insert i() { into orders (qty) values (1), (2) }"), ["QL233"]);
    assert_eq!(errors("insert i() { into customers (name) values (5) }"), ["QL211"]);
    // every row's errors are reported, not just the first
    assert_eq!(errors("insert i() { into customers (name) values (5), (6) }"), ["QL211", "QL211"]);
    // the upsert and returning clauses follow either form
    mutation("insert i(a: text, e: varchar(100)) { into customers (name, email) values (:a, :e) on conflict (email) do nothing }");
}

#[test]
fn insert_from_a_query() {
    let m = mutation("insert copy() { into items (order_id, sku, price, qty) from orders o where o.paid select o.id, \"x\", o.total, 1 returning id }");
    assert!(
        m.sql.starts_with("INSERT INTO \"items\" AS \"items\" (\"order_id\", \"sku\", \"price\", \"qty\")\nSELECT \"o\".\"id\" AS \"id\", 'x' AS \"column2\""),
        "{}",
        m.sql
    );
    assert!(m.sql.contains("WHERE \"o\".\"paid\""), "{}", m.sql);
    assert!(m.ir.source.is_some() && m.ir.rows.is_empty());
    // the count must match the column list
    assert_eq!(errors("insert i() { into items (order_id, sku, price, qty) from orders o select o.id }"), ["QL242"]);
    assert_eq!(errors("insert i() { into items (order_id, sku, price, qty) from orders o select o.id, o.total }"), ["QL242"]);
    // types and nullability are checked per column
    assert_eq!(errors("insert i() { into items (order_id, sku, price, qty) from customers c select c.id, c.note, 1, 1 }"), ["QL232"]);
    assert_eq!(errors("insert i() { into items (order_id, sku, price, qty) from customers c select c.id, c.id, 1, 1 }"), ["QL211"]);
    // the source may use subqueries itself
    mutation("insert i() { into items (order_id, sku, price, qty) from orders o where o.id in (from orders p select p.id) select o.id, \"x\", o.total, 1 }");
}

#[test]
fn statements_with_subqueries_serialise() {
    let (s, _) = compile(
        &schema(),
        "query q() { from customers c where exists (from orders o where o.customer_id == c.id select 1) select c.id }",
        Dialect::Postgres,
    );
    let j = to_json(&s.unwrap());
    let filter = &j[0]["ir"]["filter"];
    assert_eq!(filter["kind"], "exists");
    assert_eq!(filter["query"]["sources"][0]["table"], "orders");
    let back: QueryIR = serde_json::from_value(j[0]["ir"].clone()).unwrap();
    assert!(matches!(back.filter, Some(QExpr::Exists { .. })));
}

// ---- window functions ------------------------------------------------------------- //

#[test]
fn window_functions_lower_and_are_typed() {
    let q = &ok(
        "query q() { from orders o select o.id,
            row_number() over (partition by o.customer_id order by o.total desc) as rn,
            rank() over (order by o.total) as r,
            sum(o.total) over (partition by o.customer_id) as running,
            count(*) over () as n,
            lag(o.total) over (order by o.id) as prev,
            lead(o.total, 1, 0) over (order by o.id) as next,
            first_value(o.status) over (partition by o.customer_id order by o.id) as first_status,
            ntile(4) over (order by o.id) as quartile,
            percent_rank() over (order by o.id) as pr
            order by o.id }",
    )[0];
    assert!(
        q.sql.contains("row_number() OVER (PARTITION BY \"o\".\"customer_id\" ORDER BY \"o\".\"total\" DESC) AS \"rn\""),
        "{}",
        q.sql
    );
    assert!(q.sql.contains("count(*) OVER () AS \"n\""), "{}", q.sql);
    assert!(q.sql.contains("lead(\"o\".\"total\", 1, 0) OVER (ORDER BY \"o\".\"id\")"), "{}", q.sql);
    let c = |i: usize| (q.ir.select[i].ty.clone(), q.ir.select[i].nullable);
    assert_eq!(c(1), (b(Builtin::BigInt), false), "row_number is a non-null bigint");
    assert_eq!(c(2), (b(Builtin::BigInt), false));
    assert_eq!(c(3), (b(Builtin::Decimal), true), "sum over a window is nullable like sum");
    assert_eq!(c(4), (b(Builtin::BigInt), false), "count is never null");
    assert_eq!(c(5), (TypeIR::Builtin(Builtin::Numeric(10, 2)), true), "lag may find no earlier row");
    assert_eq!(c(6), (TypeIR::Builtin(Builtin::Numeric(10, 2)), false), "lead with a non-null default and a non-null value");
    assert_eq!(c(7), (b(Builtin::Text), false), "first_value of a NOT NULL column");
    assert_eq!(c(8), (b(Builtin::Int), false), "ntile is an integer");
    assert_eq!(c(9), (b(Builtin::Float), false));
}

#[test]
fn window_functions_in_grouped_queries_and_ordering() {
    // rank the groups by an aggregate: the aggregate is inside the window's own order by
    let q = ok("query q() { from orders o group by o.customer_id
        select o.customer_id, sum(o.total) as total, rank() over (order by sum(o.total) desc) as r order by r }");
    assert!(q[0].sql.contains("rank() OVER (ORDER BY sum(\"o\".\"total\") DESC) AS \"r\""), "{}", q[0].sql);
    // a window over an ungrouped column of a grouped query is the usual grouping error
    assert_eq!(
        errors("query q() { from orders o group by o.customer_id select o.customer_id, count(*) as n, rank() over (partition by o.status order by count(*)) as r }"),
        ["QL214"]
    );
    // a window function alone does not make the query grouped
    ok("query q() { from orders o select o.id, o.total, sum(o.total) over () as grand_total }");
    // windows may drive the ordering
    ok("query q() { from orders o select o.id order by row_number() over (order by o.total desc) }");
}

#[test]
fn window_function_rules() {
    assert_eq!(errors("query q() { from orders o where row_number() over (order by o.id) == 1 select o.id }"), ["QL251"]);
    assert_eq!(errors("query q() { from orders o group by row_number() over (order by o.id) select count(*) as n }"), ["QL251"]);
    assert_eq!(errors("query q() { from orders o select sum(count(*) over ()) over () as x }").len(), 1);
    assert_eq!(errors("query q() { from orders o select rank() over (partition by row_number() over (order by o.id)) as r }"), ["QL251"]);
    assert_eq!(errors("query q() { from orders o select row_number() as rn }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select foo(o.id) over () as x }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select ntile(0) over (order by o.id) as x }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select lag(o.id, -1) over (order by o.id) as x }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select lag(o.status, 1, 5) over (order by o.id) as x }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select count(distinct o.id) over () as x }"), ["QL252"]);
    assert_eq!(errors("query q() { from orders o select row_number(1) over () as x }"), ["QL252"]);
    // parameters work where a number is wanted
    ok("query q(n: int, k: int) { from orders o select ntile(:n) over (order by o.id) as x, lag(o.id, :k) over (order by o.id) as y }");
}

// ---- union / intersect / except --------------------------------------------------- //

#[test]
fn set_operations_lower_and_merge_their_columns() {
    let q = &ok(
        "query q(lim: int) { from customers c select c.id, c.email as contact
         union all
         from items i select i.id, i.sku
         order by id desc limit :lim }",
    )[0];
    assert_eq!(
        q.sql,
        "SELECT \"c\".\"id\" AS \"id\", \"c\".\"email\" AS \"contact\"\nFROM \"customers\" AS \"c\"\nUNION ALL\nSELECT \"i\".\"id\" AS \"id\", \"i\".\"sku\" AS \"sku\"\nFROM \"items\" AS \"i\"\nORDER BY \"id\" DESC\nLIMIT ($1::integer)"
    );
    let cols = &q.ir.select;
    assert_eq!(cols[0].name, "id");
    assert_eq!(cols[0].ty, b(Builtin::BigInt), "int and bigint merge to bigint");
    assert_eq!(cols[1].name, "contact", "the first branch names the columns");
    assert!(cols[1].nullable, "nullable in one branch makes the column nullable");
    assert!(!cols[0].nullable);
    assert_eq!(q.ir.unions.len(), 1);

    // union removes duplicates, intersect and except are available, branches have their own clauses
    let s = sql_of("query q() { from customers c select c.id union from orders o where o.paid select o.customer_id }");
    assert!(s.contains("\nUNION\nSELECT") && !s.contains("UNION ALL"), "{s}");
    assert!(sql_of("query q() { from customers c select c.id intersect from orders o select o.customer_id }").contains("\nINTERSECT\n"));
    assert!(sql_of("query q() { from customers c select c.id except from orders o select o.customer_id }").contains("\nEXCEPT\n"));
    // three branches, parameters shared across them
    let q = ok("query q(a: int, b: int) { from customers c where c.id == :a select c.id
        union from customers c where c.id == :b select c.id
        union all from orders o where o.id == :a select o.id }");
    assert_eq!(q[0].param_order, ["a", "b"]);
    assert_eq!(q[0].ir.unions.len(), 2);
    // a branch may have an aggregate and a group by of its own
    ok("query q() { from orders o group by o.status select o.status, count(*) as n union all from items i group by i.sku select i.sku, count(*) as n }");
}

#[test]
fn set_operation_rules() {
    // the shapes must line up
    assert_eq!(errors("query q() { from customers c select c.id, c.name union from orders o select o.id }"), ["QL253"]);
    assert_eq!(errors("query q() { from customers c select c.id union from orders o select o.status }"), ["QL253"]);
    assert_eq!(errors("query q() { from customers c select c.id union from orders o select o.id intersect from items i select i.id }"), ["QL253"]);
    assert_eq!(errors("query q() { from customers c select c.id intersect all from orders o select o.id }"), ["QL253"]);
    assert_eq!(errors("query q() { from customers c select c.id except all from orders o select o.id }"), ["QL253"]);
    // ordering names an output column of the whole result, not a table's column
    assert_eq!(errors("query q() { from customers c select c.id union from orders o select o.id order by c.id }"), ["QL250"]);
    assert_eq!(errors("query q() { from customers c select c.id union from orders o select o.id order by nope }"), ["QL250"]);
    assert_eq!(errors("query q() { from customers c select c.id union from orders o select o.id order by id + 1 }"), ["QL250"]);
    // a branch cannot see another branch's tables, but each reports its own mistakes
    assert_eq!(errors("query q() { from customers c select c.id union from orders o where c.id == 1 select o.id }"), ["QL205"]);
    assert_eq!(errors("query q() { from customers c select c.nope union from orders o select o.nope }"), ["QL206", "QL206"]);
    // enums combine with the same enum
    ok("query q() { from customers c select c.role union from customers d select d.role }");
    assert_eq!(errors("query q() { from customers c select c.role union from orders o select o.status }"), ["QL253"]);
}

#[test]
fn set_operations_inside_subqueries_and_inserts() {
    let s = sql_of(
        "query q() { from customers c where c.id in (from orders o select o.customer_id union from orders p select p.id) select c.id }",
    );
    assert!(s.contains("IN (SELECT \"o\".\"customer_id\" AS \"customer_id\" FROM \"orders\" AS \"o\" UNION SELECT \"p\".\"id\""), "{s}");
    let m = mutation("insert i() { into items (order_id, sku, price, qty) from orders o select o.id, \"a\", o.total, 1 union all from orders p select p.id, \"b\", p.total, 2 }");
    assert!(m.sql.contains("\nUNION ALL\nSELECT"), "{}", m.sql);
    // a compound query is not a single row, even with aggregates in each branch
    assert_eq!(
        errors("query q() { from customers c select (from orders o select count(*) union from orders p select count(*)) as x }"),
        ["QL241"]
    );
}

#[test]
fn compound_queries_serialise() {
    let (s, _) = compile(&schema(), "query q() { from customers c select c.id union from orders o select o.id order by id }", Dialect::Postgres);
    let j = to_json(&s.unwrap());
    assert_eq!(j[0]["ir"]["unions"][0]["op"], "union");
    assert_eq!(j[0]["ir"]["order_by"][0]["expr"]["column"], "id");
    let back: QueryIR = serde_json::from_value(j[0]["ir"].clone()).unwrap();
    assert_eq!(back.unions.len(), 1);
}

// ---- with (common table expressions) ---------------------------------------------- //

#[test]
fn with_queries_become_tables() {
    let q = &ok(
        "query q(min: decimal(10,2)) {
            with spend as (from orders o where o.total >= :min group by o.customer_id select o.customer_id, sum(o.total) as total, count(*) as n)
            from customers c join spend s on s.customer_id == c.id
            select c.name, s.total, s.n order by s.total desc }",
    )[0];
    assert!(
        q.sql.starts_with("WITH \"spend\" AS (SELECT \"o\".\"customer_id\" AS \"customer_id\", sum(\"o\".\"total\") AS \"total\""),
        "{}",
        q.sql
    );
    assert!(q.sql.contains("\nFROM \"customers\" AS \"c\"\nINNER JOIN \"spend\" AS \"s\" ON (\"s\".\"customer_id\" = \"c\".\"id\")"), "{}", q.sql);
    assert_eq!(q.param_order, ["min"]);
    let c = &q.ir.select;
    assert_eq!((c[0].ty.clone(), c[0].nullable), (b(Builtin::Text), false), "a NOT NULL column stays NOT NULL");
    assert_eq!((c[1].ty.clone(), c[1].nullable), (b(Builtin::Decimal), true), "the CTE's sum is nullable");
    assert_eq!((c[2].ty.clone(), c[2].nullable), (b(Builtin::BigInt), false), "its count is not");
    assert_eq!(q.ir.ctes.len(), 1);

    // a left join to a CTE makes its columns nullable, like any table
    let q = &ok("query q() { with spend as (from orders o group by o.customer_id select o.customer_id, count(*) as n)
        from customers c left join spend s on s.customer_id == c.id select c.id, s.n }")[0];
    assert!(q.ir.select[1].nullable);
}

#[test]
fn with_queries_chain_nest_and_shadow() {
    // a later one reads an earlier one; one may be used twice
    ok("query q() { with a as (from orders o select o.id, o.customer_id),
                         b as (from a x where x.id > 1 select x.id, x.customer_id)
        from a p join b q on q.id == p.id select p.id, q.customer_id }");
    // inside a subquery, and a subquery may have its own
    ok("query q() { with spend as (from orders o select o.customer_id) from customers c where c.id in (from spend s select s.customer_id) select c.id }");
    let s = sql_of("query q() { from customers c where exists (with t as (from orders o select o.customer_id) from t where t.customer_id == c.id select 1) select c.id }");
    assert!(s.contains("EXISTS (WITH \"t\" AS (SELECT"), "{s}");
    // a name hides a table of the same name, but only after it is defined
    let s = sql_of("query q() { with orders as (from orders o where o.paid select o.id) from orders x select x.id }");
    assert!(s.starts_with("WITH \"orders\" AS (SELECT \"o\".\"id\" AS \"id\" FROM \"orders\" AS \"o\""), "{s}");
    // set operations inside a with query
    ok("query q() { with ids as (from customers c select c.id union from orders o select o.customer_id) from ids i select i.id }");
    // windows and aggregates too
    ok("query q() { with ranked as (from orders o select o.id, row_number() over (order by o.total desc) as rn) from ranked r where r.rn <= 3 select r.id }");
}

#[test]
fn with_query_rules() {
    assert_eq!(errors("query q() { with a as (from orders o select o.id), a as (from orders p select p.id) from a select a.id }"), ["QL254"]);
    // its columns need names, once
    assert_eq!(errors("query q() { with a as (from orders o select o.total + 1) from orders p select p.id }"), ["QL215"]);
    assert_eq!(errors("query q() { with a as (from orders o select o.id, o.id) from a select a.id }"), ["QL216"]);
    // reading a column it does not have, or a name before it exists or outside its query
    assert_eq!(errors("query q() { with a as (from orders o select o.id) from a select a.nope }"), ["QL206"]);
    assert_eq!(errors("query q() { with a as (from b select b.id), b as (from orders o select o.id) from a select a.id }"), ["QL203"]);
    assert_eq!(errors("query q() { from customers c where exists (with t as (from orders o select o.id) from t select 1) and c.id in (from t select t.id) select c.id }"), ["QL203"]);
    // a with query does not see its own name
    assert_eq!(errors("query q() { with a as (from a select a.id) from a select a.id }"), ["QL203"]);
    // `recursive` alone is fine (it only matters to a query that reads itself)
    let (_, d) = parse("query q() { with recursive a as (from orders o select o.id) from a select a.id }");
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn mutations_take_with_queries() {
    let m = mutation(
        "update tag() { with paid_ids as (from orders o where o.paid select o.id)
            orders x set status = \"paid\" where x.id in (from paid_ids p select p.id) }",
    );
    assert!(m.sql.starts_with("WITH \"paid_ids\" AS (SELECT \"o\".\"id\" AS \"id\" FROM \"orders\" AS \"o\" WHERE \"o\".\"paid\")\nUPDATE \"orders\" AS \"x\""), "{}", m.sql);
    let m = mutation(
        "insert copy() { with src as (from orders o select o.id, o.total)
            into items (order_id, sku, price, qty) from src s select s.id, \"x\", s.total, 1 }",
    );
    assert!(m.sql.starts_with("WITH \"src\" AS (SELECT"), "{}", m.sql);
    assert!(m.sql.contains("INSERT INTO \"items\""), "{}", m.sql);
    mutation("delete gone() { with idle as (from customers c select c.id) from orders o where o.customer_id in (from idle i select i.id) }");
    // a `with` query of a mutation may use the mutation's parameters
    let m = mutation("update pick(id: int) { with one as (from orders o where o.id == :id select o.id) orders x set status = \"p\" where x.id in (from one i select i.id) }");
    assert_eq!(m.param_order, ["id"]);
    // the target of a mutation is a real table, not a with query
    assert_eq!(
        errors("update u() { with t as (from orders o select o.id) t set id = 1 all rows }"),
        ["QL203"]
    );
}

#[test]
fn with_queries_serialise() {
    let (s, _) = compile(
        &schema(),
        "query q() { with a as (from orders o select o.id) from a select a.id }",
        Dialect::Postgres,
    );
    let j = to_json(&s.unwrap());
    assert_eq!(j[0]["ir"]["ctes"][0]["name"], "a");
    assert_eq!(j[0]["ir"]["ctes"][0]["query"]["sources"][0]["table"], "orders");
    let back: QueryIR = serde_json::from_value(j[0]["ir"].clone()).unwrap();
    assert_eq!(back.ctes.len(), 1);
}

// ---- window frames ------------------------------------------------------------------ //

#[test]
fn window_frames_lower() {
    let q = &ok(
        "query q(n: int) { from orders o select o.id,
            sum(o.total) over (partition by o.customer_id order by o.id rows between unbounded preceding and current row) as running,
            avg(o.qty) over (order by o.id rows between 2 preceding and 2 following) as moving,
            count(*) over (order by o.id rows 1 preceding) as pair,
            max(o.total) over (order by o.id rows between :n preceding and current row) as recent,
            sum(o.total) over (order by o.total range between 10 preceding and current row) as near,
            count(*) over (order by o.status groups between current row and 1 following) as peers
            order by o.id }",
    )[0];
    for want in [
        "OVER (PARTITION BY \"o\".\"customer_id\" ORDER BY \"o\".\"id\" ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)",
        "OVER (ORDER BY \"o\".\"id\" ROWS BETWEEN 2 PRECEDING AND 2 FOLLOWING)",
        "OVER (ORDER BY \"o\".\"id\" ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)",
        "ROWS BETWEEN ($1::integer) PRECEDING AND CURRENT ROW",
        "RANGE BETWEEN 10 PRECEDING AND CURRENT ROW",
        "OVER (ORDER BY \"o\".\"status\" GROUPS BETWEEN CURRENT ROW AND 1 FOLLOWING)",
    ] {
        assert!(q.sql.contains(want), "missing {want}\n{}", q.sql);
    }
    assert_eq!(q.param_order, ["n"]);
}

#[test]
fn a_frame_that_may_be_empty_makes_first_and_last_value_nullable() {
    let nullable = |expr: &str| {
        let q = &ok(&format!("query q() {{ from orders o select {expr} as x }}"))[0];
        q.ir.select[0].nullable
    };
    // status is NOT NULL: with the current row in the frame the value always exists
    assert!(!nullable("first_value(o.status) over (order by o.id)"), "the default frame includes the current row");
    assert!(!nullable("first_value(o.status) over (order by o.id rows between 1 preceding and 1 following)"));
    assert!(!nullable("last_value(o.status) over (order by o.id rows between current row and unbounded following)"));
    // a frame of only later rows can be empty on the last row
    assert!(nullable("first_value(o.status) over (order by o.id rows between 1 following and 2 following)"));
    assert!(nullable("last_value(o.status) over (order by o.id rows between 3 preceding and 1 preceding)"));
    // count over an empty frame is 0, never NULL
    assert!(!nullable("count(*) over (order by o.id rows between 1 following and 2 following)"));
}

#[test]
fn window_frame_rules() {
    let frame = |over: &str| errors(&format!("query q(n: int, f: float) {{ from orders o select sum(o.total) over ({over}) as x }}"));
    // functions that ignore a frame refuse one
    assert_eq!(
        errors("query q() { from orders o select row_number() over (order by o.id rows between unbounded preceding and current row) as x }"),
        ["QL255"]
    );
    assert_eq!(errors("query q() { from orders o select lag(o.id) over (order by o.id rows 1 preceding) as x }"), ["QL255"]);
    // bounds that cannot make a frame
    assert_eq!(frame("order by o.id rows between unbounded following and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between current row and unbounded preceding"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between current row and 1 preceding"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between 1 following and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id rows 1 following"), ["QL255"]);
    // offsets
    assert_eq!(frame("order by o.id rows between -1 preceding and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between 1.5 preceding and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between :f preceding and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id rows between o.id preceding and current row"), ["QL255"]);
    // groups needs an ordering; range with an offset needs exactly one numeric key
    assert_eq!(frame("groups between current row and current row"), ["QL255"]);
    assert_eq!(frame("range between 1 preceding and current row"), ["QL255"]);
    assert_eq!(frame("order by o.id, o.total range between 1 preceding and current row"), ["QL255"]);
    assert_eq!(frame("order by o.status range between 1 preceding and current row"), ["QL255"]);
    // and what is fine
    ok("query q(n: int, d: decimal(10,2)) { from orders o select
        sum(o.total) over (order by o.total range between :d preceding and current row) as a,
        sum(o.total) over (order by o.id rows between :n preceding and :n following) as b,
        sum(o.total) over (rows between unbounded preceding and unbounded following) as c,
        sum(o.total) over (range between unbounded preceding and current row) as d2,
        sum(o.total) over (partition by o.customer_id order by o.id groups between 1 preceding and 1 following) as e }");
}

#[test]
fn frames_serialise() {
    let (s, _) = compile(
        &schema(),
        "query q() { from orders o select sum(o.total) over (order by o.id rows between 1 preceding and current row) as x }",
        Dialect::Postgres,
    );
    let j = to_json(&s.unwrap());
    let frame = &j[0]["ir"]["select"][0]["expr"]["frame"];
    assert_eq!(frame["units"], "rows");
    assert_eq!(frame["start"]["kind"], "preceding");
    assert_eq!(frame["end"]["kind"], "current_row");
    let back: QueryIR = serde_json::from_value(j[0]["ir"].clone()).unwrap();
    assert!(matches!(&back.select[0].expr, QExpr::Window { frame: Some(_), .. }));
}

// ---- with recursive ---------------------------------------------------------------- //

const TREE: &str = "
table categories {
    id: serial primary key
    parent_id: int references categories
    name: text not null
    note: text
}";

fn tree_compile(ql: &str) -> (Option<Vec<Statement>>, Vec<certo_diagnostics::Diagnostic>) {
    let (ir, d) = compile_sdl(TREE);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    compile(&schema, ql, Dialect::Postgres)
}

fn tree_ok(ql: &str) -> Vec<CompiledQuery> {
    let (s, d) = tree_compile(ql);
    s.unwrap_or_else(|| panic!("{d:?}")).into_iter().filter_map(|s| s.as_query().cloned()).collect()
}

fn tree_errors(ql: &str) -> Vec<String> {
    let (s, d) = tree_compile(ql);
    assert!(s.is_none(), "expected failure for: {ql}");
    d.iter().filter(|x| x.severity == Severity::Error).map(|x| x.code.clone()).collect()
}

const DEPTH: &str = "query q() {
    with recursive tree as (
        from categories c where c.parent_id is null select c.id, c.parent_id, c.name, 0 as depth
        union all
        from categories c join tree t on c.parent_id == t.id select c.id, c.parent_id, c.name, t.depth + 1 as depth)
    from tree t select t.name, t.depth order by t.depth, t.name }";

#[test]
fn recursive_queries_walk_a_hierarchy() {
    let q = &tree_ok(DEPTH)[0];
    assert!(q.sql.starts_with("WITH RECURSIVE \"tree\" AS (SELECT \"c\".\"id\" AS \"id\""), "{}", q.sql);
    assert!(q.sql.contains(" UNION ALL SELECT \"c\".\"id\" AS \"id\""), "{}", q.sql);
    assert!(q.sql.contains("INNER JOIN \"tree\" AS \"t\" ON (\"c\".\"parent_id\" = \"t\".\"id\")"), "{}", q.sql);
    assert_eq!(q.ir.ctes.len(), 1);
    assert!(q.ir.ctes[0].recursive);
    assert_eq!(q.ir.ctes[0].query.unions.len(), 1);
    let c = &q.ir.select;
    assert_eq!((c[0].ty.clone(), c[0].nullable), (b(Builtin::Text), false));
    assert_eq!((c[1].ty.clone(), c[1].nullable), (b(Builtin::Int), false), "depth: a literal 0, then depth + 1");

    // `union` drops duplicates instead of `union all`
    let q = &tree_ok(&DEPTH.replace("union all", "union"))[0];
    assert!(q.sql.contains(" UNION SELECT"), "{}", q.sql);
    // a plain `with` query is fine under `with recursive`, and later queries may read the recursive one
    let q = &tree_ok(
        "query q() { with recursive roots as (from categories c where c.parent_id is null select c.id),
             tree as (from categories c where c.id in (from roots r select r.id) select c.id, c.parent_id
                      union all from categories c join tree t on c.parent_id == t.id select c.id, c.parent_id),
             counted as (from tree t select count(*) as n)
           from counted select counted.n }",
    )[0];
    assert!(q.sql.starts_with("WITH RECURSIVE \"roots\" AS"), "{}", q.sql);
    assert!(q.ir.ctes[0..1].iter().all(|c| !c.recursive) && q.ir.ctes[1].recursive && !q.ir.ctes[2].recursive);
    // `recursive` on a query that does not read itself changes nothing
    assert!(!tree_ok("query q() { with recursive a as (from categories c select c.id) from a select a.id }")[0].ir.ctes[0].recursive);
}

#[test]
fn a_step_that_makes_a_column_nullable_is_taken_into_account() {
    // the starting select's label is NOT NULL; the step's is nullable, so the whole column is
    let q = &tree_ok(
        "query q() { with recursive t as (
             from categories c where c.parent_id is null select c.id, c.name as label
             union all
             from categories c join t on c.parent_id == t.id select c.id, c.note as label)
           from t select t.id, t.label, lower(t.label) as shout }",
    )[0];
    assert!(q.ir.select[1].nullable, "label can be NULL once a step contributes rows");
    assert!(q.ir.select[2].nullable);
    assert!(q.ir.ctes[0].query.select[1].nullable);
}

#[test]
fn recursive_query_rules() {
    let rec = |anchor: &str, step: &str| {
        format!(
            "query q() {{ with recursive t as ({anchor} union all {step}) from t select t.id }}"
        )
    };
    let anchor = "from categories c where c.parent_id is null select c.id, c.parent_id";
    let step = "from categories c join t on c.parent_id == t.id select c.id, c.parent_id";
    tree_ok(&rec(anchor, step));
    // a step reads the query exactly once, and not on the nullable side of a left join
    // selects that never read the query make a plain union, which is fine
    tree_ok(&rec(anchor, "from categories c select c.id, c.parent_id"));
    // but if one step reads it, every step must
    assert_eq!(
        tree_errors(&format!("query q() {{ with recursive t as ({anchor} union all {step} union all from categories c select c.id, c.parent_id) from t select t.id }}")),
        ["QL256"]
    );
    assert_eq!(
        tree_errors(&rec(anchor, "from t a join t b on a.id == b.parent_id join categories c on c.id == a.id select c.id, c.parent_id")),
        ["QL256"]
    );
    assert_eq!(tree_errors(&rec(anchor, "from categories c left join t on c.parent_id == t.id select c.id, c.parent_id")), ["QL256"]);
    // no subquery over it, no aggregates, windows, distinct, group by
    assert_eq!(
        tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id where exists (from t u select 1) select c.id, c.parent_id")),
        ["QL256"]
    );
    assert_eq!(tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id select distinct c.id, c.parent_id")), ["QL256"]);
    assert_eq!(
        tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id group by c.id, c.parent_id select c.id, c.parent_id")),
        ["QL256"]
    );
    assert_eq!(
        tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id select c.id, row_number() over (order by c.id) as parent_id")),
        ["QL256", "QL256"]
    );
    // the columns must line up with the starting select
    assert_eq!(tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id select c.id")), ["QL256"]);
    assert_eq!(tree_errors(&rec(anchor, "from categories c join t on c.parent_id == t.id select c.id, c.name")), ["QL256"]);
    // only union / union all, no ordering, not named like a table, the start cannot read itself
    assert_eq!(
        tree_errors(&format!("query q() {{ with recursive t as ({anchor} intersect {step}) from t select t.id }}")),
        ["QL256"]
    );
    assert_eq!(
        tree_errors(&format!("query q() {{ with recursive t as ({anchor} union all {step} order by id) from t select t.id }}")),
        ["QL256"]
    );
    assert_eq!(
        tree_errors(&format!("query q() {{ with recursive categories as ({anchor} union all from categories c join categories t on c.parent_id == t.id select c.id, c.parent_id) from categories select categories.id }}")),
        ["QL256"]
    );
    assert_eq!(
        tree_errors("query q() { with recursive t as (from t select t.id union all from categories c join t on c.parent_id == t.id select c.id) from t select t.id }"),
        ["QL203"]
    );
    // a mistake in the starting select is reported once, not once per round
    assert_eq!(tree_errors(&rec("from categories c select c.nope, c.parent_id", step)), ["QL206"]);
}

#[test]
fn mutations_take_recursive_queries() {
    let (s, d) = tree_compile(
        "delete prune() { with recursive sub as (
             from categories c where c.id == 1 select c.id
             union all from categories c join sub s on c.parent_id == s.id select c.id)
           from categories x where x.id in (from sub s select s.id) }",
    );
    let s = s.unwrap_or_else(|| panic!("{d:?}"));
    let m = s[0].as_mutation().unwrap();
    assert!(m.sql.starts_with("WITH RECURSIVE \"sub\" AS"), "{}", m.sql);
    assert!(m.sql.contains("\nDELETE FROM \"categories\" AS \"x\""), "{}", m.sql);
}

#[test]
fn recursive_queries_serialise() {
    let (s, _) = tree_compile(DEPTH);
    let j = to_json(&s.unwrap());
    assert_eq!(j[0]["ir"]["ctes"][0]["recursive"], true);
    assert_eq!(j[0]["ir"]["ctes"][0]["query"]["unions"][0]["op"], "union");
    let back: QueryIR = serde_json::from_value(j[0]["ir"].clone()).unwrap();
    assert!(back.ctes[0].recursive);
}

// ---- `||` ----------------------------------------------------------------------------- //

#[test]
fn concat_lowers_and_is_typed() {
    let q = &ok("query q(n: text) { from customers c select c.name || \" <\" || c.email || \">\" as who, c.name || \"!\" as shout, \"hi \" || :n as greeting }")[0];
    assert!(q.sql.contains("(\"c\".\"name\" || ' <' || \"c\".\"email\" || '>') AS \"who\""), "{}", q.sql);
    assert!(q.sql.contains("('hi ' || ($1::text)) AS \"greeting\""), "{}", q.sql);
    let c = &q.ir.select;
    assert_eq!((c[0].ty.clone(), c[0].nullable), (b(Builtin::Text), true), "NULL if any part is: email is nullable");
    assert_eq!((c[1].ty.clone(), c[1].nullable), (b(Builtin::Text), false));
    assert_eq!((c[2].ty.clone(), c[2].nullable), (b(Builtin::Text), false));
    // a nullable parameter makes the result nullable
    let q = &ok("query q(n: text null) { from customers c select \"hi \" || :n as g }")[0];
    assert!(q.ir.select[0].nullable);
}

#[test]
fn concat_joins_text_with_whole_numbers_enums_and_uuids_only() {
    ok("query q() { from customers c select c.name || c.id as a, c.name || c.role as b, c.name || c.token as c, \"n=\" || 5 as d }");
    // nothing else prints the same in PostgreSQL and SQLite
    assert_eq!(errors("query q() { from customers c select c.name || c.balance as x }"), ["QL257"]);
    assert_eq!(errors("query q() { from customers c select c.name || c.born as x }"), ["QL257"]);
    assert_eq!(errors("query q() { from customers c select c.name || (c.id == 1) as x }"), ["QL257"]);
    assert_eq!(errors("query q() { from customers c select c.name || 1.5 as x }"), ["QL257"]);
    // at least one side is text
    assert_eq!(errors("query q() { from customers c select c.id || 5 as x }"), ["QL257"]);
    assert_eq!(errors("query q() { from customers c select null || null as x }"), ["QL257"]);
    // a NULL literal is fine next to text, and the result may be NULL
    let q = &ok("query q() { from customers c select c.name || null as x }")[0];
    assert!(q.ir.select[0].nullable);
    // an error in a part is reported once
    assert_eq!(errors("query q() { from customers c select c.name || c.nope as x }"), ["QL206"]);
}

#[test]
fn concat_precedence_is_additive_and_left_to_right() {
    // looser than nothing but comparison: `||` binds tighter than `==`
    let s = sql_of("query q() { from customers c where c.name || \"x\" == \"ax\" select c.id }");
    assert!(s.contains("((\"c\".\"name\" || 'x') = 'ax')"), "{s}");
    // like `+`: arithmetic before it is finished first, after it is not
    let s = sql_of("query q() { from customers c select (1 + 2) || \"a\" || c.name as x }");
    assert!(s.contains("(((1 + 2)) || 'a' || \"c\".\"name\")") || s.contains("((1 + 2) || 'a' || \"c\".\"name\")"), "{s}");
    assert_eq!(errors("query q() { from customers c select \"a\" || 1 + 2 as x }"), ["QL211"]);
    // grouping, windows and aggregates see through it
    ok("query q() { from customers c group by c.name || \"!\" select c.name || \"!\" as g, count(*) as n }");
    ok("query q() { from customers c select max(c.name || \"x\") as m }");
    assert_eq!(errors("query q() { from customers c group by c.id select c.name || \"!\" as g }"), ["QL214"]);
}

#[test]
fn recursive_queries_can_build_paths() {
    let q = &tree_ok(
        "query q() { with recursive t as (
            from categories c where c.parent_id is null select c.id, c.name as path
            union all
            from categories c join t on c.parent_id == t.id select c.id, t.path || \"/\" || c.name as path)
          from t select t.path order by t.path }",
    )[0];
    assert!(q.sql.contains("(\"t\".\"path\" || '/' || \"c\".\"name\") AS \"path\""), "{}", q.sql);
    assert!(!q.ir.select[0].nullable);
}

// ---- text and date functions ---------------------------------------------------------------- //

#[test]
fn text_and_date_functions_are_typed() {
    let q = &ok("query q() { from customers c select substr(c.name, 2) as a, substr(c.email, 1, 3) as b, replace(c.name, \"a\", \"b\") as c2,
        position(c.name, \"a\") as d, position(c.email, \"@\") as e, date_part(\"year\", c.born) as f, date_part(\"day\", c.born) as g }")[0];
    let s: Vec<_> = q.ir.select.iter().map(|c| (c.ty.clone(), c.nullable)).collect();
    assert_eq!(s[0], (b(Builtin::Text), false));
    assert_eq!(s[1], (b(Builtin::Text), true), "email is nullable");
    assert_eq!(s[2], (b(Builtin::Text), false));
    assert_eq!(s[3], (b(Builtin::Int), false));
    assert_eq!(s[4], (b(Builtin::Int), true));
    assert_eq!(s[5], (b(Builtin::Int), true), "born is nullable");
    assert!(q.sql.contains("substr(\"c\".\"name\", 2)"), "{}", q.sql);
    assert!(q.sql.contains("strpos(\"c\".\"name\", 'a')"), "{}", q.sql);
    assert!(q.sql.contains("CAST(EXTRACT(YEAR FROM \"c\".\"born\") AS integer)"), "{}", q.sql);
    // timestamps allow hours and minutes
    ok("query q() { from orders o select date_part(\"hour\", o.created) as h, date_part(\"minute\", o.shipped) as m }");
}

#[test]
fn text_and_date_function_errors() {
    assert_eq!(errors("query q() { from customers c select substr(c.id, 1) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select substr(c.name, 0) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select substr(c.name, 1, -1) as x }"), ["QL209"]);
    assert_eq!(errors("query q(n: int) { from customers c select substr(c.name, :n) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select replace(c.name, \"a\") as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select position(c.name, 1) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select date_part(\"week\", c.born) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select date_part(\"hour\", c.born) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select date_part(\"year\", c.name) as x }"), ["QL209"]);
    assert_eq!(errors("query q() { from customers c select date_part(c.name, c.born) as x }"), ["QL209"]);
}

#[test]
fn text_and_date_functions_lower_per_dialect() {
    let (s, d) = compile(&schema(), "query q() { from customers c select position(c.name, \"a\") as p, date_part(\"month\", c.born) as m }", Dialect::Sqlite);
    let s = s.unwrap_or_else(|| panic!("{d:?}"));
    let sql = &s[0].as_query().unwrap().sql;
    assert!(sql.contains("instr(\"c\".\"name\", 'a')"), "{sql}");
    assert!(sql.contains("CAST(strftime('%m', \"c\".\"born\") AS INTEGER)"), "{sql}");
}
