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
//! unique key. QL290 (warning): unused parameter.

use crate::ast::*;
use crate::ir::*;
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use certo_sdl::{
    compatible, describe_type, from_rank, is_text, ordered, rank, BinaryOp, Builtin, SchemaIR, TableIR, TypeIR,
    TypeRef,
};
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
        let mut cx = Checker { schema, diags, sources: Vec::new(), params: Vec::new(), failed: false };
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
        let mut cx = Checker { schema, diags, sources: Vec::new(), params: Vec::new(), failed: false };
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
}

struct SourceInfo<'a> {
    alias: String,
    table: &'a TableIR,
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

struct Output {
    name: String,
    typed: Typed,
    span: Span,
}

struct Checker<'a> {
    schema: &'a SchemaIR,
    diags: &'a mut Vec<Diagnostic>,
    sources: Vec<SourceInfo<'a>>,
    params: Vec<ParamInfo>,
    failed: bool,
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

/// The first column reference that is not covered by a `group by` expression
/// or an aggregate, if any.
fn uncovered(e: &QExpr, groups: &[QExpr]) -> Option<String> {
    if groups.contains(e) {
        return None;
    }
    match e {
        QExpr::Number { .. } | QExpr::Decimal { .. } | QExpr::String { .. } | QExpr::Bool { .. } | QExpr::Null
        | QExpr::Param { .. } | QExpr::Agg { .. } => None,
        QExpr::Column { source, column } => Some(format!("{source}.{column}")),
        QExpr::Binary { lhs, rhs, .. } => uncovered(lhs, groups).or_else(|| uncovered(rhs, groups)),
        QExpr::Not { expr } | QExpr::IsNull { expr, .. } => uncovered(expr, groups),
        QExpr::In { expr, list, .. } => {
            uncovered(expr, groups).or_else(|| list.iter().find_map(|i| uncovered(i, groups)))
        }
        QExpr::Like { expr, pattern, .. } => uncovered(expr, groups).or_else(|| uncovered(pattern, groups)),
        QExpr::Call { args, .. } => args.iter().find_map(|a| uncovered(a, groups)),
        QExpr::Case { whens, otherwise } => whens
            .iter()
            .find_map(|w| uncovered(&w.when, groups).or_else(|| uncovered(&w.then, groups)))
            .or_else(|| otherwise.as_ref().and_then(|o| uncovered(o, groups))),
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
    fn outputs(&mut self, items: &[SelectItem], ctx: Ctx) -> Vec<Output> {
        let mut outputs: Vec<Output> = Vec::new();
        for item in items {
            match item {
                SelectItem::Star(span) => {
                    let all: Vec<_> = (0..self.sources.len()).collect();
                    for i in all {
                        self.expand(i, *span, &mut outputs);
                    }
                }
                SelectItem::SourceStar(alias) => match self.sources.iter().position(|s| s.alias == alias.name) {
                    Some(i) => self.expand(i, alias.span, &mut outputs),
                    None => self.fail("QL205", format!("unknown table alias `{}`", alias.name), alias.span),
                },
                SelectItem::Expr { expr, alias } => {
                    let Some(typed) = self.expr(expr, ctx) else { continue };
                    let name = match (alias, expr) {
                        (Some(a), _) => a.name.clone(),
                        (None, Expr::Column { name, .. }) => name.name.clone(),
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
        for o in &outputs {
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

        // sources, each join's ON seeing the sources up to and including itself
        let mut sources_ir = Vec::new();
        self.add_source(&q.from, false)?;
        sources_ir.push(SourceIR { alias: self.sources[0].alias.clone(), table: q.from.table.name.clone(), join: None, on: None });
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
        let outputs = self.outputs(&q.select.items, Ctx::Select);

        let having = match &q.having {
            Some(h) => self.expr(h, Ctx::Having).inspect(|t| {
                if !is_bool(&t.t) {
                    self.fail("QL212", format!("`having` must be boolean, found {}", describe_t(&t.t)), h.span());
                }
            }),
            None => None,
        };

        let mut order: Vec<(Typed, bool, Span)> = Vec::new();
        for o in &q.order_by {
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

        let limit = self.limit_expr(&q.limit, "limit").flatten();
        let offset = self.limit_expr(&q.offset, "offset").flatten();

        // once aggregates or `group by` are involved, every column must be grouped or aggregated
        let grouped = !group_by.is_empty()
            || outputs.iter().any(|o| o.typed.agg)
            || having.as_ref().is_some_and(|h| h.agg)
            || order.iter().any(|o| o.0.agg);
        if grouped && !self.failed {
            let check = |cx: &mut Self, e: &QExpr, span: Span| {
                if let Some(col) = uncovered(e, &group_by) {
                    cx.fail("QL214", format!("`{col}` must appear in `group by` or be used inside an aggregate"), span);
                }
            };
            for o in &outputs { check(self, &o.typed.e, o.span); }
            if let (Some(h), Some(src)) = (&having, &q.having) { check(self, &h.e, src.span()); }
            for (t, _, span) in &order { check(self, &t.e, *span); }
        }

        self.warn_unused_params();

        Some(QueryIR {
            name: q.name.name.clone(),
            params: self.param_irs(),
            sources: sources_ir,
            filter,
            group_by,
            having: having.map(|h| h.e),
            distinct: q.select.distinct,
            select: outputs
                .into_iter()
                .filter_map(|o| match o.typed.t {
                    T::Known(ty) => Some(ColumnOut { name: o.name, expr: o.typed.e, ty, nullable: o.typed.nullable }),
                    T::Null => None,
                })
                .collect(),
            order_by: order.into_iter().map(|(t, desc, _)| OrderIR { expr: t.e, desc }).collect(),
            limit,
            offset,
        })
    }

    // ---- mutations -------------------------------------------------------- //

    fn mutation(&mut self, m: &Mutation) -> Option<MutationIR> {
        self.declare_params(&m.params);
        let Some(table) = self.schema.table(&m.table.table.name) else {
            self.fail("QL203", format!("unknown table `{}`", m.table.table.name), m.table.table.span);
            return None;
        };
        let alias = m.table.alias.as_ref().map_or_else(|| m.table.table.name.clone(), |a| a.name.clone());

        // An insert's values cannot read the row being created, so its `set`
        // expressions are checked with no table in scope; an update's can.
        let mut assignments = Vec::new();
        if m.kind == MutationKind::Insert {
            assignments = self.assignments(table, &m.assignments);
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
            let set: HashSet<&str> = m.assignments.iter().map(|a| a.column.name.as_str()).collect();
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
            .outputs(&m.returning, Ctx::Returning)
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
            filter,
            all_rows: m.all_rows,
            conflict,
            returning,
        })
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
                self.sources.push(SourceInfo { alias: "excluded".into(), table, nullable: false });
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
        let Some(table) = self.schema.table(&t.table.name) else {
            self.fail("QL203", format!("unknown table `{}`", t.table.name), t.table.span);
            return None;
        };
        let alias = t.alias.as_ref().map_or_else(|| t.table.name.clone(), |a| a.name.clone());
        if self.sources.iter().any(|s| s.alias == alias) {
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

    fn column(&mut self, qualifier: &Option<certo_sdl::Ident>, name: &certo_sdl::Ident) -> Option<Typed> {
        let found: Vec<(usize, &certo_sdl::ColumnIR)> = match qualifier {
            Some(q) => {
                let Some(i) = self.sources.iter().position(|s| s.alias == q.name) else {
                    self.fail("QL205", format!("unknown table alias `{}`", q.name), q.span);
                    return None;
                };
                self.sources[i].table.column(&name.name).map(|c| (i, c)).into_iter().collect()
            }
            None => self
                .sources
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.table.column(&name.name).map(|c| (i, c)))
                .collect(),
        };
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
            Expr::Call { func, args, star, distinct, span } => self.call(func, args, *star, *distinct, *span, ctx),
        }
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

    fn call(&mut self, func: &certo_sdl::Ident, args: &[Expr], star: bool, distinct: bool, span: Span, ctx: Ctx) -> Option<Typed> {
        let name = func.name.as_str();
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

fn narrowing_hint(col: &TypeIR, value: &TypeIR) -> &'static str {
    match (rank(col), rank(value)) {
        (Some(c), Some(v)) if v > c => ": that would narrow it; declare the parameter with the column's type",
        _ => "",
    }
}
