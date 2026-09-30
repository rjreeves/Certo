//! QL: a typed query language, checked against a `SchemaIR` and lowered to SQL.
//!
//! ```text
//! query recent_orders(min_total: decimal, since: timestamp null) {
//!     from orders o
//!     join customers c on o.customer_id == c.id
//!     where o.total >= :min_total and o.created > :since
//!     select o.id, c.name as customer, o.total
//!     order by o.total desc
//!     limit 50
//! }
//!
//! insert add_customer(name: text) { into customers set name = :name returning id }
//! ```

pub mod ast;
pub mod check;
pub mod ir;
pub mod lower;
pub mod parser;

pub use ast::*;
pub use check::{check, check_mutations};
pub use ir::*;
pub use lower::{lower, lower_mutation, Lowered};
pub use parser::parse;

use certo_diagnostics::{Diagnostic, Severity};
use certo_sdl::SchemaIR;
use certo_sql::Dialect;

/// A checked query with its SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledQuery {
    pub ir: QueryIR,
    pub sql: String,
    /// `param_order[i]` is the declared parameter bound to placeholder `$(i + 1)`.
    pub param_order: Vec<String>,
}

/// A checked `insert` / `update` / `delete` with its SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMutation {
    pub ir: MutationIR,
    pub sql: String,
    pub param_order: Vec<String>,
}

/// One compiled statement of a QL file.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Query(CompiledQuery),
    Mutation(CompiledMutation),
}

impl Statement {
    pub fn name(&self) -> &str {
        match self {
            Statement::Query(q) => &q.ir.name,
            Statement::Mutation(m) => &m.ir.name,
        }
    }

    pub fn sql(&self) -> &str {
        match self {
            Statement::Query(q) => &q.sql,
            Statement::Mutation(m) => &m.sql,
        }
    }

    pub fn param_order(&self) -> &[String] {
        match self {
            Statement::Query(q) => &q.param_order,
            Statement::Mutation(m) => &m.param_order,
        }
    }

    pub fn params(&self) -> &[ParamIR] {
        match self {
            Statement::Query(q) => &q.ir.params,
            Statement::Mutation(m) => &m.ir.params,
        }
    }

    /// The typed result columns; empty for a mutation without `returning`.
    pub fn columns(&self) -> &[ColumnOut] {
        match self {
            Statement::Query(q) => &q.ir.select,
            Statement::Mutation(m) => &m.ir.returning,
        }
    }

    pub fn as_query(&self) -> Option<&CompiledQuery> {
        match self {
            Statement::Query(q) => Some(q),
            Statement::Mutation(_) => None,
        }
    }

    pub fn as_mutation(&self) -> Option<&CompiledMutation> {
        match self {
            Statement::Mutation(m) => Some(m),
            Statement::Query(_) => None,
        }
    }
}

/// Parse, check and lower every statement in `src` against `schema`. The result
/// is `Some` only when there are no error diagnostics (warnings may accompany
/// it). Queries come first, then mutations, each in source order.
pub fn compile(schema: &SchemaIR, src: &str, dialect: Dialect) -> (Option<Vec<Statement>>, Vec<Diagnostic>) {
    let (file, mut diags) = parse(src);
    if has_errors(&diags) {
        return (None, diags);
    }
    let queries = check(schema, &file, &mut diags);
    let mutations = check_mutations(schema, &file, &mut diags);
    if has_errors(&diags) {
        return (None, diags);
    }
    let mut out: Vec<Statement> = queries
        .into_iter()
        .map(|ir| {
            let Lowered { sql, param_order } = lower(dialect, &ir);
            Statement::Query(CompiledQuery { ir, sql, param_order })
        })
        .collect();
    out.extend(mutations.into_iter().map(|ir| {
        let Lowered { sql, param_order } = lower_mutation(dialect, &ir);
        Statement::Mutation(CompiledMutation { ir, sql, param_order })
    }));
    (Some(out), diags)
}

/// The host-facing form of compiled statements: one object per statement with
/// its `kind` (`query`, `insert`, `update`, `delete`), parameters, typed result
/// columns (a mutation's are its `returning` list), SQL and placeholder order
/// (plus the full checked `ir`). Shared by the CLI and the C ABI.
pub fn to_json(statements: &[Statement]) -> serde_json::Value {
    use serde_json::json;
    statements
        .iter()
        .map(|s| {
            let (kind, ir) = match s {
                Statement::Query(q) => ("query", json!(q.ir)),
                Statement::Mutation(m) => (
                    match m.ir.kind {
                        MutationKind::Insert => "insert",
                        MutationKind::Update => "update",
                        MutationKind::Delete => "delete",
                    },
                    json!(m.ir),
                ),
            };
            json!({
                "kind": kind,
                "name": s.name(),
                "params": s.params(),
                "columns": s.columns().iter()
                    .map(|c| json!({ "name": c.name, "type": c.ty, "nullable": c.nullable }))
                    .collect::<Vec<_>>(),
                "sql": s.sql(),
                "param_order": s.param_order(),
                "ir": ir,
            })
        })
        .collect()
}

fn has_errors(d: &[Diagnostic]) -> bool { d.iter().any(|x| x.severity == Severity::Error) }

#[cfg(test)]
mod parser_tests;
#[cfg(test)]
mod check_tests;
