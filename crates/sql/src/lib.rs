//! Lower an engine-agnostic `MigrationPlan` to SQL.
//!
//! One module per dialect. Each `lower` returns one statement per element
//! (each ending in `;`), in plan order, or an error naming the op it cannot
//! express so the user can write that step by hand.

mod mysql;
mod postgres;
mod sqlite;

use certo_mdl::{MigrationPlan, Op};
use certo_sdl::SchemaIR;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
    Mysql,
}

impl Dialect {
    pub fn from_name(s: &str) -> Option<Dialect> {
        match s {
            "postgres" | "postgresql" | "pg" => Some(Dialect::Postgres),
            "sqlite" | "sqlite3" => Some(Dialect::Sqlite),
            "mysql" => Some(Dialect::Mysql),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LowerError {
    /// Human summary of the op that could not be lowered (`Op::describe`).
    pub op: String,
    pub reason: String,
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot lower `{}`: {}", self.op, self.reason)
    }
}

impl std::error::Error for LowerError {}

pub(crate) fn unsupported(op: &Op, reason: impl Into<String>) -> LowerError {
    LowerError { op: op.describe(), reason: reason.into() }
}

/// The SQL spelling of a schema type (`character varying(20)`, `"Role"`, ...).
pub fn render_type(dialect: Dialect, t: &certo_sdl::TypeIR) -> String {
    match dialect {
        Dialect::Postgres => postgres::ty(t),
        Dialect::Sqlite => sqlite::ty(t),
        Dialect::Mysql => mysql::ty(t, None),
    }
}

/// A quoted identifier.
pub fn quote_ident(dialect: Dialect, s: &str) -> String {
    match dialect {
        Dialect::Postgres => postgres::q(s),
        Dialect::Sqlite => sqlite::q(s),
        Dialect::Mysql => mysql::q(s),
    }
}

/// A quoted string literal.
pub fn quote_literal(dialect: Dialect, s: &str) -> String {
    match dialect {
        Dialect::Postgres => postgres::lit(s),
        Dialect::Sqlite => sqlite::lit(s),
        Dialect::Mysql => mysql::lit(s),
    }
}

/// SQL text for one IR expression in `dialect` (how defaults and CHECK bodies
/// are written). Used to compare against what a live database reports.
pub fn render_expr(dialect: Dialect, e: &certo_sdl::ExprIR) -> String {
    match dialect {
        Dialect::Postgres => postgres::expr(e),
        Dialect::Sqlite => sqlite::expr(e).unwrap_or_else(|why| format!("/* {why} */")),
        Dialect::Mysql => mysql::expr(e).unwrap_or_else(|why| format!("/* {why} */")),
    }
}

/// The schemas a plan was made between. SQLite needs them: it rebuilds tables
/// (whose final shape comes from `new`), and writes enums as CHECK constraints.
/// MySQL needs them too: an enum is part of each column that uses it, and changing a column restates it in full.
/// PostgreSQL lowers from the plan alone and ignores them.
#[derive(Debug, Clone, Copy)]
pub struct Schemas<'a> {
    pub old: &'a SchemaIR,
    pub new: &'a SchemaIR,
}

fn needs_schemas(dialect: Dialect) -> LowerError {
    LowerError {
        op: format!("(the whole plan, for {dialect:?})"),
        reason: "SQLite and MySQL lowering need the schemas the plan was made between (use the `_with` functions)".into(),
    }
}

/// Lower every op in `plan`, flat and in plan order. Fails on the first op
/// the dialect cannot express. Prefer `lower_batches` when executing: it
/// separates the statements that cannot run inside a transaction. SQLite
/// needs schemas, so it is only available through [`lower_with`].
pub fn lower(plan: &MigrationPlan, dialect: Dialect) -> Result<Vec<String>, LowerError> {
    let mut out = Vec::new();
    for op in &plan.ops {
        lower_op(op, dialect, &mut out)?;
    }
    Ok(out)
}

/// Like [`lower`], with the schemas the plan came from (required for SQLite).
pub fn lower_with(plan: &MigrationPlan, dialect: Dialect, schemas: Schemas) -> Result<Vec<String>, LowerError> {
    match dialect {
        Dialect::Postgres => lower(plan, dialect),
        Dialect::Sqlite | Dialect::Mysql => Ok(lower_batches_with(plan, dialect, schemas)?.into_iter().flat_map(|b| b.statements).collect()),
    }
}

fn lower_op(op: &Op, dialect: Dialect, out: &mut Vec<String>) -> Result<(), LowerError> {
    match dialect {
        Dialect::Postgres => postgres::lower_op(op, out),
        Dialect::Sqlite | Dialect::Mysql => Err(needs_schemas(dialect)),
    }
}

/// A group of statements to run together.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// Run all statements in one transaction (`false`: each on its own).
    pub transactional: bool,
    pub statements: Vec<String>,
}

/// Lower a plan into execution batches.
///
/// PostgreSQL cannot use a new enum value in the transaction that added it,
/// so `ADD VALUE` statements (purely additive, safe to run early) form a
/// leading non-transactional batch; everything else is one atomic batch.
/// (SQLite: see [`lower_batches_with`].)
pub fn lower_batches(plan: &MigrationPlan, dialect: Dialect) -> Result<Vec<Batch>, LowerError> {
    if dialect != Dialect::Postgres {
        return Err(needs_schemas(dialect));
    }
    let (mut early, mut main) = (Vec::new(), Vec::new());
    for op in &plan.ops {
        let target = match (dialect, op) {
            (Dialect::Postgres, Op::AddEnumVariant { .. }) => &mut early,
            _ => &mut main,
        };
        lower_op(op, dialect, target)?;
    }
    let mut batches = Vec::new();
    if !early.is_empty() {
        batches.push(Batch { transactional: false, statements: early });
    }
    if !main.is_empty() {
        batches.push(Batch { transactional: true, statements: main });
    }
    Ok(batches)
}

/// Like [`lower_batches`], with the schemas the plan came from. SQLite needs
/// them; when a table has to be rebuilt the result is three batches:
/// `PRAGMA foreign_keys = OFF`, the transactional body, `PRAGMA foreign_keys = ON`.
pub fn lower_batches_with(plan: &MigrationPlan, dialect: Dialect, schemas: Schemas) -> Result<Vec<Batch>, LowerError> {
    match dialect {
        Dialect::Postgres => lower_batches(plan, dialect),
        Dialect::Sqlite => sqlite::lower_batches(plan, &schemas),
        Dialect::Mysql => mysql::lower_batches(plan, &schemas),
    }
}

/// A runnable script: non-transactional batches as-is, transactional ones
/// wrapped in `BEGIN;` / `COMMIT;`.
pub fn render(plan: &MigrationPlan, dialect: Dialect) -> Result<String, LowerError> {
    Ok(script(lower_batches(plan, dialect)?))
}

/// Like [`render`], with the schemas the plan came from (required for SQLite).
pub fn render_with(plan: &MigrationPlan, dialect: Dialect, schemas: Schemas) -> Result<String, LowerError> {
    Ok(script(lower_batches_with(plan, dialect, schemas)?))
}

fn script(batches: Vec<Batch>) -> String {
    let mut lines = Vec::new();
    for b in batches {
        if b.transactional { lines.push("BEGIN;".to_string()); }
        lines.extend(b.statements);
        if b.transactional { lines.push("COMMIT;".to_string()); }
    }
    lines.join("
")
}

#[cfg(test)]
mod tests;
