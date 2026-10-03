//! Syntactic AST for QL. No name resolution or typing happens here.

use certo_ast::span::Span;
use certo_sdl::{BinaryOp, Ident, TypeRef};

#[derive(Debug, Clone, PartialEq)]
pub struct QlFile {
    pub queries: Vec<Query>,
    pub mutations: Vec<Mutation>,
    /// `fragment name() { ... }`: named, parameter-free queries used as tables (see `fragments`).
    pub fragments: Vec<Fragment>,
}

/// `fragment name() { from ... select ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub name: Ident,
    pub query: Query,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// Empty (and `params` empty) for a subquery.
    pub name: Ident,
    pub params: Vec<ParamDecl>,
    pub from: TableRef,
    pub joins: Vec<Join>,
    pub filter: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub select: Select,
    pub order_by: Vec<OrderItem>,
    pub limit: Option<Expr>,
    pub offset: Option<Expr>,
    /// `union` / `intersect` / `except` branches after this one's `select`. `order by`,
    /// `limit` and `offset` above then apply to the whole combination.
    pub compound: Vec<SetBranch>,
    /// `with name as (...)`: named queries the body (and its subqueries) may use as tables.
    pub ctes: Vec<Cte>,
    pub span: Span,
}

/// `name as (from ... select ...)`
#[derive(Debug, Clone, PartialEq)]
pub struct Cte {
    /// Written `with recursive`: the query may read itself.
    pub recursive: bool,
    pub name: Ident,
    pub query: Box<Query>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOp {
    Union,
    Intersect,
    Except,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetBranch {
    pub op: SetOp,
    /// `union all` keeps duplicates.
    pub all: bool,
    /// Only `from` ... `select` (no `order by` / `limit` / `offset` of its own).
    pub query: Box<Query>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: Ident,
    pub ty: TypeRef,
    /// `name: type null`
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub table: Ident,
    /// Defaults to the table name.
    pub alias: Option<Ident>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinKind {
    Inner,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub kind: JoinKind,
    pub table: TableRef,
    pub on: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub distinct: bool,
    pub items: Vec<SelectItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    /// `*`: every column of every source.
    Star(Span),
    /// `alias.*`
    SourceStar(Ident),
    Expr { expr: Expr, alias: Option<Ident> },
}

/// `over (partition by ... order by ... [rows between ... and ...])`
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    pub partition_by: Vec<Expr>,
    pub order_by: Vec<OrderItem>,
    pub frame: Option<Box<Frame>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameUnits {
    Rows,
    Range,
    Groups,
}

/// Which rows around the current one a window function sees:
/// `rows between 2 preceding and current row`, or `rows unbounded preceding` (to the current row).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub units: FrameUnits,
    pub start: FrameBound,
    /// `None` for the one-bound form, which ends at the current row.
    pub end: Option<FrameBound>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FrameBound {
    UnboundedPreceding,
    Preceding(Expr),
    CurrentRow,
    Following(Expr),
    UnboundedFollowing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub expr: Expr,
    pub desc: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(i64, Span),
    /// Decimal text as written, sign included.
    Decimal(String, Span),
    Str(String, Span),
    Bool(bool, Span),
    Null(Span),
    /// `name` or `qualifier.name`
    Column { qualifier: Option<Ident>, name: Ident },
    /// `:name`
    Param(Ident),
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Span },
    Not(Box<Expr>, Span),
    IsNull { expr: Box<Expr>, negated: bool, span: Span },
    In { expr: Box<Expr>, list: Vec<Expr>, negated: bool, span: Span },
    Like { expr: Box<Expr>, pattern: Box<Expr>, negated: bool, span: Span },
    Between { expr: Box<Expr>, low: Box<Expr>, high: Box<Expr>, negated: bool, span: Span },
    /// A function or aggregate call. `star` is `count(*)`; `distinct` is `count(distinct x)`;
    /// `filter` is `filter (where ...)` after an aggregate; `agg_order` is the `order by` inside
    /// `string_agg(x, ", " order by y)`.
    Call { func: Ident, args: Vec<Expr>, star: bool, distinct: bool, filter: Option<Box<Expr>>, agg_order: Vec<OrderItem>, over: Option<WindowSpec>, span: Span },
    Case { whens: Vec<(Expr, Expr)>, otherwise: Option<Box<Expr>>, span: Span },
    Paren(Box<Expr>, Span),
    /// `a || b || c`: text joined end to end (NULL if any part is).
    Concat(Vec<Expr>, Span),
    /// `exists (from ... select ...)`
    Exists(Box<Query>, Span),
    /// `x in (from ... select col)`
    InQuery { expr: Box<Expr>, query: Box<Query>, negated: bool, span: Span },
    /// `(from ... select expr)`: one column, at most one row.
    Scalar(Box<Query>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Number(_, s) | Expr::Decimal(_, s) | Expr::Str(_, s) | Expr::Bool(_, s) | Expr::Null(s)
            | Expr::Not(_, s) | Expr::Paren(_, s) | Expr::Exists(_, s) | Expr::Scalar(_, s) | Expr::Concat(_, s) => *s,
            Expr::Column { qualifier, name } => qualifier.as_ref().map_or(name.span, |q| q.span.to(name.span)),
            Expr::Param(i) => i.span,
            Expr::Binary { span, .. } | Expr::IsNull { span, .. } | Expr::In { span, .. } | Expr::Like { span, .. }
            | Expr::Between { span, .. } | Expr::Call { span, .. } | Expr::Case { span, .. }
            | Expr::InQuery { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    Insert,
    Update,
    Delete,
}

/// `insert` / `update` / `delete`: a named, parameterised write.
///
/// ```text
/// insert add(name: text) { into customers set name = :name returning id }
/// update rename(id: int, n: text) { customers c set name = :n where c.id == :id }
/// delete purge(before: timestamp) { from orders o where o.created < :before }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Mutation {
    pub kind: MutationKind,
    pub name: Ident,
    pub params: Vec<ParamDecl>,
    pub table: TableRef,
    /// `set col = expr, ...` (insert and update).
    pub assignments: Vec<Assignment>,
    /// Insert, tabular form: `into t (a, b) values (..), (..)` or `into t (a, b) from ... select ...`.
    pub insert_columns: Vec<Ident>,
    pub rows: Vec<Vec<Expr>>,
    pub source: Option<Box<Query>>,
    /// `where expr` (update and delete).
    pub filter: Option<Expr>,
    /// `all rows`: an update or delete that deliberately has no `where`.
    pub all_rows: bool,
    /// `on conflict (...) do ...` (insert).
    pub conflict: Option<Conflict>,
    pub returning: Vec<SelectItem>,
    /// `with name as (...)` before the statement.
    pub ctes: Vec<Cte>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub column: Ident,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub columns: Vec<Ident>,
    pub action: ConflictAction,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConflictAction {
    Nothing,
    /// `do update set col = expr, ...` (may read the existing row and `excluded`).
    Update(Vec<Assignment>),
}
