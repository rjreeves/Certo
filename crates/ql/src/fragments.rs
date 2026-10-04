//! Fragments: named queries declared once and used as tables.
//!
//! ```text
//! fragment since(cutoff: timestamp) { from orders o where o.created >= :cutoff select o.id, o.customer_id, o.total }
//! query recent(at: timestamp) { from since(:at) r join customers c on r.customer_id == c.id select c.name, r.total }
//! query fixed() { from since("2026-01-01") r select count(*) as n }
//! ```
//!
//! A fragment is not a database object (no migration, nothing to drift): each statement that uses one gets it as a
//! `with` query, placed before the statement's own, so typing, nullability and lowering are exactly those of `with`.
//! A fragment's parameters are filled in where it is used, by literals or by the statement's own parameters (never
//! columns, so a fragment still reads only the schema): the call's arguments are substituted into a copy of the body,
//! and each distinct call becomes its own `with` query (`since#1`, `since#2`; a fragment without parameters keeps its
//! own name). Fragments may use other fragments (not themselves, QL260) and cannot be written to. Each is checked on
//! its own first, with its parameters declared, so a mistake in one is reported once, where it is written, even if
//! nothing uses it.

use crate::ast::*;
use certo_diagnostics::{Diagnostic, Severity};
use certo_sdl::{Ident, SchemaIR};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Check the fragments of `file` and expand them into the statements that use them. Afterwards `file.fragments`
/// is empty and every statement is self-contained. Errors go to `diags`; the caller stops if there are any.
pub fn expand(schema: &SchemaIR, file: &mut QlFile, diags: &mut Vec<Diagnostic>) -> Vec<crate::ir::FragmentInfo> {
    // (with no fragments there is nothing to expand, but a table written with arguments is still an error)
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
    // a view is read like a table, as a fragment is: the two share one namespace
    for v in &file.views {
        if by_name.contains_key(&v.name.name) {
            diags.push(
                Diagnostic::error("QL262", format!("`{}` is both a fragment and a view: pick another name for one", v.name.name)).with_span(v.name.span),
            );
        }
    }
    if has_errors(diags) {
        return Vec::new();
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
    let mut state: BTreeMap<String, u8> = BTreeMap::new(); // 1 = on the path, 2 = done
    fn visit(n: &str, deps: &BTreeMap<String, BTreeSet<String>>, state: &mut BTreeMap<String, u8>, path: &mut Vec<String>) -> Result<(), Vec<String>> {
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
            visit(d, deps, state, path)?;
        }
        path.pop();
        state.insert(n.to_string(), 2);
        Ok(())
    }
    for n in by_name.keys() {
        if let Err(cycle) = visit(n, &deps, &mut state, &mut Vec::new()) {
            diags.push(
                Diagnostic::error("QL260", format!("fragments cannot use themselves, directly or through others: {}", cycle.join(" -> ")))
                    .with_span(by_name[&cycle[0]].name.span),
            );
            return Vec::new();
        }
    }

    // ---- each fragment on its own, with its parameters declared: a mistake is reported where it is written, once
    let mut alone = QlFile { queries: Vec::new(), mutations: Vec::new(), fragments: Vec::new(), views: Vec::new() };
    for f in by_name.values() {
        let mut q = f.query.clone();
        let mut ex = Expander::new(&by_name, q.params.clone());
        ex.query(&mut q, &HashSet::new());
        diags.append(&mut ex.errors);
        ex.prepend_to(&mut q.ctes);
        alone.queries.push(q);
    }
    let checked = if has_errors(diags) { Vec::new() } else { crate::check::check(schema, &alone, diags) };
    // a broken fragment is reported again by every fragment that uses it: once is enough
    let mut seen = HashSet::new();
    diags.retain(|d| seen.insert(format!("{d:?}")));
    if has_errors(diags) {
        return Vec::new();
    }
    let infos: Vec<crate::ir::FragmentInfo> = checked
        .into_iter()
        .map(|q| crate::ir::FragmentInfo { name: q.name, params: q.params, columns: q.select })
        .collect();

    // ---- the statements -----------------------------------------------------------------------------------
    for q in &mut file.queries {
        let mut ex = Expander::new(&by_name, q.params.clone());
        ex.query(q, &HashSet::new());
        diags.append(&mut ex.errors);
        ex.prepend_to(&mut q.ctes);
    }
    for v in &mut file.views {
        let mut ex = Expander::new(&by_name, Vec::new());
        ex.query(&mut v.query, &HashSet::new());
        diags.append(&mut ex.errors);
        ex.prepend_to(&mut v.query.ctes);
    }
    for m in &mut file.mutations {
        if names.contains(m.table.table.name.as_str()) {
            diags.push(
                Diagnostic::error("QL260", format!("`{}` is a fragment, which is read-only: it cannot be written to", m.table.table.name))
                    .with_span(m.table.table.span),
            );
            continue;
        }
        let mut ex = Expander::new(&by_name, m.params.clone());
        ex.mutation(m);
        diags.append(&mut ex.errors);
        ex.prepend_to(&mut m.ctes);
    }
    infos
}

fn has_errors(d: &[Diagnostic]) -> bool { d.iter().any(|x| x.severity == Severity::Error) }

// ---- turning each use of a fragment into a `with` query ---------------------------------------------------------

/// The expansion of one statement (or of one fragment checked on its own): the `with` queries its fragment calls
/// need, in the order they must come (what a call needs comes before it).
struct Expander<'a> {
    frags: &'a BTreeMap<String, &'a Fragment>,
    /// The parameters the statement declares, which a call may pass on.
    params: Vec<ParamDecl>,
    out: Vec<Cte>,
    /// call (fragment and arguments) -> the name of its `with` query
    cache: HashMap<String, String>,
    counters: HashMap<String, usize>,
    errors: Vec<Diagnostic>,
}

impl<'a> Expander<'a> {
    fn new(frags: &'a BTreeMap<String, &'a Fragment>, params: Vec<ParamDecl>) -> Self {
        Expander { frags, params, out: Vec::new(), cache: HashMap::new(), counters: HashMap::new(), errors: Vec::new() }
    }

    /// Put the generated `with` queries in front of the statement's own.
    fn prepend_to(self, own: &mut Vec<Cte>) {
        let mut all = self.out;
        all.append(own);
        *own = all;
    }

    fn fail(&mut self, msg: String, at: certo_ast::span::Span) {
        self.errors.push(Diagnostic::error("QL260", msg).with_span(at));
    }

    /// Rewrite every fragment use in `q` (and inside it). `shadow` holds the names of `with` queries in scope: a
    /// statement's own `with` of a fragment's name takes precedence over the fragment.
    fn query(&mut self, q: &mut Query, shadow: &HashSet<String>) {
        let mut shadow = shadow.clone();
        shadow.extend(q.ctes.iter().map(|c| c.name.name.clone()));
        for c in &mut q.ctes {
            self.query(&mut c.query, &shadow);
        }
        self.table(&mut q.from, &shadow);
        for j in &mut q.joins {
            self.table(&mut j.table, &shadow);
        }
        for b in &mut q.compound {
            self.query(&mut b.query, &shadow);
        }
        for e in query_exprs_mut(q) {
            self.expr(e, &shadow);
        }
    }

    fn mutation(&mut self, m: &mut Mutation) {
        let mut shadow = HashSet::new();
        shadow.extend(m.ctes.iter().map(|c| c.name.name.clone()));
        for c in &mut m.ctes {
            self.query(&mut c.query, &shadow);
        }
        if let Some(s) = &mut m.source {
            self.query(s, &shadow);
        }
        for e in mutation_exprs_mut(m) {
            self.expr(e, &shadow);
        }
    }

    /// Subqueries inside an expression.
    fn expr(&mut self, e: &mut Expr, shadow: &HashSet<String>) {
        walk_expr_mut(e, &mut |_| {}, &mut |sub| self.query(sub, shadow));
    }

    fn table(&mut self, t: &mut TableRef, shadow: &HashSet<String>) {
        let name = t.table.name.clone();
        let Some(&f) = self.frags.get(&name).filter(|_| !shadow.contains(&name)) else {
            if !t.args.is_empty() {
                self.fail(format!("`{name}` is not a fragment, so it takes no arguments"), t.table.span);
            }
            return;
        };
        let params = &f.query.params;
        if t.args.len() != params.len() {
            let list = params.iter().map(|p| p.name.name.as_str()).collect::<Vec<_>>().join(", ");
            self.fail(
                format!("fragment `{name}` takes {} argument(s){}, got {}", params.len(), if params.is_empty() { String::new() } else { format!(" ({list})") }, t.args.len()),
                t.table.span,
            );
            return;
        }
        // arguments: literals or the statement's own parameters, matching what the fragment declares
        let mut key = name.clone();
        let mut map: BTreeMap<String, Expr> = BTreeMap::new();
        let mut bad = false;
        for (p, a) in params.iter().zip(&t.args) {
            let inner = strip_parens(a);
            let Some(k) = literal_key(inner) else {
                self.fail(
                    format!("an argument of fragment `{name}` must be a literal or one of the statement's parameters (`:name`), not a column or an expression"),
                    a.span(),
                );
                bad = true;
                continue;
            };
            if !self.arg_fits(&name, p, inner) {
                bad = true;
                continue;
            }
            key.push('|');
            key.push_str(&k);
            map.insert(p.name.name.clone(), inner.clone());
        }
        if bad {
            return;
        }
        let inst = match self.cache.get(&key) {
            Some(n) => n.clone(),
            None => {
                let inst = if params.is_empty() {
                    name.clone()
                } else {
                    let n = self.counters.entry(name.clone()).or_insert(0);
                    *n += 1;
                    format!("{name}#{n}")
                };
                self.cache.insert(key, inst.clone());
                let mut body = f.query.clone();
                body.params = Vec::new();
                substitute(&mut body, &map);
                // what the body itself uses is needed before it
                self.query(&mut body, &HashSet::new());
                self.out.push(Cte { recursive: false, name: Ident { name: inst.clone(), span: f.name.span }, query: Box::new(body), span: f.span });
                inst
            }
        };
        let span = t.table.span;
        if t.alias.is_none() {
            t.alias = Some(Ident { name: name.clone(), span });
        }
        t.table = Ident { name: inst, span };
        t.args.clear();
    }

    /// An argument that is one of the statement's parameters must have the type the fragment declares, and `null`
    /// (a literal or a nullable parameter) needs a parameter declared `null`. Other literals are checked by the
    /// checker, where they end up in the body.
    fn arg_fits(&mut self, frag: &str, p: &ParamDecl, arg: &Expr) -> bool {
        match arg {
            Expr::Null(span) if !p.nullable => {
                self.fail(format!("parameter `{}` of fragment `{frag}` is not nullable, so it cannot be `null`", p.name.name), *span);
                false
            }
            Expr::Param(id) => {
                let Some(have) = self.params.iter().find(|d| d.name.name == id.name).cloned() else {
                    return true; // an undeclared parameter is reported by the checker
                };
                if have.ty.name.name != p.ty.name.name || have.ty.args != p.ty.args {
                    self.fail(
                        format!(
                            "`:{}` is declared as {}, but parameter `{}` of fragment `{frag}` is {}",
                            id.name, type_text(&have.ty), p.name.name, type_text(&p.ty)
                        ),
                        id.span,
                    );
                    return false;
                }
                if have.nullable && !p.nullable {
                    self.fail(
                        format!("`:{}` may be null, but parameter `{}` of fragment `{frag}` is not declared `null`", id.name, p.name.name),
                        id.span,
                    );
                    return false;
                }
                true
            }
            _ => true,
        }
    }
}

fn type_text(t: &certo_sdl::TypeRef) -> String {
    if t.args.is_empty() {
        t.name.name.clone()
    } else {
        format!("{}({})", t.name.name, t.args.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
    }
}

fn strip_parens(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(i, _) => strip_parens(i),
        other => other,
    }
}

/// What identifies an argument (two calls with the same key share one `with` query); `None` if it is not allowed.
fn literal_key(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Number(n, _) => format!("n{n}"),
        Expr::Decimal(d, _) => format!("d{d}"),
        Expr::Str(s, _) => format!("s{}:{s}", s.len()),
        Expr::Bool(b, _) => format!("b{b}"),
        Expr::Null(_) => "null".to_string(),
        Expr::Param(id) => format!("p{}", id.name),
        _ => return None,
    })
}

/// Replace each `:name` in `q` (and in what it contains, including the arguments of the fragments it calls) by its
/// argument.
fn substitute(q: &mut Query, map: &BTreeMap<String, Expr>) {
    for e in query_exprs_mut(q) {
        walk_expr_mut(
            e,
            &mut |x| {
                if let Expr::Param(id) = x
                    && let Some(r) = map.get(&id.name)
                {
                    *x = r.clone();
                }
            },
            &mut |sub| substitute(sub, map),
        );
    }
    for c in &mut q.ctes {
        substitute(&mut c.query, map);
    }
    for b in &mut q.compound {
        substitute(&mut b.query, map);
    }
}

// ---- walking the syntax tree ------------------------------------------------------------------------------------------

/// A query's own expressions (not those of the queries inside it), including the arguments of the fragments it calls.
fn query_exprs_mut(q: &mut Query) -> Vec<&mut Expr> {
    let mut out: Vec<&mut Expr> = Vec::new();
    out.extend(q.from.args.iter_mut());
    for j in &mut q.joins {
        out.extend(j.table.args.iter_mut());
        out.push(&mut j.on);
    }
    out.extend(q.filter.iter_mut());
    out.extend(q.group_by.iter_mut());
    out.extend(q.having.iter_mut());
    for item in &mut q.select.items {
        if let SelectItem::Expr { expr, .. } = item {
            out.push(expr);
        }
    }
    out.extend(q.order_by.iter_mut().map(|o| &mut o.expr));
    out.extend(q.limit.iter_mut());
    out.extend(q.offset.iter_mut());
    out
}

fn mutation_exprs_mut(m: &mut Mutation) -> Vec<&mut Expr> {
    let mut out: Vec<&mut Expr> = Vec::new();
    out.extend(m.assignments.iter_mut().map(|a| &mut a.value));
    out.extend(m.rows.iter_mut().flatten());
    out.extend(m.filter.iter_mut());
    if let Some(Conflict { action: ConflictAction::Update(a), .. }) = &mut m.conflict {
        out.extend(a.iter_mut().map(|a| &mut a.value));
    }
    for item in &mut m.returning {
        if let SelectItem::Expr { expr, .. } = item {
            out.push(expr);
        }
    }
    out
}

/// Visit `e` and everything inside it (before its parts); a query inside it goes to `on_query`.
fn walk_expr_mut(e: &mut Expr, on_expr: &mut dyn FnMut(&mut Expr), on_query: &mut dyn FnMut(&mut Query)) {
    on_expr(e);
    match e {
        Expr::Number(..) | Expr::Decimal(..) | Expr::Str(..) | Expr::Bool(..) | Expr::Null(..) | Expr::Column { .. } | Expr::Param(..) => {}
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr_mut(lhs, on_expr, on_query);
            walk_expr_mut(rhs, on_expr, on_query);
        }
        Expr::Not(i, _) | Expr::Paren(i, _) => walk_expr_mut(i, on_expr, on_query),
        Expr::IsNull { expr, .. } => walk_expr_mut(expr, on_expr, on_query),
        Expr::In { expr, list, .. } => {
            walk_expr_mut(expr, on_expr, on_query);
            for i in list {
                walk_expr_mut(i, on_expr, on_query);
            }
        }
        Expr::Like { expr, pattern, .. } => {
            walk_expr_mut(expr, on_expr, on_query);
            walk_expr_mut(pattern, on_expr, on_query);
        }
        Expr::Between { expr, low, high, .. } => {
            walk_expr_mut(expr, on_expr, on_query);
            walk_expr_mut(low, on_expr, on_query);
            walk_expr_mut(high, on_expr, on_query);
        }
        Expr::Call { args, filter, agg_order, over, .. } => {
            for a in args {
                walk_expr_mut(a, on_expr, on_query);
            }
            if let Some(f) = filter {
                walk_expr_mut(f, on_expr, on_query);
            }
            for o in agg_order {
                walk_expr_mut(&mut o.expr, on_expr, on_query);
            }
            if let Some(w) = over {
                for p in &mut w.partition_by {
                    walk_expr_mut(p, on_expr, on_query);
                }
                for o in &mut w.order_by {
                    walk_expr_mut(&mut o.expr, on_expr, on_query);
                }
            }
        }
        Expr::Case { whens, otherwise, .. } => {
            for (c, v) in whens {
                walk_expr_mut(c, on_expr, on_query);
                walk_expr_mut(v, on_expr, on_query);
            }
            if let Some(o) = otherwise {
                walk_expr_mut(o, on_expr, on_query);
            }
        }
        Expr::Concat(parts, _) => {
            for p in parts {
                walk_expr_mut(p, on_expr, on_query);
            }
        }
        Expr::Exists(q, _) | Expr::Scalar(q, _) => on_query(q),
        Expr::InQuery { expr, query, .. } => {
            walk_expr_mut(expr, on_expr, on_query);
            on_query(query);
        }
    }
}

// ---- which tables (or fragments) a query reads (for the dependencies between fragments) -----------------------

pub(crate) fn tables_in_query(q: &Query, out: &mut BTreeSet<String>) {
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
