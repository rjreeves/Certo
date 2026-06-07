use crate::span::{S, Span};
use crate::types::{TypeExpr, ModulePath};
use crate::pattern::Pattern;

// ------------------------------------------------------------------ //
// Literals
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64),
    Float(f64),
    /// Exact decimal string, e.g. `"19.99"` from `d"19.99"`
    Decimal(String),
    Bool(bool),
    String(String),
    /// Interpolated string segments — raw text and embedded expressions interleaved
    FString(Vec<FStringPart>),
    Uuid(String),
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Literal(String),
    Interpolated(Box<S<Expr>>),
}

// ------------------------------------------------------------------ //
// Expressions
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal value
    Lit { value: Lit, span: Span },

    /// An identifier or qualified path: `x`, `Stdlib.Result.Ok`
    Path { path: ModulePath, span: Span },

    /// `f(a, b)` — function application; named args via `Arg`
    App {
        func: Box<S<Expr>>,
        args: Vec<Arg>,
        span: Span,
    },

    /// `a |> b` — pipeline
    Pipe {
        left:  Box<S<Expr>>,
        right: Box<S<Expr>>,
        span:  Span,
    },

    /// `a op b` — binary operation
    BinOp {
        op:    BinOp,
        left:  Box<S<Expr>>,
        right: Box<S<Expr>>,
        span:  Span,
    },

    /// `op a` — unary operation
    UnOp {
        op:   UnOp,
        expr: Box<S<Expr>>,
        span: Span,
    },

    /// `expr.field` — field access
    Field {
        expr:  Box<S<Expr>>,
        field: S<String>,
        span:  Span,
    },

    /// `expr?.field` — safe field access, returns `Option`
    SafeField {
        expr:  Box<S<Expr>>,
        field: S<String>,
        span:  Span,
    },

    /// `if cond then a else b`
    If {
        cond:      Box<S<Expr>>,
        then_expr: Box<S<Expr>>,
        else_expr: Box<S<Expr>>,
        span:      Span,
    },

    /// `match expr { pat => expr ... }`
    Match {
        scrutinee: Box<S<Expr>>,
        arms:      Vec<MatchArm>,
        span:      Span,
    },

    /// `{ stmt; ...; expr }` — block; last expression is the value
    Block {
        stmts: Vec<Stmt>,
        span:  Span,
    },

    /// `a => b` — anonymous function / lambda
    Lambda {
        params: Vec<LambdaParam>,
        body:   Box<S<Expr>>,
        span:   Span,
    },

    /// `[e1, e2, e3]` — list literal
    List {
        elements: Vec<S<Expr>>,
        span:     Span,
    },

    /// `(e1, e2)` — tuple literal
    Tuple {
        elements: Vec<S<Expr>>,
        span:     Span,
    },

    /// `{ field: value, ... }` — record literal or record update
    Record {
        base:   Option<Box<S<Expr>>>,   // `expr with { ... }` update syntax
        fields: Vec<RecordField>,
        span:   Span,
    },

    /// `e?` — error propagation (equivalent to Rust's `?`)
    Try {
        expr: Box<S<Expr>>,
        span: Span,
    },

    /// `await expr`
    Await {
        expr: Box<S<Expr>>,
        span: Span,
    },

    /// `guard cond else expr` — early-return guard clause
    Guard {
        cond:      Box<S<Expr>>,
        else_expr: Box<S<Expr>>,
        span:      Span,
    },

    /// `require expr (ErrorVariant(...))`  — unwrap or return error
    Require {
        expr:  Box<S<Expr>>,
        error: Box<S<Expr>>,
        span:  Span,
    },

    /// `parallel { expr, expr, ... }` — concurrent execution block
    Parallel {
        tasks:   Vec<S<Expr>>,
        timeout: Option<Box<S<Expr>>>,
        span:    Span,
    },

    /// `db.transaction { ... }` — database transaction block
    Transaction {
        body: Box<S<Expr>>,
        span: Span,
    },

    /// `unsafe { ... }` — unsafe block
    Unsafe {
        body: Box<S<Expr>>,
        span: Span,
    },

    /// Type ascription: `expr: Type`
    Ascribe {
        expr: Box<S<Expr>>,
        ty:   Box<S<TypeExpr>>,
        span: Span,
    },
}

// ------------------------------------------------------------------ //
// Sub-nodes used by Expr
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub label: Option<S<String>>,  // named arg: `page: 2`
    pub value: S<Expr>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: S<Pattern>,
    pub guard:   Option<S<Expr>>,
    pub body:    S<Expr>,
    pub span:    Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordField {
    pub name:  S<String>,
    pub value: S<Expr>,
    pub span:  Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: S<String>,
    pub ty:   Option<S<TypeExpr>>,
    pub span: Span,
}

// ------------------------------------------------------------------ //
// Statements (inside blocks)
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `val name: Type = expr` or `val name = expr`
    Val {
        pattern: S<Pattern>,
        ty:      Option<S<TypeExpr>>,
        value:   S<Expr>,
        span:    Span,
    },
    /// `var name: Type = expr`
    Var {
        name:  S<String>,
        ty:    Option<S<TypeExpr>>,
        value: S<Expr>,
        span:  Span,
    },
    /// `name = expr` — assignment to a mutable var
    Assign {
        target: S<String>,
        value:  S<Expr>,
        span:   Span,
    },
    /// `defer { expr }` — run on scope exit
    Defer {
        body: S<Expr>,
        span: Span,
    },
    /// A bare expression used as a statement
    Expr {
        expr: S<Expr>,
        span: Span,
    },
}

// ------------------------------------------------------------------ //
// Operators
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp {
    // Arithmetic
    Add, Sub, Mul, Div, Rem, Pow,
    // Comparison
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    // Logical
    And, Or,
    // Range
    RangeInclusive,   // ..
    RangeExclusive,   // ...
    // Null-coalesce
    NullCoalesce,     // ??
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnOp {
    Neg,  // -
    Not,  // not
}
