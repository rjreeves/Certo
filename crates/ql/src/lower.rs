//! Lower a checked `QueryIR` to SQL.
//!
//! Parameters become numbered placeholders (`$1`, `$2`, ...) in order of first
//! use, each cast to its declared type so PostgreSQL never has to guess;
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

pub fn lower(dialect: Dialect, q: &QueryIR) -> Lowered {
    let mut l = Lowerer { dialect, params: &q.params, order: Vec::new() };
    let sql = l.query(q);
    Lowered { sql, param_order: l.order }
}

/// `INSERT` / `UPDATE` / `DELETE`, with `RETURNING` when the mutation has one.
pub fn lower_mutation(dialect: Dialect, m: &MutationIR) -> Lowered {
    let mut l = Lowerer { dialect, params: &m.params, order: Vec::new() };
    let sql = l.mutation(m);
    Lowered { sql, param_order: l.order }
}

struct Lowerer<'a> {
    dialect: Dialect,
    params: &'a [ParamIR],
    order: Vec<String>,
}

impl Lowerer<'_> {
    fn id(&self, s: &str) -> String { quote_ident(self.dialect, s) }

    fn mutation(&mut self, m: &MutationIR) -> String {
        let table = format!("{} AS {}", self.id(&m.table), self.id(&m.alias));
        let mut lines = Vec::new();
        match m.kind {
            MutationKind::Insert => {
                let cols: Vec<String> = m.assignments.iter().map(|a| self.id(&a.column)).collect();
                let vals: Vec<String> = m.assignments.iter().map(|a| self.expr(&a.expr)).collect();
                if cols.is_empty() {
                    lines.push(format!("INSERT INTO {table} DEFAULT VALUES"));
                } else {
                    lines.push(format!("INSERT INTO {table} ({})", cols.join(", ")));
                    lines.push(format!("VALUES ({})", vals.join(", ")));
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
            let cols: Vec<String> =
                m.returning.iter().map(|c| format!("{} AS {}", self.expr(&c.expr), self.id(&c.name))).collect();
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

    fn query(&mut self, q: &QueryIR) -> String {
        let mut lines = Vec::new();

        let cols: Vec<String> =
            q.select.iter().map(|c| format!("{} AS {}", self.expr(&c.expr), self.id(&c.name))).collect();
        lines.push(format!("SELECT {}{}", if q.distinct { "DISTINCT " } else { "" }, cols.join(", ")));

        for (i, s) in q.sources.iter().enumerate() {
            let table = format!("{} AS {}", self.id(&s.table), self.id(&s.alias));
            if i == 0 {
                lines.push(format!("FROM {table}"));
            } else {
                let kind = if s.join == Some(JoinKind::Left) { "LEFT JOIN" } else { "INNER JOIN" };
                let on = s.on.as_ref().map(|e| self.expr(e)).unwrap_or_else(|| "TRUE".into());
                lines.push(format!("{kind} {table} ON {on}"));
            }
        }
        if let Some(f) = &q.filter {
            lines.push(format!("WHERE {}", self.expr(f)));
        }
        if !q.group_by.is_empty() {
            let g: Vec<String> = q.group_by.iter().map(|e| self.expr(e)).collect();
            lines.push(format!("GROUP BY {}", g.join(", ")));
        }
        if let Some(h) = &q.having {
            lines.push(format!("HAVING {}", self.expr(h)));
        }
        if !q.order_by.is_empty() {
            let o: Vec<String> = q
                .order_by
                .iter()
                .map(|o| format!("{}{}", self.expr(&o.expr), if o.desc { " DESC" } else { "" }))
                .collect();
            lines.push(format!("ORDER BY {}", o.join(", ")));
        }
        if let Some(l) = &q.limit {
            lines.push(format!("LIMIT {}", self.expr(l)));
        }
        if let Some(o) = &q.offset {
            lines.push(format!("OFFSET {}", self.expr(o)));
        }
        lines.join("\n")
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
        match ty {
            Some(t) => format!("(${idx}::{t})"),
            None => format!("${idx}"),
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
                other => {
                    let name = if other == "trim" { "btrim" } else { other };
                    let a: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                    format!("{name}({})", a.join(", "))
                }
            },
            QExpr::Agg { func, arg, distinct } => match arg {
                None => format!("{func}(*)"),
                Some(a) => format!("{func}({}{})", if *distinct { "DISTINCT " } else { "" }, self.expr(a)),
            },
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
