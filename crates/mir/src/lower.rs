use certo_typeck::Ty;
use certo_hir::{HirFn, HirExpr, HirExprKind, HirStmt, HirPat, HirLitPat, LocalId};
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
}

impl Builder {
    fn new(fn_name: &str) -> Self {
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
        self.push_stmt(MirStmt::Assign { dest, rvalue });
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
        lower_expr(body, b);
    }
    b.terminate(Terminator::Return(return_op));
}

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

/// Lower a HIR function to MIR. Returns the primary function plus any lambdas lifted to top level.
pub fn lower_fn(f: &HirFn) -> (MirFn, Vec<MirFn>) {
    let mut b = Builder::new(&f.name);

    // Declare params as locals (index 0 = return slot, type patched below).
    let ret_slot = b.declare_local("_ret", Ty::Error);
    for p in &f.params {
        b.map_hir_local(p.local, &p.name, p.ty.clone());
    }

    let result = if let Some(body) = &f.body {
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
/// A lambda literal passed directly as the callback argument to one of
/// these needs the boxed-ABI treatment (see `lower_lambda_boxed`) — BACKLOG
/// item 112. Deliberately narrow: `List.sort`'s two-argument, Int-returning
/// comparator and user-defined higher-order functions (which already use a
/// consistent native ABI on both sides, see item 108) are out of scope.
const BOXED_ABI_CALLEES: &[&str] = &[
    "List.map", "List.filter", "List.find", "List.any", "List.all", "List.groupBy",
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

    let mut lb = Builder::new(&lam_name);
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

/// Bind a `Result` payload from `__result_unwrap`. The Result payload slot is
/// pointer-sized, so a `Float` comes back as raw int64 bits and must be
/// bit-restored to a `double`; other payload types are used as-is.
fn unwrap_result_payload(b: &mut Builder, scrut: &Operand, hir_local: LocalId, name: &str, payload_ty: &Ty) {
    if matches!(payload_ty, Ty::Float) {
        let bits = b.declare_local("_bits", Ty::Int);
        let unwrap_bb = b.new_block();
        b.terminate(Terminator::Call {
            func: Operand::Global("__result_unwrap".into()),
            args: vec![scrut.clone()],
            dest: bits,
            next: unwrap_bb,
        });
        b.switch_to(unwrap_bb);
        let ml = b.map_hir_local(hir_local, name, Ty::Float);
        b.assign(ml, Rvalue::Call {
            func: Operand::Global("__certo_i2f".into()),
            args: vec![Operand::Local(bits)],
        });
    } else {
        let ml = b.map_hir_local(hir_local, name, Ty::Error);
        let unwrap_bb = b.new_block();
        b.terminate(Terminator::Call {
            func: Operand::Global("__result_unwrap".into()),
            args: vec![scrut.clone()],
            dest: ml,
            next: unwrap_bb,
        });
        b.switch_to(unwrap_bb);
    }
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
            let arg_ops: Vec<Operand> = args.iter().map(|a| {
                if needs_boxed_callback {
                    if let HirExprKind::Lambda { params, body } = &a.kind {
                        return lower_lambda_boxed(params, body, elem_ty_hint.as_ref(), b);
                    }
                }
                lower_expr(a, b)
            }).collect();
            let dest = b.declare_local("_call", expr.ty.clone());
            let next = b.new_block();
            b.terminate(Terminator::Call { func: func_op, args: arg_ops, dest, next });
            b.switch_to(next);
            Operand::Local(dest)
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
                                for (i, field_pat) in fields.iter().enumerate() {
                                    if let HirPat::Bind { local, name: fname } = field_pat {
                                        let ml = b.map_hir_local(*local, fname, field_types[i].clone());
                                        // Read the payload directly via a nested path
                                        // `scrut.<variant>.<field>` — avoids an intermediate
                                        // local whose (anonymous struct) type we can't name.
                                        b.assign(ml, Rvalue::Field {
                                            base: scrut_op.clone(),
                                            field: format!("{}.{}", variant, field_names[i]),
                                        });
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

        HirExprKind::Record(fields) => {
            let ops: Vec<Operand> = fields.iter().map(|(_, v)| lower_expr(v, b)).collect();
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

        HirExprKind::Field { base, field } => {
            let base_op = lower_expr(base, b);
            let dest = b.declare_local(&format!("_field_{}", field), expr.ty.clone());
            b.assign(dest, Rvalue::Field { base: base_op, field: field.clone() });
            Operand::Local(dest)
        }

        HirExprKind::Lambda { params, body } => {
            // Lift the lambda to a top-level MIR function named `__lam_<parent>_N`.
            let idx = b.lambda_count;
            b.lambda_count += 1;
            let lam_name = format!("__lam_{}_{}", b.fn_name, idx);

            // Build MIR for the lambda body using a fresh builder.
            let mut lb = Builder::new(&lam_name);
            let ret_slot = lb.declare_local("_ret", Ty::Error);
            for p in params {
                lb.map_hir_local(p.local, &p.name, p.ty.clone());
            }
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
            let after_unwrap = b.new_block();
            b.terminate(Terminator::Call {
                func: Operand::Global("__result_unwrap".into()),
                args: vec![Operand::Local(result_local)],
                dest: val_local,
                next: after_unwrap,
            });
            b.switch_to(after_unwrap);
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
            if matches!(elem_ty, Ty::Float | Ty::Decimal) {
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
    }
}

fn lower_stmt(stmt: &HirStmt, b: &mut Builder) {
    match stmt {
        HirStmt::Let { local, name, ty, init } => {
            let init_op  = lower_expr(init, b);
            // When the declared type is unknown (Ty::Error), recover it from the
            // initialiser. This preserves e.g. a `__CertoTask<R>` spawn handle so
            // a later `await` can join it with the right result type.
            let decl_ty = if matches!(ty, Ty::Error) { infer_operand_ty(&init_op, b) } else { ty.clone() };
            let mir_local = b.map_hir_local(*local, name, decl_ty);
            b.assign(mir_local, Rvalue::Use(init_op));
        }
        HirStmt::Assign { local, value } => {
            let val_op   = lower_expr(value, b);
            let mir_local = b.get_local(*local);
            b.assign(mir_local, Rvalue::Use(val_op));
        }
        HirStmt::Expr(e) => {
            lower_expr(e, b);
        }
        HirStmt::Defer { body } => {
            b.defers.push(body.clone());
        }
    }
}
