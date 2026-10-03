//! Lower a checked `QueryIR` to SQL.
//!
//! Parameters become numbered placeholders (PostgreSQL `$1`, SQLite `?1`, ...)
//! in order of first use, each cast to its declared type so the engine never
//! has to guess;
//! `param_order` says which declared parameter is which placeholder. Every
//! identifier is quoted, every binary expression parenthesised.
//!
//! Note: `/` on integers is integer division, as in SQL.

use crate::ast::{JoinKind, MutationKind};
use crate::ir::*;
use certo_sdl::BinaryOp;
use certo_sql::{quote_ident, quote_literal, render_type, Dialect};

/// SQL text plus the parameters, in placeholder order.
#[derive(Debug, Clone, PartialEq)]
pub struct Lowered {
    pub sql: String,
    /// `param_order[i]` is the declared parameter bound to `$(i + 1)`.
    pub param_order: Vec<String>,
}

/// Variations of the SQL for a particular consumer.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowerOptions {
    /// Select enum result columns as `text` (PostgreSQL). Drivers that do not know the
    /// enum type, such as Npgsql without a mapping, cannot read the column otherwise.
    pub enums_as_text: bool,
}

pub fn lower(dialect: Dialect, q: &QueryIR) -> Lowered { lower_with(dialect, q, LowerOptions::default()) }

pub fn lower_with(dialect: Dialect, q: &QueryIR, opts: LowerOptions) -> Lowered {
    let mut l = Lowerer { dialect, params: &q.params, order: Vec::new(), bare_columns: false, enums_as_text: opts.enums_as_text, cast_enums: false };
    let sql = l.query(q);
    Lowered { sql, param_order: l.order }
}

/// `INSERT` / `UPDATE` / `DELETE`, with `RETURNING` when the mutation has one.
pub fn lower_mutation(dialect: Dialect, m: &MutationIR) -> Lowered { lower_mutation_with(dialect, m, LowerOptions::default()) }

pub fn lower_mutation_with(dialect: Dialect, m: &MutationIR, opts: LowerOptions) -> Lowered {
    let mut l = Lowerer { dialect, params: &m.params, order: Vec::new(), bare_columns: false, enums_as_text: opts.enums_as_text, cast_enums: false };
    let sql = l.mutation(m);
    Lowered { sql, param_order: l.order }
}

struct Lowerer<'a> {
    dialect: Dialect,
    params: &'a [ParamIR],
    order: Vec<String>,
    /// SQLite's RETURNING cannot use table-qualified names.
    bare_columns: bool,
    enums_as_text: bool,
    /// Applies to the next column list only (the statement's own, not a subquery's).
    cast_enums: bool,
}

impl Lowerer<'_> {
    fn id(&self, s: &str) -> String { quote_ident(self.dialect, s) }

    fn mutation(&mut self, m: &MutationIR) -> String {
        let table = format!("{} AS {}", self.id(&m.table), self.id(&m.alias));
        let mut lines = self.with_lines(&m.ctes);
        match m.kind {
            MutationKind::Insert => {
                if let Some(src) = &m.source {
                    let cols: Vec<String> = m.insert_columns.iter().map(|c| self.id(c)).collect();
                    lines.push(format!("INSERT INTO {table} ({})", cols.join(", ")));
                    let prefix = self.with_lines(&src.ctes);
                    let mut sql = self.select_lines(&src.sources, &src.select, &src.filter, &src.group_by, &src.having, src.distinct, &src.unions, &src.order_by, &src.limit, &src.offset);
                    // SQLite cannot tell `ON CONFLICT` from a join's `ON` after a bare `INSERT ... SELECT`
                    if m.conflict.is_some() && self.dialect == Dialect::Sqlite && src.filter.is_none() {
                        let at = sql.iter().position(|l| l.starts_with("GROUP BY") || l.starts_with("HAVING") || l.starts_with("ORDER BY") || l.starts_with("LIMIT") || l.starts_with("OFFSET")).unwrap_or(sql.len());
                        sql.insert(at, "WHERE TRUE".to_string());
                    }
                    lines.extend(prefix);
                    lines.extend(sql);
                } else if !m.rows.is_empty() {
                    let cols: Vec<String> = m.insert_columns.iter().map(|c| self.id(c)).collect();
                    lines.push(format!("INSERT INTO {table} ({})", cols.join(", ")));
                    let rows: Vec<String> = m
                        .rows
                        .iter()
                        .map(|r| format!("({})", r.iter().map(|e| self.expr(e)).collect::<Vec<_>>().join(", ")))
                        .collect();
                    lines.push(format!("VALUES {}", rows.join(", ")));
                } else {
                    let cols: Vec<String> = m.assignments.iter().map(|a| self.id(&a.column)).collect();
                    let vals: Vec<String> = m.assignments.iter().map(|a| self.expr(&a.expr)).collect();
                    if cols.is_empty() {
                        lines.push(format!("INSERT INTO {table} DEFAULT VALUES"));
                    } else {
                        lines.push(format!("INSERT INTO {table} ({})", cols.join(", ")));
                        lines.push(format!("VALUES ({})", vals.join(", ")));
                    }
                }
                if let Some(c) = &m.conflict {
                    let target: Vec<String> = c.columns.iter().map(|x| self.id(x)).collect();
                    match &c.action {
                        ConflictActionIR::Nothing => {
                            lines.push(format!("ON CONFLICT ({}) DO NOTHING", target.join(", ")));
                        }
                        ConflictActionIR::Update { assignments } => {
                            let set = self.set_list(assignments);
                            lines.push(format!("ON CONFLICT ({}) DO UPDATE SET {set}", target.join(", ")));
                        }
                    }
                }
            }
            MutationKind::Update => {
                lines.push(format!("UPDATE {table}"));
                let set = self.set_list(&m.assignments);
                lines.push(format!("SET {set}"));
            }
            MutationKind::Delete => lines.push(format!("DELETE FROM {table}")),
        }
        if let Some(f) = &m.filter {
            lines.push(format!("WHERE {}", self.expr(f)));
        }
        if !m.returning.is_empty() {
            self.bare_columns = self.dialect == Dialect::Sqlite;
            let cast = self.enums_as_text;
            let cols: Vec<String> = m.returning.iter().map(|c| self.output(c, cast)).collect();
            self.bare_columns = false;
            lines.push(format!("RETURNING {}", cols.join(", ")));
        }
        lines.join("\n")
    }

    /// `"col" = expr, ...` (PostgreSQL forbids qualifying the column being set).
    fn set_list(&mut self, list: &[AssignIR]) -> String {
        let parts: Vec<String> =
            list.iter().map(|a| format!("{} = {}", self.id(&a.column), self.expr(&a.expr))).collect();
        parts.join(", ")
    }

    /// `expr AS "name"`, selecting an enum as text when asked.
    fn output(&mut self, c: &ColumnOut, cast: bool) -> String {
        let e = self.expr(&c.expr);
        let e = if cast && self.dialect == Dialect::Postgres && matches!(c.ty, certo_sdl::TypeIR::Enum(_)) {
            format!("({e})::text")
        } else {
            e
        };
        format!("{e} AS {}", self.id(&c.name))
    }

    fn query(&mut self, q: &QueryIR) -> String {
        self.cast_enums = self.enums_as_text;
        let mut lines = self.with_lines(&q.ctes);
        lines.extend(self.select_lines(&q.sources, &q.select, &q.filter, &q.group_by, &q.having, q.distinct, &q.unions, &q.order_by, &q.limit, &q.offset));
        lines.join("\n")
    }

    /// `WITH "a" AS (...), "b" AS (...)`, or nothing.
    fn with_lines(&mut self, ctes: &[CteIR]) -> Vec<String> {
        if ctes.is_empty() {
            return Vec::new();
        }
        let defs: Vec<String> = ctes.iter().map(|c| format!("{} AS ({})", self.id(&c.name), self.nested(&c.query))).collect();
        let keyword = if ctes.iter().any(|c| c.recursive) { "WITH RECURSIVE" } else { "WITH" };
        vec![format!("{keyword} {}", defs.join(", "))]
    }

    /// The clauses of a SELECT, one per line.
    #[allow(clippy::too_many_arguments)]
    fn select_lines(
        &mut self,
        sources: &[SourceIR],
        select: &[ColumnOut],
        filter: &Option<QExpr>,
        group_by: &[QExpr],
        having: &Option<QExpr>,
        distinct: bool,
        unions: &[UnionIR],
        order_by: &[OrderIR],
        limit: &Option<QExpr>,
        offset: &Option<QExpr>,
    ) -> Vec<String> {
        let cast = std::mem::take(&mut self.cast_enums);
        let mut lines = self.branch_lines(sources, select, select, filter, group_by, having, distinct, cast);
        for u in unions {
            let op = match (u.op, u.all) {
                (crate::ast::SetOp::Union, false) => "UNION",
                (crate::ast::SetOp::Union, true) => "UNION ALL",
                (crate::ast::SetOp::Intersect, _) => "INTERSECT",
                (crate::ast::SetOp::Except, _) => "EXCEPT",
            };
            lines.push(op.to_string());
            let b = &u.branch;
            // the combined column's type decides whether an enum is selected as text
            lines.extend(self.branch_lines(&b.sources, &b.select, select, &b.filter, &b.group_by, &b.having, b.distinct, cast));
        }
        self.tail_lines(&mut lines, order_by, limit, offset);
        lines
    }

    /// The clauses of one SELECT up to `HAVING`. `merged` gives each column's final type.
    #[allow(clippy::too_many_arguments)]
    fn branch_lines(
        &mut self,
        sources: &[SourceIR],
        select: &[ColumnOut],
        merged: &[ColumnOut],
        filter: &Option<QExpr>,
        group_by: &[QExpr],
        having: &Option<QExpr>,
        distinct: bool,
        cast: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let cols: Vec<String> = select
            .iter()
            .zip(merged)
            .map(|(c, m)| {
                let typed = ColumnOut { ty: m.ty.clone(), ..c.clone() };
                self.output(&typed, cast)
            })
            .collect();
        lines.push(format!("SELECT {}{}", if distinct { "DISTINCT " } else { "" }, cols.join(", ")));

        for (i, s) in sources.iter().enumerate() {
            let table = format!("{} AS {}", self.id(&s.table), self.id(&s.alias));
            if i == 0 {
                lines.push(format!("FROM {table}"));
            } else {
                let kind = if s.join == Some(JoinKind::Left) { "LEFT JOIN" } else { "INNER JOIN" };
                let on = s.on.as_ref().map(|e| self.expr(e)).unwrap_or_else(|| "TRUE".into());
                lines.push(format!("{kind} {table} ON {on}"));
            }
        }
        if let Some(f) = filter {
            lines.push(format!("WHERE {}", self.expr(f)));
        }
        if !group_by.is_empty() {
            let g: Vec<String> = group_by.iter().map(|e| self.expr(e)).collect();
            lines.push(format!("GROUP BY {}", g.join(", ")));
        }
        if let Some(h) = having {
            lines.push(format!("HAVING {}", self.expr(h)));
        }
        lines
    }

    fn tail_lines(&mut self, lines: &mut Vec<String>, order_by: &[OrderIR], limit: &Option<QExpr>, offset: &Option<QExpr>) {
        if !order_by.is_empty() {
            let o: Vec<String> = order_by
                .iter()
                .map(|o| format!("{}{}", self.expr(&o.expr), if o.desc { " DESC" } else { "" }))
                .collect();
            lines.push(format!("ORDER BY {}", o.join(", ")));
        }
        if let Some(l) = limit {
            lines.push(format!("LIMIT {}", self.expr(l)));
        } else if offset.is_some() && self.dialect == Dialect::Sqlite {
            lines.push("LIMIT -1".to_string()); // SQLite has no OFFSET without LIMIT
        }
        if let Some(o) = offset {
            lines.push(format!("OFFSET {}", self.expr(o)));
        }
    }

    /// A subquery on one line (its columns are qualified as usual, even under SQLite's bare RETURNING).
    fn nested(&mut self, q: &SubqueryIR) -> String {
        let bare = std::mem::replace(&mut self.bare_columns, false);
        let mut lines = self.with_lines(&q.ctes);
        lines.extend(self.select_lines(&q.sources, &q.select, &q.filter, &q.group_by, &q.having, q.distinct, &q.unions, &q.order_by, &q.limit, &q.offset));
        let sql = lines.join(" ");
        self.bare_columns = bare;
        sql
    }

    fn param(&mut self, name: &str) -> String {
        let idx = match self.order.iter().position(|p| p == name) {
            Some(i) => i + 1,
            None => {
                self.order.push(name.to_string());
                self.order.len()
            }
        };
        let ty = self.params.iter().find(|p| p.name == name).map(|p| render_type(self.dialect, &p.ty));
        match (self.dialect, ty) {
            (Dialect::Postgres, Some(t)) => format!("(${idx}::{t})"),
            (Dialect::Postgres, None) => format!("${idx}"),
            (Dialect::Sqlite, Some(t)) => format!("CAST(?{idx} AS {t})"),
            (Dialect::Sqlite, None) => format!("?{idx}"),
        }
    }

    fn expr(&mut self, e: &QExpr) -> String {
        match e {
            QExpr::Number { value } if *value < 0 => format!("({value})"),
            QExpr::Number { value } => value.to_string(),
            QExpr::Decimal { value } if value.starts_with('-') => format!("({value})"),
            QExpr::Decimal { value } => value.clone(),
            QExpr::String { value } => quote_literal(self.dialect, value),
            QExpr::Bool { value } => if *value { "TRUE" } else { "FALSE" }.to_string(),
            QExpr::Null => "NULL".to_string(),
            // the output column of a set operation (what `order by` names)
            QExpr::Column { source, column } if source.is_empty() => self.id(column),
            QExpr::Column { column, .. } if self.bare_columns => self.id(column),
            QExpr::Column { source, column } => format!("{}.{}", self.id(source), self.id(column)),
            QExpr::Param { name } => self.param(name),
            QExpr::Binary { op, lhs, rhs } => {
                let sql_op = match op {
                    BinaryOp::Eq => "=",
                    BinaryOp::Ne => "<>",
                    BinaryOp::Lt => "<",
                    BinaryOp::Le => "<=",
                    BinaryOp::Gt => ">",
                    BinaryOp::Ge => ">=",
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::And => "AND",
                    BinaryOp::Or => "OR",
                };
                format!("({} {sql_op} {})", self.expr(lhs), self.expr(rhs))
            }
            QExpr::Not { expr } => format!("(NOT {})", self.expr(expr)),
            QExpr::IsNull { expr, negated } => {
                format!("({} IS {}NULL)", self.expr(expr), if *negated { "NOT " } else { "" })
            }
            QExpr::In { expr, list, negated } => {
                let items: Vec<String> = list.iter().map(|i| self.expr(i)).collect();
                format!("({} {}IN ({}))", self.expr(expr), if *negated { "NOT " } else { "" }, items.join(", "))
            }
            QExpr::Like { expr, pattern, negated } => {
                format!("({} {}LIKE {})", self.expr(expr), if *negated { "NOT " } else { "" }, self.expr(pattern))
            }
            QExpr::Call { func, args } => match func.as_str() {
                "today" => "CURRENT_DATE".to_string(),
                "now" if self.dialect == Dialect::Sqlite => "CURRENT_TIMESTAMP".to_string(),
                "date_part" | "date_part_utc" => {
                    let (QExpr::String { value: part }, Some(x)) = (&args[0], args.get(1)) else { unreachable!("checked") };
                    let x = self.expr(x);
                    match self.dialect {
                        Dialect::Postgres if func == "date_part_utc" => format!("CAST(EXTRACT({} FROM ({x} AT TIME ZONE 'UTC')) AS integer)", part.to_uppercase()),
                        Dialect::Postgres => format!("CAST(EXTRACT({} FROM {x}) AS integer)", part.to_uppercase()),
                        Dialect::Sqlite => {
                            let f = match part.as_str() { "year" => "%Y", "month" => "%m", "day" => "%d", "hour" => "%H", _ => "%M" };
                            format!("CAST(strftime('{f}', {x}) AS INTEGER)")
                        }
                    }
                }
                "left" | "right" => {
                    let (s, QExpr::Number { value: n }) = (self.expr(&args[0]), &args[1]) else { unreachable!("checked") };
                    match (func.as_str(), self.dialect) {
                        ("left", _) => format!("substr({s}, 1, {n})"),
                        ("right", Dialect::Postgres) => format!("right({s}, {n})"),
                        _ => format!("substr({s}, -{n})"),
                    }
                }
                "starts_with" => {
                    // the same on both: the prefix is the start of the text (an empty prefix always is)
                    let (s, p1, p2) = (self.expr(&args[0]), self.expr(&args[1]), self.expr(&args[1]));
                    format!("(substr({s}, 1, length({p1})) = {p2})")
                }
                "add_days" => {
                    let (d, n) = (self.expr(&args[0]), self.expr(&args[1]));
                    match self.dialect {
                        Dialect::Postgres => format!("({d} + CAST({n} AS integer))"),
                        Dialect::Sqlite => format!("date({d}, CAST({n} AS TEXT) || ' days')"),
                    }
                }
                "days_between" => {
                    let (from, to) = (self.expr(&args[0]), self.expr(&args[1]));
                    match self.dialect {
                        Dialect::Postgres => format!("({to} - {from})"),
                        Dialect::Sqlite => format!("CAST(julianday({to}) - julianday({from}) AS INTEGER)"),
                    }
                }
                "position" => {
                    // PostgreSQL strpos(haystack, needle) and SQLite instr(haystack, needle) agree
                    let a: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                    let f = if self.dialect == Dialect::Postgres { "strpos" } else { "instr" };
                    format!("{f}({})", a.join(", "))
                }
                other => {
                    let name = match (self.dialect, other) {
                        (Dialect::Postgres, "trim") => "btrim",
                        _ => other,
                    };
                    let a: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                    format!("{name}({})", a.join(", "))
                }
            },
            QExpr::Agg { func, arg, distinct, filter, separator, order_by } => {
                let mut sql = match (arg, separator) {
                    (None, _) => format!("{func}(*)"),
                    // string_agg is group_concat in SQLite; the text, the separator, then the order (the order the
                    // placeholders appear in)
                    (Some(a), Some(sep)) => {
                        let name = if self.dialect == Dialect::Sqlite { "group_concat" } else { "string_agg" };
                        let text = self.expr(a);
                        let order = if order_by.is_empty() {
                            String::new()
                        } else {
                            let o: Vec<String> = order_by.iter().map(|o| format!("{}{}", self.expr(&o.expr), if o.desc { " DESC" } else { "" })).collect();
                            format!(" ORDER BY {}", o.join(", "))
                        };
                        format!("{name}({text}, {}{order})", quote_literal(self.dialect, sep))
                    }
                    (Some(a), None) => format!("{func}({}{})", if *distinct { "DISTINCT " } else { "" }, self.expr(a)),
                };
                if let Some(f) = filter {
                    sql.push_str(&format!(" FILTER (WHERE {})", self.expr(f)));
                }
                sql
            }
            QExpr::Concat { parts } => {
                let p: Vec<String> = parts.iter().map(|e| self.expr(e)).collect();
                format!("({})", p.join(" || "))
            }
            QExpr::Window { call, partition_by, order_by, frame } => {
                let call = self.expr(call);
                let mut spec = Vec::new();
                if !partition_by.is_empty() {
                    let p: Vec<String> = partition_by.iter().map(|e| self.expr(e)).collect();
                    spec.push(format!("PARTITION BY {}", p.join(", ")));
                }
                if !order_by.is_empty() {
                    let o: Vec<String> = order_by
                        .iter()
                        .map(|o| format!("{}{}", self.expr(&o.expr), if o.desc { " DESC" } else { "" }))
                        .collect();
                    spec.push(format!("ORDER BY {}", o.join(", ")));
                }
                if let Some(f) = frame {
                    let units = match f.units {
                        crate::ast::FrameUnits::Rows => "ROWS",
                        crate::ast::FrameUnits::Range => "RANGE",
                        crate::ast::FrameUnits::Groups => "GROUPS",
                    };
                    let bound = |l: &mut Self, b: &FrameBoundIR| match b {
                        FrameBoundIR::UnboundedPreceding => "UNBOUNDED PRECEDING".to_string(),
                        FrameBoundIR::Preceding { offset } => format!("{} PRECEDING", l.expr(offset)),
                        FrameBoundIR::CurrentRow => "CURRENT ROW".to_string(),
                        FrameBoundIR::Following { offset } => format!("{} FOLLOWING", l.expr(offset)),
                        FrameBoundIR::UnboundedFollowing => "UNBOUNDED FOLLOWING".to_string(),
                    };
                    let (a, b) = (bound(self, &f.start), bound(self, &f.end));
                    spec.push(format!("{units} BETWEEN {a} AND {b}"));
                }
                format!("{call} OVER ({})", spec.join(" "))
            }
            QExpr::Exists { query } => format!("EXISTS ({})", self.nested(query)),
            QExpr::InQuery { expr, query, negated } => {
                format!("({} {}IN ({}))", self.expr(expr), if *negated { "NOT " } else { "" }, self.nested(query))
            }
            QExpr::Scalar { query } => format!("({})", self.nested(query)),
            QExpr::Case { whens, otherwise } => {
                let mut s = String::from("CASE");
                for w in whens {
                    s.push_str(&format!(" WHEN {} THEN {}", self.expr(&w.when), self.expr(&w.then)));
                }
                if let Some(o) = otherwise {
                    s.push_str(&format!(" ELSE {}", self.expr(o)));
                }
                s.push_str(" END");
                s
            }
        }
    }
}
