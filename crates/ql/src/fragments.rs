//! Fragments: named, parameter-free queries declared once and used as tables.
//!
//! ```text
//! fragment active_customers() { from customers c where c.balance > 0 select c.id, c.name }
//! query big_spenders() { from active_customers a join orders o on o.customer_id == a.id ... }
//! ```
//!
//! A fragment is not a database object (no migration, nothing to drift): each statement that uses one gets it
//! as a `with` query, placed before the statement's own, so typing, nullability and lowering are exactly those of
//! `with`. Fragments may use other fragments (not themselves, QL260). A fragment cannot be written to, and takes
//! no parameters: it reads only the schema. Each is checked on its own first, so a mistake in one is reported
//! once, where it is written, even if nothing uses it.

use crate::ast::*;
use certo_diagnostics::{Diagnostic, Severity};
use certo_sdl::SchemaIR;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Check the fragments of `file` and expand them into the statements that use them. Afterwards `file.fragments`
/// is empty and every statement is self-contained. Errors go to `diags`; the caller stops if there are any.
pub fn expand(schema: &SchemaIR, file: &mut QlFile, diags: &mut Vec<Diagnostic>) {
    if file.fragments.is_empty() {
        return;
    }
    let frags = std::mem::take(&mut file.fragments);

    // ---- names ---------------------------------------------------------------------------------
    let mut by_name: BTreeMap<String, &Fragment> = BTreeMap::new();
    for f in &frags {
        if by_name.insert(f.name.name.clone(), f).is_some() {
            diags.push(Diagnostic::error("QL260", format!("fragment `{}` is defined more than once", f.name.name)).with_span(f.name.span));
        }
        if schema.tables.iter().any(|t| t.name == f.name.name) {
            diags.push(
                Diagnostic::error("QL260", format!("fragment `{}` has the same name as a table: pick another name", f.name.name))
                    .with_span(f.name.span),
            );
        }
    }
    if has_errors(diags) {
        return;
    }

    // ---- which fragments each fragment uses, and cycles ----------------------------------------------
    let names: HashSet<&str> = by_name.keys().map(String::as_str).collect();
    let deps: BTreeMap<String, BTreeSet<String>> = by_name
        .iter()
        .map(|(n, f)| {
            let mut used = BTreeSet::new();
            tables_in_query(&f.query, &mut used);
            used.retain(|t| names.contains(t.as_str()));
            (n.clone(), used)
        })
        .collect();
    let mut order: Vec<String> = Vec::new(); // dependencies before their users
    let mut state: BTreeMap<String, u8> = BTreeMap::new(); // 1 = on the path, 2 = done
    fn visit(
        n: &str,
        deps: &BTreeMap<String, BTreeSet<String>>,
        state: &mut BTreeMap<String, u8>,
        order: &mut Vec<String>,
        path: &mut Vec<String>,
    ) -> Result<(), Vec<String>> {
        match state.get(n) {
            Some(2) => return Ok(()),
            Some(1) => {
                let at = path.iter().position(|p| p == n).unwrap_or(0);
                let mut cycle: Vec<String> = path[at..].to_vec();
                cycle.push(n.to_string());
                return Err(cycle);
            }
            _ => {}
        }
        state.insert(n.to_string(), 1);
        path.push(n.to_string());
        for d in &deps[n] {
            visit(d, deps, state, order, path)?;
        }
        path.pop();
        state.insert(n.to_string(), 2);
        order.push(n.to_string());
        Ok(())
    }
    for n in by_name.keys() {
        if let Err(cycle) = visit(n, &deps, &mut state, &mut order, &mut Vec::new()) {
            diags.push(
                Diagnostic::error("QL260", format!("fragments cannot use themselves, directly or through others: {}", cycle.join(" -> ")))
                    .with_span(by_name[&cycle[0]].name.span),
            );
            return;
        }
    }

    // a fragment together with everything it needs, as `with` queries (dependencies first)
    let as_cte = |f: &Fragment| Cte { recursive: false, name: f.name.clone(), query: Box::new(f.query.clone()), span: f.span };
    let closure = |used: &BTreeSet<String>| -> Vec<Cte> {
        let mut need: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = used.iter().filter(|n| names.contains(n.as_str())).map(String::as_str).collect();
        while let Some(n) = stack.pop() {
            if need.insert(n) {
                stack.extend(deps[n].iter().map(String::as_str));
            }
        }
        order.iter().filter(|n| need.contains(n.as_str())).map(|n| as_cte(by_name[n])).collect()
    };
    // the statement's own `with` queries come after the fragments; one with a fragment's name replaces it
    let prepend = |own: &mut Vec<Cte>, used: &BTreeSet<String>| {
        let mut all: Vec<Cte> = closure(used).into_iter().filter(|c| !own.iter().any(|o| o.name.name == c.name.name)).collect();
        all.append(own);
        *own = all;
    };

    // ---- each fragment on its own: a mistake is reported where it is written, once --------------------
    let mut alone = QlFile { queries: Vec::new(), mutations: Vec::new(), fragments: Vec::new() };
    for f in by_name.values() {
        let mut q = f.query.clone();
        let mut used = BTreeSet::new();
        tables_in_query(&q, &mut used);
        prepend(&mut q.ctes, &used);
        alone.queries.push(q);
    }
    crate::check::check(schema, &alone, diags);
    // a broken fragment is reported again by every fragment that uses it: once is enough
    let mut seen = HashSet::new();
    diags.retain(|d| seen.insert(format!("{d:?}")));
    if has_errors(diags) {
        return;
    }

    // ---- the statements -----------------------------------------------------------------------------------
    for q in &mut file.queries {
        let mut used = BTreeSet::new();
        tables_in_query(q, &mut used);
        prepend(&mut q.ctes, &used);
    }
    for m in &mut file.mutations {
        if names.contains(m.table.table.name.as_str()) {
            diags.push(
                Diagnostic::error("QL260", format!("`{}` is a fragment, which is read-only: it cannot be written to", m.table.table.name))
                    .with_span(m.table.table.span),
            );
            continue;
        }
        let mut used = BTreeSet::new();
        tables_in_mutation(m, &mut used);
        prepend(&mut m.ctes, &used);
    }
}

fn has_errors(d: &[Diagnostic]) -> bool { d.iter().any(|x| x.severity == Severity::Error) }

// ---- which tables (or fragments) a statement reads ------------------------------------------------------

fn tables_in_query(q: &Query, out: &mut BTreeSet<String>) {
    out.insert(q.from.table.name.clone());
    for j in &q.joins {
        out.insert(j.table.table.name.clone());
        tables_in_expr(&j.on, out);
    }
    for c in &q.ctes {
        tables_in_query(&c.query, out);
    }
    for b in &q.compound {
        tables_in_query(&b.query, out);
    }
    q.filter.iter().for_each(|e| tables_in_expr(e, out));
    q.group_by.iter().for_each(|e| tables_in_expr(e, out));
    q.having.iter().for_each(|e| tables_in_expr(e, out));
    for item in &q.select.items {
        if let SelectItem::Expr { expr, .. } = item {
            tables_in_expr(expr, out);
        }
    }
    q.order_by.iter().for_each(|o| tables_in_expr(&o.expr, out));
    q.limit.iter().for_each(|e| tables_in_expr(e, out));
    q.offset.iter().for_each(|e| tables_in_expr(e, out));
}

fn tables_in_mutation(m: &Mutation, out: &mut BTreeSet<String>) {
    for c in &m.ctes {
        tables_in_query(&c.query, out);
    }
    m.assignments.iter().for_each(|a| tables_in_expr(&a.value, out));
    m.rows.iter().flatten().for_each(|e| tables_in_expr(e, out));
    if let Some(s) = &m.source {
        tables_in_query(s, out);
    }
    m.filter.iter().for_each(|e| tables_in_expr(e, out));
    if let Some(Conflict { action: ConflictAction::Update(a), .. }) = &m.conflict {
        a.iter().for_each(|a| tables_in_expr(&a.value, out));
    }
    for item in &m.returning {
        if let SelectItem::Expr { expr, .. } = item {
            tables_in_expr(expr, out);
        }
    }
}

fn tables_in_expr(e: &Expr, out: &mut BTreeSet<String>) {
    match e {
        Expr::Number(..) | Expr::Decimal(..) | Expr::Str(..) | Expr::Bool(..) | Expr::Null(..) | Expr::Column { .. } | Expr::Param(..) => {}
        Expr::Binary { lhs, rhs, .. } => {
            tables_in_expr(lhs, out);
            tables_in_expr(rhs, out);
        }
        Expr::Not(i, _) | Expr::Paren(i, _) => tables_in_expr(i, out),
        Expr::IsNull { expr, .. } => tables_in_expr(expr, out),
        Expr::In { expr, list, .. } => {
            tables_in_expr(expr, out);
            list.iter().for_each(|i| tables_in_expr(i, out));
        }
        Expr::Like { expr, pattern, .. } => {
            tables_in_expr(expr, out);
            tables_in_expr(pattern, out);
        }
        Expr::Between { expr, low, high, .. } => {
            tables_in_expr(expr, out);
            tables_in_expr(low, out);
            tables_in_expr(high, out);
        }
        Expr::Call { args, filter, agg_order, over, .. } => {
            args.iter().for_each(|a| tables_in_expr(a, out));
            filter.iter().for_each(|f| tables_in_expr(f, out));
            agg_order.iter().for_each(|o| tables_in_expr(&o.expr, out));
            if let Some(w) = over {
                w.partition_by.iter().for_each(|p| tables_in_expr(p, out));
                w.order_by.iter().for_each(|o| tables_in_expr(&o.expr, out));
            }
        }
        Expr::Case { whens, otherwise, .. } => {
            for (c, v) in whens {
                tables_in_expr(c, out);
                tables_in_expr(v, out);
            }
            otherwise.iter().for_each(|o| tables_in_expr(o, out));
        }
        Expr::Concat(parts, _) => parts.iter().for_each(|p| tables_in_expr(p, out)),
        Expr::Exists(q, _) | Expr::Scalar(q, _) => tables_in_query(q, out),
        Expr::InQuery { expr, query, .. } => {
            tables_in_expr(expr, out);
            tables_in_query(query, out);
        }
    }
}
