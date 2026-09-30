//! The checked form of a query: every name resolved, every output column
//! typed, ready to lower to SQL or hand to a host as a typed contract.

use crate::ast::{JoinKind, MutationKind};
use certo_sdl::{BinaryOp, TypeIR};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryIR {
    pub name: String,
    pub params: Vec<ParamIR>,
    /// The `from` table first, then each join in order.
    pub sources: Vec<SourceIR>,
    pub filter: Option<QExpr>,
    pub group_by: Vec<QExpr>,
    pub having: Option<QExpr>,
    pub distinct: bool,
    /// The result columns, in order: this is the typed contract for the host.
    pub select: Vec<ColumnOut>,
    pub order_by: Vec<OrderIR>,
    pub limit: Option<QExpr>,
    pub offset: Option<QExpr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamIR {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeIR,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceIR {
    pub alias: String,
    pub table: String,
    /// `None` for the `from` table.
    pub join: Option<JoinKind>,
    pub on: Option<QExpr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnOut {
    pub name: String,
    pub expr: QExpr,
    #[serde(rename = "type")]
    pub ty: TypeIR,
    /// Can this column be NULL in a result row? Accounts for left joins,
    /// nullable columns and parameters, and aggregates over possibly empty sets.
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIR {
    pub expr: QExpr,
    pub desc: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct When {
    pub when: QExpr,
    pub then: QExpr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QExpr {
    Number { value: i64 },
    /// Decimal text as written, sign included; never rounded.
    Decimal { value: String },
    /// A string literal. Compared with an enum column it has been checked
    /// against the enum's values.
    String { value: String },
    Bool { value: bool },
    Null,
    Column { source: String, column: String },
    Param { name: String },
    Binary { op: BinaryOp, lhs: Box<QExpr>, rhs: Box<QExpr> },
    Not { expr: Box<QExpr> },
    IsNull { expr: Box<QExpr>, negated: bool },
    In { expr: Box<QExpr>, list: Vec<QExpr>, negated: bool },
    Like { expr: Box<QExpr>, pattern: Box<QExpr>, negated: bool },
    /// A scalar function (`lower`, `coalesce`, ...).
    Call { func: String, args: Vec<QExpr> },
    /// An aggregate; `arg: None` is `count(*)`.
    Agg { func: String, arg: Option<Box<QExpr>>, distinct: bool },
    Case { whens: Vec<When>, otherwise: Option<Box<QExpr>> },
    /// `exists (subquery)`
    Exists { query: Box<SubqueryIR> },
    /// `expr in (subquery)`: the subquery has one column.
    InQuery { expr: Box<QExpr>, query: Box<SubqueryIR>, negated: bool },
    /// `(subquery)` as a value: one column, at most one row (NULL if none).
    Scalar { query: Box<SubqueryIR> },
}

/// A query nested in an expression. It may refer to the tables of the queries
/// around it (correlation); `select` columns are named but nameless in SQL use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubqueryIR {
    pub sources: Vec<SourceIR>,
    pub filter: Option<QExpr>,
    pub group_by: Vec<QExpr>,
    pub having: Option<QExpr>,
    pub distinct: bool,
    pub select: Vec<ColumnOut>,
    pub order_by: Vec<OrderIR>,
    pub limit: Option<QExpr>,
    pub offset: Option<QExpr>,
}

/// A checked `insert` / `update` / `delete`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationIR {
    pub name: String,
    pub kind: MutationKind,
    pub params: Vec<ParamIR>,
    pub table: String,
    pub alias: String,
    /// `set` assignments (insert and update), in the order written.
    pub assignments: Vec<AssignIR>,
    /// Insert, tabular form: the columns, and the `values` rows or the `select` that feeds them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insert_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<QExpr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Box<SubqueryIR>>,
    /// The `where` condition (update and delete); `None` with `all_rows` when written `all rows`.
    pub filter: Option<QExpr>,
    pub all_rows: bool,
    pub conflict: Option<ConflictIR>,
    /// Typed columns of the `returning` clause: the result contract (empty if none,
    /// in which case the host gets a row count).
    pub returning: Vec<ColumnOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssignIR {
    pub column: String,
    pub expr: QExpr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictIR {
    pub columns: Vec<String>,
    pub action: ConflictActionIR,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConflictActionIR {
    Nothing,
    Update { assignments: Vec<AssignIR> },
}
