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
