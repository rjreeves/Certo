use certo_typeck::Ty;
use certo_hir::{HirFn, HirExpr, HirExprKind, HirStmt, HirPat, HirLitPat, LocalId};
use certo_ast::span::Span;
use crate::mir::*;

// ------------------------------------------------------------------ //
// Builder
// ------------------------------------------------------------------ //

struct Builder {
    locals:      Vec<MirLocalDecl>,
    blocks:      Vec<BasicBlock>,
    current:     BlockId,
    /// Maps HIR LocalId to MirLocal (same indices initially).
    local_map:   std::collections::HashMap<LocalId, MirLocal>,
    next_tmp:    MirLocal,
    /// Lambdas lifted to top-level functions during lowering.
    lifted_fns:  Vec<MirFn>,
    lambda_count: u32,
    /// Name prefix from the enclosing function (for naming lifted lambdas).
    fn_name:     String,
    /// Deferred expressions accumulated by `defer { ... }` statements.
    /// Emitted in LIFO order before every `Return` terminator.
    defers:      Vec<certo_hir::HirExpr>,
    /// Record type name → ordered declared field types (BACKLOG item 119) —
    /// a `Ty::Var(0)` entry marks a bare type-param field, which is
    /// heap-boxed on construction and unboxed on read since its C storage
    /// is `void*` regardless of what concrete type it's instantiated to.
    record_field_types:  std::collections::HashMap<String, Vec<Ty>>,
    /// Sum variant name → ordered declared payload field types. Same
    /// purpose as `record_field_types`, for variant constructors/patterns.
    variant_field_types: std::collections::HashMap<String, Vec<Ty>>,
    /// Function name → ordered *declared* param types, as written (BACKLOG
    /// item 120) — a `Ty::Var(0)` entry marks a bare type-param parameter,
    /// which needs a concrete argument heap-boxed at the call site.
    fn_param_tys: std::collections::HashMap<String, Vec<Ty>>,
    /// The span of the Certo *statement* currently being lowered (BACKLOG
    /// item 126) — updated only at statement boundaries (`lower_stmt`, a
    /// block's tail expression, a function/lambda's own top-level body),
    /// not for every sub-expression, so every `MirStmt::Assign` created
    /// while lowering one statement's sub-expressions is tagged with that
    /// *whole statement's* span. Coarser than expression-level, but that's
    /// the right granularity for line-based coverage (`certo test
    /// --coverage`) — see `crates/codegen/src/emit_mir.rs`'s `#line`
    /// emission, the only consumer.
    current_span: Span,
}

impl Builder {
    fn new(
        fn_name: &str,
        record_field_types:  std::collections::HashMap<String, Vec<Ty>>,
        variant_field_types: std::collections::HashMap<String, Vec<Ty>>,
        fn_param_tys:        std::collections::HashMap<String, Vec<Ty>>,
    ) -> Self {
        let entry = BasicBlock { id: 0, ..Default::default() };
        Builder {
            locals:    Vec::new(),
            blocks:    vec![entry],
            current:   0,
            local_map: std::collections::HashMap::new(),
            next_tmp:  0,
            lifted_fns: Vec::new(),
            lambda_count: 0,
            fn_name: fn_name.to_string(),
            defers:    Vec::new(),
            record_field_types,
            variant_field_types,
            fn_param_tys,
            current_span: Span::DUMMY,
        }
    }

    fn declare_local(&mut self, name: &str, ty: Ty) -> MirLocal {
        let id = self.next_tmp;
        self.next_tmp += 1;
        self.locals.push(MirLocalDecl { id, name: name.to_string(), ty });
        id
    }

    fn map_hir_local(&mut self, hir: LocalId, name: &str, ty: Ty) -> MirLocal {
        let mir = self.declare_local(name, ty);
        self.local_map.insert(hir, mir);
        mir
    }

    fn get_local(&self, hir: LocalId) -> MirLocal {
        *self.local_map.get(&hir).unwrap_or(&hir)
    }

    fn push_stmt(&mut self, stmt: MirStmt) {
        self.blocks[self.current].stmts.push(stmt);
    }

    fn terminate(&mut self, t: Terminator) {
        self.blocks[self.current].terminator = Some(t);
    }

    fn new_block(&mut self) -> BlockId {
        let id = self.blocks.len();
        self.blocks.push(BasicBlock { id, ..Default::default() });
        id
    }

    fn switch_to(&mut self, block: BlockId) {
        self.current = block;
    }

    fn assign(&mut self, dest: MirLocal, rvalue: Rvalue) {
        self.push_stmt(MirStmt::Assign { dest, rvalue, span: self.current_span });
    }
}

// ------------------------------------------------------------------ //
// Defer helper
// ------------------------------------------------------------------ //

/// Emit all pending deferred expressions (LIFO), then terminate with Return.
/// Clones the defer list so the Builder can be mutably borrowed during lowering.
fn emit_defers_then_return(return_op: Operand, b: &mut Builder) {
    let defers: Vec<certo_hir::HirExpr> = b.defers.clone();
    for body in defers.iter().rev() {
        b.current_span = body.span;
        lower_expr(body, b);
    }
    b.terminate(Terminator::Return(return_op));
}

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

/// Lower a HIR function to MIR. Returns the primary function plus any lambdas lifted to top level.
/// `record_field_types`/`variant_field_types` come from `HirModule` (BACKLOG item 119) —
/// used to decide when a generic type's field construction/read needs heap-boxing.
/// `fn_param_tys` (BACKLOG item 120) is the same idea for a plain function's
/// own declared parameter types, used to box a concrete argument passed into
/// a generic function's bare-`T` parameter.
pub fn lower_fn(
    f: &HirFn,
    record_field_types:  &std::collections::HashMap<String, Vec<Ty>>,
    variant_field_types: &std::collections::HashMap<String, Vec<Ty>>,
    fn_param_tys:        &std::collections::HashMap<String, Vec<Ty>>,
) -> (MirFn, Vec<MirFn>) {
    let mut b = Builder::new(&f.name, record_field_types.clone(), variant_field_types.clone(), fn_param_tys.clone());

    // Declare params as locals (index 0 = return slot, type patched below).
    let ret_slot = b.declare_local("_ret", Ty::Error);
    for p in &f.params {
        b.map_hir_local(p.local, &p.name, p.ty.clone());
    }

    let result = if let Some(body) = &f.body {
        b.current_span = body.span;
        lower_expr(body, &mut b)
    } else {
        Operand::Const(MirConst::Unit)
    };

    // Infer the actual return type from the result operand and patch slot 0.
    // HirFn.ret_ty is Ty::Error (placeholder), so we derive it here instead.
    let ret_ty = infer_operand_ty(&result, &b);
    b.locals[ret_slot as usize].ty = ret_ty.clone();

    // For Unit functions, skip the return-slot assignment entirely — the
    // terminator emits `return;` and there is no `_l0` variable to write.
    if !matches!(ret_ty, Ty::Unit) {
        b.assign(ret_slot, Rvalue::Use(result.clone()));
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut b);

    let lifted = b.lifted_fns;
    (MirFn { name: f.name.clone(), param_count: f.params.len(), locals: b.locals, blocks: b.blocks }, lifted)
}

/// Stdlib higher-order functions whose C runtime parameter is a single
/// generic `void* (*)(void*)`-style function pointer (`CertoFn1`/`CertoPred`
/// in `collections.rs`) rather than the callee's real native signature.
/// A lambda literal *or a named function reference* passed directly as the
/// callback argument to one of these needs the boxed-ABI treatment (see
/// `lower_lambda_boxed`/`lower_named_fn_boxed`) — BACKLOG items 112 and 134.
/// Deliberately narrow: `List.sort`'s two-argument, Int-returning comparator
/// and user-defined higher-order functions (which already use a consistent
/// native ABI on both sides, see item 108) are out of scope.
///
/// `dbQueryTyped`/`Query.list`/`Query.first`/`Query.groupedList` (item 134)
/// were added after confirming their mapper callback is always a *named*
/// function reference in practice (`certo db pull`'s own generated code:
/// `dbQueryTyped(conn, sql, params, widgetsFromRow)`), never an inline
/// lambda — the exact case `lower_lambda_boxed` alone didn't cover, and
/// which segfaulted (not just misread bits) since the real mapper's return
/// type is a struct, not a pointer-sized value.
const BOXED_ABI_CALLEES: &[&str] = &[
    "List.map", "List.filter", "List.find", "List.any", "List.all", "List.groupBy",
    "dbQueryTyped", "Query.list", "Query.first", "Query.groupedList",
];

/// Lift a lambda literal that's being passed directly as the callback
/// argument to one of `BOXED_ABI_CALLEES`. Every param is unboxed on entry
/// and the return value boxed on exit, so the lambda's C signature is
/// uniformly `void* (*)(void*, ...)` — matching the generic function
/// pointer type those C runtime functions declare, instead of the lambda's
/// real native signature (e.g. `double(double)`), which is what caused a
/// `Float`-returning callback to silently misread the wrong return
/// register (BACKLOG item 112).
fn lower_lambda_boxed(params: &[certo_hir::HirParam], body: &HirExpr, param_ty_hint: Option<&Ty>, b: &mut Builder) -> Operand {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let lam_name = format!("__lam_{}_{}_boxed", b.fn_name, idx);

    let mut lb = Builder::new(&lam_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Var(0));

    // The C signature's params (locals 1..=param_count) are always void* —
    // declare all of them first, then the "real" typed locals the body
    // actually uses, unboxed from the raw params. Lambda params are almost
    // always unannotated (`(x) => ...`), so `p.ty` is `Ty::Error`; fall back
    // to `param_ty_hint` (the callee's scrutinee element type, known at the
    // call site) rather than mis-unboxing as a raw pointer cast.
    let raw_locals: Vec<MirLocal> = params.iter()
        .map(|p| lb.declare_local(&format!("{}_boxed", p.name), Ty::Var(0)))
        .collect();
    for (p, raw) in params.iter().zip(raw_locals.iter()) {
        let real_ty = if matches!(p.ty, Ty::Error) {
            param_ty_hint.cloned().unwrap_or(Ty::Error)
        } else {
            p.ty.clone()
        };
        let real = lb.map_hir_local(p.local, &p.name, real_ty.clone());
        lb.assign(real, Rvalue::Unbox { value: Operand::Local(*raw), ty: real_ty });
    }

    lb.current_span = body.span;
    let lam_result = lower_expr(body, &mut lb);
    let lam_ret_ty = infer_operand_ty(&lam_result, &lb);
    if matches!(lam_ret_ty, Ty::Unit) {
        lb.locals[ret_slot as usize].ty = Ty::Var(0);
    } else {
        lb.assign(ret_slot, Rvalue::Box { value: lam_result, ty: lam_ret_ty });
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let lam_fn = MirFn {
        name: lam_name.clone(),
        param_count: params.len(),
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(lam_fn);

    let dest = b.declare_local("_lam_ptr", Ty::Error);
    b.assign(dest, Rvalue::Use(Operand::Global(lam_name)));
    Operand::Local(dest)
}

/// Synthesize a small wrapper function for a *named* function reference
/// (not an inline lambda) passed directly as the callback argument to one
/// of `BOXED_ABI_CALLEES` — e.g. `dbQueryTyped(conn, sql, params,
/// widgetsFromRow)`, `certo db pull`'s own generated calling convention.
/// Mirrors `lower_lambda_boxed`'s shape (unbox each param, call the real
/// function, box the return) but wraps a *call* to the existing named
/// function instead of re-lowering a lambda body — BACKLOG item 134.
/// Before this, a bare named-function reference fell through
/// `lower_lambda_boxed`'s lambda-only check entirely and got passed as-is:
/// the C compiler accepts the resulting incompatible function-pointer cast
/// (`-Wno-incompatible-function-pointer-types` is already required
/// elsewhere in this toolchain), but calling a function whose real
/// signature returns a struct by value through a generic `void* (*)(void*)`
/// pointer is undefined behavior — the struct-return calling convention
/// (a hidden caller-supplied return-slot pointer) doesn't match a plain
/// register return, which is what caused `certo db pull`'s generated
/// `*FindById` to segfault. `real_param_tys`/`real_ret_ty` come from the
/// named function's own already-resolved `Ty::Fn` (the HIR `Global`
/// reference's own `.ty`, populated from `cx.global_types` during HIR
/// lowering), not a param-type hint — a named function's real signature is
/// always fully known, unlike an unannotated lambda param.
fn lower_named_fn_boxed(name: &str, real_param_tys: &[Ty], real_ret_ty: &Ty, b: &mut Builder) -> Operand {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__fnref_{}_{}_boxed", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Var(0));

    // C signature params are always void*; unbox each into the real
    // function's own declared param type before calling it.
    let raw_locals: Vec<MirLocal> = (0..real_param_tys.len())
        .map(|i| lb.declare_local(&format!("_p{}_boxed", i), Ty::Var(0)))
        .collect();
    let real_locals: Vec<MirLocal> = real_param_tys.iter().zip(raw_locals.iter())
        .map(|(ty, raw)| {
            let real = lb.declare_local("_p_real", ty.clone());
            lb.assign(real, Rvalue::Unbox { value: Operand::Local(*raw), ty: ty.clone() });
            real
        })
        .collect();

    let call_dest = lb.declare_local("_inner_call", real_ret_ty.clone());
    let next = lb.new_block();
    lb.terminate(Terminator::Call {
        func: Operand::Global(name.to_string()),
        args: real_locals.into_iter().map(Operand::Local).collect(),
        dest: call_dest,
        next,
    });
    lb.switch_to(next);

    if matches!(real_ret_ty, Ty::Unit) {
        lb.locals[ret_slot as usize].ty = Ty::Var(0);
    } else {
        lb.assign(ret_slot, Rvalue::Box { value: Operand::Local(call_dest), ty: real_ret_ty.clone() });
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: real_param_tys.len(),
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let dest = b.declare_local("_fnref_ptr", Ty::Error);
    b.assign(dest, Rvalue::Use(Operand::Global(wrap_name)));
    Operand::Local(dest)
}

/// Derive the Certo type of a MIR operand from its constant or declared local type.
fn infer_operand_ty(op: &Operand, b: &Builder) -> Ty {
    match op {
        Operand::Const(c) => match c {
            MirConst::Unit       => Ty::Unit,
            MirConst::Int(_)     => Ty::Int,
            MirConst::Float(_)   => Ty::Float,
            MirConst::Bool(_)    => Ty::Bool,
            MirConst::Str(_)     => Ty::Text,
            MirConst::Decimal(_) => Ty::Float, // Decimal compiles as double
            MirConst::Uuid(_)    => Ty::Text,
        },
        Operand::Local(id) => b.locals
            .get(*id as usize)
            .map(|l| l.ty.clone())
            .unwrap_or(Ty::Error),
        Operand::Global(_) => Ty::Error,
    }
}

/// A payload type that doesn't fit `certo_result_t`'s pointer-sized `intptr_t`
/// slot as-is and was heap-boxed on construction (see the `Ok`/`Err` special
/// case in `HirExprKind::Call` below) — `Decimal`/`UUID` are themselves
/// multi-field C structs, and any other user-declared `Ty::Named` could be a
/// record or a payload-carrying sum type (also a real struct); the handful
/// of opaque int64-handle types are excluded since those already fit.
/// Conservative on purpose: a nullary-enum `Named` type would technically
/// also fit unboxed, but boxing it anyway is harmless (one extra small
/// allocation), whereas *not* boxing an actual struct is a C compile error —
/// see BACKLOG item 114.
fn needs_result_box(ty: &Ty) -> bool {
    matches!(ty, Ty::Decimal(_) | Ty::Uuid)
        || matches!(ty, Ty::Named { name, args } if args.is_empty()
            && !matches!(name.as_str(),
                "HttpRequest" | "HttpResponse" | "Bytes" | "DbResult" | "Query" | "Mutation" | "__CertoTask"))
}

/// Extract a `Result` payload from `__result_unwrap` into `dest`, shared by
/// both `Ok(v)`/`Err(e)` match-arm binding and the `?` (Try) desugar. The
/// payload slot is pointer-sized, so anything that doesn't naturally fit
/// needs a conversion: `Float` comes back as raw int64 bits and must be
/// bit-restored via `__certo_i2f`; a `needs_result_box` type was heap-boxed
/// on construction and must be dereferenced via `Rvalue::UnboxSome`;
/// everything else already fits and is used as-is.
fn unwrap_result_into(b: &mut Builder, scrut: &Operand, payload_ty: &Ty, dest: MirLocal) {
    if matches!(payload_ty, Ty::Float) {
        let bits = b.declare_local("_bits", Ty::Int);
        let next = b.new_block();
        b.terminate(Terminator::Call {
            func: Operand::Global("__result_unwrap".into()),
            args: vec![scrut.clone()],
            dest: bits,
            next,
        });
        b.switch_to(next);
        b.assign(dest, Rvalue::Call {
            func: Operand::Global("__certo_i2f".into()),
            args: vec![Operand::Local(bits)],
        });
    } else if needs_result_box(payload_ty) {
        let boxed = b.declare_local("_boxed", Ty::Int);
        let next = b.new_block();
        b.terminate(Terminator::Call {
            func: Operand::Global("__result_unwrap".into()),
            args: vec![scrut.clone()],
            dest: boxed,
            next,
        });
        b.switch_to(next);
        b.assign(dest, Rvalue::UnboxSome { opt: Operand::Local(boxed), ty: payload_ty.clone() });
    } else {
        let next = b.new_block();
        b.terminate(Terminator::Call {
            func: Operand::Global("__result_unwrap".into()),
            args: vec![scrut.clone()],
            dest,
            next,
        });
        b.switch_to(next);
    }
}

/// Bind a match-arm's `Ok(v)`/`Err(e)` payload local; see `unwrap_result_into`.
fn unwrap_result_payload(b: &mut Builder, scrut: &Operand, hir_local: LocalId, name: &str, payload_ty: &Ty) {
    let ml = b.map_hir_local(hir_local, name, payload_ty.clone());
    unwrap_result_into(b, scrut, payload_ty, ml);
}

/// Best-effort result type for a binary operation, used when HIR lowering left
/// the node's type as `Ty::Error` (e.g. expressions synthesized by the f-string
/// desugar). Comparisons/logicals yield `Bool`, concat yields `Text`, and
/// arithmetic follows its operands.
fn binop_result_ty(op: &certo_hir::BinOp, l: &Operand, r: &Operand, b: &Builder) -> Ty {
    use certo_hir::BinOp::*;
    match op {
        Eq | NotEq | Lt | LtEq | Gt | GtEq | And | Or => Ty::Bool,
        Concat => Ty::Text,
        NullCoalesce => match infer_operand_ty(l, b) {
            Ty::Option(inner) => *inner,
            _ => infer_operand_ty(r, b),
        },
        Add | Sub | Mul | Div | Rem | Pow => match infer_operand_ty(l, b) {
            Ty::Error => infer_operand_ty(r, b),
            t => t,
        },
    }
}

// ------------------------------------------------------------------ //
// Expression lowering — returns the Operand holding the result
// ------------------------------------------------------------------ //

fn lower_expr(expr: &HirExpr, b: &mut Builder) -> Operand {
    match &expr.kind {
        HirExprKind::Int(n)     => Operand::Const(MirConst::Int(*n)),
        HirExprKind::Float(f)   => Operand::Const(MirConst::Float(*f)),
        HirExprKind::Decimal(s) => Operand::Const(MirConst::Decimal(s.clone())),
        HirExprKind::Bool(v)    => Operand::Const(MirConst::Bool(*v)),
        HirExprKind::Str(s)     => Operand::Const(MirConst::Str(s.clone())),
        HirExprKind::Uuid(u)    => Operand::Const(MirConst::Uuid(u.clone())),
        HirExprKind::Unit       => Operand::Const(MirConst::Unit),

        HirExprKind::Local(id)    => Operand::Local(b.get_local(*id)),
        HirExprKind::Global(name) => Operand::Global(name.clone()),

        HirExprKind::BinOp { op, lhs, rhs } => {
            let l = lower_expr(lhs, b);
            let r = lower_expr(rhs, b);
            // HIR lowering leaves most node types as `Ty::Error`; bindings recover
            // the real type via `infer_operand_ty`, but a temp binop dest used to
            // copy the Error verbatim. That bit f-string interpolations of inline
            // expressions (`f"{2 + 3}"`): the Error-typed temp meant codegen could
            // not pick the right `*_to_text` conversion and emitted a raw int into
            // `certo_text_concat` → segfault. Infer the result type when HIR didn't.
            let dest_ty = if matches!(expr.ty, Ty::Error) {
                binop_result_ty(op, &l, &r, b)
            } else {
                expr.ty.clone()
            };
            let dest = b.declare_local("_binop", dest_ty);
            b.assign(dest, Rvalue::BinOp { op: op.clone(), lhs: l, rhs: r });
            Operand::Local(dest)
        }

        HirExprKind::UnOp { op, arg } => {
            let a = lower_expr(arg, b);
            let dest = b.declare_local("_unop", expr.ty.clone());
            b.assign(dest, Rvalue::UnOp { op: op.clone(), arg: a });
            Operand::Local(dest)
        }

        HirExprKind::Call { func, args } => {
            // `Some(v)` is heap-boxed so the payload survives (incl. Float bits and
            // `Some(0)` vs `None`). Intercept the constructor before the generic call.
            if let HirExprKind::Global(name) = &func.kind {
                if name == "Some" && args.len() == 1 {
                    let payload_ty = args[0].ty.clone();
                    let value = lower_expr(&args[0], b);
                    let dest = b.declare_local("_some", certo_typeck::Ty::Option(Box::new(payload_ty.clone())));
                    b.assign(dest, Rvalue::BoxSome { value, ty: payload_ty });
                    return Operand::Local(dest);
                }
                // `Ok(f)` / `Err(f)` with a Float payload: bit-cast the double to
                // int64 first, so the pointer-sized Result payload keeps its bits.
                if (name == "Ok" || name == "Err") && args.len() == 1
                    && matches!(args[0].ty, certo_typeck::Ty::Float)
                {
                    let value = lower_expr(&args[0], b);
                    let bits = b.declare_local("_fbits", certo_typeck::Ty::Int);
                    b.assign(bits, Rvalue::Call {
                        func: Operand::Global("__certo_f2i".into()),
                        args: vec![value],
                    });
                    let dest = b.declare_local("_res", expr.ty.clone());
                    let next = b.new_block();
                    b.terminate(Terminator::Call {
                        func: Operand::Global(name.clone()),
                        args: vec![Operand::Local(bits)],
                        dest,
                        next,
                    });
                    b.switch_to(next);
                    return Operand::Local(dest);
                }
                // `Ok(v)` / `Err(e)` with a struct-shaped payload (`Decimal`,
                // `UUID`, or any user-declared record/sum type): heap-box it
                // the same way `Some(v)` already does, since it doesn't fit
                // the pointer-sized `intptr_t` slot `certo_ok`/`certo_err`
                // take — see `needs_result_box` (BACKLOG item 114).
                if (name == "Ok" || name == "Err") && args.len() == 1
                    && needs_result_box(&args[0].ty)
                {
                    let payload_ty = args[0].ty.clone();
                    let value = lower_expr(&args[0], b);
                    let boxed = b.declare_local("_boxed", certo_typeck::Ty::Error);
                    b.assign(boxed, Rvalue::BoxSome { value, ty: payload_ty });
                    let dest = b.declare_local("_res", expr.ty.clone());
                    let next = b.new_block();
                    b.terminate(Terminator::Call {
                        func: Operand::Global(name.clone()),
                        args: vec![Operand::Local(boxed)],
                        dest,
                        next,
                    });
                    b.switch_to(next);
                    return Operand::Local(dest);
                }
            }
            let func_op = lower_expr(func, b);
            // A lambda literal passed directly to one of BOXED_ABI_CALLEES
            // needs the boxed-ABI lift instead of the normal native one —
            // see lower_lambda_boxed's doc comment (BACKLOG item 112).
            let needs_boxed_callback = matches!(&func.kind, HirExprKind::Global(name) if BOXED_ABI_CALLEES.contains(&name.as_str()));
            // The callback's param type is almost never annotated in source
            // (`(x) => ...`) — recover it from the scrutinee list's own
            // known element type instead (all BOXED_ABI_CALLEES take
            // `(List<T>, T => ...)`, so it's always the first argument).
            let elem_ty_hint: Option<Ty> = if needs_boxed_callback {
                args.first().and_then(|a| match &a.ty { Ty::List(inner) => Some((**inner).clone()), _ => None })
            } else {
                None
            };
            // A generic sum-type variant constructor call (e.g. `Secret(42)`)
            // must heap-box any argument whose corresponding *declared*
            // field type is a bare type parameter (`Ty::Var(_)`) — its C
            // storage is `void*` regardless of the concrete type instantiated
            // here, mirroring how `Some`/`Ok`/`Err` box their own payload
            // above — BACKLOG item 119.
            let variant_field_types: Option<Vec<Ty>> = match &func.kind {
                HirExprKind::Global(name) => b.variant_field_types.get(name).cloned(),
                _ => None,
            };
            // Declared param types for a plain (non-constructor) call to a
            // user-defined generic function/impl-method, e.g. `wrap(v: T)`
            // — `variant_field_types` above only covers sum-type
            // constructors. Same boxing need: a concrete argument passed
            // into a bare-`T` parameter must be heap-boxed, since the C
            // signature's `T` is uniformly `void*` — BACKLOG item 120
            // (the call-site argument half; the callee's own bare-`T`
            // *return* value isn't recoverable to a concrete type at an
            // arbitrary call site with current inference, so that half
            // remains a design sketch — see the BACKLOG entry).
            let fn_param_tys: Option<Vec<Ty>> = match &func.kind {
                HirExprKind::Global(name) => b.fn_param_tys.get(name).cloned(),
                _ => None,
            };
            let arg_ops: Vec<Operand> = args.iter().enumerate().map(|(i, a)| {
                if needs_boxed_callback {
                    if let HirExprKind::Lambda { params, body } = &a.kind {
                        return lower_lambda_boxed(params, body, elem_ty_hint.as_ref(), b);
                    }
                    // A named function reference (`dbQueryTyped(..., widgetsFromRow)`),
                    // as opposed to an inline lambda — its real signature is
                    // already fully known via its own resolved `Ty::Fn`
                    // (populated from `fn_params`/`fn_ret_types` in HIR's
                    // `Expr::Path` lowering), so no `elem_ty_hint` fallback
                    // is needed the way an unannotated lambda param requires
                    // — BACKLOG item 134.
                    if let (HirExprKind::Global(fn_name), Ty::Fn { params, ret }) = (&a.kind, &a.ty) {
                        return lower_named_fn_boxed(fn_name, params, ret, b);
                    }
                }
                let value_op = lower_expr(a, b);
                // Box only when the declared param/field is a bare type-param
                // AND the argument's own type is a *known concrete* type —
                // if the argument is itself `Ty::Var(_)` (e.g. `v` inside a
                // generic `fn wrap<T>(v: T) = Secret(v)`, where `v` is
                // already an opaque, already-boxed `void*` coming from
                // `wrap`'s own caller), boxing it again would wrap an extra,
                // spurious level of pointer indirection around a value MIR
                // has no way to interpret — the same double-boxing failure
                // class item 134 fixed for `List.first`/etc. — BACKLOG item 120.
                let declared = variant_field_types.as_ref().and_then(|tys| tys.get(i))
                    .or_else(|| fn_param_tys.as_ref().and_then(|tys| tys.get(i)));
                if matches!(declared, Some(Ty::Var(_))) && !matches!(a.ty, Ty::Var(_)) {
                    let boxed = b.declare_local("_boxed_arg", Ty::Var(0));
                    b.assign(boxed, Rvalue::BoxSome { value: value_op, ty: a.ty.clone() });
                    return Operand::Local(boxed);
                }
                value_op
            }).collect();
            // `List.getOrPanic` (and any similarly-shaped stdlib function
            // returning a bare, unwrapped element type) always returns a raw
            // `void*` at the C level, regardless of the logical element type
            // HIR now recovers via `generic_container_ret` (BACKLOG item
            // 113) — when that logical type needs bit-preservation
            // (`Float`) or is a real struct that doesn't fit a pointer-sized
            // slot at all (`Decimal`, `UUID`, a user record/sum type —
            // BACKLOG item 134), the raw pointer must be unboxed rather than
            // assigned directly, or the bits get numeric-converted (Float)
            // or the assignment doesn't even compile (a struct).
            const RAW_RETURN_CALLEES: &[&str] = &["List.getOrPanic"];
            let needs_return_unbox = matches!(&func.kind, HirExprKind::Global(name) if RAW_RETURN_CALLEES.contains(&name.as_str()))
                && (matches!(expr.ty, Ty::Float) || expr.ty.needs_heap_box());

            // `List.first`/`.last`/`.get`/`.find`, `Map.get`, `Query.first`
            // all box their `Option<T>` payload via the C runtime's generic
            // `__certo_opt_box` (always allocates one `int64_t`-sized box),
            // which double-boxes when `T` itself needs real heap-boxing —
            // `List<T>`'s own storage already heap-boxes struct elements
            // (BACKLOG item 134), so the raw payload `__certo_opt_box` boxes
            // is already a `T*`, not a bit-pattern to box fresh. Needs one
            // extra level of pointer unwrap, done null-safely (`Rvalue::
            // UnwrapOptStructBox`) so an empty list / not-found `None`
            // isn't mistaken for a real value.
            const OPT_UNWRAP_CALLEES: &[&str] =
                &["List.first", "List.last", "List.get", "List.find", "Map.get", "Query.first"];
            let needs_opt_unwrap = matches!(&func.kind, HirExprKind::Global(name) if OPT_UNWRAP_CALLEES.contains(&name.as_str()))
                && matches!(&expr.ty, Ty::Option(inner) if inner.needs_heap_box());

            let dest = b.declare_local("_call", if needs_return_unbox { Ty::Var(0) } else { expr.ty.clone() });
            let next = b.new_block();
            b.terminate(Terminator::Call { func: func_op, args: arg_ops, dest, next });
            b.switch_to(next);
            if needs_return_unbox {
                let real = b.declare_local("_call_unboxed", expr.ty.clone());
                b.assign(real, Rvalue::Unbox { value: Operand::Local(dest), ty: expr.ty.clone() });
                Operand::Local(real)
            } else if needs_opt_unwrap {
                let real = b.declare_local("_call_opt_unwrapped", expr.ty.clone());
                b.assign(real, Rvalue::UnwrapOptStructBox { value: Operand::Local(dest), ty: expr.ty.clone() });
                Operand::Local(real)
            } else {
                Operand::Local(dest)
            }
        }

        HirExprKind::If { cond, then_expr, else_expr } => {
            let cond_op = lower_expr(cond, b);
            let then_bb = b.new_block();
            let else_bb = b.new_block();
            let join_bb = b.new_block();
            b.terminate(Terminator::If { cond: cond_op, true_bb: then_bb, false_bb: else_bb });

            // Use int64_t for Unit results — avoids certo_unit_t ↔ int64_t mismatches
            // when arm bodies are stdlib calls typed as Ty::Error.
            let if_ty = if matches!(expr.ty, Ty::Unit) { Ty::Error } else { expr.ty.clone() };
            let result = b.declare_local("_if", if_ty);

            b.switch_to(then_bb);
            let then_op = lower_expr(then_expr, b);
            // See the analogous patch in the Match case below: HIR often
            // can't infer expr.ty for a computed branch body, so recover it
            // from the branch's own operand once it's known.
            if matches!(b.locals[result as usize].ty, Ty::Error) {
                let inferred = infer_operand_ty(&then_op, b);
                if !matches!(inferred, Ty::Error) {
                    b.locals[result as usize].ty = inferred;
                }
            }
            b.assign(result, Rvalue::Use(then_op));
            b.terminate(Terminator::Goto(join_bb));

            b.switch_to(else_bb);
            let else_op = lower_expr(else_expr, b);
            if matches!(b.locals[result as usize].ty, Ty::Error) {
                let inferred = infer_operand_ty(&else_op, b);
                if !matches!(inferred, Ty::Error) {
                    b.locals[result as usize].ty = inferred;
                }
            }
            b.assign(result, Rvalue::Use(else_op));
            b.terminate(Terminator::Goto(join_bb));

            b.switch_to(join_bb);
            Operand::Local(result)
        }

        HirExprKind::Block { stmts, tail } => {
            for stmt in stmts { lower_stmt(stmt, b); }
            lower_expr(tail, b)
        }

        HirExprKind::Match { scrutinee, arms } => {
            let scrut_op = lower_expr(scrutinee, b);
            // Payload type for `Some(x)` extraction (Option is heap-boxed).
            let opt_payload = match &scrutinee.ty {
                Ty::Option(inner) => (**inner).clone(),
                _ => Ty::Error,
            };
            // Payload types for `Ok(x)`/`Err(e)` — needed to bit-restore a Float
            // that was stored in the pointer-sized Result payload slot.
            let (ok_ty, err_ty) = match &scrutinee.ty {
                Ty::Result(t, e) => ((**t).clone(), (**e).clone()),
                _ => (Ty::Error, Ty::Error),
            };
            let join_bb  = b.new_block();
            let match_ty = if matches!(expr.ty, Ty::Unit) { Ty::Error } else { expr.ty.clone() };
            let result   = b.declare_local("_match", match_ty);

            for arm in arms {
                let arm_bb = b.new_block();
                // next_arm_bb: where to go if this arm's pattern or guard fails.
                // Filled in below; starts as a fresh block that becomes the
                // entry point of the next arm's pattern check.
                let next_arm_bb = b.new_block();

                // ── Pattern check ────────────────────────────────────
                match &arm.pat {
                    HirPat::Lit(lit) => {
                        let expected = match lit {
                            HirLitPat::Int(n)  => Operand::Const(MirConst::Int(*n)),
                            HirLitPat::Bool(v) => Operand::Const(MirConst::Bool(*v)),
                            HirLitPat::Str(s)  => Operand::Const(MirConst::Str(s.clone())),
                        };
                        let cmp = b.declare_local("_cmp", Ty::Bool);
                        b.assign(cmp, Rvalue::BinOp {
                            op:  certo_hir::BinOp::Eq,
                            lhs: scrut_op.clone(),
                            rhs: expected,
                        });
                        // On pattern match → check guard (or go straight to arm).
                        // On pattern fail → try next arm.
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: after_pat, false_bb: next_arm_bb });
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    HirPat::Bind { local, name } => {
                        let mir_local = b.map_hir_local(*local, name, Ty::Error);
                        b.assign(mir_local, Rvalue::Use(scrut_op.clone()));
                        // Pattern always matches — still need to check guard.
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    HirPat::Wildcard => {
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    HirPat::Constructor { name, fields, field_names, field_types } => {
                        // Special-case built-in Option/Result constructors which use
                        // pointer/struct representations rather than tagged-union enums.
                        match name.as_str() {
                            "None" => {
                                // None: scrutinee == NULL
                                let cmp = b.declare_local("_cmp", Ty::Bool);
                                b.assign(cmp, Rvalue::BinOp {
                                    op: certo_hir::BinOp::Eq,
                                    lhs: scrut_op.clone(),
                                    rhs: Operand::Global("__NULL".into()),
                                });
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: after_pat, false_bb: next_arm_bb });
                                if arm.guard.is_some() { b.switch_to(after_pat); }
                            }
                            "Some" => {
                                // Some(n): scrutinee != NULL; bind n = *(payload*)scrutinee.
                                let cmp = b.declare_local("_cmp", Ty::Bool);
                                b.assign(cmp, Rvalue::BinOp {
                                    op: certo_hir::BinOp::NotEq,
                                    lhs: scrut_op.clone(),
                                    rhs: Operand::Global("__NULL".into()),
                                });
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: after_pat, false_bb: next_arm_bb });
                                // Dereference the payload only in the matched (non-null)
                                // block — never on the `None`/NULL path.
                                b.switch_to(after_pat);
                                for field_pat in fields.iter() {
                                    if let HirPat::Bind { local, name: fname } = field_pat {
                                        let ml = b.map_hir_local(*local, fname, opt_payload.clone());
                                        b.assign(ml, Rvalue::UnboxSome {
                                            opt: scrut_op.clone(),
                                            ty:  opt_payload.clone(),
                                        });
                                    }
                                }
                            }
                            "Ok" => {
                                // Ok(v): __result_is_ok(scrutinee); bind v = __result_unwrap(scrutinee)
                                let is_ok = b.declare_local("_is_ok", Ty::Bool);
                                let next_bb = b.new_block();
                                b.terminate(Terminator::Call {
                                    func: Operand::Global("__result_is_ok".into()),
                                    args: vec![scrut_op.clone()],
                                    dest: is_ok,
                                    next: next_bb,
                                });
                                b.switch_to(next_bb);
                                for field_pat in fields.iter() {
                                    if let HirPat::Bind { local, name: fname } = field_pat {
                                        unwrap_result_payload(b, &scrut_op, *local, fname, &ok_ty);
                                    }
                                }
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                b.terminate(Terminator::If { cond: Operand::Local(is_ok), true_bb: after_pat, false_bb: next_arm_bb });
                                if arm.guard.is_some() { b.switch_to(after_pat); }
                            }
                            "Err" => {
                                // Err(e): !__result_is_ok(scrutinee); bind e = __result_unwrap(scrutinee)
                                let is_ok = b.declare_local("_is_ok", Ty::Bool);
                                let next_bb = b.new_block();
                                b.terminate(Terminator::Call {
                                    func: Operand::Global("__result_is_ok".into()),
                                    args: vec![scrut_op.clone()],
                                    dest: is_ok,
                                    next: next_bb,
                                });
                                b.switch_to(next_bb);
                                for field_pat in fields.iter() {
                                    if let HirPat::Bind { local, name: fname } = field_pat {
                                        unwrap_result_payload(b, &scrut_op, *local, fname, &err_ty);
                                    }
                                }
                                let not_ok = b.declare_local("_not_ok", Ty::Bool);
                                b.assign(not_ok, Rvalue::UnOp { op: certo_hir::UnOp::Not, arg: Operand::Local(is_ok) });
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                b.terminate(Terminator::If { cond: Operand::Local(not_ok), true_bb: after_pat, false_bb: next_arm_bb });
                                if arm.guard.is_some() { b.switch_to(after_pat); }
                            }
                            _ => {
                                // User-defined sum type: compare .tag field to variant constant.
                                let tag_local = b.declare_local("_tag", Ty::Int);
                                b.assign(tag_local, Rvalue::Field { base: scrut_op.clone(), field: "tag".into() });
                                // The union member is named after the variant (lowercased).
                                // Payload fields use whatever name the codegen-emitted struct
                                // gave them — the variant's declared field name, or the
                                // positional fallback `f<i>` — per `field_names[i]` (computed
                                // in HIR lowering from the same rule `emit_module.rs` uses).
                                // `name` here is the fully-qualified `Type__Variant`, so take
                                // the last segment.
                                let variant = name.rsplit("__").next().unwrap_or(name).to_lowercase();
                                // Substitute a bare-type-param field's declared
                                // `Ty::Var(_)` with the scrutinee's own recovered
                                // instantiation argument (single-type-param scope
                                // — `args.first()`) and unbox the read, mirroring
                                // `Some`/`Ok`/`Err`'s existing boxed-payload
                                // handling — BACKLOG item 119. Only unbox when the
                                // recovered argument is itself a *known concrete*
                                // type — if `scrut_args.first()` is itself
                                // `Ty::Var(_)` (the scrutinee's own instantiation is
                                // still abstract, e.g. matching `s: Secret<T>` inside
                                // a generic `fn expose<T>(s: Secret<T>): T`), the
                                // field was never (re-)boxed on construction either
                                // (see the Record/Call-arg guards above), so
                                // unboxing here would strip a level of pointer
                                // indirection that was never added — BACKLOG item 120.
                                let scrut_args: &[Ty] = match &scrutinee.ty { Ty::Named { args, .. } => args, _ => &[] };
                                for (i, field_pat) in fields.iter().enumerate() {
                                    if let HirPat::Bind { local, name: fname } = field_pat {
                                        let declared = &field_types[i];
                                        let (real_ty, needs_unbox) = match declared {
                                            Ty::Var(_) => {
                                                let concrete = scrut_args.first().cloned().unwrap_or(Ty::Error);
                                                let needs_unbox = !matches!(concrete, Ty::Var(_));
                                                (concrete, needs_unbox)
                                            }
                                            other => (other.clone(), false),
                                        };
                                        let ml = b.map_hir_local(*local, fname, real_ty.clone());
                                        // Read the payload directly via a nested path
                                        // `scrut.<variant>.<field>` — avoids an intermediate
                                        // local whose (anonymous struct) type we can't name.
                                        if needs_unbox {
                                            let raw = b.declare_local("_variant_field_raw", Ty::Var(0));
                                            b.assign(raw, Rvalue::Field {
                                                base: scrut_op.clone(),
                                                field: format!("{}.{}", variant, field_names[i]),
                                            });
                                            b.assign(ml, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: real_ty });
                                        } else {
                                            b.assign(ml, Rvalue::Field {
                                                base: scrut_op.clone(),
                                                field: format!("{}.{}", variant, field_names[i]),
                                            });
                                        }
                                    }
                                }
                                let cmp = b.declare_local("_cmp", Ty::Bool);
                                let variant_tag = Operand::Global(format!("__tag__{}", name));
                                b.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::Eq, lhs: Operand::Local(tag_local), rhs: variant_tag });
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: after_pat, false_bb: next_arm_bb });
                                if arm.guard.is_some() { b.switch_to(after_pat); }
                            }
                        }
                    }
                    HirPat::Tuple(fields) => {
                        // Tuple is stored as a CertoList*. Extract each field by index.
                        for (i, field_pat) in fields.iter().enumerate() {
                            if let HirPat::Bind { local, name: fname } = field_pat {
                                let elem_local = b.map_hir_local(*local, fname, Ty::Error);
                                let next_bb = b.new_block();
                                b.terminate(Terminator::Call {
                                    func: Operand::Global("__tuple_get".into()),
                                    args: vec![
                                        scrut_op.clone(),
                                        Operand::Const(MirConst::Int(i as i64)),
                                    ],
                                    dest: elem_local,
                                    next: next_bb,
                                });
                                b.switch_to(next_bb);
                            }
                        }
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    _ => {
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                }

                // ── Guard check (if present) ─────────────────────────
                // We are now in `after_pat` (only reached this block when
                // the guard is Some). Evaluate it and branch.
                if let Some(guard_expr) = &arm.guard {
                    let guard_op = lower_expr(guard_expr, b);
                    b.terminate(Terminator::If {
                        cond:     guard_op,
                        true_bb:  arm_bb,
                        false_bb: next_arm_bb,
                    });
                }

                // ── Arm body ─────────────────────────────────────────
                b.switch_to(arm_bb);
                let arm_op = lower_expr(&arm.body, b);
                // HIR often can't infer expr.ty for a computed arm body
                // (`Circle(r) => r * r * 3.14159` is a BinOp, always
                // Ty::Error at the HIR level — see the BinOp case above).
                // Recover the join local's real type from the first arm
                // whose operand reveals one, same idiom as `lower_fn`'s
                // return-type patch-after-the-fact via `infer_operand_ty`.
                if matches!(b.locals[result as usize].ty, Ty::Error) {
                    let inferred = infer_operand_ty(&arm_op, b);
                    if !matches!(inferred, Ty::Error) {
                        b.locals[result as usize].ty = inferred;
                    }
                }
                b.assign(result, Rvalue::Use(arm_op));
                b.terminate(Terminator::Goto(join_bb));

                // Next iteration will emit into next_arm_bb.
                b.switch_to(next_arm_bb);
            }

            // Any unmatched falls through to Unreachable.
            b.terminate(Terminator::Unreachable);
            b.switch_to(join_bb);
            Operand::Local(result)
        }

        HirExprKind::Record { fields, field_types } => {
            // A field whose *declared* type is a bare type parameter
            // (`Ty::Var(_)`) is stored as `void*` in the struct regardless of
            // its concrete instantiation, so its value must be heap-boxed
            // the same way `Some(v)` boxes an Option payload — BACKLOG item 119.
            // Skip boxing when the value is *itself* already `Ty::Var(_)`
            // (already opaque/boxed, e.g. inside a generic function's own
            // body) — same double-boxing guard as the Call-args case above,
            // BACKLOG item 120.
            let ops: Vec<Operand> = fields.iter().zip(field_types.iter()).map(|((_, v), fty)| {
                let value_op = lower_expr(v, b);
                if matches!(fty, Ty::Var(_)) && !matches!(v.ty, Ty::Var(_)) {
                    let boxed = b.declare_local("_boxed_field", Ty::Var(0));
                    b.assign(boxed, Rvalue::BoxSome { value: value_op, ty: v.ty.clone() });
                    Operand::Local(boxed)
                } else {
                    value_op
                }
            }).collect();
            let names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
            let dest = b.declare_local("_rec", expr.ty.clone());
            b.assign(dest, Rvalue::Aggregate(AggregateKind::Record(names), ops));
            Operand::Local(dest)
        }

        HirExprKind::Tuple(elems) => {
            let ops: Vec<Operand> = elems.iter().map(|e| lower_expr(e, b)).collect();
            let dest = b.declare_local("_tup", expr.ty.clone());
            b.assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            Operand::Local(dest)
        }

        HirExprKind::List(elems) => {
            let ops: Vec<Operand> = elems.iter().map(|e| lower_expr(e, b)).collect();
            let dest = b.declare_local("_arr", expr.ty.clone());
            b.assign(dest, Rvalue::Aggregate(AggregateKind::Array, ops));
            Operand::Local(dest)
        }

        HirExprKind::Field { base, field, boxed } => {
            let base_op = lower_expr(base, b);
            if *boxed {
                // Declared as a bare type-param field — the raw `.field` read
                // is a `void*` box that must be unboxed to the substituted
                // concrete type (`expr.ty`) — BACKLOG item 119.
                let raw = b.declare_local(&format!("_field_{}", field), Ty::Var(0));
                b.assign(raw, Rvalue::Field { base: base_op, field: field.clone() });
                let dest = b.declare_local(&format!("_field_{}_unboxed", field), expr.ty.clone());
                b.assign(dest, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: expr.ty.clone() });
                Operand::Local(dest)
            } else {
                let dest = b.declare_local(&format!("_field_{}", field), expr.ty.clone());
                b.assign(dest, Rvalue::Field { base: base_op, field: field.clone() });
                Operand::Local(dest)
            }
        }

        HirExprKind::Lambda { params, body } => {
            // Lift the lambda to a top-level MIR function named `__lam_<parent>_N`.
            let idx = b.lambda_count;
            b.lambda_count += 1;
            let lam_name = format!("__lam_{}_{}", b.fn_name, idx);

            // Build MIR for the lambda body using a fresh builder.
            let mut lb = Builder::new(&lam_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone());
            let ret_slot = lb.declare_local("_ret", Ty::Error);
            for p in params {
                lb.map_hir_local(p.local, &p.name, p.ty.clone());
            }
            lb.current_span = body.span;
            let lam_result = lower_expr(body, &mut lb);
            let lam_ret_ty = infer_operand_ty(&lam_result, &lb);
            lb.locals[ret_slot as usize].ty = lam_ret_ty.clone();
            if !matches!(lam_ret_ty, Ty::Unit) {
                lb.assign(ret_slot, Rvalue::Use(lam_result));
            }
            emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

            let lam_fn = MirFn {
                name: lam_name.clone(),
                param_count: params.len(),
                locals: lb.locals,
                blocks: lb.blocks,
            };
            // Propagate any nested lambdas lifted inside this lambda.
            b.lifted_fns.extend(lb.lifted_fns);
            b.lifted_fns.push(lam_fn);

            // Represent the lambda as a function pointer (int64_t cast of the global address).
            let dest = b.declare_local("_lam_ptr", Ty::Error);
            b.assign(dest, Rvalue::Use(Operand::Global(lam_name)));
            Operand::Local(dest)
        }

        HirExprKind::Try(inner) => {
            // Desugar `e?` into:
            //   _result = e
            //   _is_ok  = __result_is_ok(_result)     // bb: check
            //   if _is_ok goto bb_ok else goto bb_err  // bb: branch
            // bb_err:
            //   return _result                         // propagate Err
            // bb_ok:
            //   _val = __result_unwrap(_result)        // extract Ok value
            //   ... (continue)

            let inner_op = lower_expr(inner, b);

            // Store the result value so we can reference it in both branches.
            let result_local = b.declare_local("_result", Ty::Error); // void*
            b.assign(result_local, Rvalue::Use(inner_op));

            // Call __result_is_ok → bool
            let is_ok_local  = b.declare_local("_is_ok", Ty::Bool);
            let after_is_ok  = b.new_block();
            b.terminate(Terminator::Call {
                func: Operand::Global("__result_is_ok".into()),
                args: vec![Operand::Local(result_local)],
                dest: is_ok_local,
                next: after_is_ok,
            });
            b.switch_to(after_is_ok);

            // Branch on is_ok.
            let ok_block  = b.new_block();
            let err_block = b.new_block();
            b.terminate(Terminator::If {
                cond:     Operand::Local(is_ok_local),
                true_bb:  ok_block,
                false_bb: err_block,
            });

            // err_block: run defers then propagate the Err.
            b.switch_to(err_block);
            emit_defers_then_return(Operand::Local(result_local), b);

            // ok_block: extract the Ok payload.
            b.switch_to(ok_block);
            let val_local = b.declare_local("_try_val", expr.ty.clone());
            unwrap_result_into(b, &Operand::Local(result_local), &expr.ty, val_local);
            Operand::Local(val_local)
        }

        HirExprKind::Unsafe(inner) => lower_expr(inner, b),

        HirExprKind::For { binding, binding_name, binding_ty, iter, body } => {
            // Evaluate the iterable once.
            let iter_op = lower_expr(iter, b);
            // Derive element type from the list type if available, otherwise use binding_ty.
            let elem_ty = match &iter.ty {
                Ty::List(inner) => *inner.clone(),
                _ => binding_ty.clone(),
            };

            // Obtain the length: _len = List.len(iter)
            let len_local = b.declare_local("_len", Ty::Int);
            let len_done  = b.new_block();
            b.terminate(Terminator::Call {
                func: Operand::Global("List.len".into()),
                args: vec![iter_op.clone()],
                dest: len_local,
                next: len_done,
            });
            b.switch_to(len_done);

            // Loop index: _i = 0
            let i_local = b.declare_local("_i", Ty::Int);
            b.assign(i_local, Rvalue::Use(Operand::Const(MirConst::Int(0))));

            let loop_test_bb = b.new_block();
            let loop_body_bb = b.new_block();
            let loop_exit_bb = b.new_block();
            b.terminate(Terminator::Goto(loop_test_bb));

            // loop_test: if _i < _len goto loop_body else loop_exit
            b.switch_to(loop_test_bb);
            let cmp = b.declare_local("_for_cmp", Ty::Bool);
            b.assign(cmp, Rvalue::BinOp {
                op:  certo_hir::BinOp::Lt,
                lhs: Operand::Local(i_local),
                rhs: Operand::Local(len_local),
            });
            b.terminate(Terminator::If {
                cond:     Operand::Local(cmp),
                true_bb:  loop_body_bb,
                false_bb: loop_exit_bb,
            });

            // loop_body: binding = List.getOrPanic(iter, _i); body; _i = _i + 1
            b.switch_to(loop_body_bb);
            let binding_local = b.map_hir_local(*binding, binding_name, elem_ty.clone());
            let elem_done = b.new_block();
            if matches!(elem_ty, Ty::Float | Ty::Decimal(_)) {
                // Element was stored bit-cast; restore the double from its bits.
                let boxed = b.declare_local("_elem_bits", Ty::Int);
                b.terminate(Terminator::Call {
                    func: Operand::Global("List.getOrPanic".into()),
                    args: vec![iter_op.clone(), Operand::Local(i_local)],
                    dest: boxed,
                    next: elem_done,
                });
                b.switch_to(elem_done);
                b.assign(binding_local, Rvalue::Call {
                    func: Operand::Global("__certo_i2f".into()),
                    args: vec![Operand::Local(boxed)],
                });
            } else {
                b.terminate(Terminator::Call {
                    func: Operand::Global("List.getOrPanic".into()),
                    args: vec![iter_op.clone(), Operand::Local(i_local)],
                    dest: binding_local,
                    next: elem_done,
                });
                b.switch_to(elem_done);
            }
            lower_expr(body, b); // result discarded
            let inc = b.declare_local("_i_inc", Ty::Int);
            b.assign(inc, Rvalue::BinOp {
                op:  certo_hir::BinOp::Add,
                lhs: Operand::Local(i_local),
                rhs: Operand::Const(MirConst::Int(1)),
            });
            b.assign(i_local, Rvalue::Use(Operand::Local(inc)));
            b.terminate(Terminator::Goto(loop_test_bb));

            b.switch_to(loop_exit_bb);
            Operand::Const(MirConst::Unit)
        }

        HirExprKind::While { cond, body } => {
            // loop_header: cond_val = eval(cond); if cond_val → body_bb else exit_bb
            // body_bb:     eval(body); goto loop_header
            // exit_bb:     Unit
            let loop_header_bb = b.new_block();
            let loop_body_bb   = b.new_block();
            let loop_exit_bb   = b.new_block();

            b.terminate(Terminator::Goto(loop_header_bb));
            b.switch_to(loop_header_bb);

            let cond_op = lower_expr(cond, b);
            b.terminate(Terminator::If {
                cond:     cond_op,
                true_bb:  loop_body_bb,
                false_bb: loop_exit_bb,
            });

            b.switch_to(loop_body_bb);
            lower_expr(body, b); // result discarded — while evaluates to Unit
            b.terminate(Terminator::Goto(loop_header_bb));

            b.switch_to(loop_exit_bb);
            Operand::Const(MirConst::Unit)
        }

        HirExprKind::Spawn { fn_name: _, args } => {
            // `spawn f(a, b)` — run f on a new OS thread, evaluating the arguments
            // eagerly in the current thread first. Only a direct call to a named
            // function (Global) can be threaded; anything else falls back to
            // sequential evaluation (still correct, just not concurrent).
            let inner = args.first();
            if let Some(HirExpr { kind: HirExprKind::Call { func, args: call_args }, ty: ret_ty, .. }) = inner {
                let func_op = lower_expr(func, b);
                if let Operand::Global(_) = func_op {
                    let arg_ops: Vec<Operand> = call_args.iter().map(|a| lower_expr(a, b)).collect();
                    let task_ty = certo_typeck::Ty::Named {
                        name: "__CertoTask".into(),
                        args: vec![ret_ty.clone()],
                    };
                    let dest = b.declare_local("__task", task_ty);
                    b.assign(dest, Rvalue::Spawn { func: func_op, args: arg_ops, ret_ty: ret_ty.clone() });
                    return Operand::Local(dest);
                }
            }
            // Fallback: evaluate eagerly (sequential), handle is the value itself.
            let inner_op = inner.map(|e| lower_expr(e, b)).unwrap_or(Operand::Const(MirConst::Int(0)));
            let dest = b.declare_local("__task", certo_typeck::Ty::Error);
            b.assign(dest, Rvalue::Use(inner_op));
            Operand::Local(dest)
        }

        HirExprKind::Await(inner) => {
            let task_op = lower_expr(inner, b);
            // A real threaded task carries a `__CertoTask<R>` handle type; join it.
            // A sequential-fallback handle holds the value directly — just copy it.
            if let certo_typeck::Ty::Named { name, args } = infer_operand_ty(&task_op, b) {
                if name == "__CertoTask" {
                    let ret_ty = args.into_iter().next().unwrap_or(certo_typeck::Ty::Error);
                    let dest = b.declare_local("__await_result", ret_ty.clone());
                    b.assign(dest, Rvalue::Join { task: task_op, ret_ty });
                    return Operand::Local(dest);
                }
            }
            let val_ty = infer_operand_ty(&task_op, b);
            let dest = b.declare_local("__await_result", val_ty);
            b.assign(dest, Rvalue::Use(task_op));
            Operand::Local(dest)
        }

        HirExprKind::AwaitTimed { task, deadline } => {
            let task_op = lower_expr(task, b);
            let deadline_op = Operand::Local(b.get_local(*deadline));
            // Same real-task-vs-sequential-fallback split as plain `Await`.
            if let certo_typeck::Ty::Named { name, args } = infer_operand_ty(&task_op, b) {
                if name == "__CertoTask" {
                    let ret_ty = args.into_iter().next().unwrap_or(certo_typeck::Ty::Error);
                    let dest = b.declare_local("__await_timed_result", ret_ty.clone());
                    b.assign(dest, Rvalue::JoinTimed { task: task_op, deadline: deadline_op, ret_ty });
                    return Operand::Local(dest);
                }
            }
            // Fallback: sequential-eval handle already holds the value
            // directly — nothing to wait on, so the deadline is moot.
            let val_ty = infer_operand_ty(&task_op, b);
            let dest = b.declare_local("__await_timed_result", val_ty);
            b.assign(dest, Rvalue::Use(task_op));
            Operand::Local(dest)
        }
    }
}

fn lower_stmt(stmt: &HirStmt, b: &mut Builder) {
    match stmt {
        HirStmt::Let { local, name, ty, init } => {
            b.current_span = init.span;
            let init_op  = lower_expr(init, b);
            // When the declared type is unknown (Ty::Error), recover it from the
            // initialiser. This preserves e.g. a `__CertoTask<R>` spawn handle so
            // a later `await` can join it with the right result type.
            let decl_ty = if matches!(ty, Ty::Error) { infer_operand_ty(&init_op, b) } else { ty.clone() };
            let mir_local = b.map_hir_local(*local, name, decl_ty);
            b.assign(mir_local, Rvalue::Use(init_op));
        }
        HirStmt::Assign { local, value } => {
            b.current_span = value.span;
            let val_op   = lower_expr(value, b);
            let mir_local = b.get_local(*local);
            b.assign(mir_local, Rvalue::Use(val_op));
        }
        HirStmt::Expr(e) => {
            b.current_span = e.span;
            lower_expr(e, b);
        }
        HirStmt::Defer { body } => {
            b.defers.push(body.clone());
        }
    }
}
