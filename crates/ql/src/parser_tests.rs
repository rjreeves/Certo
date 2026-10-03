use crate::*;
use certo_sdl::BinaryOp;

fn ok(src: &str) -> QlFile {
    let (f, d) = parse(src);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    f
}

fn one(body: &str) -> Query {
    ok(&format!("query q() {{ {body} }}")).queries.remove(0)
}

fn where_of(cond: &str) -> Expr {
    one(&format!("from t where {cond} select t.a")).filter.unwrap()
}

#[test]
fn every_clause() {
    let f = ok("query recent(min_total: decimal, since: timestamp null, name: varchar(20)) {
        from orders o
        inner join customers as c on o.customer_id == c.id
        left join notes n on n.order_id = o.id
        where o.total >= :min_total and o.created > :since
        group by c.name
        having count(*) > 1
        select distinct c.name, sum(o.total) as total
        order by total desc, c.name asc
        limit 50
        offset :skip
    }");
    let q = &f.queries[0];
    assert_eq!(q.name.name, "recent");
    assert_eq!(q.params.len(), 3);
    assert!(!q.params[0].nullable && q.params[1].nullable);
    assert_eq!(q.params[2].ty.args, [20]);
    assert_eq!(q.from.table.name, "orders");
    assert_eq!(q.from.alias.as_ref().unwrap().name, "o");
    assert_eq!(q.joins.len(), 2);
    assert_eq!(q.joins[0].kind, JoinKind::Inner);
    assert_eq!(q.joins[0].table.alias.as_ref().unwrap().name, "c");
    assert_eq!(q.joins[1].kind, JoinKind::Left);
    assert!(q.filter.is_some() && q.having.is_some());
    assert_eq!(q.group_by.len(), 1);
    assert!(q.select.distinct);
    assert_eq!(q.select.items.len(), 2);
    assert!(matches!(&q.select.items[1], SelectItem::Expr { alias: Some(a), .. } if a.name == "total"));
    assert_eq!(q.order_by.len(), 2);
    assert!(q.order_by[0].desc && !q.order_by[1].desc);
    assert!(matches!(q.limit, Some(Expr::Number(50, _))));
    assert!(matches!(q.offset, Some(Expr::Param(_))));
}

#[test]
fn several_queries_in_one_file_and_minimal_forms() {
    let f = ok("query a() { from t select t.x }  query b(p: int) { from u select * }");
    assert_eq!(f.queries.len(), 2);
    assert!(matches!(f.queries[1].select.items[0], SelectItem::Star(_)));
    let q = one("from t x join u on x.a == u.a select x.*, u.b");
    assert!(matches!(&q.select.items[0], SelectItem::SourceStar(i) if i.name == "x"));
    // aliases are optional and never swallow a clause word
    let q = one("from t where t.a > 1 select t.a");
    assert!(q.from.alias.is_none());
    let q = one("from t left join u on t.a == u.a select t.a");
    assert!(q.joins[0].table.alias.is_none());
}

#[test]
fn expressions_and_precedence() {
    // or < and < not < comparison < + - < * /
    let Expr::Binary { op: BinaryOp::Or, rhs, .. } = where_of("a == 1 or b == 2 and c == 3") else { panic!() };
    assert!(matches!(*rhs, Expr::Binary { op: BinaryOp::And, .. }));
    let Expr::Binary { op: BinaryOp::And, lhs, .. } = where_of("not a == 1 and b == 2") else { panic!() };
    assert!(matches!(*lhs, Expr::Not(..)));
    let Expr::Binary { op: BinaryOp::Gt, lhs, .. } = where_of("a + b * c > 1") else { panic!() };
    let Expr::Binary { op: BinaryOp::Add, rhs, .. } = *lhs else { panic!() };
    assert!(matches!(*rhs, Expr::Binary { op: BinaryOp::Mul, .. }));
    // both `==` and `=` mean equality; `<>` and `!=` mean not-equal
    assert!(matches!(where_of("a = 1"), Expr::Binary { op: BinaryOp::Eq, .. }));
    assert!(matches!(where_of("a == 1"), Expr::Binary { op: BinaryOp::Eq, .. }));
    assert!(matches!(where_of("a <> 1"), Expr::Binary { op: BinaryOp::Ne, .. }));
    assert!(matches!(where_of("a != 1"), Expr::Binary { op: BinaryOp::Ne, .. }));
    assert!(matches!(where_of("a <= 1 and b >= 2 and c < 3"), Expr::Binary { .. }));
}

#[test]
fn predicates() {
    assert!(matches!(where_of("a is null"), Expr::IsNull { negated: false, .. }));
    assert!(matches!(where_of("a is not null"), Expr::IsNull { negated: true, .. }));
    assert!(matches!(where_of("a in (1, 2, 3)"), Expr::In { negated: false, ref list, .. } if list.len() == 3));
    assert!(matches!(where_of("a not in (:x, :y)"), Expr::In { negated: true, .. }));
    assert!(matches!(where_of("a like \"x%\""), Expr::Like { negated: false, .. }));
    assert!(matches!(where_of("a not like :p"), Expr::Like { negated: true, .. }));
    assert!(matches!(where_of("a between 1 and 10"), Expr::Between { negated: false, .. }));
    assert!(matches!(where_of("a not between :lo and :hi"), Expr::Between { negated: true, .. }));
    // `between ... and` does not swallow the `and` that follows the predicate
    let Expr::Binary { op: BinaryOp::And, lhs, .. } = where_of("a between 1 and 10 and b == 2") else { panic!() };
    assert!(matches!(*lhs, Expr::Between { .. }));
}

#[test]
fn literals_parameters_columns_calls_and_case() {
    assert!(matches!(where_of("a == -5"), Expr::Binary { rhs, .. } if matches!(*rhs, Expr::Number(-5, _))));
    assert!(matches!(where_of("a > -1.5"), Expr::Binary { rhs, .. } if matches!(*rhs, Expr::Decimal(ref d, _) if d == "-1.5")));
    assert!(matches!(where_of("a == \"x\""), Expr::Binary { rhs, .. } if matches!(*rhs, Expr::Str(ref s, _) if s == "x")));
    assert!(matches!(where_of("a == true"), Expr::Binary { rhs, .. } if matches!(*rhs, Expr::Bool(true, _))));
    assert!(matches!(where_of("a == :p"), Expr::Binary { rhs, .. } if matches!(*rhs, Expr::Param(ref i) if i.name == "p")));
    let Expr::Binary { lhs, .. } = where_of("t.a == 1") else { panic!() };
    assert!(matches!(*lhs, Expr::Column { qualifier: Some(ref q), ref name } if q.name == "t" && name.name == "a"));

    let q = one("from t select count(*) as n, count(distinct t.a) as d, sum(t.b) as s, lower(t.c) as l, coalesce(t.a, 0) as c");
    let calls: Vec<_> = q.select.items.iter().map(|i| match i {
        SelectItem::Expr { expr: Expr::Call { func, star, distinct, args, .. }, .. } => (func.name.clone(), *star, *distinct, args.len()),
        _ => panic!(),
    }).collect();
    assert_eq!(calls, [("count".into(), true, false, 0), ("count".into(), false, true, 1), ("sum".into(), false, false, 1), ("lower".into(), false, false, 1), ("coalesce".into(), false, false, 2)]);

    let q = one("from t select case when t.a > 1 then \"big\" when t.a > 0 then \"small\" else \"none\" end as size");
    let SelectItem::Expr { expr: Expr::Case { whens, otherwise, .. }, .. } = &q.select.items[0] else { panic!() };
    assert_eq!(whens.len(), 2);
    assert!(otherwise.is_some());
    let q = one("from t select case when t.a is null then 0 end as z");
    assert!(matches!(&q.select.items[0], SelectItem::Expr { expr: Expr::Case { otherwise: None, .. }, .. }));
}

#[test]
fn column_names_that_look_like_keywords() {
    // words are only special where the grammar expects them
    ok("query q() { from t where t.in == 1 and t.like == 2 select t.case, t.end }");
}

#[test]
fn syntax_errors_recover_at_the_next_query() {
    let (f, d) = parse("query a( { }  query b() { from t select t.x }  query c() { select }  query d() { from u select u.y }");
    assert!(d.len() >= 2, "{d:?}");
    let names: Vec<_> = f.queries.iter().map(|q| q.name.name.as_str()).collect();
    assert_eq!(names, ["b", "d"]);
    for bad in [
        "", "query", "query q", "query q() {", "query q() { from }", "query q() { from t }",
        "query q() { from t select }", "query q() { select 1 }", "query q() { from t select t.a limit }",
        "query q() { from t join u select t.a }", "query q() { from t where select t.a }",
        "query q(a) { from t select t.a }", "query q() { from t select case end }",
        "query q() { from t select t.a as }", "query q() { from t select (t.a }",
    ] {
        let (_, d) = parse(bad);
        if bad.is_empty() { assert!(d.is_empty()); } else { assert!(!d.is_empty(), "{bad:?} should be an error"); }
    }
}

#[test]
fn oversized_and_deeply_nested_expressions_are_refused_not_crashed() {
    let chain = vec!["1"; 5000].join(" + ");
    let (_, d) = parse(&format!("query q() {{ from t where t.a > {chain} select t.a }}"));
    assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    let nested = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let (_, d) = parse(&format!("query q() {{ from t where t.a > {nested} select t.a }}"));
    assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    let nots = format!("{}true", "not ".repeat(5000));
    let (_, d) = parse(&format!("query q() {{ from t where {nots} select t.a }}"));
    assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    let cases = format!("{}1{}", "case when true then ".repeat(2000), " end".repeat(2000));
    let (_, d) = parse(&format!("query q() {{ from t select {cases} as x }}"));
    assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    // a long flat IN list is fine
    let list = vec!["1"; 300].join(", ");
    ok(&format!("query q() {{ from t where t.a in ({list}) select t.a }}"));
}

#[test]
fn a_parameter_may_follow_any_keyword_operator() {
    // regression: `word :name` must not be mistaken for something else
    assert!(matches!(where_of("a like :p"), Expr::Like { negated: false, .. }));
    assert!(matches!(where_of("a between :lo and :hi"), Expr::Between { negated: false, .. }));
    assert!(matches!(where_of("a == 1 and :flag"), Expr::Binary { op: BinaryOp::And, .. }));
    assert!(matches!(where_of("a == 1 or :flag"), Expr::Binary { op: BinaryOp::Or, .. }));
    assert!(matches!(where_of(":p and :q"), Expr::Binary { op: BinaryOp::And, .. }));
    assert!(matches!(where_of("not :flag"), Expr::Not(..)));
    assert!(matches!(where_of("a in (:x, :y) and b like :z"), Expr::Binary { op: BinaryOp::And, .. }));
    assert!(matches!(where_of("a is null or :p is not null"), Expr::Binary { op: BinaryOp::Or, .. }));
    assert!(matches!(where_of("a not in (:x) and b not like :p and c not between :lo and :hi"), Expr::Binary { .. }));
}


// ---- mutations --------------------------------------------------------------- //

fn mutation(src: &str) -> Mutation {
    let (f, d) = parse(src);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    f.mutations.into_iter().next().expect("a mutation")
}

#[test]
fn insert_update_delete() {
    let m = mutation("insert add(name: text, email: varchar(100) null) {
        into customers c
        set name = :name, email = :email, role = \"user\"
        returning c.id, c.name as who
    }");
    assert_eq!(m.kind, MutationKind::Insert);
    assert_eq!(m.name.name, "add");
    assert_eq!(m.params.len(), 2);
    assert!(m.params[1].nullable);
    assert_eq!(m.table.table.name, "customers");
    assert_eq!(m.table.alias.as_ref().unwrap().name, "c");
    assert_eq!(m.assignments.len(), 3);
    assert_eq!(m.assignments[0].column.name, "name");
    assert!(m.conflict.is_none() && m.filter.is_none() && !m.all_rows);
    assert_eq!(m.returning.len(), 2);

    let m = mutation("update rename(id: int, n: text) { customers set name = :n, note = null where customers.id == :id returning * }");
    assert_eq!(m.kind, MutationKind::Update);
    assert!(m.table.alias.is_none(), "`set` is not an alias");
    assert_eq!(m.assignments.len(), 2);
    assert!(m.filter.is_some() && !m.all_rows);
    assert!(matches!(m.returning[0], SelectItem::Star(_)));

    let m = mutation("delete purge(before: timestamp) { from orders o where o.created < :before }");
    assert_eq!(m.kind, MutationKind::Delete);
    assert!(m.assignments.is_empty() && m.filter.is_some() && m.returning.is_empty());
}

#[test]
fn all_rows_must_be_said_out_loud() {
    let m = mutation("delete wipe() { from orders all rows }");
    assert!(m.all_rows && m.filter.is_none());
    let m = mutation("update touch() { orders o set qty = 1 all rows returning o.id }");
    assert!(m.all_rows && m.returning.len() == 1);
    // leaving out the condition is an error, not a silent full-table write
    for bad in ["delete wipe() { from orders }", "update t() { orders set qty = 1 }", "delete d() { from orders returning orders.id }"] {
        let (_, d) = parse(bad);
        assert!(d.iter().any(|x| x.message.contains("`where <condition>` or `all rows`")), "{bad}: {d:?}");
    }
    // `all` alone is not enough
    let (_, d) = parse("delete wipe() { from orders all }");
    assert!(!d.is_empty());
}

#[test]
fn upserts() {
    let m = mutation("insert up(email: text, name: text) {
        into customers c set email = :email, name = :name
        on conflict (email) do update set name = excluded.name, note = c.note
        returning c.id
    }");
    let c = m.conflict.unwrap();
    assert_eq!(c.columns.len(), 1);
    let ConflictAction::Update(sets) = c.action else { panic!() };
    assert_eq!(sets.len(), 2);
    assert_eq!(m.returning.len(), 1);
    let m = mutation("insert ins(a: int, b: int) { into t set a = :a on conflict (a, b) do nothing }");
    let c = m.conflict.unwrap();
    assert_eq!(c.columns.len(), 2);
    assert!(matches!(c.action, ConflictAction::Nothing));
}

#[test]
fn queries_and_mutations_share_a_file_and_recovery_finds_the_next_statement() {
    let (f, d) = parse("query a() { from t select t.x }
        insert i() { into t set x = 1 }
        query b() { from t select t.y }
        delete d() { from t where t.x > 1 }
        update u() { t set x = 2 where t.x == 1 }");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!((f.queries.len(), f.mutations.len()), (2, 3));
    let kinds: Vec<_> = f.mutations.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [MutationKind::Insert, MutationKind::Delete, MutationKind::Update]);
    // a broken statement does not hide the others
    let (f, d) = parse("insert i( { into t set x = 1 } update ok() { t set x = 2 where t.x == 1 } delete d() { from }");
    assert!(d.len() >= 2, "{d:?}");
    assert_eq!(f.mutations.len(), 1);
    assert_eq!(f.mutations[0].name.name, "ok");
    for bad in [
        "insert", "insert i", "insert i() {", "insert i() { set x = 1 }", "insert i() { into t }", "insert i() { into t set }",
        "insert i() { into t set x }", "insert i() { into t set x = }", "update u() { where true }",
        "insert i() { into t set x = 1 on conflict do nothing }", "insert i() { into t set x = 1 on conflict (x) }",
        "insert i() { into t set x = 1 on conflict (x) do update }", "delete d() { t where true }",
    ] {
        let (_, d) = parse(bad);
        assert!(!d.is_empty(), "{bad:?} should be an error");
    }
}

#[test]
fn a_column_may_be_called_like_a_statement_keyword() {
    ok("query q() { from t select t.update, t.delete, t.insert, t.set }");
    let m = mutation("update u() { t set returning = 1, all = 2 where t.set == 1 }");
    assert_eq!(m.assignments[0].column.name, "returning");
}

// ---- subqueries and tabular inserts ------------------------------------------------ //

#[test]
fn subquery_forms_parse() {
    let e = where_of("a in (from t select t.x)");
    assert!(matches!(e, Expr::InQuery { negated: false, .. }));
    assert!(matches!(where_of("a not in (from t select t.x)"), Expr::InQuery { negated: true, .. }));
    assert!(matches!(where_of("exists (from t where t.a == 1 select 1)"), Expr::Exists(..)));
    let Expr::Binary { rhs, .. } = where_of("a > (from t select max(t.x))") else { panic!() };
    assert!(matches!(*rhs, Expr::Scalar(..)));
    // a plain list and a plain parenthesised expression are unchanged
    assert!(matches!(where_of("a in (1, 2)"), Expr::In { .. }));
    assert!(matches!(where_of("(a + 1) > 2"), Expr::Binary { .. }));
    // `exists` stays usable as a column name
    assert!(matches!(where_of("exists == 1"), Expr::Binary { .. }));
}

#[test]
fn subqueries_nest_but_only_so_far() {
    let mut src = String::from("1 == 1");
    for _ in 0..6 {
        src = format!("exists (from t where {src} select 1)");
    }
    let (_, d) = parse(&format!("query q() {{ from t where {src} select t.x }}"));
    assert!(d.is_empty(), "{d:?}");
    let src = format!("exists (from t where {src} select 1)");
    let (_, d) = parse(&format!("query q() {{ from t where {src} select t.x }}"));
    assert!(d.iter().any(|x| x.code == "QL243"), "{d:?}");
}

#[test]
fn tabular_insert_forms_parse() {
    let m = mutation("insert i() { into t (a, b) values (1, 2), (3, 4) }");
    assert_eq!(m.insert_columns.len(), 2);
    assert_eq!(m.rows.len(), 2);
    assert!(m.assignments.is_empty() && m.source.is_none());
    let m = mutation("insert i() { into t (a) from u select u.x returning a }");
    assert!(m.source.is_some() && m.rows.is_empty());
    assert_eq!(m.returning.len(), 1);
    let (_, d) = parse("insert i() { into t (a) }");
    assert!(d.iter().any(|x| x.message.contains("values")), "{d:?}");
}

// ---- window functions ------------------------------------------------------------- //

#[test]
fn window_specs_parse() {
    let q = one("from t select row_number() over (partition by t.a, t.b order by t.c desc, t.d) as rn");
    let SelectItem::Expr { expr: Expr::Call { func, over: Some(w), .. }, .. } = &q.select.items[0] else { panic!() };
    assert_eq!(func.name, "row_number");
    assert_eq!(w.partition_by.len(), 2);
    assert_eq!(w.order_by.len(), 2);
    assert!(w.order_by[0].desc && !w.order_by[1].desc);
    // empty, partition-only and order-only forms
    for spec in ["", "partition by t.a", "order by t.a"] {
        let q = one(&format!("from t select count(*) over ({spec}) as n"));
        assert!(matches!(&q.select.items[0], SelectItem::Expr { expr: Expr::Call { over: Some(_), star: true, .. }, .. }));
    }
    // a plain call has no window; `over` stays usable as a column name
    let q = one("from t select count(*) as n, t.over as o");
    assert!(matches!(&q.select.items[0], SelectItem::Expr { expr: Expr::Call { over: None, .. }, .. }));
}

// ---- set operations ------------------------------------------------------------------ //

#[test]
fn set_operations_parse() {
    let q = one("from t select t.a union all from u select u.a intersect from v select v.a order by a limit 5 offset 2");
    assert_eq!(q.compound.len(), 2);
    assert_eq!((q.compound[0].op, q.compound[0].all), (SetOp::Union, true));
    assert_eq!((q.compound[1].op, q.compound[1].all), (SetOp::Intersect, false));
    // the trailing clauses belong to the whole combination, not to the last branch
    assert_eq!(q.order_by.len(), 1);
    assert!(q.limit.is_some() && q.offset.is_some());
    assert!(q.compound.iter().all(|b| b.query.order_by.is_empty() && b.query.limit.is_none()));
    assert!(one("from t select t.a except from u select u.a").compound[0].op == SetOp::Except);
    // a plain query has none
    assert!(one("from t select t.a order by a").compound.is_empty());
    // a branch needs its own `from` and `select`
    let (_, d) = parse("query q() { from t select t.a union select 1 }");
    assert!(!d.is_empty());
}

// ---- with ------------------------------------------------------------------------------ //

#[test]
fn with_queries_parse() {
    let q = one("with a as (from t select t.x), b as (from a select a.x) from b select b.x");
    assert_eq!(q.ctes.len(), 2);
    assert_eq!(q.ctes[0].name.name, "a");
    assert_eq!(q.ctes[1].query.from.table.name, "a");
    assert!(one("from t select t.x").ctes.is_empty());
    // a with query may itself be a combination, and ordering still belongs to the main query
    let q = one("with a as (from t select t.x union from u select u.x) from a select a.x order by x");
    assert_eq!(q.ctes[0].query.compound.len(), 1);
    assert_eq!(q.order_by.len(), 1);
    // in a mutation, before the statement's own clauses
    let m = mutation("update u() { with a as (from t select t.x) t set y = 1 where t.x in (from a select a.x) }");
    assert_eq!(m.ctes.len(), 1);
    assert!(mutation("update u() { t set y = 1 all rows }").ctes.is_empty());
    let (_, d) = parse("query q() { with recursive a as (from t select t.x) from a select a.x }");
    assert!(d.iter().any(|x| x.code == "QL254"), "{d:?}");
}

// ---- window frames ---------------------------------------------------------------------- //

#[test]
fn window_frames_parse() {
    let frame_of = |over: &str| -> Frame {
        let q = one(&format!("from t select sum(t.a) over ({over}) as s"));
        let SelectItem::Expr { expr: Expr::Call { over: Some(w), .. }, .. } = &q.select.items[0] else { panic!() };
        *w.frame.clone().expect("a frame")
    };
    let f = frame_of("order by t.b rows between unbounded preceding and current row");
    assert_eq!(f.units, FrameUnits::Rows);
    assert!(matches!(f.start, FrameBound::UnboundedPreceding) && matches!(f.end, Some(FrameBound::CurrentRow)));
    let f = frame_of("order by t.b range between 5 preceding and 2 following");
    assert_eq!(f.units, FrameUnits::Range);
    assert!(matches!(f.start, FrameBound::Preceding(Expr::Number(5, _))) && matches!(f.end, Some(FrameBound::Following(_))));
    let f = frame_of("order by t.b groups between current row and unbounded following");
    assert_eq!(f.units, FrameUnits::Groups);
    assert!(matches!(f.end, Some(FrameBound::UnboundedFollowing)));
    // the one-bound form, and a parameter as the offset
    let f = frame_of("order by t.b rows 3 preceding");
    assert!(f.end.is_none() && matches!(f.start, FrameBound::Preceding(_)));
    let f = frame_of("order by t.b rows between :n preceding and :n following");
    assert!(matches!(f.start, FrameBound::Preceding(Expr::Param(_))));
    // a frame without an order by, and none at all
    assert!(frame_of("rows between unbounded preceding and unbounded following").units == FrameUnits::Rows);
    let q = one("from t select sum(t.a) over (order by t.b) as s");
    let SelectItem::Expr { expr: Expr::Call { over: Some(w), .. }, .. } = &q.select.items[0] else { panic!() };
    assert!(w.frame.is_none());
    // `rows` and `range` remain usable as column names
    one("from t select t.rows, t.range");
    // a frame must be complete
    let (_, d) = parse("query q() { from t select sum(t.a) over (order by t.b rows between 1 preceding) as s }");
    assert!(!d.is_empty());
}
