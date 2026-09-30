//! Lower an engine-agnostic `MigrationPlan` to SQL.
//!
//! One module per dialect. Each `lower` returns one statement per element
//! (each ending in `;`), in plan order, or an error naming the op it cannot
//! express so the user can write that step by hand.

mod postgres;

use certo_mdl::{MigrationPlan, Op};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
}

impl Dialect {
    pub fn from_name(s: &str) -> Option<Dialect> {
        match s {
            "postgres" | "postgresql" | "pg" => Some(Dialect::Postgres),
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
    }
}

/// A quoted identifier.
pub fn quote_ident(dialect: Dialect, s: &str) -> String {
    match dialect {
        Dialect::Postgres => postgres::q(s),
    }
}

/// A quoted string literal.
pub fn quote_literal(dialect: Dialect, s: &str) -> String {
    match dialect {
        Dialect::Postgres => postgres::lit(s),
    }
}

/// SQL text for one IR expression in `dialect` (how defaults and CHECK bodies
/// are written). Used to compare against what a live database reports.
pub fn render_expr(dialect: Dialect, e: &certo_sdl::ExprIR) -> String {
    match dialect {
        Dialect::Postgres => postgres::expr(e),
    }
}

/// Lower every op in `plan`, flat and in plan order. Fails on the first op
/// the dialect cannot express. Prefer `lower_batches` when executing: it
/// separates the statements that cannot run inside a transaction.
pub fn lower(plan: &MigrationPlan, dialect: Dialect) -> Result<Vec<String>, LowerError> {
    let mut out = Vec::new();
    for op in &plan.ops {
        lower_op(op, dialect, &mut out)?;
    }
    Ok(out)
}

fn lower_op(op: &Op, dialect: Dialect, out: &mut Vec<String>) -> Result<(), LowerError> {
    match dialect {
        Dialect::Postgres => postgres::lower_op(op, out),
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
pub fn lower_batches(plan: &MigrationPlan, dialect: Dialect) -> Result<Vec<Batch>, LowerError> {
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

/// A runnable script: non-transactional batches as-is, transactional ones
/// wrapped in `BEGIN;` / `COMMIT;`.
pub fn render(plan: &MigrationPlan, dialect: Dialect) -> Result<String, LowerError> {
    let mut lines = Vec::new();
    for b in lower_batches(plan, dialect)? {
        if b.transactional { lines.push("BEGIN;".to_string()); }
        lines.extend(b.statements);
        if b.transactional { lines.push("COMMIT;".to_string()); }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests;
