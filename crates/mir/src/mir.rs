use certo_typeck::Ty;
use certo_hir::{BinOp, UnOp};

/// Index into a `MirFn`'s `blocks` vec.
pub type BlockId = usize;

/// Index into a `MirFn`'s `locals` vec.
pub type MirLocal = u32;

// ------------------------------------------------------------------ //
// Function in MIR form
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct MirFn {
    pub name:        String,
    /// Number of actual function parameters (locals 1..=param_count after the ret slot).
    pub param_count: usize,
    /// Declared locals (ret slot at 0, params 1..=param_count, then temporaries).
    pub locals:      Vec<MirLocalDecl>,
    /// Basic blocks; block 0 is the entry.
    pub blocks:      Vec<BasicBlock>,
}

#[derive(Debug, Clone)]
pub struct MirLocalDecl {
    pub id:   MirLocal,
    pub name: String,
    pub ty:   Ty,
}

// ------------------------------------------------------------------ //
// Basic block
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, Default)]
pub struct BasicBlock {
    pub id:          BlockId,
    pub stmts:       Vec<MirStmt>,
    pub terminator:  Option<Terminator>,
}

// ------------------------------------------------------------------ //
// Statements
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub enum MirStmt {
    /// `local = rvalue`
    Assign { dest: MirLocal, rvalue: Rvalue },
}

#[derive(Debug, Clone)]
pub enum Rvalue {
    /// Copy / move a local.
    Use(Operand),
    /// Binary operation.
    BinOp { op: BinOp, lhs: Operand, rhs: Operand },
    /// Unary operation.
    UnOp { op: UnOp, arg: Operand },
    /// Function call — result stored in `dest`.
    Call { func: Operand, args: Vec<Operand> },
    /// Struct field access: `base.field`.
    Field { base: Operand, field: String },
    /// Aggregate construction: tuple, record.
    Aggregate(AggregateKind, Vec<Operand>),
    /// Spawn `func(args)` on a new OS thread. Produces an opaque task handle
    /// (a heap pointer). `ret_ty` is the type the task will compute.
    Spawn { func: Operand, args: Vec<Operand>, ret_ty: Ty },
    /// Join a task handle produced by `Spawn`, yielding its result of `ret_ty`.
    Join { task: Operand, ret_ty: Ty },
    /// Join a task handle with a shared deadline (an absolute monotonic-clock
    /// millisecond value) instead of waiting indefinitely — `parallel(timeout:
    /// ...) { ... }`, BACKLOG item 81. Panics at runtime if the deadline
    /// passes before the task finishes.
    JoinTimed { task: Operand, deadline: Operand, ret_ty: Ty },
    /// Heap-box a value into an `Option` (`Some(v)`): allocate a slot, store
    /// `value` with its own type (bit-preserving), yield the pointer. `ty` is
    /// the payload type. `None` is a null pointer.
    BoxSome { value: Operand, ty: Ty },
    /// Read the payload out of a non-null `Option` pointer, as `ty`.
    UnboxSome { opt: Operand, ty: Ty },
    /// Pack `value` (of type `ty`) into a pointer-sized generic slot,
    /// bit-preserving (a `Float` is bit-cast via `__certo_f2i`, everything
    /// else is already pointer-sized). Used to adapt a lambda's real,
    /// natively-typed parameters/return into the boxed `void*` ABI a
    /// handful of stdlib higher-order functions require — see BACKLOG
    /// item 112.
    Box { value: Operand, ty: Ty },
    /// Recover a value of type `ty` from a pointer-sized generic slot —
    /// the inverse of `Box`.
    Unbox { value: Operand, ty: Ty },
}

#[derive(Debug, Clone)]
pub enum AggregateKind {
    Tuple,
    Record(Vec<String>),  // field names
    Array,
}

#[derive(Debug, Clone)]
pub enum Operand {
    /// A local variable.
    Local(MirLocal),
    /// A compile-time constant.
    Const(MirConst),
    /// Reference to a named global / function.
    Global(String),
}

#[derive(Debug, Clone)]
pub enum MirConst {
    Int(i64),
    Float(f64),
    Decimal(String),
    Bool(bool),
    Str(String),
    Uuid(String),
    Unit,
}

// ------------------------------------------------------------------ //
// Terminators — control flow at the end of each block
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub enum Terminator {
    /// Unconditional jump.
    Goto(BlockId),
    /// Conditional branch: `if cond goto true_bb else false_bb`.
    If { cond: Operand, true_bb: BlockId, false_bb: BlockId },
    /// Return the value of `local` from the function.
    Return(Operand),
    /// Unreachable (e.g. after a panic).
    Unreachable,
    /// Call with a continuation: after the call, jump to `next`.
    Call {
        func:   Operand,
        args:   Vec<Operand>,
        dest:   MirLocal,
        next:   BlockId,
    },
    /// Switch over a discriminant for match expressions.
    Switch {
        discr:    Operand,
        targets:  Vec<(SwitchTarget, BlockId)>,
        otherwise: BlockId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwitchTarget {
    Int(i64),
    Bool(bool),
}
