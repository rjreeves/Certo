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
}

impl Builder {
    fn new() -> Self {
        let entry = BasicBlock { id: 0, ..Default::default() };
        Builder {
            locals:    Vec::new(),
            blocks:    vec![entry],
            current:   0,
            local_map: std::collections::HashMap::new(),
            next_tmp:  0,
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
// Entry point
// ------------------------------------------------------------------ //

pub fn lower_fn(f: &HirFn) -> MirFn {
    let mut b = Builder::new();

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
    b.terminate(Terminator::Return(Operand::Local(ret_slot)));

    MirFn { name: f.name.clone(), param_count: f.params.len(), locals: b.locals, blocks: b.blocks }
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
            let dest = b.declare_local("_binop", expr.ty.clone());
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
            let func_op = lower_expr(func, b);
            let arg_ops: Vec<Operand> = args.iter().map(|a| lower_expr(a, b)).collect();
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

            let result = b.declare_local("_if", expr.ty.clone());

            b.switch_to(then_bb);
            let then_op = lower_expr(then_expr, b);
            b.assign(result, Rvalue::Use(then_op));
            b.terminate(Terminator::Goto(join_bb));

            b.switch_to(else_bb);
            let else_op = lower_expr(else_expr, b);
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
            let join_bb  = b.new_block();
            let result   = b.declare_local("_match", expr.ty.clone());

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
            // Model field access as a call to a compiler builtin.
            let dest = b.declare_local(&format!("_field_{}", field), expr.ty.clone());
            let field_fn = Operand::Global(format!("__field_{}", field));
            let next = b.new_block();
            b.terminate(Terminator::Call { func: field_fn, args: vec![base_op], dest, next });
            b.switch_to(next);
            Operand::Local(dest)
        }

        HirExprKind::Lambda { params, body } => {
            // Closures — represent as a global reference for now (full closure lifting later).
            let dest = b.declare_local("_lambda", expr.ty.clone());
            b.assign(dest, Rvalue::Use(Operand::Global("__lambda".into())));
            Operand::Local(dest)
        }

        HirExprKind::Try(inner) => {
            let inner_op = lower_expr(inner, b);
            let dest = b.declare_local("_try", expr.ty.clone());
            let next = b.new_block();
            b.terminate(Terminator::Call {
                func: Operand::Global("__try_unwrap".into()),
                args: vec![inner_op],
                dest,
                next,
            });
            b.switch_to(next);
            Operand::Local(dest)
        }

        HirExprKind::Unsafe(inner) => lower_expr(inner, b),

        HirExprKind::For { binding, binding_name, iter, body } => {
            // Evaluate the iterable once.
            let iter_op = lower_expr(iter, b);

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
            let binding_local = b.map_hir_local(*binding, binding_name, Ty::Error);
            let elem_done = b.new_block();
            b.terminate(Terminator::Call {
                func: Operand::Global("List.getOrPanic".into()),
                args: vec![iter_op.clone(), Operand::Local(i_local)],
                dest: binding_local,
                next: elem_done,
            });
            b.switch_to(elem_done);
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
    }
}

fn lower_stmt(stmt: &HirStmt, b: &mut Builder) {
    match stmt {
        HirStmt::Let { local, name, ty, init } => {
            let init_op  = lower_expr(init, b);
            let mir_local = b.map_hir_local(*local, name, ty.clone());
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
    }
}
