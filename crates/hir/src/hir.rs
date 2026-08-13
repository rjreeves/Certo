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
    /// Record type name → ordered declared field types (a bare type-param
    /// field is `Ty::Var(0)`). Exposed so MIR's `lower_fn` can decide when a
    /// generic type's field construction/read needs heap-boxing — BACKLOG
    /// item 119.
    pub record_field_types: std::collections::HashMap<String, Vec<Ty>>,
    /// Sum variant name → ordered declared payload field types. Same purpose
    /// as `record_field_types`, for sum-type variant constructors/patterns.
    pub variant_field_types: std::collections::HashMap<String, Vec<Ty>>,
    /// Function name → ordered *declared* param types, as written (a bare
    /// type-param param is `Ty::Var(0)`, unsubstituted — not the call site's
    /// concrete argument type). Top-level `fn`s are keyed by their bare name,
    /// `impl` methods by `Type.method`, matching `record_field_types`'
    /// per-call-site declared-vs-concrete split. Lets MIR's `lower_fn` box a
    /// concrete argument passed into a generic function's bare-`T` parameter
    /// — BACKLOG item 120 (the call-site half). Item 135 landed the return
    /// side: a `val`-annotated or argument-position call site can now
    /// recover the concrete type, see `fn_ret_tys` below.
    pub fn_param_tys: std::collections::HashMap<String, Vec<Ty>>,
    /// Function name → *declared* return type, as written (a bare type-param
    /// return is `Ty::Var(0)`, unsubstituted) — same naming convention as
    /// `fn_param_tys`. Lets MIR's `lower_fn` know when a call's own
    /// now-resolved concrete `Ty` (see `crates/hir/src/lower.rs`'s
    /// `resolve_bare_generic_return`, BACKLOG item 135) actually needs
    /// unboxing from the raw `void*` the C function still returns.
    pub fn_ret_tys: std::collections::HashMap<String, Ty>,
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
        /// True when the field's *declared* type is a bare type parameter
        /// (`Ty::Var(0)` in `record_field_types`/`variant_field_types`) —
        /// its C storage is `void*` regardless of the concrete type it's
        /// instantiated to, so the read must unbox (BACKLOG item 119).
        /// The outer `HirExpr.ty` carries the substituted concrete type
        /// (or `Ty::Error` when it can't be recovered).
        boxed: bool,
    },

    /// Record construction: `{ field: value, ... }`.
    /// `field_types` are the *declared* field types (parallel to the value
    /// list, by index) — a `Ty::Var(0)` entry marks a field that must be
    /// heap-boxed on construction (BACKLOG item 119).
    Record {
        fields:      Vec<(String, HirExpr)>,
        field_types: Vec<Ty>,
    },

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
        /// Locals referenced from an enclosing scope (BACKLOG item 140) —
        /// real closure capture. Each is boxed into the lambda's own heap
        /// environment at the construction site and unboxed back into a
        /// same-named local inside the lambda's own lowered function.
        captures: Vec<LocalId>,
        /// The declared return type of whatever `Ty::Fn` parameter position
        /// this lambda literal was passed directly into, when known (BACKLOG
        /// item 76) — `Ty::Error` when there's no such context (the common
        /// case; matches every pre-existing construction site unchanged).
        /// When it's `Ty::Var(_)` (an erased slot — e.g. a higher-kinded
        /// `F<B>` return) but the lambda body's own inferred result is a
        /// real concrete type, MIR must box that result before returning,
        /// or the lambda's real native return (a struct, by value) can't
        /// possibly satisfy the closure's uniform `void*`-returning ABI
        /// every caller of a `Ty::Fn` value assumes.
        ret_hint: Ty,
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
        binding_ty:   certo_typeck::Ty,
        iter:         Box<HirExpr>,
        body:         Box<HirExpr>,
    },

    /// `while cond { body }` — condition-based loop; evaluates to Unit.
    While {
        cond: Box<HirExpr>,
        body: Box<HirExpr>,
    },

    /// `spawn expr` — run `expr` in a new OS thread, return Task handle
    /// (int64_t pointer). `args` holds the single spawned expression (kept
    /// as a `Vec` for historical reasons — always exactly one element).
    /// `captures` are locals `expr` references from the enclosing scope
    /// (BACKLOG item 140/141) — only load-bearing when `expr` isn't a
    /// direct call to a named function (MIR evaluates a direct call's own
    /// arguments eagerly in the current thread, so no lifting/capture is
    /// needed there); any other shape (a block, `if`, `while`, ...) must
    /// run the *whole* body on the worker thread, so MIR lifts it into a
    /// synthesized top-level function taking these captures as ordinary
    /// parameters.
    Spawn {
        fn_name:  String,
        args:     Vec<HirExpr>,
        captures: Vec<LocalId>,
    },

    /// `await task` — join a spawned Task, returning its result as int64_t.
    Await(Box<HirExpr>),

    /// `await task` inside a `parallel(timeout: ...) { ... }` block — like
    /// `Await`, but joins with a shared deadline (an absolute monotonic-clock
    /// millisecond value, computed once for the whole block and referenced
    /// by every task's join) instead of waiting indefinitely. Panics at
    /// runtime if the deadline passes before the task finishes — BACKLOG item 81.
    AwaitTimed {
        task:     Box<HirExpr>,
        deadline: LocalId,
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
    /// `defer { body }` — body runs at function exit, LIFO order.
    Defer { body: HirExpr },
}

// ------------------------------------------------------------------ //
// Match arms
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct HirArm {
    pub pat:   HirPat,
    pub guard: Option<HirExpr>,
    pub body:  HirExpr,
}

#[derive(Debug, Clone)]
pub enum HirPat {
    Wildcard,
    Bind { local: LocalId, name: String },
    Lit(HirLitPat),
    Tuple(Vec<HirPat>),
    /// `field_names[i]` is the C struct member the codegen-emitted tagged
    /// union actually uses for `fields[i]`'s payload slot — the field's real
    /// name if the variant declared one (`Circle(radius: Float)`), else the
    /// positional fallback `f{i}` (see `emit_module.rs`'s struct emission,
    /// which the two must agree with byte-for-byte). `field_types[i]` is the
    /// field's declared type, so a bound local gets that type instead of
    /// `Ty::Error` (which codegen maps to `int64_t`, silently truncating
    /// e.g. `Float` payloads on read).
    Constructor { name: String, fields: Vec<HirPat>, field_names: Vec<String>, field_types: Vec<Ty> },
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
