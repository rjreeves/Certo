use certo_typeck::Ty;
use certo_ast::span::Span;

/// A unique identifier for a local variable in the HIR.
pub type LocalId = u32;

/// A unique identifier for a top-level function.
pub type FnId = u32;

// ------------------------------------------------------------------ //
// Top-level
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct HirModule {
    pub name:  String,
    pub items: Vec<HirItem>,
}

#[derive(Debug, Clone)]
pub enum HirItem {
    Fn(HirFn),
    Const(HirConst),
}

#[derive(Debug, Clone)]
pub struct HirFn {
    pub id:     FnId,
    pub name:   String,
    pub params: Vec<HirParam>,
    pub ret_ty: Ty,
    pub body:   Option<HirExpr>,
    pub span:   Span,
}

#[derive(Debug, Clone)]
pub struct HirParam {
    pub local: LocalId,
    pub name:  String,
    pub ty:    Ty,
    pub span:  Span,
}

#[derive(Debug, Clone)]
pub struct HirConst {
    pub name:  String,
    pub ty:    Ty,
    pub value: HirExpr,
    pub span:  Span,
}

// ------------------------------------------------------------------ //
// Expressions — desugared, type-annotated
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty:   Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum HirExprKind {
    /// An integer literal.
    Int(i64),
    /// A float literal.
    Float(f64),
    /// A decimal literal (kept as string for exact representation).
    Decimal(String),
    /// A boolean literal.
    Bool(bool),
    /// A string literal (FString already interpolated to a sequence of parts).
    Str(String),
    /// A UUID literal.
    Uuid(String),
    /// Unit value.
    Unit,

    /// Reference to a local variable.
    Local(LocalId),
    /// Reference to a top-level function or global.
    Global(String),

    /// Function call: `f(args)`.
    /// `|>` and method calls are desugared to this.
    Call {
        func: Box<HirExpr>,
        args: Vec<HirExpr>,
    },

    /// Binary intrinsic: `a + b`, `a == b`, etc.
    /// After trait resolution these become `Call`; we keep them as intrinsics
    /// for primitive types so the C backend can emit operators directly.
    BinOp {
        op:  BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },

    /// Unary intrinsic: `-x`, `!b`.
    UnOp {
        op:  UnOp,
        arg: Box<HirExpr>,
    },

    /// Field access: `expr.field`.
    Field {
        base:  Box<HirExpr>,
        field: String,
    },

    /// Record construction: `{ field: value, ... }`.
    Record(Vec<(String, HirExpr)>),

    /// Tuple construction: `(a, b)`.
    Tuple(Vec<HirExpr>),

    /// List construction: `[a, b, c]`.
    List(Vec<HirExpr>),

    /// `if cond then_expr else else_expr` (always has both branches).
    If {
        cond:      Box<HirExpr>,
        then_expr: Box<HirExpr>,
        else_expr: Box<HirExpr>,
    },

    /// A block: sequence of statements ending in an expression.
    Block {
        stmts: Vec<HirStmt>,
        tail:  Box<HirExpr>,
    },

    /// Anonymous function (lambda).
    Lambda {
        params: Vec<HirParam>,
        body:   Box<HirExpr>,
    },

    /// Match expression — arms have been desugared to flat patterns.
    Match {
        scrutinee: Box<HirExpr>,
        arms:      Vec<HirArm>,
    },

    /// `e?` — error propagation (requires enclosing fn to return Result).
    Try(Box<HirExpr>),

    /// `unsafe { body }`.
    Unsafe(Box<HirExpr>),

    /// `for binding in iter { body }` — desugars to a runtime loop in MIR.
    For {
        binding:      LocalId,
        binding_name: String,
        iter:         Box<HirExpr>,
        body:         Box<HirExpr>,
    },
}

// ------------------------------------------------------------------ //
// Statements
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub enum HirStmt {
    /// `let local: ty = expr`
    Let { local: LocalId, name: String, ty: Ty, init: HirExpr },
    /// `local = expr`
    Assign { local: LocalId, value: HirExpr },
    /// Expression statement.
    Expr(HirExpr),
}

// ------------------------------------------------------------------ //
// Match arms
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct HirArm {
    pub pat:  HirPat,
    pub body: HirExpr,
}

#[derive(Debug, Clone)]
pub enum HirPat {
    Wildcard,
    Bind { local: LocalId, name: String },
    Lit(HirLitPat),
    Tuple(Vec<HirPat>),
    Constructor { name: String, fields: Vec<HirPat> },
    Or(Box<HirPat>, Box<HirPat>),
}

#[derive(Debug, Clone)]
pub enum HirLitPat {
    Int(i64),
    Bool(bool),
    Str(String),
}

// ------------------------------------------------------------------ //
// Operators
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem, Pow,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    And, Or,
    NullCoalesce,
    Concat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnOp { Neg, Not }
