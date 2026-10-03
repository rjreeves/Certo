//! Semantic analysis: resolve every name against a `SchemaIR`, type every
//! expression, and produce a `QueryIR`.
//!
//! Error codes: QL201 duplicate query, QL202 duplicate parameter, QL203
//! unknown table, QL204 duplicate alias, QL205 unknown alias, QL206 unknown
//! column, QL207 ambiguous column, QL208 unknown parameter, QL209 unknown
//! function or wrong arguments, QL211 type mismatch, QL212 condition is not
//! boolean, QL213 misplaced aggregate, QL214 column not grouped, QL215
//! select item needs an alias, QL216 duplicate output column, QL217 bad
//! limit/offset, QL218 unknown enum value, QL219 bad parameter type, QL220
//! untyped NULL output, QL221 `order by` not in a `select distinct` list.
//! Mutations: QL231 (unused), QL232 NULL into a NOT NULL column, QL233
//! required column missing from an insert, QL234 generated-always column
//! assigned, QL235 column assigned twice, QL236 conflict target is not a
//! unique key. `with`: QL254 duplicate name (or recursion, which is not supported).
//! Set operations: QL250 `order by` must name an output column, QL253
//! the branches do not line up (columns, types, mixed or unsupported operators).
//! Window functions: QL251 misplaced or nested, QL252 unknown
//! function or wrong arguments, QL255 bad frame. Subqueries: QL240 must return one column, QL241 scalar
//! subquery may return several rows, QL242 insert row/column count, QL243
//! (parser) nested too deeply. QL290 (warning): unused parameter.

use crate::ast::*;
use crate::ir::*;
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use certo_sdl::{
    compatible, describe_type, from_rank, is_text, ordered, rank, BinaryOp, Builtin, SchemaIR, TableIR, TypeIR,
    TypeRef,
};
use std::borrow::Cow;
use std::collections::HashSet;

/// Check every query in `file`. Queries with errors are left out of the result
/// (their diagnostics are in `diags`).
pub fn check(schema: &SchemaIR, file: &QlFile, diags: &mut Vec<Diagnostic>) -> Vec<QueryIR> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for q in &file.queries {
        if !seen.insert(q.name.name.clone()) {
            diags.push(
                Diagnostic::error("QL201", format!("query `{}` is defined more than once", q.name.name))
                    .with_span(q.name.span),
            );
            continue;
        }
        let mut cx = Checker { schema, diags, sources: Vec::new(), scopes: vec![0], ctes: Vec::new(), params: Vec::new(), failed: false, in_window: false };
        if let Some(ir) = cx.query(q)
            && !cx.failed
        {
            out.push(ir);
        }
    }
    out
}

/// Check every `insert` / `update` / `delete` in `file`. Statement names are
/// unique across queries and mutations. Mutations with errors are left out.
pub fn check_mutations(schema: &SchemaIR, file: &QlFile, diags: &mut Vec<Diagnostic>) -> Vec<MutationIR> {
    let mut seen: HashSet<String> = file.queries.iter().map(|q| q.name.name.clone()).collect();
    let mut out = Vec::new();
    for m in &file.mutations {
        if !seen.insert(m.name.name.clone()) {
            diags.push(
                Diagnostic::error("QL201", format!("`{}` is defined more than once", m.name.name)).with_span(m.name.span),
            );
            continue;
        }
        let mut cx = Checker { schema, diags, sources: Vec::new(), scopes: vec![0], ctes: Vec::new(), params: Vec::new(), failed: false, in_window: false };
        if let Some(ir) = cx.mutation(m)
            && !cx.failed
        {
            out.push(ir);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Known(TypeIR),
    /// The type of the literal `null`: compatible with anything.
    Null,
}

#[derive(Clone)]
struct Typed {
    e: QExpr,
    t: T,
    nullable: bool,
    /// Contains an aggregate.
    agg: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Where,
    On,
    Group,
    Having,
    Select,
    Order,
    Limit,
    /// The right-hand side of `set` in a mutation.
    Set,
    Returning,
}

impl Ctx {
    fn allows_agg(self) -> bool { matches!(self, Ctx::Having | Ctx::Select | Ctx::Order) }
    /// Window functions run after grouping, so only the select list and `order by` may use them.
    fn allows_window(self) -> bool { matches!(self, Ctx::Select | Ctx::Order) }
}

struct SourceInfo<'a> {
    alias: String,
    /// A schema table, or the columns of a `with` query.
    table: Cow<'a, TableIR>,
    /// On the nullable side of a left join.
    nullable: bool,
}

struct ParamInfo {
    name: String,
    ty: TypeIR,
    nullable: bool,
    used: bool,
    span: Span,
}

/// A checked `select` body, before it becomes a query or a subquery.
#[derive(Clone)]
struct Body {
    sources: Vec<SourceIR>,
    filter: Option<QExpr>,
    group_by: Vec<QExpr>,
    having: Option<Typed>,
    outputs: Vec<Output>,
    order: Vec<(Typed, bool, Span)>,
    limit: Option<QExpr>,
    offset: Option<QExpr>,
    /// Aggregates without `group by`: exactly one row, however many match.
    single_row: bool,
    unions: Vec<UnionIR>,
    ctes: Vec<CteIR>,
}

fn columns_of(outputs: Vec<Output>) -> Vec<ColumnOut> {
    outputs
        .into_iter()
        .filter_map(|o| match o.typed.t {
            T::Known(ty) => Some(ColumnOut { name: o.name, expr: o.typed.e, ty, nullable: o.typed.nullable }),
            T::Null => None,
        })
        .collect()
}

#[derive(Clone)]
struct Output {
    name: String,
    typed: Typed,
    span: Span,
}

struct Checker<'a> {
    schema: &'a SchemaIR,
    diags: &'a mut Vec<Diagnostic>,
    sources: Vec<SourceInfo<'a>>,
    /// Where each open query's sources start in `sources`: the outermost query is first, a
    /// subquery being checked is last. Names resolve innermost scope first.
    scopes: Vec<usize>,
    /// `with` queries in scope, innermost last: a table name finds these before the schema's tables.
    ctes: Vec<(String, TableIR)>,
    params: Vec<ParamInfo>,
    failed: bool,
    /// Checking the inside of a window function (they do not nest).
    in_window: bool,
}

fn describe_t(t: &T) -> String {
    match t {
        T::Known(t) => describe_type(t),
        T::Null => "null".into(),
    }
}

fn builtin(b: Builtin) -> TypeIR { TypeIR::Builtin(b) }

fn is_bool(t: &T) -> bool { matches!(t, T::Known(TypeIR::Builtin(Builtin::Bool)) | T::Null) }

/// The type two branches / arguments share, if any.
fn unify(a: &T, b: &T) -> Option<T> {
    match (a, b) {
        (T::Null, x) | (x, T::Null) => Some(x.clone()),
        (T::Known(x), T::Known(y)) => {
            if x == y {
                Some(a.clone())
            } else if let (Some(ra), Some(rb)) = (rank(x), rank(y)) {
                Some(T::Known(from_rank(ra.max(rb))))
            } else if is_text(x) && is_text(y) {
                Some(T::Known(builtin(Builtin::Text)))
            } else if compatible(x, y) {
                Some(a.clone())
            } else {
                None
            }
        }
    }
}

/// Column references in `e` (and in any subquery inside it) whose table is not
/// defined by a subquery that encloses the reference: a subquery's correlation
/// with the query around it.
fn refs_expr(e: &QExpr, bound: &HashSet<String>, out: &mut Vec<QExpr>) {
    match e {
        QExpr::Column { source, .. } => {
            if !bound.contains(source) {
                out.push(e.clone());
            }
        }
        QExpr::Number { .. } | QExpr::Decimal { .. } | QExpr::String { .. } | QExpr::Bool { .. } | QExpr::Null
        | QExpr::Param { .. } => {}
        QExpr::Binary { lhs, rhs, .. } => {
            refs_expr(lhs, bound, out);
            refs_expr(rhs, bound, out);
        }
        QExpr::Not { expr } | QExpr::IsNull { expr, .. } => refs_expr(expr, bound, out),
        QExpr::In { expr, list, .. } => {
            refs_expr(expr, bound, out);
            list.iter().for_each(|i| refs_expr(i, bound, out));
        }
        QExpr::Like { expr, pattern, .. } => {
            refs_expr(expr, bound, out);
            refs_expr(pattern, bound, out);
        }
        QExpr::Call { args, .. } => args.iter().for_each(|a| refs_expr(a, bound, out)),
        QExpr::Window { call, partition_by, order_by, .. } => {
            refs_expr(call, bound, out);
            partition_by.iter().for_each(|p| refs_expr(p, bound, out));
            order_by.iter().for_each(|o| refs_expr(&o.expr, bound, out));
        }
        QExpr::Agg { arg, .. } => {
            if let Some(a) = arg {
                refs_expr(a, bound, out);
            }
        }
        QExpr::Case { whens, otherwise } => {
            for w in whens {
                refs_expr(&w.when, bound, out);
                refs_expr(&w.then, bound, out);
            }
            if let Some(o) = otherwise {
                refs_expr(o, bound, out);
            }
        }
        QExpr::Exists { query } | QExpr::Scalar { query } => refs_sub(query, bound, out),
        QExpr::InQuery { expr, query, .. } => {
            refs_expr(expr, bound, out);
            refs_sub(query, bound, out);
        }
    }
}

fn refs_sub(sub: &SubqueryIR, bound: &HashSet<String>, out: &mut Vec<QExpr>) {
    let mut inner = bound.clone();
    inner.extend(sub.sources.iter().map(|s| s.alias.clone()));
    for s in &sub.sources {
        if let Some(on) = &s.on {
            refs_expr(on, &inner, out);
        }
    }
    let opt = |e: &Option<QExpr>, out: &mut Vec<QExpr>| {
        if let Some(e) = e {
            refs_expr(e, &inner, out);
        }
    };
    opt(&sub.filter, out);
    sub.group_by.iter().for_each(|g| refs_expr(g, &inner, out));
    opt(&sub.having, out);
    sub.select.iter().for_each(|c| refs_expr(&c.expr, &inner, out));
    sub.order_by.iter().for_each(|o| refs_expr(&o.expr, &inner, out));
    opt(&sub.limit, out);
    opt(&sub.offset, out);
    for c in &sub.ctes {
        refs_sub(&c.query, bound, out);
    }
    for u in &sub.unions {
        let mut inner = bound.clone();
        inner.extend(u.branch.sources.iter().map(|s| s.alias.clone()));
        for s in &u.branch.sources {
            if let Some(on) = &s.on {
                refs_expr(on, &inner, out);
            }
        }
        if let Some(f) = &u.branch.filter {
            refs_expr(f, &inner, out);
        }
        u.branch.group_by.iter().for_each(|g| refs_expr(g, &inner, out));
        if let Some(h) = &u.branch.having {
            refs_expr(h, &inner, out);
        }
        u.branch.select.iter().for_each(|c| refs_expr(&c.expr, &inner, out));
    }
}

/// The first column reference of this query (its tables are `scope`) that is not
/// covered by a `group by` expression or an aggregate, if any. Columns of the
/// queries around a subquery are constants to it, so they never need grouping.
fn uncovered(e: &QExpr, groups: &[QExpr], scope: &HashSet<String>) -> Option<String> {
    if groups.contains(e) {
        return None;
    }
    let via_subquery = |sub: &SubqueryIR| {
        let mut refs = Vec::new();
        refs_sub(sub, &HashSet::new(), &mut refs);
        refs.into_iter().find_map(|r| match &r {
            QExpr::Column { source, column } if scope.contains(source) && !groups.contains(&r) => {
                Some(format!("{source}.{column}"))
            }
            _ => None,
        })
    };
    match e {
        QExpr::Number { .. } | QExpr::Decimal { .. } | QExpr::String { .. } | QExpr::Bool { .. } | QExpr::Null
        | QExpr::Param { .. } | QExpr::Agg { .. } => None,
        QExpr::Column { source, column } => scope.contains(source).then(|| format!("{source}.{column}")),
        QExpr::Binary { lhs, rhs, .. } => uncovered(lhs, groups, scope).or_else(|| uncovered(rhs, groups, scope)),
        QExpr::Not { expr } | QExpr::IsNull { expr, .. } => uncovered(expr, groups, scope),
        QExpr::In { expr, list, .. } => {
            uncovered(expr, groups, scope).or_else(|| list.iter().find_map(|i| uncovered(i, groups, scope)))
        }
        QExpr::Like { expr, pattern, .. } => uncovered(expr, groups, scope).or_else(|| uncovered(pattern, groups, scope)),
        QExpr::Call { args, .. } => args.iter().find_map(|a| uncovered(a, groups, scope)),
        // a window function sees the grouped rows: its inputs follow the same rule as the select list
        QExpr::Window { call, partition_by, order_by, .. } => {
            let inner = match &**call {
                QExpr::Agg { arg, .. } => arg.as_ref().and_then(|a| uncovered(a, groups, scope)),
                other => uncovered(other, groups, scope),
            };
            inner
                .or_else(|| partition_by.iter().find_map(|p| uncovered(p, groups, scope)))
                .or_else(|| order_by.iter().find_map(|o| uncovered(&o.expr, groups, scope)))
        }
        QExpr::Case { whens, otherwise } => whens
            .iter()
            .find_map(|w| uncovered(&w.when, groups, scope).or_else(|| uncovered(&w.then, groups, scope)))
            .or_else(|| otherwise.as_ref().and_then(|o| uncovered(o, groups, scope))),
        QExpr::Exists { query } | QExpr::Scalar { query } => via_subquery(query),
        QExpr::InQuery { expr, query, .. } => uncovered(expr, groups, scope).or_else(|| via_subquery(query)),
    }
}

impl<'a> Checker<'a> {
    fn fail(&mut self, code: &str, msg: impl Into<String>, span: Span) {
        self.failed = true;
        self.diags.push(Diagnostic::error(code, msg).with_span(span));
    }

    // ---- query ------------------------------------------------------------ //

    fn declare_params(&mut self, params: &[ParamDecl]) {
        for p in params {
            if self.params.iter().any(|x| x.name == p.name.name) {
                self.fail("QL202", format!("parameter `{}` is declared twice", p.name.name), p.name.span);
                continue;
            }
            if let Some(ty) = self.param_type(&p.ty) {
                self.params.push(ParamInfo {
                    name: p.name.name.clone(), ty, nullable: p.nullable, used: false, span: p.name.span,
                });
            }
        }
    }

    fn warn_unused_params(&mut self) {
        for p in &self.params {
            if !p.used {
                self.diags.push(
                    Diagnostic::warning("QL290", format!("parameter `{}` is never used", p.name)).with_span(p.span),
                );
            }
        }
    }

    fn param_irs(&self) -> Vec<ParamIR> {
        self.params.iter().map(|p| ParamIR { name: p.name.clone(), ty: p.ty.clone(), nullable: p.nullable }).collect()
    }

    /// The result columns of a `select` list or `returning` clause: stars
    /// expanded, every column named, names unique, no bare NULL.
    fn scope_start(&self) -> usize { self.scopes.last().copied().unwrap_or(0) }

    /// `named`: the columns are the query's result, so each needs a unique name.
    /// A subquery's columns are used by position, and get placeholder names.
    fn outputs(&mut self, items: &[SelectItem], ctx: Ctx, named: bool) -> Vec<Output> {
        let mut outputs: Vec<Output> = Vec::new();
        let base = self.scope_start();
        for item in items {
            match item {
                SelectItem::Star(span) => {
                    let all: Vec<_> = (base..self.sources.len()).collect();
                    for i in all {
                        self.expand(i, *span, &mut outputs);
                    }
                }
                SelectItem::SourceStar(alias) => {
                    match self.sources[base..].iter().position(|s| s.alias == alias.name) {
                        Some(i) => self.expand(base + i, alias.span, &mut outputs),
                        None => self.fail("QL205", format!("unknown table alias `{}`", alias.name), alias.span),
                    }
                }
                SelectItem::Expr { expr, alias } => {
                    let Some(typed) = self.expr(expr, ctx) else { continue };
                    let name = match (alias, expr) {
                        (Some(a), _) => a.name.clone(),
                        (None, Expr::Column { name, .. }) => name.name.clone(),
                        (None, _) if !named => format!("column{}", outputs.len() + 1),
                        (None, _) => {
                            self.fail("QL215", "this expression needs a name: add `as <name>`", expr.span());
                            continue;
                        }
                    };
                    outputs.push(Output { name, typed, span: expr.span() });
                }
            }
        }
        let mut names = HashSet::new();
        for o in outputs.iter().filter(|_| named) {
            if !names.insert(o.name.clone()) {
                self.fail("QL216", format!("the result has two columns named `{}`; add `as <name>`", o.name), o.span);
            }
            if o.typed.t == T::Null {
                self.fail("QL220", format!("cannot tell the type of column `{}`: it is only NULL", o.name), o.span);
            }
        }
        outputs
    }

    fn query(&mut self, q: &Query) -> Option<QueryIR> {
        self.declare_params(&q.params);
        let b = self.select_body(q, true)?;
        self.warn_unused_params();
        Some(QueryIR {
            name: q.name.name.clone(),
            params: self.param_irs(),
            sources: b.sources,
            filter: b.filter,
            group_by: b.group_by,
            having: b.having.map(|h| h.e),
            distinct: q.select.distinct,
            select: columns_of(b.outputs),
            order_by: b.order.into_iter().map(|(t, desc, _)| OrderIR { expr: t.e, desc }).collect(),
            limit: b.limit,
            offset: b.offset,
            unions: b.unions,
            ctes: b.ctes,
        })
    }

    /// Check a query nested in an expression (or feeding an `insert`): its own
    /// scope, able to see the tables of the queries around it.
    fn subquery(&mut self, q: &Query) -> Option<(SubqueryIR, Body)> {
        let base = self.sources.len();
        self.scopes.push(base);
        let body = self.select_body(q, false);
        self.sources.truncate(base);
        self.scopes.pop();
        let b = body?;
        let ir = SubqueryIR {
            sources: b.sources.clone(),
            filter: b.filter.clone(),
            group_by: b.group_by.clone(),
            having: b.having.as_ref().map(|h| h.e.clone()),
            distinct: q.select.distinct,
            select: b
                .outputs
                .iter()
                .map(|o| match &o.typed.t {
                    T::Known(ty) => ColumnOut { name: o.name.clone(), expr: o.typed.e.clone(), ty: ty.clone(), nullable: o.typed.nullable },
                    // only `exists` tolerates this, and it never looks at the columns
                    T::Null => ColumnOut { name: o.name.clone(), expr: o.typed.e.clone(), ty: builtin(Builtin::Text), nullable: true },
                })
                .collect(),
            order_by: b.order.iter().map(|(t, desc, _)| OrderIR { expr: t.e.clone(), desc: *desc }).collect(),
            limit: b.limit.clone(),
            offset: b.offset.clone(),
            unions: b.unions.clone(),
            ctes: b.ctes.clone(),
        };
        Some((ir, b))
    }

    /// The clauses shared by a query and a subquery, checked in the current scope: one
    /// `select`, or several combined with `union` / `intersect` / `except`.
    fn select_body(&mut self, q: &Query, named: bool) -> Option<Body> {
        let mark = self.ctes.len();
        let ctes = self.with_queries(&q.ctes);
        let body = ctes.and_then(|ctes| self.select_body_inner(q, named).map(|mut b| {
            b.ctes = ctes;
            b
        }));
        self.ctes.truncate(mark);
        body
    }

    /// Check `with` queries in order, each seeing the ones before it, and bring them into scope.
    fn with_queries(&mut self, ctes: &[Cte]) -> Option<Vec<CteIR>> {
        let mut out = Vec::new();
        let mut ok = true;
        for c in ctes {
            if ctes.iter().take_while(|o| !std::ptr::eq(*o, c)).any(|o| o.name.name == c.name.name) {
                self.fail("QL254", format!("`{}` is defined twice in this `with`", c.name.name), c.name.span);
                ok = false;
                continue;
            }
            // its columns are the query's named outputs
            let base = self.sources.len();
            self.scopes.push(base);
            let body = self.select_body(&c.query, true);
            self.sources.truncate(base);
            self.scopes.pop();
            let Some(b) = body else {
                ok = false;
                continue;
            };
            let columns: Vec<certo_sdl::ColumnIR> = b
                .outputs
                .iter()
                .filter_map(|o| match &o.typed.t {
                    T::Known(ty) => Some(certo_sdl::ColumnIR {
                        name: o.name.clone(),
                        ty: ty.clone(),
                        primary_key: false,
                        unique: false,
                        nullable: o.typed.nullable,
                        default: None,
                        references: None,
                        generated: None,
                    }),
                    T::Null => None,
                })
                .collect();
            let ir = SubqueryIR {
                sources: b.sources.clone(),
                filter: b.filter.clone(),
                group_by: b.group_by.clone(),
                having: b.having.as_ref().map(|h| h.e.clone()),
                distinct: c.query.select.distinct,
                select: columns_of(b.outputs.clone()),
                order_by: b.order.iter().map(|(t, desc, _)| OrderIR { expr: t.e.clone(), desc: *desc }).collect(),
                limit: b.limit.clone(),
                offset: b.offset.clone(),
                unions: b.unions.clone(),
                ctes: b.ctes.clone(),
            };
            self.ctes.push((
                c.name.name.clone(),
                TableIR { name: c.name.name.clone(), columns, relationships: vec![], indexes: vec![], constraints: vec![] },
            ));
            out.push(CteIR { name: c.name.name.clone(), query: ir });
        }
        ok.then_some(out)
    }

    fn select_body_inner(&mut self, q: &Query, named: bool) -> Option<Body> {
        if q.compound.is_empty() {
            return self.branch_body(q, named, true);
        }
        let base = self.scope_start();
        let mut first = self.branch_body(q, named, false)?;
        self.sources.truncate(base); // the next branch cannot see this one's tables

        // `a union b intersect c` means different things in PostgreSQL and SQLite: no mixing
        let ops: HashSet<SetOp> = q.compound.iter().map(|b| b.op).collect();
        if ops.len() > 1 {
            self.fail("QL253", "a query may combine `union`s, or `intersect`s, or `except`s, but not a mixture", q.compound[0].span);
            return None;
        }
        let mut unions = Vec::new();
        let mut ok = true;
        for br in &q.compound {
            if br.all && br.op != SetOp::Union {
                self.fail("QL253", "`all` is only available with `union`", br.span);
                ok = false;
            }
            let Some(b) = self.branch_body(&br.query, false, false) else {
                self.sources.truncate(base);
                ok = false;
                continue;
            };
            self.sources.truncate(base);
            if b.outputs.len() != first.outputs.len() {
                self.fail(
                    "QL253",
                    format!("this branch returns {} column(s) but the first returns {}", b.outputs.len(), first.outputs.len()),
                    br.span,
                );
                ok = false;
                continue;
            }
            for (i, (a, c)) in first.outputs.iter_mut().zip(&b.outputs).enumerate() {
                match unify(&a.typed.t, &c.typed.t) {
                    Some(t) => {
                        a.typed.t = t;
                        a.typed.nullable |= c.typed.nullable;
                    }
                    None => {
                        self.fail(
                            "QL253",
                            format!(
                                "column {} (`{}`) has different types in the branches: {} and {}",
                                i + 1, a.name, describe_t(&a.typed.t), describe_t(&c.typed.t)
                            ),
                            c.span,
                        );
                        ok = false;
                    }
                }
            }
            unions.push(UnionIR {
                op: br.op,
                all: br.all,
                branch: BranchIR {
                    sources: b.sources,
                    filter: b.filter,
                    group_by: b.group_by,
                    having: b.having.map(|h| h.e),
                    distinct: br.query.select.distinct,
                    select: columns_of(b.outputs),
                },
            });
        }

        // ordering and limits belong to the whole combination, and name its output columns
        let mut order: Vec<(Typed, bool, Span)> = Vec::new();
        for o in &q.order_by {
            match &o.expr {
                Expr::Column { qualifier: None, name } if first.outputs.iter().any(|x| x.name == name.name) => {
                    let out = first.outputs.iter().find(|x| x.name == name.name).unwrap();
                    order.push((
                        Typed { e: QExpr::Column { source: String::new(), column: name.name.clone() }, t: out.typed.t.clone(), nullable: out.typed.nullable, agg: false },
                        o.desc,
                        o.expr.span(),
                    ));
                }
                other => {
                    self.fail("QL250", "with `union`, `intersect` or `except`, `order by` can only name an output column of the result", other.span());
                    ok = false;
                }
            }
        }
        let limit = self.limit_expr(&q.limit, "limit").flatten();
        let offset = self.limit_expr(&q.offset, "offset").flatten();
        if !ok {
            return None;
        }
        first.order = order;
        first.limit = limit;
        first.offset = offset;
        first.unions = unions;
        first.single_row = false;
        Some(first)
    }

    /// One `select`. `tail`: also check its `order by` / `limit` / `offset` (a branch of a
    /// set operation has none; the combination does).
    fn branch_body(&mut self, q: &Query, named: bool, tail: bool) -> Option<Body> {
        let base = self.scope_start();

        // sources, each join's ON seeing the sources up to and including itself
        let mut sources_ir = Vec::new();
        self.add_source(&q.from, false)?;
        sources_ir.push(SourceIR { alias: self.sources[base].alias.clone(), table: q.from.table.name.clone(), join: None, on: None });
        for j in &q.joins {
            self.add_source(&j.table, j.kind == JoinKind::Left)?;
            let on = self.expr(&j.on, Ctx::On);
            if let Some(on) = &on
                && !is_bool(&on.t)
            {
                self.fail("QL212", format!("a join condition must be boolean, found {}", describe_t(&on.t)), j.on.span());
            }
            let last = self.sources.last().unwrap();
            sources_ir.push(SourceIR {
                alias: last.alias.clone(),
                table: j.table.table.name.clone(),
                join: Some(j.kind),
                on: on.map(|t| t.e),
            });
        }

        let filter = match &q.filter {
            Some(f) => self.expr(f, Ctx::Where).map(|t| {
                if !is_bool(&t.t) {
                    self.fail("QL212", format!("`where` must be boolean, found {}", describe_t(&t.t)), f.span());
                }
                t.e
            }),
            None => None,
        };

        let mut group_by = Vec::new();
        for g in &q.group_by {
            if let Some(t) = self.expr(g, Ctx::Group) {
                group_by.push(t.e);
            }
        }

        // select (needed before `having` / `order by`, which may name its outputs)
        let outputs = self.outputs(&q.select.items, Ctx::Select, named);

        let having = match &q.having {
            Some(h) => self.expr(h, Ctx::Having).inspect(|t| {
                if !is_bool(&t.t) {
                    self.fail("QL212", format!("`having` must be boolean, found {}", describe_t(&t.t)), h.span());
                }
            }),
            None => None,
        };

        let mut order: Vec<(Typed, bool, Span)> = Vec::new();
        for o in q.order_by.iter().filter(|_| tail) {
            // a bare name that matches an output column refers to that column
            let by_alias = match &o.expr {
                Expr::Column { qualifier: None, name } => outputs.iter().find(|x| x.name == name.name),
                _ => None,
            };
            let typed = match by_alias {
                Some(out) => Typed { e: out.typed.e.clone(), t: out.typed.t.clone(), nullable: out.typed.nullable, agg: out.typed.agg },
                None => {
                    let Some(t) = self.expr(&o.expr, Ctx::Order) else { continue };
                    t
                }
            };
            order.push((typed, o.desc, o.expr.span()));
        }

        if q.select.distinct {
            // PostgreSQL requires this; say so here rather than at run time
            for (t, _, span) in &order {
                if !outputs.iter().any(|o| o.typed.e == t.e) {
                    self.fail("QL221", "with `select distinct`, `order by` must use columns that are in the select list", *span);
                }
            }
        }

        let limit = if tail { self.limit_expr(&q.limit, "limit").flatten() } else { None };
        let offset = if tail { self.limit_expr(&q.offset, "offset").flatten() } else { None };

        // once aggregates or `group by` are involved, every column must be grouped or aggregated
        let aggregates = outputs.iter().any(|o| o.typed.agg)
            || having.as_ref().is_some_and(|h| h.agg)
            || order.iter().any(|o| o.0.agg);
        let grouped = !group_by.is_empty() || aggregates;
        if grouped && !self.failed {
            let scope: HashSet<String> = self.sources[base..].iter().map(|s| s.alias.clone()).collect();
            let check = |cx: &mut Self, e: &QExpr, span: Span| {
                if let Some(col) = uncovered(e, &group_by, &scope) {
                    cx.fail("QL214", format!("`{col}` must appear in `group by` or be used inside an aggregate"), span);
                }
            };
            for o in &outputs { check(self, &o.typed.e, o.span); }
            if let (Some(h), Some(src)) = (&having, &q.having) { check(self, &h.e, src.span()); }
            for (t, _, span) in &order { check(self, &t.e, *span); }
        }

        Some(Body {
            sources: sources_ir,
            filter,
            // with aggregates and no `group by` the query always yields exactly one row
            single_row: group_by.is_empty() && aggregates && having.is_none(),
            group_by,
            having,
            outputs,
            order,
            limit,
            offset,
            unions: Vec::new(),
            ctes: Vec::new(),
        })
    }

    // ---- mutations -------------------------------------------------------- //

    fn mutation(&mut self, m: &Mutation) -> Option<MutationIR> {
        let mark = self.ctes.len();
        let ctes = self.with_queries(&m.ctes);
        let r = ctes.and_then(|ctes| self.mutation_inner(m).map(|mut ir| {
            ir.ctes = ctes;
            ir
        }));
        self.ctes.truncate(mark);
        r
    }

    fn mutation_inner(&mut self, m: &Mutation) -> Option<MutationIR> {
        self.declare_params(&m.params);
        let Some(table) = self.schema.table(&m.table.table.name) else {
            self.fail("QL203", format!("unknown table `{}`", m.table.table.name), m.table.table.span);
            return None;
        };
        let alias = m.table.alias.as_ref().map_or_else(|| m.table.table.name.clone(), |a| a.name.clone());

        // An insert's values cannot read the row being created, so its `set`
        // expressions are checked with no table in scope; an update's can.
        let mut assignments = Vec::new();
        let (mut insert_columns, mut rows, mut source) = (Vec::new(), Vec::new(), None);
        if m.kind == MutationKind::Insert {
            if m.insert_columns.is_empty() {
                assignments = self.assignments(table, &m.assignments);
            } else {
                (insert_columns, rows, source) = self.insert_rows(table, m);
            }
        }
        self.add_source(&m.table, false)?;
        if m.kind == MutationKind::Update {
            assignments = self.assignments(table, &m.assignments);
        }

        let filter = match &m.filter {
            Some(f) => self.expr(f, Ctx::Where).map(|t| {
                if !is_bool(&t.t) {
                    self.fail("QL212", format!("`where` must be boolean, found {}", describe_t(&t.t)), f.span());
                }
                t.e
            }),
            None => None,
        };

        if m.kind == MutationKind::Insert {
            // judged on what was written, so a bad value is not also reported as missing
            let set: HashSet<&str> = m
                .assignments
                .iter()
                .map(|a| a.column.name.as_str())
                .chain(m.insert_columns.iter().map(|c| c.name.as_str()))
                .collect();
            let missing: Vec<&str> = table
                .columns
                .iter()
                .filter(|c| !c.nullable && c.default.is_none() && c.generated.is_none() && !set.contains(c.name.as_str()))
                .map(|c| c.name.as_str())
                .collect();
            if !missing.is_empty() {
                self.fail(
                    "QL233",
                    format!("`{}` needs a value for the required column(s): {}", m.table.table.name, missing.join(", ")),
                    m.table.table.span,
                );
            }
        }

        let conflict = match &m.conflict {
            Some(c) => self.conflict(table, &alias, c),
            None => None,
        };

        let returning: Vec<ColumnOut> = self
            .outputs(&m.returning, Ctx::Returning, true)
            .into_iter()
            .filter_map(|o| match o.typed.t {
                T::Known(ty) => Some(ColumnOut { name: o.name, expr: o.typed.e, ty, nullable: o.typed.nullable }),
                T::Null => None,
            })
            .collect();

        self.warn_unused_params();
        Some(MutationIR {
            name: m.name.name.clone(),
            kind: m.kind,
            params: self.param_irs(),
            table: m.table.table.name.clone(),
            alias,
            assignments,
            insert_columns,
            rows,
            source,
            filter,
            all_rows: m.all_rows,
            conflict,
            returning,
            ctes: Vec::new(),
        })
    }

    /// `into t (a, b) values (..), (..)` or `into t (a, b) from ... select ...`:
    /// the columns, then the checked rows or the checked feeding query.
    fn insert_rows(&mut self, table: &TableIR, m: &Mutation) -> (Vec<String>, Vec<Vec<QExpr>>, Option<Box<SubqueryIR>>) {
        // a `None` keeps a bad column's place so the rest of each row still lines up and is checked
        let mut cols: Vec<Option<&certo_sdl::ColumnIR>> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for id in &m.insert_columns {
            let Some(col) = table.column(&id.name) else {
                self.fail("QL206", format!("unknown column `{}` in `{}`", id.name, table.name), id.span);
                cols.push(None);
                continue;
            };
            if seen.contains(&col.name.as_str()) {
                self.fail("QL235", format!("column `{}` is listed more than once", col.name), id.span);
                cols.push(None);
                continue;
            }
            seen.push(&col.name);
            if col.generated == Some(certo_sdl::Generation::Always) {
                self.fail("QL234", format!("`{}` is `generated always`; its value cannot be set", col.name), id.span);
                cols.push(None);
                continue;
            }
            cols.push(Some(col));
        }
        let names: Vec<String> = cols.iter().flatten().map(|c| c.name.clone()).collect();

        let mut rows = Vec::new();
        for row in &m.rows {
            if row.len() != cols.len() {
                let span = row[0].span().to(row[row.len() - 1].span());
                self.fail("QL242", format!("this row has {} value(s) but {} column(s) are listed", row.len(), cols.len()), span);
                continue;
            }
            let mut out = Vec::new();
            for (col, e) in cols.iter().zip(row) {
                let Some(v) = self.expr(e, Ctx::Set) else { continue };
                if let Some(col) = col
                    && self.assignable(col, &v, e.span())
                {
                    out.push(v.e);
                }
            }
            rows.push(out);
        }

        let mut source = None;
        if let Some(q) = &m.source
            && let Some((ir, body)) = self.subquery(q)
        {
            if body.outputs.len() != cols.len() {
                self.fail(
                    "QL242",
                    format!("the query returns {} column(s) but {} column(s) are listed", body.outputs.len(), cols.len()),
                    q.span,
                );
            } else {
                for (col, out) in cols.iter().zip(&body.outputs) {
                    if let Some(col) = col {
                        self.assignable(col, &out.typed, out.span);
                    }
                }
                source = Some(Box::new(ir));
            }
        }
        (names, rows, source)
    }

    fn conflict(&mut self, table: &'a TableIR, alias: &str, c: &Conflict) -> Option<ConflictIR> {
        let mut cols = Vec::new();
        for id in &c.columns {
            if table.column(&id.name).is_none() {
                self.fail("QL206", format!("unknown column `{}` in `{}`", id.name, table.name), id.span);
                return None;
            }
            cols.push(id.name.clone());
        }
        let pk: Vec<&str> = table.columns.iter().filter(|x| x.primary_key).map(|x| x.name.as_str()).collect();
        let same = |a: &[&str], b: &[String]| a.len() == b.len() && a.iter().all(|x| b.iter().any(|y| y == x));
        let unique = (!pk.is_empty() && same(&pk, &cols))
            || (cols.len() == 1 && table.column(&cols[0]).is_some_and(|x| x.unique));
        if !unique {
            self.fail(
                "QL236",
                format!("({}) is not a unique key of `{}`, so `on conflict` cannot use it", cols.join(", "), table.name),
                c.span,
            );
            return None;
        }
        let action = match &c.action {
            ConflictAction::Nothing => ConflictActionIR::Nothing,
            ConflictAction::Update(list) => {
                if alias == "excluded" {
                    self.fail("QL204", "the table alias `excluded` is reserved for the incoming row in `on conflict`", c.span);
                    return None;
                }
                // the update may read the existing row and the row that was refused
                self.sources.push(SourceInfo { alias: "excluded".into(), table: Cow::Borrowed(table), nullable: false });
                let assignments = self.assignments(table, list);
                self.sources.pop();
                ConflictActionIR::Update { assignments }
            }
        };
        Some(ConflictIR { columns: cols, action })
    }

    /// Check `set col = expr, ...` against `table`'s columns.
    fn assignments(&mut self, table: &TableIR, list: &[Assignment]) -> Vec<AssignIR> {
        let mut out: Vec<AssignIR> = Vec::new();
        for a in list {
            let Some(col) = table.column(&a.column.name) else {
                self.fail("QL206", format!("unknown column `{}` in `{}`", a.column.name, table.name), a.column.span);
                let _ = self.expr(&a.value, Ctx::Set);
                continue;
            };
            if out.iter().any(|x| x.column == col.name) {
                self.fail("QL235", format!("column `{}` is assigned more than once", col.name), a.column.span);
                continue;
            }
            if col.generated == Some(certo_sdl::Generation::Always) {
                self.fail("QL234", format!("`{}` is `generated always`; its value cannot be set", col.name), a.column.span);
                continue;
            }
            let Some(v) = self.expr(&a.value, Ctx::Set) else { continue };
            if self.assignable(col, &v, a.value.span()) {
                out.push(AssignIR { column: col.name.clone(), expr: v.e });
            }
        }
        out
    }

    /// Can `v` be stored in `col`? Stricter than a comparison: no narrowing,
    /// and a value that may be NULL never goes into a NOT NULL column.
    fn assignable(&mut self, col: &certo_sdl::ColumnIR, v: &Typed, span: Span) -> bool {
        let T::Known(vt) = &v.t else {
            if col.nullable { return true; }
            self.fail("QL232", format!("column `{}` is NOT NULL; it cannot be set to null", col.name), span);
            return false;
        };
        if v.nullable && !col.nullable {
            self.fail(
                "QL232",
                format!("this value may be NULL but column `{}` is NOT NULL (declare the parameter without `null`, or use `coalesce`)", col.name),
                span,
            );
            return false;
        }
        let ct = &col.ty;
        let ok = match (&v.e, ct) {
            // a decimal literal only fits a column that can hold a fraction
            (QExpr::Decimal { .. }, _) => matches!(ct, TypeIR::Builtin(Builtin::Decimal | Builtin::Numeric(..) | Builtin::Real | Builtin::Float)),
            (QExpr::String { value }, TypeIR::Enum(en)) => {
                let variants = self.schema.enums.iter().find(|e| &e.name == en).map(|e| e.variants.clone()).unwrap_or_default();
                if !variants.contains(value) {
                    self.fail("QL218", format!("`{value}` is not a value of enum `{en}` (values: {})", variants.join(", ")), span);
                    return false;
                }
                true
            }
            (QExpr::String { .. }, TypeIR::Builtin(Builtin::Uuid | Builtin::Timestamp | Builtin::TimestampNaive | Builtin::Date | Builtin::Json)) => true,
            (QExpr::Number { .. }, _) if rank(ct).is_some() => true,
            _ => {
                // `qty = qty + 1` on a smallint column: arithmetic over literals and
                // values no wider than the column is accepted (PostgreSQL range-checks it).
                let widening = matches!((rank(ct), rank(vt)), (Some(c), Some(x)) if x <= c)
                    || (matches!(rank(ct), Some(0..=2)) && self.fits(&v.e, rank(ct).unwrap_or(0)));
                let time = matches!(
                    (ct, vt),
                    (TypeIR::Builtin(Builtin::Timestamp | Builtin::TimestampNaive), TypeIR::Builtin(Builtin::Date))
                        | (TypeIR::Builtin(Builtin::Timestamp), TypeIR::Builtin(Builtin::TimestampNaive))
                        | (TypeIR::Builtin(Builtin::TimestampNaive), TypeIR::Builtin(Builtin::Timestamp))
                );
                ct == vt || widening || (is_text(ct) && is_text(vt)) || time
            }
        };
        if !ok {
            self.fail(
                "QL211",
                format!("cannot store {} in column `{}` ({}){}", describe_t(&v.t), col.name, describe_type(ct), narrowing_hint(ct, vt)),
                span,
            );
        }
        ok
    }

    /// Is `e` integer arithmetic over literals and values of rank <= `max`?
    fn fits(&self, e: &QExpr, max: u8) -> bool {
        match e {
            QExpr::Number { .. } => true,
            QExpr::Column { source, column } => self
                .sources
                .iter()
                .rev()
                .find(|s| &s.alias == source)
                .and_then(|s| s.table.column(column))
                .and_then(|c| rank(&c.ty))
                .is_some_and(|r| r <= max),
            QExpr::Param { name } => self.params.iter().find(|p| &p.name == name).and_then(|p| rank(&p.ty)).is_some_and(|r| r <= max),
            QExpr::Binary { op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div, lhs, rhs } => {
                self.fits(lhs, max) && self.fits(rhs, max)
            }
            _ => false,
        }
    }

    fn expand(&mut self, i: usize, _span: Span, out: &mut Vec<Output>) {
        let src = &self.sources[i];
        let (alias, nullable_side) = (src.alias.clone(), src.nullable);
        for c in &src.table.columns {
            out.push(Output {
                name: c.name.clone(),
                typed: Typed {
                    e: QExpr::Column { source: alias.clone(), column: c.name.clone() },
                    t: T::Known(c.ty.clone()),
                    nullable: c.nullable || nullable_side,
                    agg: false,
                },
                span: _span,
            });
        }
    }

    fn add_source(&mut self, t: &TableRef, nullable: bool) -> Option<()> {
        let table: Cow<'a, TableIR> = match self.ctes.iter().rev().find(|(n, _)| *n == t.table.name) {
            Some((_, cte)) => Cow::Owned(cte.clone()),
            None => match self.schema.table(&t.table.name) {
                Some(table) => Cow::Borrowed(table),
                None => {
                    self.fail("QL203", format!("unknown table `{}`", t.table.name), t.table.span);
                    return None;
                }
            },
        };
        let alias = t.alias.as_ref().map_or_else(|| t.table.name.clone(), |a| a.name.clone());
        if self.sources[self.scope_start()..].iter().any(|s| s.alias == alias) {
            let span = t.alias.as_ref().map_or(t.table.span, |a| a.span);
            self.fail("QL204", format!("`{alias}` is already used as a table alias in this query"), span);
            return None;
        }
        self.sources.push(SourceInfo { alias, table, nullable });
        Some(())
    }

    fn limit_expr(&mut self, e: &Option<Expr>, what: &str) -> Option<Option<QExpr>> {
        let Some(e) = e else { return Some(None) };
        let t = self.expr(e, Ctx::Limit)?;
        let ok = match (&t.e, &t.t) {
            (QExpr::Number { value }, _) => *value >= 0,
            (QExpr::Param { .. }, T::Known(TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt))) => true,
            _ => false,
        };
        if !ok {
            self.fail("QL217", format!("`{what}` takes a non-negative whole number or an integer parameter"), e.span());
        }
        Some(Some(t.e))
    }

    // ---- parameters ------------------------------------------------------- //

    fn param_type(&mut self, t: &TypeRef) -> Option<TypeIR> {
        let name = t.name.name.as_str();
        let b = |x: Builtin| Some(TypeIR::Builtin(x));
        let bad = |cx: &mut Self, msg: String| {
            cx.fail("QL219", msg, t.span);
            None
        };
        match (name, t.args.as_slice()) {
            ("varchar" | "char", [n]) if (1..=10_485_760).contains(n) => {
                b(if name == "varchar" { Builtin::Varchar(*n) } else { Builtin::Char(*n) })
            }
            ("decimal" | "numeric", []) => b(Builtin::Decimal),
            ("decimal" | "numeric", [p]) if (1..=1000).contains(p) => b(Builtin::Numeric(*p as u16, 0)),
            ("decimal" | "numeric", [p, s]) if (1..=1000).contains(p) && s <= p => b(Builtin::Numeric(*p as u16, *s as u16)),
            (_, []) => {
                if let Some(bi) = Builtin::from_name(name) {
                    b(bi)
                } else if self.schema.enums.iter().any(|e| e.name == name) {
                    Some(TypeIR::Enum(name.to_string()))
                } else if self.schema.types.iter().any(|c| c.name == name) {
                    bad(self, format!("`{name}` is a composite type, which cannot be a parameter"))
                } else {
                    bad(self, format!("unknown parameter type `{name}`"))
                }
            }
            _ => bad(self, format!("invalid parameters for type `{name}`")),
        }
    }

    // ---- name resolution -------------------------------------------------- //

    /// The `sources` index ranges of the open scopes, innermost first.
    fn scope_ranges(&self) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut end = self.sources.len();
        for &start in self.scopes.iter().rev() {
            out.push(start..end);
            end = start;
        }
        out
    }

    /// Resolve `[qualifier.]name`. The innermost query's tables win; a name that
    /// is not there is looked for in the queries around (a correlated reference).
    fn column(&mut self, qualifier: &Option<certo_sdl::Ident>, name: &certo_sdl::Ident) -> Option<Typed> {
        let mut found: Vec<(usize, &certo_sdl::ColumnIR)> = Vec::new();
        let ranges = self.scope_ranges();
        match qualifier {
            Some(q) => {
                let Some(i) = ranges
                    .iter()
                    .find_map(|r| self.sources[r.clone()].iter().position(|s| s.alias == q.name).map(|p| r.start + p))
                else {
                    self.fail("QL205", format!("unknown table alias `{}`", q.name), q.span);
                    return None;
                };
                found.extend(self.sources[i].table.column(&name.name).map(|c| (i, c)));
            }
            None => {
                for r in ranges {
                    found = r
                        .filter_map(|i| self.sources[i].table.column(&name.name).map(|c| (i, c)))
                        .collect();
                    if !found.is_empty() {
                        break;
                    }
                }
            }
        }
        match found.as_slice() {
            [(i, c)] => {
                let s = &self.sources[*i];
                Some(Typed {
                    e: QExpr::Column { source: s.alias.clone(), column: c.name.clone() },
                    t: T::Known(c.ty.clone()),
                    nullable: c.nullable || s.nullable,
                    agg: false,
                })
            }
            [] => {
                let span = qualifier.as_ref().map_or(name.span, |q| q.span.to(name.span));
                let where_ = match qualifier {
                    Some(q) => format!(" in `{}`", q.name),
                    None => String::new(),
                };
                self.fail("QL206", format!("unknown column `{}`{where_}", name.name), span);
                None
            }
            many => {
                let aliases: Vec<_> = many.iter().map(|(i, _)| self.sources[*i].alias.clone()).collect();
                self.fail(
                    "QL207",
                    format!("column `{}` is ambiguous (in {}); qualify it, e.g. `{}.{}`", name.name, aliases.join(" and "), aliases[0], name.name),
                    name.span,
                );
                None
            }
        }
    }

    // ---- expressions ------------------------------------------------------ //

    fn expr(&mut self, e: &Expr, ctx: Ctx) -> Option<Typed> {
        let lit = |e: QExpr, ty: Builtin| Some(Typed { e, t: T::Known(builtin(ty)), nullable: false, agg: false });
        match e {
            Expr::Number(n, _) => lit(QExpr::Number { value: *n }, Builtin::Int),
            Expr::Decimal(d, _) => lit(QExpr::Decimal { value: d.clone() }, Builtin::Decimal),
            Expr::Str(s, _) => lit(QExpr::String { value: s.clone() }, Builtin::Text),
            Expr::Bool(b, _) => lit(QExpr::Bool { value: *b }, Builtin::Bool),
            Expr::Null(_) => Some(Typed { e: QExpr::Null, t: T::Null, nullable: true, agg: false }),
            Expr::Paren(inner, _) => self.expr(inner, ctx),
            Expr::Column { qualifier, name } => self.column(qualifier, name),
            Expr::Param(id) => {
                match self.params.iter_mut().find(|p| p.name == id.name) {
                    Some(p) => {
                        p.used = true;
                        Some(Typed { e: QExpr::Param { name: p.name.clone() }, t: T::Known(p.ty.clone()), nullable: p.nullable, agg: false })
                    }
                    None => {
                        self.fail("QL208", format!("unknown parameter `:{}`", id.name), id.span);
                        None
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let (l, r) = (self.expr(lhs, ctx), self.expr(rhs, ctx));
                self.binary(*op, l?, r?, *span)
            }
            Expr::Not(inner, span) => {
                let t = self.expr(inner, ctx)?;
                if !is_bool(&t.t) {
                    self.fail("QL211", format!("`not` requires a boolean, found {}", describe_t(&t.t)), *span);
                    return None;
                }
                Some(Typed { nullable: t.nullable, agg: t.agg, t: T::Known(builtin(Builtin::Bool)), e: QExpr::Not { expr: Box::new(t.e) } })
            }
            Expr::IsNull { expr, negated, .. } => {
                let t = self.expr(expr, ctx)?;
                Some(Typed { agg: t.agg, e: QExpr::IsNull { expr: Box::new(t.e), negated: *negated }, t: T::Known(builtin(Builtin::Bool)), nullable: false })
            }
            Expr::In { expr, list, negated, .. } => {
                let l = self.expr(expr, ctx)?;
                let (mut items, mut nullable, mut agg, mut failed) = (Vec::new(), l.nullable, l.agg, false);
                for item in list {
                    let Some(t) = self.expr(item, ctx) else { failed = true; continue };
                    let before = self.diags.len();
                    if !self.comparable(&l, &t, item.span()) {
                        // `comparable` already explained an unknown enum value; don't add a second error
                        if self.diags.len() == before {
                            self.fail("QL211", format!("cannot compare {} with {}", describe_t(&l.t), describe_t(&t.t)), item.span());
                        }
                        failed = true;
                        continue;
                    }
                    nullable |= t.nullable;
                    agg |= t.agg;
                    items.push(t.e);
                }
                if failed { return None; }
                Some(Typed { e: QExpr::In { expr: Box::new(l.e), list: items, negated: *negated }, t: T::Known(builtin(Builtin::Bool)), nullable, agg })
            }
            Expr::Like { expr, pattern, negated, span } => {
                let (l, p) = (self.expr(expr, ctx), self.expr(pattern, ctx));
                let (l, p) = (l?, p?);
                let text = |t: &T| matches!(t, T::Null) || matches!(t, T::Known(k) if is_text(k));
                if !text(&l.t) || !text(&p.t) {
                    self.fail("QL211", format!("`like` compares text, found {} and {}", describe_t(&l.t), describe_t(&p.t)), *span);
                    return None;
                }
                Some(Typed {
                    nullable: l.nullable || p.nullable,
                    agg: l.agg || p.agg,
                    t: T::Known(builtin(Builtin::Bool)),
                    e: QExpr::Like { expr: Box::new(l.e), pattern: Box::new(p.e), negated: *negated },
                })
            }
            Expr::Between { expr, low, high, negated, span } => {
                // x between a and b  ==  x >= a and x <= b
                let x = self.expr(expr, ctx)?;
                let lo = self.expr(low, ctx)?;
                let hi = self.expr(high, ctx)?;
                let x2 = Typed { e: x.e.clone(), t: x.t.clone(), nullable: x.nullable, agg: x.agg };
                let ge = self.binary(BinaryOp::Ge, x, lo, *span)?;
                let le = self.binary(BinaryOp::Le, x2, hi, *span)?;
                let both = self.binary(BinaryOp::And, ge, le, *span)?;
                if *negated {
                    Some(Typed { nullable: both.nullable, agg: both.agg, t: both.t, e: QExpr::Not { expr: Box::new(both.e) } })
                } else {
                    Some(both)
                }
            }
            Expr::Case { whens, otherwise, .. } => {
                let (mut ws, mut result, mut nullable, mut agg) = (Vec::new(), T::Null, false, false);
                for (w, t) in whens {
                    let c = self.expr(w, ctx)?;
                    if !is_bool(&c.t) {
                        self.fail("QL212", format!("a `when` condition must be boolean, found {}", describe_t(&c.t)), w.span());
                        return None;
                    }
                    let v = self.expr(t, ctx)?;
                    let Some(u) = unify(&result, &v.t) else {
                        self.fail("QL211", format!("`case` results have different types ({} and {})", describe_t(&result), describe_t(&v.t)), t.span());
                        return None;
                    };
                    result = u;
                    nullable |= v.nullable;
                    agg |= c.agg || v.agg;
                    ws.push(When { when: c.e, then: v.e });
                }
                let other = match otherwise {
                    Some(o) => {
                        let v = self.expr(o, ctx)?;
                        let Some(u) = unify(&result, &v.t) else {
                            self.fail("QL211", format!("`case` results have different types ({} and {})", describe_t(&result), describe_t(&v.t)), o.span());
                            return None;
                        };
                        result = u;
                        nullable |= v.nullable;
                        agg |= v.agg;
                        Some(Box::new(v.e))
                    }
                    None => {
                        nullable = true; // no branch matched -> NULL
                        None
                    }
                };
                Some(Typed { e: QExpr::Case { whens: ws, otherwise: other }, t: result, nullable, agg })
            }
            Expr::Call { func, args, star, distinct, over, span } => self.call(func, args, *star, *distinct, over.as_ref(), *span, ctx),
            Expr::Exists(q, _) => {
                let (ir, _) = self.subquery(q)?;
                Some(Typed { e: QExpr::Exists { query: Box::new(ir) }, t: T::Known(builtin(Builtin::Bool)), nullable: false, agg: false })
            }
            Expr::InQuery { expr, query, negated, span } => {
                let l = self.expr(expr, ctx);
                let sub = self.subquery(query);
                let (l, (ir, body)) = (l?, sub?);
                let col = self.one_column(&body, *span, "`in`")?;
                if !self.comparable(&l, &col, *span) {
                    if !self.failed {
                        self.fail("QL211", format!("cannot compare {} with the subquery's {}", describe_t(&l.t), describe_t(&col.t)), *span);
                    }
                    return None;
                }
                Some(Typed {
                    nullable: l.nullable || col.nullable,
                    agg: l.agg,
                    t: T::Known(builtin(Builtin::Bool)),
                    e: QExpr::InQuery { expr: Box::new(l.e), query: Box::new(ir), negated: *negated },
                })
            }
            Expr::Scalar(q, span) => {
                let (ir, body) = self.subquery(q)?;
                let col = self.one_column(&body, *span, "a subquery used as a value")?;
                let one_row = body.single_row || matches!(&body.limit, Some(QExpr::Number { value }) if *value <= 1);
                if !one_row {
                    self.fail(
                        "QL241",
                        "a subquery used as a value must return at most one row: use aggregates without `group by`, or `limit 1`",
                        *span,
                    );
                    return None;
                }
                // an aggregate always yields a row; otherwise the row may not exist
                let nullable = col.nullable || !body.single_row;
                Some(Typed { e: QExpr::Scalar { query: Box::new(ir) }, t: col.t, nullable, agg: false })
            }
        }
    }

    /// The single column a subquery must have, as a typed value.
    fn one_column(&mut self, body: &Body, span: Span, what: &str) -> Option<Typed> {
        let [out] = body.outputs.as_slice() else {
            self.fail("QL240", format!("the subquery for {what} must return exactly one column, not {}", body.outputs.len()), span);
            return None;
        };
        if out.typed.t == T::Null {
            self.fail("QL220", "cannot tell the type of the subquery's column: it is only NULL", out.span);
            return None;
        }
        Some(out.typed.clone())
    }

    /// Can these two be compared? A string literal may stand for an enum
    /// value (checked against the enum), a uuid, a timestamp, a date or json.
    fn comparable(&mut self, l: &Typed, r: &Typed, span: Span) -> bool {
        let (T::Known(a), T::Known(b)) = (&l.t, &r.t) else { return true };
        if compatible(a, b) {
            return true;
        }
        for (lit, other) in [(l, b), (r, a)] {
            if let QExpr::String { value } = &lit.e {
                match other {
                    TypeIR::Enum(en) => {
                        let variants = self.schema.enums.iter().find(|e| &e.name == en).map(|e| e.variants.clone()).unwrap_or_default();
                        if variants.contains(value) {
                            return true;
                        }
                        self.fail("QL218", format!("`{value}` is not a value of enum `{en}` (values: {})", variants.join(", ")), span);
                        return false;
                    }
                    TypeIR::Builtin(Builtin::Uuid | Builtin::Timestamp | Builtin::TimestampNaive | Builtin::Date | Builtin::Json) => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }

    fn binary(&mut self, op: BinaryOp, l: Typed, r: Typed, span: Span) -> Option<Typed> {
        use BinaryOp::*;
        let agg = l.agg || r.agg;
        let nullable = l.nullable || r.nullable;
        let boolean = T::Known(builtin(Builtin::Bool));
        let t = match op {
            And | Or => {
                if !is_bool(&l.t) || !is_bool(&r.t) {
                    self.fail("QL211", "`and` / `or` require boolean operands", span);
                    return None;
                }
                boolean
            }
            Add | Sub | Mul | Div => {
                let num = |t: &T| match t {
                    T::Null => Some(None),
                    T::Known(k) => rank(k).map(Some),
                };
                match (num(&l.t), num(&r.t)) {
                    (Some(a), Some(b)) => match (a, b) {
                        (Some(a), Some(b)) => T::Known(from_rank(a.max(b))),
                        (Some(a), None) | (None, Some(a)) => T::Known(from_rank(a)),
                        (None, None) => T::Null,
                    },
                    _ => {
                        self.fail("QL211", format!("arithmetic requires numbers, found {} and {}", describe_t(&l.t), describe_t(&r.t)), span);
                        return None;
                    }
                }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                let order_ok = matches!(op, Eq | Ne)
                    || [&l.t, &r.t].iter().all(|t| matches!(t, T::Null) || matches!(t, T::Known(k) if ordered(k)));
                if !order_ok || !self.comparable(&l, &r, span) {
                    if !self.failed {
                        self.fail("QL211", format!("cannot compare {} with {}", describe_t(&l.t), describe_t(&r.t)), span);
                    }
                    return None;
                }
                boolean
            }
        };
        Some(Typed { e: QExpr::Binary { op, lhs: Box::new(l.e), rhs: Box::new(r.e) }, t, nullable, agg })
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        func: &certo_sdl::Ident,
        args: &[Expr],
        star: bool,
        distinct: bool,
        over: Option<&WindowSpec>,
        span: Span,
        ctx: Ctx,
    ) -> Option<Typed> {
        let name = func.name.as_str();
        if let Some(w) = over {
            return self.window(name, args, star, distinct, w, span, ctx);
        }
        if matches!(name, "row_number" | "rank" | "dense_rank" | "ntile" | "percent_rank" | "cume_dist" | "lag" | "lead" | "first_value" | "last_value") {
            self.fail("QL252", format!("`{name}` is a window function: add `over (...)`"), span);
            return None;
        }
        let is_agg = matches!(name, "count" | "sum" | "avg" | "min" | "max");

        if is_agg {
            if !ctx.allows_agg() {
                self.fail("QL213", format!("aggregate `{name}` is not allowed here (only in `select`, `having` and `order by`)"), span);
                return None;
            }
            return self.aggregate(name, args, star, distinct, span, ctx);
        }
        if star || distinct {
            self.fail("QL209", format!("`{name}` is not an aggregate, so `*` and `distinct` do not apply"), span);
            return None;
        }
        let mut typed = Vec::new();
        for a in args {
            typed.push(self.expr(a, ctx)?);
        }
        let agg = typed.iter().any(|t| t.agg);
        let n = typed.len();
        let arity = |cx: &mut Self, want: &str| {
            cx.fail("QL209", format!("`{name}` takes {want}, got {n}"), span);
            None::<Typed>
        };
        let text_arg = |t: &T| matches!(t, T::Null) || matches!(t, T::Known(k) if is_text(k));
        let (ret, nullable): (T, bool) = match name {
            "lower" | "upper" | "trim" => {
                if n != 1 { return arity(self, "one argument"); }
                if !text_arg(&typed[0].t) {
                    self.fail("QL209", format!("`{name}` expects text, found {}", describe_t(&typed[0].t)), args[0].span());
                    return None;
                }
                (T::Known(builtin(Builtin::Text)), typed[0].nullable)
            }
            "length" => {
                if n != 1 { return arity(self, "one argument"); }
                if !text_arg(&typed[0].t) {
                    self.fail("QL209", format!("`length` expects text, found {}", describe_t(&typed[0].t)), args[0].span());
                    return None;
                }
                (T::Known(builtin(Builtin::Int)), typed[0].nullable)
            }
            "abs" | "round" => {
                if n != 1 { return arity(self, "one argument"); }
                if !matches!(&typed[0].t, T::Known(k) if rank(k).is_some()) {
                    self.fail("QL209", format!("`{name}` expects a number, found {}", describe_t(&typed[0].t)), args[0].span());
                    return None;
                }
                (typed[0].t.clone(), typed[0].nullable)
            }
            "coalesce" => {
                if n == 0 { return arity(self, "at least one argument"); }
                let mut ty = T::Null;
                for (a, src) in typed.iter().zip(args) {
                    let Some(u) = unify(&ty, &a.t) else {
                        self.fail("QL209", "`coalesce` arguments must have compatible types", src.span());
                        return None;
                    };
                    ty = u;
                }
                (ty, typed.iter().all(|t| t.nullable))
            }
            "nullif" => {
                if n != 2 { return arity(self, "two arguments"); }
                if !self.comparable(&typed[0], &typed[1], span) {
                    if !self.failed { self.fail("QL209", "`nullif` arguments must have compatible types", span); }
                    return None;
                }
                (typed[0].t.clone(), true)
            }
            "now" => {
                if n != 0 { return arity(self, "no arguments"); }
                (T::Known(builtin(Builtin::Timestamp)), false)
            }
            "today" => {
                if n != 0 { return arity(self, "no arguments"); }
                (T::Known(builtin(Builtin::Date)), false)
            }
            other => {
                self.fail("QL209", format!("unknown function `{other}`"), func.span);
                return None;
            }
        };
        Some(Typed { e: QExpr::Call { func: name.to_string(), args: typed.into_iter().map(|t| t.e).collect() }, t: ret, nullable, agg })
    }

    /// `f(args) over (partition by ... order by ...)`.
    #[allow(clippy::too_many_arguments)]
    fn window(&mut self, name: &str, args: &[Expr], star: bool, distinct: bool, over: &WindowSpec, span: Span, ctx: Ctx) -> Option<Typed> {
        if !ctx.allows_window() {
            self.fail("QL251", "window functions are only allowed in `select` and `order by`", span);
            return None;
        }
        if self.in_window {
            self.fail("QL251", "window functions cannot be nested", span);
            return None;
        }
        self.in_window = true;
        let r = self.window_inner(name, args, star, distinct, over, span, ctx);
        self.in_window = false;
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn window_inner(&mut self, name: &str, args: &[Expr], star: bool, distinct: bool, over: &WindowSpec, span: Span, ctx: Ctx) -> Option<Typed> {
        if distinct {
            self.fail("QL252", "`distinct` is not supported in a window function", span);
            return None;
        }

        // the window's own expressions (they may use aggregates, which then make the query grouped)
        let mut agg = false;
        let mut partition_by = Vec::new();
        let mut ok = true;
        for p in &over.partition_by {
            match self.expr(p, ctx) {
                Some(t) => {
                    agg |= t.agg;
                    partition_by.push(t.e);
                }
                None => ok = false,
            }
        }
        let mut order_by = Vec::new();
        let mut order_types = Vec::new();
        for o in &over.order_by {
            match self.expr(&o.expr, ctx) {
                Some(t) => {
                    agg |= t.agg;
                    order_types.push(t.t.clone());
                    order_by.push(OrderIR { expr: t.e, desc: o.desc });
                }
                None => ok = false,
            }
        }

        // the frame: which rows around the current one the function sees
        let mut frame_ir = None;
        let mut includes_current = true; // the default frame ends at the current row
        if let Some(f) = &over.frame {
            if !matches!(name, "count" | "sum" | "avg" | "min" | "max" | "first_value" | "last_value") {
                self.fail("QL255", format!("`{name}` does not use a frame: remove the `{}` clause", frame_word(f.units)), f.span);
                ok = false;
            } else if let Some((ir, current)) = self.frame(f, &order_types) {
                frame_ir = Some(Box::new(ir));
                includes_current = current;
            } else {
                ok = false;
            }
        }

        let n = args.len();
        let arity = |cx: &mut Self, want: &str| {
            cx.fail("QL252", format!("`{name}` takes {want}, got {n}"), span);
            None::<Typed>
        };
        let int_arg = |cx: &mut Self, e: &Expr, what: &str, min: i64| -> Option<QExpr> {
            let t = cx.expr(e, Ctx::Limit)?;
            let fine = match (&t.e, &t.t) {
                (QExpr::Number { value }, _) => *value >= min,
                (QExpr::Param { .. }, T::Known(TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt))) => true,
                _ => false,
            };
            if !fine {
                cx.fail("QL252", format!("{what} must be a whole number of at least {min}, or an integer parameter"), e.span());
                return None;
            }
            Some(t.e)
        };
        let (call, t, nullable): (QExpr, T, bool) = match name {
            "count" | "sum" | "avg" | "min" | "max" => {
                // the aggregate's own typing, for its result type and nullability
                let a = self.aggregate(name, args, star, false, span, Ctx::Select)?;
                (a.e, a.t, a.nullable)
            }
            "row_number" | "rank" | "dense_rank" | "percent_rank" | "cume_dist" => {
                if n != 0 { return arity(self, "no arguments"); }
                let ty = if matches!(name, "percent_rank" | "cume_dist") { Builtin::Float } else { Builtin::BigInt };
                (QExpr::Call { func: name.into(), args: vec![] }, T::Known(builtin(ty)), false)
            }
            "ntile" => {
                if n != 1 { return arity(self, "one argument"); }
                let a = int_arg(self, &args[0], "the number of buckets", 1)?;
                (QExpr::Call { func: name.into(), args: vec![a] }, T::Known(builtin(Builtin::Int)), false)
            }
            "first_value" | "last_value" => {
                if n != 1 { return arity(self, "one argument"); }
                let a = self.expr(&args[0], Ctx::Select)?;
                if a.agg {
                    self.fail("QL252", "an aggregate cannot be the argument of a window function", args[0].span());
                    return None;
                }
                // a frame that leaves out the current row may be empty
                (QExpr::Call { func: name.into(), args: vec![a.e] }, a.t, a.nullable || !includes_current)
            }
            "lag" | "lead" => {
                if !(1..=3).contains(&n) { return arity(self, "one to three arguments"); }
                let a = self.expr(&args[0], Ctx::Select)?;
                if a.agg {
                    self.fail("QL252", "an aggregate cannot be the argument of a window function", args[0].span());
                    return None;
                }
                let mut parts = vec![a.e.clone()];
                if n >= 2 {
                    parts.push(int_arg(self, &args[1], "the offset", 0)?);
                }
                let mut nullable = a.nullable;
                if n == 3 {
                    let d = self.expr(&args[2], Ctx::Select)?;
                    if !self.comparable(&a, &d, args[2].span()) {
                        if !self.failed { self.fail("QL252", format!("the default must have the same type as the value ({})", describe_t(&a.t)), args[2].span()); }
                        return None;
                    }
                    nullable |= d.nullable;
                    parts.push(d.e);
                } else {
                    nullable = true; // no row that far back / ahead
                }
                (QExpr::Call { func: name.into(), args: parts }, a.t, nullable)
            }
            other => {
                self.fail("QL252", format!("`{other}` is not a window function (use count, sum, avg, min, max, row_number, rank, dense_rank, ntile, lag, lead, first_value, last_value, percent_rank or cume_dist)"), span);
                return None;
            }
        };
        if !ok {
            return None;
        }
        Some(Typed { e: QExpr::Window { call: Box::new(call), partition_by, order_by, frame: frame_ir }, t, nullable, agg })
    }

    /// Check a window frame. Returns it lowered, and whether it includes the current row.
    fn frame(&mut self, f: &Frame, order_types: &[T]) -> Option<(FrameIR, bool)> {
        // rank of each kind of bound along the partition, to catch frames that end before they start
        let rank = |b: &FrameBound| match b {
            FrameBound::UnboundedPreceding => 0u8,
            FrameBound::Preceding(_) => 1,
            FrameBound::CurrentRow => 2,
            FrameBound::Following(_) => 3,
            FrameBound::UnboundedFollowing => 4,
        };
        let end = f.end.as_ref().unwrap_or(&FrameBound::CurrentRow);
        if matches!(f.start, FrameBound::UnboundedFollowing) {
            self.fail("QL255", "a frame cannot start at `unbounded following`", f.span);
            return None;
        }
        if matches!(end, FrameBound::UnboundedPreceding) {
            self.fail("QL255", "a frame cannot end at `unbounded preceding`", f.span);
            return None;
        }
        if rank(&f.start) > rank(end) {
            self.fail("QL255", "the frame ends before it starts", f.span);
            return None;
        }
        let offsets = matches!(f.start, FrameBound::Preceding(_) | FrameBound::Following(_))
            || matches!(end, FrameBound::Preceding(_) | FrameBound::Following(_));
        match f.units {
            FrameUnits::Groups if order_types.is_empty() => {
                self.fail("QL255", "a `groups` frame needs an `order by`", f.span);
                return None;
            }
            FrameUnits::Range
                if offsets && (order_types.len() != 1 || !matches!(&order_types[0], T::Known(k) if rank_of(k))) =>
            {
                self.fail("QL255", "a `range` frame with an offset needs exactly one numeric `order by` key", f.span);
                return None;
            }
            _ => {}
        }
        let lower = |cx: &mut Self, b: &FrameBound| -> Option<FrameBoundIR> {
            Some(match b {
                FrameBound::UnboundedPreceding => FrameBoundIR::UnboundedPreceding,
                FrameBound::CurrentRow => FrameBoundIR::CurrentRow,
                FrameBound::UnboundedFollowing => FrameBoundIR::UnboundedFollowing,
                FrameBound::Preceding(e) => FrameBoundIR::Preceding { offset: cx.frame_offset(e, f.units)? },
                FrameBound::Following(e) => FrameBoundIR::Following { offset: cx.frame_offset(e, f.units)? },
            })
        };
        let start = lower(self, &f.start);
        let stop = lower(self, end);
        let (start, stop) = (start?, stop?);
        let includes_current = rank(&f.start) <= 2 && rank(end) >= 2;
        Some((FrameIR { units: f.units, start, end: stop }, includes_current))
    }

    /// A frame offset: a non-negative literal or parameter (whole numbers for `rows` / `groups`).
    fn frame_offset(&mut self, e: &Expr, units: FrameUnits) -> Option<QExpr> {
        let t = self.expr(e, Ctx::Limit)?;
        let whole = |k: &TypeIR| matches!(k, TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt));
        let fine = match (&t.e, &t.t) {
            (QExpr::Number { value }, _) => *value >= 0,
            (QExpr::Decimal { value }, _) => units == FrameUnits::Range && !value.starts_with('-'),
            (QExpr::Param { .. }, T::Known(k)) => if units == FrameUnits::Range { rank_of(k) } else { whole(k) },
            _ => false,
        };
        if !fine {
            let what = if units == FrameUnits::Range { "a non-negative number" } else { "a non-negative whole number" };
            self.fail("QL255", format!("a frame offset must be {what} or a parameter"), e.span());
            return None;
        }
        Some(t.e)
    }

    fn aggregate(&mut self, name: &str, args: &[Expr], star: bool, distinct: bool, span: Span, ctx: Ctx) -> Option<Typed> {
        let big = T::Known(builtin(Builtin::BigInt));
        if star {
            if name != "count" || distinct {
                self.fail("QL209", format!("`{name}(*)` is not valid; only `count(*)` is"), span);
                return None;
            }
            return Some(Typed { e: QExpr::Agg { func: "count".into(), arg: None, distinct: false }, t: big, nullable: false, agg: true });
        }
        if args.len() != 1 {
            self.fail("QL209", format!("`{name}` takes one argument, got {}", args.len()), span);
            return None;
        }
        let a = self.expr(&args[0], ctx)?;
        if a.agg {
            self.fail("QL213", "aggregates cannot be nested", args[0].span());
            return None;
        }
        let (ret, nullable) = match name {
            "count" => (big, false),
            "sum" | "avg" => {
                let T::Known(k) = &a.t else {
                    self.fail("QL209", format!("`{name}` expects a number, found null"), args[0].span());
                    return None;
                };
                if rank(k).is_none() {
                    self.fail("QL209", format!("`{name}` expects a number, found {}", describe_t(&a.t)), args[0].span());
                    return None;
                }
                let out = match (name, k) {
                    // PostgreSQL: sum(int) is bigint, sum(bigint) is numeric; avg of integers is numeric
                    ("sum", TypeIR::Builtin(Builtin::SmallInt | Builtin::Int)) => Builtin::BigInt,
                    ("sum", TypeIR::Builtin(Builtin::BigInt)) => Builtin::Decimal,
                    ("sum", TypeIR::Builtin(Builtin::Real)) => Builtin::Real,
                    ("sum", TypeIR::Builtin(Builtin::Float)) => Builtin::Float,
                    ("avg", TypeIR::Builtin(Builtin::Real | Builtin::Float)) => Builtin::Float,
                    _ => Builtin::Decimal,
                };
                (T::Known(builtin(out)), true)
            }
            _ => {
                // min / max
                match &a.t {
                    T::Known(k) if ordered(k) => (a.t.clone(), true),
                    _ => {
                        self.fail("QL209", format!("`{name}` expects an ordered type, found {}", describe_t(&a.t)), args[0].span());
                        return None;
                    }
                }
            }
        };
        Some(Typed {
            e: QExpr::Agg { func: name.to_string(), arg: Some(Box::new(a.e)), distinct },
            t: ret,
            nullable,
            agg: true,
        })
    }
}

fn frame_word(u: FrameUnits) -> &'static str {
    match u {
        FrameUnits::Rows => "rows",
        FrameUnits::Range => "range",
        FrameUnits::Groups => "groups",
    }
}

fn rank_of(k: &TypeIR) -> bool { rank(k).is_some() }

fn narrowing_hint(col: &TypeIR, value: &TypeIR) -> &'static str {
    match (rank(col), rank(value)) {
        (Some(c), Some(v)) if v > c => ": that would narrow it; declare the parameter with the column's type",
        _ => "",
    }
}
