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
pub mod csharp;
pub mod rust;
pub mod fragments;
pub mod views;
pub mod ir;
pub mod lower;
pub mod parser;

pub use ast::*;
pub use check::{check, check_mutations};
pub use csharp::{generate_csharp, CSharpOptions};
pub use rust::{generate_rust, RustOptions};
pub use ir::*;
pub use views::ViewOut;
pub use lower::{lower, lower_mutation, lower_mutation_with, lower_with, LowerOptions, Lowered};
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
#[allow(clippy::large_enum_variant)] // a few per file, and boxing would change the public shape
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
    let (statements, _, diags) = compile_with_fragments(schema, src, dialect);
    (statements, diags)
}

/// `compile`, and also what the file's fragments look like (name, parameters, columns), which are not statements.
pub fn compile_with_fragments(schema: &SchemaIR, src: &str, dialect: Dialect) -> (Option<Vec<Statement>>, Vec<FragmentInfo>, Vec<Diagnostic>) {
    let c = compile_full(schema, src, dialect);
    (c.statements, c.fragments, c.diagnostics)
}

/// Everything a compile produces: the statements, the file's fragments and its persisted views.
pub struct Compiled {
    /// `None` if there are errors.
    pub statements: Option<Vec<Statement>>,
    pub fragments: Vec<FragmentInfo>,
    /// The file's `view`s in the order they must be created (empty on errors).
    pub views: Vec<ViewOut>,
    pub diagnostics: Vec<Diagnostic>,
}

/// `compile`, with the file's fragments and views too. A view is read as a table by every statement in the file.
pub fn compile_full(schema: &SchemaIR, src: &str, dialect: Dialect) -> Compiled {
    let fail = |diagnostics| Compiled { statements: None, fragments: Vec::new(), views: Vec::new(), diagnostics };
    let (mut file, mut diags) = parse(src);
    if has_errors(&diags) {
        return fail(diags);
    }
    // views declared in the schema (SDL) are read like tables
    let with_sdl_views = schema.with_views_as_tables();
    let schema = &with_sdl_views;
    // fragments are checked once, then become `with` queries of the statements and views that use them
    let infos = fragments::expand(schema, &mut file, &mut diags);
    if has_errors(&diags) {
        return fail(diags);
    }
    // views are checked in dependency order; the schema the statements see has them as (read-only) tables
    let (schema, view_outs) = views::resolve(schema, &file, dialect, &mut diags);
    if has_errors(&diags) {
        return fail(diags);
    }
    let schema = &schema;
    let queries = check(schema, &file, &mut diags);
    let mutations = check_mutations(schema, &file, &mut diags);
    if has_errors(&diags) {
        return fail(diags);
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
    Compiled { statements: Some(out), fragments: infos, views: view_outs, diagnostics: diags }
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
