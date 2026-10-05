//! What QL offers on PostgreSQL and SQLite but MySQL cannot do the same way. Rather than give a different answer,
//! these are refused for MySQL, each with a reason (`QL270` to `QL273`):
//!
//! * `returning` (MySQL has no `RETURNING`);
//! * a window frame in `groups` units;
//! * `on conflict`, unless its target is the table's only unique key and the rows come from `values`/`set`
//!   (MySQL's upsert fires on *any* unique key, and cannot take a row alias with `insert ... select`);
//! * an `update` or `delete` whose conditions read the table being changed in a subquery (MySQL error 1093).

use crate::ast::FrameUnits;
use crate::ir::*;
use certo_diagnostics::Diagnostic;
use certo_sdl::SchemaIR;

fn fail(diags: &mut Vec<Diagnostic>, code: &str, name: &str, msg: String) {
    diags.push(Diagnostic::error(code, format!("`{name}`: {msg}")));
}

/// Check compiled statements for MySQL.
pub(crate) fn check(schema: &SchemaIR, queries: &[QueryIR], mutations: &[MutationIR], diags: &mut Vec<Diagnostic>) {
    for q in queries {
        let mut frames = false;
        walk_query_exprs(q, &mut |e| frames |= groups_frame(e));
        if frames {
            fail(diags, "QL271", &q.name, "MySQL has no `groups` window frames: use `rows` or `range`".into());
        }
    }
    for m in mutations {
        let name = &m.name;
        let mut frames = false;
        walk_mutation_exprs(m, &mut |e| frames |= groups_frame(e));
        if frames {
            fail(diags, "QL271", name, "MySQL has no `groups` window frames: use `rows` or `range`".into());
        }
        if !m.returning.is_empty() {
            fail(diags, "QL270", name, "MySQL has no `returning`: read the row back with a query (an insert's key is the connection's last insert id)".into());
        }
        if let Some(c) = &m.conflict {
            if m.source.is_some() {
                fail(diags, "QL272", name, "on MySQL, `on conflict` works with `set` or `values`, not with an `insert ... select`".into());
            }
            if let Some(t) = schema.table(&m.table) {
                let mut keys: Vec<Vec<&str>> = Vec::new();
                let pk: Vec<&str> = t.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
                if !pk.is_empty() {
                    keys.push(pk);
                }
                keys.extend(t.columns.iter().filter(|c| c.unique && !c.primary_key).map(|c| vec![c.name.as_str()]));
                let target: Vec<&str> = c.columns.iter().map(String::as_str).collect();
                let same = |k: &Vec<&str>| k.len() == target.len() && k.iter().all(|x| target.contains(x));
                if keys.len() != 1 || !same(&keys[0]) {
                    fail(
                        diags,
                        "QL272",
                        name,
                        format!(
                            "MySQL's upsert reacts to any unique key, so `on conflict` is only offered where its target is the table's only unique key (`{}` has {})",
                            m.table,
                            if keys.is_empty() { "none".to_string() } else { format!("{} unique keys", keys.len()) }
                        ),
                    );
                }
            }
        }
        if m.kind != crate::ast::MutationKind::Insert {
            let mut reads_target = false;
            for e in [m.filter.as_ref()].into_iter().flatten().chain(m.assignments.iter().map(|a| &a.expr)) {
                visit(e, &mut |x| {
                    if let Some(sub) = subquery_of(x) {
                        reads_target |= sub_reads(sub, &m.table);
                    }
                });
            }
            if reads_target {
                fail(diags, "QL273", name, format!("MySQL cannot read `{}` in a subquery while changing it; read the keys first, or use a `with` query", m.table));
            }
        }
    }
}

fn groups_frame(e: &QExpr) -> bool {
    matches!(e, QExpr::Window { frame: Some(f), .. } if f.units == FrameUnits::Groups)
}

fn subquery_of(e: &QExpr) -> Option<&SubqueryIR> {
    match e {
        QExpr::Exists { query } | QExpr::InQuery { query, .. } | QExpr::Scalar { query } => Some(query),
        _ => None,
    }
}

/// Does `q` (or anything inside it) read `table`?
fn sub_reads(q: &SubqueryIR, table: &str) -> bool {
    let mut found = q.sources.iter().any(|s| s.table == table);
    let mut inner = |e: &QExpr| {
        visit(e, &mut |x| {
            if let Some(s) = subquery_of(x) {
                found |= sub_reads(s, table);
            }
        })
    };
    for e in sub_exprs(q) {
        inner(e);
    }
    found
}

/// Every expression directly in a subquery (not those of its own subqueries).
fn sub_exprs(q: &SubqueryIR) -> Vec<&QExpr> {
    let mut v: Vec<&QExpr> = Vec::new();
    v.extend(q.sources.iter().filter_map(|s| s.on.as_ref()));
    v.extend(q.filter.as_ref());
    v.extend(&q.group_by);
    v.extend(q.having.as_ref());
    v.extend(q.select.iter().map(|c| &c.expr));
    v.extend(q.order_by.iter().map(|o| &o.expr));
    v.extend(q.limit.as_ref());
    v.extend(q.offset.as_ref());
    for u in &q.unions {
        v.extend(u.branch.sources.iter().filter_map(|s| s.on.as_ref()));
        v.extend(u.branch.filter.as_ref());
        v.extend(&u.branch.group_by);
        v.extend(u.branch.having.as_ref());
        v.extend(u.branch.select.iter().map(|c| &c.expr));
    }
    v
}

/// Visit `e` and every expression inside it, subqueries included.
fn visit(e: &QExpr, f: &mut dyn FnMut(&QExpr)) {
    f(e);
    let mut each = |x: &QExpr| visit(x, f);
    match e {
        QExpr::Number { .. } | QExpr::Decimal { .. } | QExpr::String { .. } | QExpr::Bool { .. } | QExpr::Null
        | QExpr::Column { .. } | QExpr::Param { .. } => {}
        QExpr::Binary { lhs, rhs, .. } => {
            each(lhs);
            each(rhs);
        }
        QExpr::Not { expr } | QExpr::IsNull { expr, .. } => each(expr),
        QExpr::In { expr, list, .. } => {
            each(expr);
            list.iter().for_each(each);
        }
        QExpr::Like { expr, pattern, .. } => {
            each(expr);
            each(pattern);
        }
        QExpr::Call { args, .. } => args.iter().for_each(each),
        QExpr::Agg { arg, filter, order_by, .. } => {
            arg.iter().for_each(|a| each(a));
            filter.iter().for_each(|a| each(a));
            order_by.iter().for_each(|o| each(&o.expr));
        }
        QExpr::Case { whens, otherwise } => {
            for w in whens {
                each(&w.when);
                each(&w.then);
            }
            otherwise.iter().for_each(|o| each(o));
        }
        QExpr::Concat { parts } => parts.iter().for_each(each),
        QExpr::Window { call, partition_by, order_by, frame } => {
            each(call);
            partition_by.iter().for_each(&mut each);
            order_by.iter().for_each(|o| each(&o.expr));
            if let Some(fr) = frame {
                for b in [&fr.start, &fr.end] {
                    if let FrameBoundIR::Preceding { offset } | FrameBoundIR::Following { offset } = b {
                        each(offset);
                    }
                }
            }
        }
        QExpr::Exists { query } | QExpr::Scalar { query } => sub_exprs(query).into_iter().for_each(each),
        QExpr::InQuery { expr, query, .. } => {
            each(expr);
            sub_exprs(query).into_iter().for_each(each);
        }
    }
}

fn walk_query_exprs(q: &QueryIR, f: &mut dyn FnMut(&QExpr)) {
    let mut v: Vec<&QExpr> = Vec::new();
    v.extend(q.sources.iter().filter_map(|s| s.on.as_ref()));
    v.extend(q.filter.as_ref());
    v.extend(&q.group_by);
    v.extend(q.having.as_ref());
    v.extend(q.select.iter().map(|c| &c.expr));
    v.extend(q.order_by.iter().map(|o| &o.expr));
    for u in &q.unions {
        v.extend(u.branch.select.iter().map(|c| &c.expr));
    }
    for c in &q.ctes {
        v.extend(sub_exprs(&c.query));
    }
    v.into_iter().for_each(|e| visit(e, f));
}

fn walk_mutation_exprs(m: &MutationIR, f: &mut dyn FnMut(&QExpr)) {
    let mut v: Vec<&QExpr> = Vec::new();
    v.extend(m.filter.as_ref());
    v.extend(m.assignments.iter().map(|a| &a.expr));
    v.extend(m.rows.iter().flatten());
    if let Some(s) = &m.source {
        v.extend(sub_exprs(s));
    }
    for c in &m.ctes {
        v.extend(sub_exprs(&c.query));
    }
    v.into_iter().for_each(|e| visit(e, f));
}
