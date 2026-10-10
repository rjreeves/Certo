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
    /// Deferred expressions accumulated by `defer { ... }` statements —
    /// BACKLOG item 193. A stack of scope frames, one per currently-open
    /// `HirExprKind::Block` (pushed/popped by that arm's own lowering, see
    /// below): a `defer` pushes onto the *innermost* open frame, not one
    /// flat function-wide list, so it fires at the natural end of the
    /// block that actually contains it — not only once, at the enclosing
    /// *function's* own eventual return, which is what this looked like
    /// before this item (any block nested inside a larger function — an
    /// `if`/`match` arm, not the function's own outer body — never got
    /// its own defers to run until the whole function finally returned).
    /// Draining a frame at its own block's natural exit is destructive
    /// (`Vec::pop`, run once); draining the *whole stack* for an early
    /// exit (`emit_defers_then_return`, used by both the function's real
    /// `Return` and `?`'s error-propagation branch) is non-destructive
    /// (clone-then-emit) since it fires from a side branch that doesn't
    /// actually end the block — code after it (the `?`'s success path)
    /// still needs every still-open frame intact.
    defer_stack: Vec<Vec<certo_hir::HirExpr>>,
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
    /// Function name → *declared* return type, as written (BACKLOG item
    /// 135) — a `Ty::Var(0)` entry marks a bare type-param return, whose C
    /// implementation always returns a raw `void*` regardless of what
    /// concrete type HIR resolved the call's own type to; used to decide
    /// when a call needs its return value unboxed, generalizing the
    /// previous `RAW_RETURN_CALLEES` stdlib-only special case below.
    fn_ret_tys: std::collections::HashMap<String, Ty>,
    /// Sum variant name → its own enclosing/parent type name (BACKLOG item
    /// 277) — lets a constructor call/pattern-match know whether one of its
    /// own declared field positions is a *direct* self-reference (the field's
    /// type is the same as the variant's own parent type), which needs
    /// heap-boxing/unboxing since it's stored as a pointer, not inline, in C.
    variant_to_type: std::collections::HashMap<String, String>,
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
    /// Set only inside `lift_spawn_body`'s own `Builder` (BACKLOG item 186)
    /// — the local holding this spawn worker's own task-header pointer,
    /// which `While`/`For`'s loop-back-edge lowering checks each iteration
    /// to exit early if a `withTimeout` has abandoned this task. `None` for
    /// every ordinary function (including per-call-site synthesized helpers
    /// like `List.sumBy`'s own accumulator loop, which get their own fresh
    /// `Builder` and are correctly *not* checkpointed even when called from
    /// inside a spawn body — see the item 186 BACKLOG writeup for why that's
    /// a real, honest scope boundary rather than an oversight).
    cancel_check_local: Option<MirLocal>,
}

impl Builder {
    fn new(
        fn_name: &str,
        record_field_types:  std::collections::HashMap<String, Vec<Ty>>,
        variant_field_types: std::collections::HashMap<String, Vec<Ty>>,
        fn_param_tys:        std::collections::HashMap<String, Vec<Ty>>,
        fn_ret_tys:          std::collections::HashMap<String, Ty>,
        variant_to_type:     std::collections::HashMap<String, String>,
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
            // Synthesized helper names below (`__lam_{fn_name}_{idx}` etc.)
            // start with `__`, so `c_fn_name` (codegen/emit_mir.rs) emits
            // them verbatim with no sanitization — BACKLOG item 238. A
            // qualified HIR name like "Cart.total" (impl methods/computed
            // properties) must have its dot replaced here, once, at the
            // single point every synthesized name is built from, rather
            // than at each of the dozen call sites that interpolate it.
            fn_name: fn_name.replace('.', "_"),
            defer_stack: Vec::new(),
            record_field_types,
            variant_field_types,
            fn_param_tys,
            fn_ret_tys,
            variant_to_type,
            current_span: Span::DUMMY,
            cancel_check_local: None,
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

    /// Like `map_hir_local`, for a *pattern* binding — BACKLOG item 343.
    /// The alternatives of an or-pattern (`Circle(r) | Square(r) => ..`)
    /// share one HIR local per name (see `Cx::or_binding_reuse` in
    /// `crates/hir`); each alternative's bind site must write the *same* MIR
    /// local, or only the last-lowered alternative's would be the one the arm
    /// body reads. A HIR local with no mapping yet is declared fresh, exactly
    /// like `map_hir_local`; an already-mapped one is reused (patching a
    /// still-unknown `Ty::Error` declared type with the better one).
    fn bind_pattern_local(&mut self, hir: LocalId, name: &str, ty: Ty) -> MirLocal {
        if let Some(&existing) = self.local_map.get(&hir) {
            let decl = &mut self.locals[existing as usize];
            if matches!(decl.ty, Ty::Error) && !matches!(ty, Ty::Error) { decl.ty = ty; }
            return existing;
        }
        self.map_hir_local(hir, name, ty)
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
/// A loop's own back-edge (BACKLOG item 186): jump to `header_bb` to run
/// another iteration, same as a plain `Terminator::Goto` always did — unless
/// this `Builder` is lowering a lifted spawn-worker body (`cancel_check_local`
/// is `Some`), in which case it first checks whether a `withTimeout` has
/// abandoned this task and jumps straight to `exit_bb` instead, skipping the
/// rest of the loop. Reusing `exit_bb` (the loop's own normal "condition
/// false" exit) rather than a separate abort path means `defer{}`s still run
/// and the function still returns normally — the checkpoint firing looks
/// identical, downstream, to the loop condition having just become false.
fn loop_back_edge(b: &mut Builder, header_bb: BlockId, exit_bb: BlockId) {
    match b.cancel_check_local {
        Some(cancel_local) => {
            let abandoned = b.declare_local("_cancel_check", Ty::Bool);
            b.assign(abandoned, Rvalue::Call {
                func: Operand::Global("__certo_task_hdr_is_abandoned".into()),
                args: vec![Operand::Local(cancel_local)],
            });
            b.terminate(Terminator::If { cond: Operand::Local(abandoned), true_bb: exit_bb, false_bb: header_bb });
        }
        None => b.terminate(Terminator::Goto(header_bb)),
    }
}

/// Runs every currently-open scope's own deferred bodies, then returns —
/// BACKLOG item 193. Used by the function's own real final return and by
/// `?`'s error-propagation branch (`HirExprKind::Try`), the only two ways
/// a Certo function ever actually returns (there is no explicit `return`
/// statement — confirmed: `Token::Return` is reserved but never consumed
/// anywhere in the parser). Both are genuine "exit the whole function"
/// points, so every still-open frame must fire, innermost first, then
/// LIFO within each frame. Non-destructive (clones the stack rather than
/// draining it): a `?`'s error branch is a side branch off the main flow,
/// not the actual end of its enclosing block — the success continuation
/// right after it still needs every currently-open frame intact, since
/// nothing has really exited yet from *that* path's point of view.
fn emit_defers_then_return(return_op: Operand, b: &mut Builder) {
    let frames: Vec<Vec<certo_hir::HirExpr>> = b.defer_stack.clone();
    for frame in frames.iter().rev() {
        for body in frame.iter().rev() {
            b.current_span = body.span;
            lower_expr(body, b);
        }
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
    fn_ret_tys:          &std::collections::HashMap<String, Ty>,
    variant_to_type:     &std::collections::HashMap<String, String>,
) -> (MirFn, Vec<MirFn>) {
    let mut b = Builder::new(&f.name, record_field_types.clone(), variant_field_types.clone(), fn_param_tys.clone(), fn_ret_tys.clone(), variant_to_type.clone());

    // Declare params as locals (index 0 = return slot, type patched below).
    let ret_slot = b.declare_local("_ret", Ty::Error);
    for p in &f.params {
        b.map_hir_local(p.local, &p.name, p.ty.clone());
    }

    let result = if let Some(body) = &f.body {
        b.current_span = body.span;
        lower_value_expr(body, &mut b)
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
    // `Option.map` (BACKLOG item 256) — the exact same single-element-typed-
    // param-closure shape as `List.map` immediately above, just over an
    // `Option<A>` receiver instead of a `List<A>` one.
    "Option.map",
    // `List.flatMap` (BACKLOG item 162) — single-element-typed-param closure
    // returning something not itself derivable from the list's own element
    // type, same shape as `List.map` immediately above; needs the identical
    // boxed-ABI treatment for the same reason.
    "List.flatMap",
    // `List.upsert` (BACKLOG item 209) — its `on` key-projection function is
    // the same shape as `groupBy`'s `key` (called to extract a value used
    // only for equality, never compared via `</>`), so it needs the same
    // ordinary boxed-closure treatment, not `sortBy`/`minBy`/`maxBy`/
    // `sumBy`'s specialized numeric-comparator synthesis (a different
    // mechanism entirely, for a numeric-only key restriction that doesn't
    // apply here).
    "List.upsert",
    // `List.forEach` (BACKLOG item 266) — same single-element-typed-param
    // closure shape as `List.map`, called for its side effects only (its
    // own return is always `Unit`).
    "List.forEach",
    "dbQueryTyped", "Query.list", "Query.first", "Query.groupedList",
];

/// Recover the MIR types of a lambda's captured HIR locals from the
/// *enclosing* builder — the lambda's own fresh builder has no record of
/// them (BACKLOG item 140).
fn capture_types(captures: &[LocalId], b: &Builder) -> Vec<Ty> {
    captures.iter().map(|cid| {
        let mir_id = b.get_local(*cid);
        b.locals.iter().find(|l| l.id == mir_id).map(|l| l.ty.clone()).unwrap_or(Ty::Error)
    }).collect()
}

/// Build a lambda's closure environment in the *enclosing* builder `b`,
/// before the lambda's own body is lowered: a heap tuple (the same
/// representation an ordinary Certo tuple literal already uses) holding
/// each captured value, boxed with its own real type so bits are preserved
/// — BACKLOG item 140. An empty capture list still produces a valid `NULL`
/// operand (see `AggregateKind::Tuple`'s own empty-case codegen), so every
/// lambda's closure has a uniform shape regardless of whether it actually
/// captures anything.
fn build_capture_env(captures: &[LocalId], capture_tys: &[Ty], b: &mut Builder) -> Operand {
    let ops: Vec<Operand> = captures.iter().map(|cid| Operand::Local(b.get_local(*cid))).collect();
    let dest = b.declare_local("_env", Ty::Tuple(capture_tys.to_vec()));
    b.assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
    Operand::Local(dest)
}

/// Inside a lambda's own fresh builder `lb`, unbox each captured value out
/// of the (already-declared) env parameter into a same-typed local mapped
/// to its original HIR `LocalId` — so a `HirExprKind::Local` reference
/// inside the lambda body resolves exactly as it would for an ordinary
/// parameter. Must run *after* every other real C parameter has already
/// been declared in `lb` (BACKLOG item 140): these are body-only
/// temporaries, not part of the generated function's own signature.
fn bind_captures(captures: &[LocalId], capture_tys: &[Ty], env_param: MirLocal, lb: &mut Builder) {
    for (idx, (cid, ty)) in captures.iter().zip(capture_tys).enumerate() {
        let real = lb.map_hir_local(*cid, "_cap", ty.clone());
        lb.assign(real, Rvalue::Field { base: Operand::Local(env_param), field: idx.to_string() });
    }
}

/// Lower an expression in *value position* (as opposed to the immediate
/// callee of a `Call`, or the `Global` special-cases at the very top of the
/// `Call` arm) — the one place a bare named-function reference needs
/// wrapping into the uniform `certo_fn_t { fn, env: NULL }` closure shape
/// every other function value now carries (BACKLOG item 140): a direct call
/// keeps calling the named C function itself with no wrapping at all (the
/// `Call` arm's own `func_op` lowering, untouched), but a function
/// *reference* used as a value — assigned to a `val`, passed as an ordinary
/// argument, held in a record field/list/tuple element — must carry a real
/// closure so it's callable uniformly through `emit_callee`.
fn lower_value_expr(e: &HirExpr, b: &mut Builder) -> Operand {
    if let (HirExprKind::Global(fn_name), Ty::Fn { params, ret }) = (&e.kind, &e.ty) {
        return wrap_named_fn_as_closure(fn_name, params, ret, b);
    }
    lower_expr(e, b)
}

/// Wrap a bare named-function reference into a real closure whose `.fn`
/// itself takes the uniform leading `void*` env parameter every closure
/// caller assumes (BACKLOG item 140/76). A *named function's own compiled
/// signature* never has that leading env parameter — only a lambda's own
/// generated function does — so simply pointing `.fn` straight at the named
/// function (as an earlier version of this code did) is a real calling-
/// convention mismatch the moment anything actually calls through the
/// closure (`emit_callee`'s cast, or a hand-written C runtime function
/// like `certo_http_serve` that unpacks `.fn`/`.env` itself): the callee
/// receives one argument fewer than the caller passes, corrupting the
/// stack/registers for every argument after the phantom env. This
/// generates a tiny env-accepting (and ignoring) wrapper function instead,
/// Emit the body of a synthesized wrapper's call to `name` (the real
/// function/constructor a bare named-value reference points at), assigning
/// the result into the already-declared `dest` local. Used by all three
/// "wrap a bare named-function reference into a callable closure" builders
/// below (`wrap_named_fn_as_closure`, `wrap_named_fn_as_erased_closure`,
/// `lower_named_fn_boxed`) in place of an unconditional `Terminator::Call`
/// to `Operand::Global(name)`.
///
/// `Some`/`Ok`/`Err` are compiler intrinsics: a *direct call* to one of them
/// is intercepted in the `HirExprKind::Call` arm above, boxing/bit-casting
/// the payload as needed and (for `Ok`/`Err`) calling the real
/// `certo_ok`/`certo_err` runtime functions — but there is no real C
/// function named `certo_some`/`certo_ok`/`certo_err` reachable by a plain
/// `Terminator::Call { func: Operand::Global(name), .. }` the way an
/// ordinary named `fn` is. A *bare reference* to one of these (`List.map(xs,
/// Some)`, BACKLOG item 339) never goes through that direct-call intercept
/// — it lowers to a `HirExprKind::Global("Some")` value, which these three
/// wrapper builders turn into a synthesized closure that itself has to call
/// `Some`/`Ok`/`Err` from scratch. Before this fix that synthesized call
/// used the ordinary named-function path unconditionally, producing a call
/// to a nonexistent `certo_some`. This mirrors the direct-call intercept's
/// own boxing rules exactly, so a wrapped bare reference behaves identically
/// to the equivalent inline lambda (`|x| Some(x)`), *provided* `param_tys`
/// is the real, concrete (or genuinely-still-erased) payload type — see the
/// `elem_ty_hint`-based intercept in the `HirExprKind::Call` arg-lowering
/// loop below for why a bare `Some`/`Ok`/`Err` reference can't just reuse
/// its own HIR-reconstructed type the way an ordinary named `fn` does.
fn emit_wrapped_call(name: &str, arg_locals: &[MirLocal], param_tys: &[Ty], dest: MirLocal, lb: &mut Builder) {
    match name {
        "Some" if arg_locals.len() == 1 => {
            let payload_ty = param_tys[0].clone();
            let value = Operand::Local(arg_locals[0]);
            // BACKLOG item 251 — a still-generic payload is already a boxed
            // pointer at this point; boxing it again would double-box. See
            // the identical guard on the direct-call intercept above.
            if matches!(payload_ty, Ty::Var(_)) {
                lb.assign(dest, Rvalue::Use(value));
            } else {
                lb.assign(dest, Rvalue::BoxSome { value, ty: payload_ty });
            }
        }
        "Ok" | "Err" if arg_locals.len() == 1 => {
            let payload_ty = param_tys[0].clone();
            let raw = Operand::Local(arg_locals[0]);
            let arg = if matches!(payload_ty, Ty::Float) {
                let bits = lb.declare_local("_fbits", Ty::Int);
                lb.assign(bits, Rvalue::Call { func: Operand::Global("__certo_f2i".into()), args: vec![raw] });
                Operand::Local(bits)
            } else if needs_result_box(&payload_ty) {
                let boxed = lb.declare_local("_boxed", Ty::Error);
                lb.assign(boxed, Rvalue::BoxSome { value: raw, ty: payload_ty });
                Operand::Local(boxed)
            } else {
                raw
            };
            let next = lb.new_block();
            lb.terminate(Terminator::Call { func: Operand::Global(name.to_string()), args: vec![arg], dest, next });
            lb.switch_to(next);
        }
        _ => {
            let next = lb.new_block();
            lb.terminate(Terminator::Call {
                func: Operand::Global(name.to_string()),
                args: arg_locals.iter().map(|l| Operand::Local(*l)).collect(),
                dest,
                next,
            });
            lb.switch_to(next);
        }
    }
}

/// mirroring `lower_named_fn_boxed`'s shape but without its
/// BOXED_ABI_CALLEES-specific void*-param/return erasure, since this path
/// keeps the named function's real native parameter/return types.
fn wrap_named_fn_as_closure(name: &str, real_param_tys: &[Ty], real_ret_ty: &Ty, b: &mut Builder) -> Operand {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__fnref_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", real_ret_ty.clone());
    let _env_param = lb.declare_local("_env", Ty::Error); // ignored — a named function never captures

    let real_locals: Vec<MirLocal> = real_param_tys.iter()
        .map(|ty| lb.declare_local("_p", ty.clone()))
        .collect();

    let call_dest = lb.declare_local("_inner_call", real_ret_ty.clone());
    emit_wrapped_call(name, &real_locals, real_param_tys, call_dest, &mut lb);

    if !matches!(real_ret_ty, Ty::Unit) {
        lb.assign(ret_slot, Rvalue::Use(Operand::Local(call_dest)));
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: real_param_tys.len() + 1,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let ty = Ty::Fn { params: real_param_tys.to_vec(), ret: Box::new(real_ret_ty.clone()) };
    make_closure(&wrap_name, Operand::Const(MirConst::Unit), ty, b)
}

/// Same as `wrap_named_fn_as_closure`, but for a named function whose value
/// is used where an *erased* (`Ty::Var(0)`-typed) closure signature is
/// expected instead of its own real native one — e.g. a higher-kinded
/// function's own `f: A => B` parameter (BACKLOG item 76): `hktMap`'s own
/// internal call to `f` treats it as uniformly `Ty::Var(0) -> Ty::Var(0)`
/// regardless of what's actually bound to it at any given call site, so the
/// wrapper itself must present that same erased shape. Unboxes each
/// incoming erased parameter into the real function's own declared type
/// before calling it, then boxes the real result back — the same
/// `BoxSome`/`UnboxSome` malloc-based pairing every other value crossing a
/// `Ty::Var(0)` boundary already uses (*not* `Rvalue::Box`/`Unbox`'s
/// bit-packing scheme, which is specifically `BOXED_ABI_CALLEES`'s own
/// stdlib-callback convention — see `lower_named_fn_boxed` — a different,
/// incompatible boxing scheme from the one a sum-type constructor call
/// like `Box(v)` actually uses for its own bare-`T` payload).
fn wrap_named_fn_as_erased_closure(name: &str, real_param_tys: &[Ty], real_ret_ty: &Ty, b: &mut Builder) -> Operand {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__fnref_erased_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Var(0));
    let _env_param = lb.declare_local("_env", Ty::Error);

    let raw_locals: Vec<MirLocal> = real_param_tys.iter()
        .map(|_| lb.declare_local("_p_erased", Ty::Var(0)))
        .collect();
    let real_locals: Vec<MirLocal> = real_param_tys.iter().zip(raw_locals.iter())
        .map(|(ty, raw)| {
            let real = lb.declare_local("_p_real", ty.clone());
            lb.assign(real, Rvalue::UnboxSome { opt: Operand::Local(*raw), ty: ty.clone() });
            real
        })
        .collect();

    let call_dest = lb.declare_local("_inner_call", real_ret_ty.clone());
    emit_wrapped_call(name, &real_locals, real_param_tys, call_dest, &mut lb);

    if matches!(real_ret_ty, Ty::Unit) {
        lb.locals[ret_slot as usize].ty = Ty::Var(0);
    } else {
        lb.assign(ret_slot, Rvalue::BoxSome { value: Operand::Local(call_dest), ty: real_ret_ty.clone() });
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: real_param_tys.len() + 1,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let ty = Ty::Fn {
        params: real_param_tys.iter().map(|_| Ty::Var(0)).collect(),
        ret:    Box::new(Ty::Var(0)),
    };
    make_closure(&wrap_name, Operand::Const(MirConst::Unit), ty, b)
}

/// `compose(f, g)` (BACKLOG item 161, spec §9.1) constructs and returns a
/// brand-new closure `(x) => f(g(x))` — unlike every other higher-order
/// stdlib function (`List.map`, `flatMap`, ...), which only ever *consumes*
/// a closure it's given. Its result's own type is fully resolved to
/// concrete types by ordinary unification at the call site (e.g.
/// `compose(intToText, double)` has real type `Int => Text`), so a single
/// hand-written, type-erased C implementation can't work — the caller of
/// the *returned* closure expects a native calling convention matching
/// those concrete types, and that convention differs per call site. This
/// synthesizes a real, concretely-typed trampoline function per call site
/// instead, the same per-use-site-synthesis strategy `wrap_named_fn_as_closure`
/// (item 140) and HKT's own closure wrapping (item 142) already use — no
/// erasure/boxing tricks needed for the *call-through* itself, since the
/// synthesized function's own real parameter/return types are known
/// exactly at generation time. `f`/`g` are lowered as ordinary function
/// *values* (`lower_value_expr` — handles a bare named-function reference,
/// an inline lambda, or a local already holding a closure uniformly) and
/// stored in a heap tuple env; `Ty::Fn`'s own `needs_heap_box() == true`
/// (item 140) means the existing `AggregateKind::Tuple`/`Rvalue::Field`
/// codegen already box/unbox each closure transparently — no manual
/// box/unbox rvalues needed here, unlike the erased-closure case.
fn lower_compose_call(f_expr: &HirExpr, g_expr: &HirExpr, b: &mut Builder) -> Operand {
    let (a_ty, b_ty) = match &g_expr.ty {
        Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
        other => (other.clone(), Ty::Error),
    };
    let c_ty = match &f_expr.ty {
        Ty::Fn { ret, .. } => (**ret).clone(),
        other => other.clone(),
    };

    let f_op = lower_value_expr(f_expr, b);
    let g_op = lower_value_expr(g_expr, b);
    let env_ty = Ty::Tuple(vec![f_expr.ty.clone(), g_expr.ty.clone()]);
    let env_dest = b.declare_local("_compose_env", env_ty);
    b.assign(env_dest, Rvalue::Aggregate(AggregateKind::Tuple, vec![f_op, g_op]));

    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__compose_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", c_ty.clone());
    let env_param = lb.declare_local("_env", Ty::Error);
    let x_param = lb.declare_local("_x", a_ty.clone());

    let f_local = lb.declare_local("_f", f_expr.ty.clone());
    lb.assign(f_local, Rvalue::Field { base: Operand::Local(env_param), field: "0".into() });
    let g_local = lb.declare_local("_g", g_expr.ty.clone());
    lb.assign(g_local, Rvalue::Field { base: Operand::Local(env_param), field: "1".into() });

    let y_local = lb.declare_local("_y", b_ty);
    let next1 = lb.new_block();
    lb.terminate(Terminator::Call {
        func: Operand::Local(g_local),
        args: vec![Operand::Local(x_param)],
        dest: y_local,
        next: next1,
    });
    lb.switch_to(next1);

    let z_local = lb.declare_local("_z", c_ty.clone());
    let next2 = lb.new_block();
    lb.terminate(Terminator::Call {
        func: Operand::Local(f_local),
        args: vec![Operand::Local(y_local)],
        dest: z_local,
        next: next2,
    });
    lb.switch_to(next2);

    lb.assign(ret_slot, Rvalue::Use(Operand::Local(z_local)));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: 2,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let closure_ty = Ty::Fn { params: vec![a_ty], ret: Box::new(c_ty) };
    make_closure(&wrap_name, Operand::Local(env_dest), closure_ty, b)
}

/// `const(a)` (BACKLOG item 161, spec §9.1) constructs and returns a
/// closure that always returns `a`, ignoring whatever it's called with —
/// same per-call-site synthesis strategy as `lower_compose_call`. Unlike
/// `compose`, the returned closure's *parameter* type (`B`) is genuinely
/// unconstrained by any argument to `const` itself — it's only knowable
/// from how the result is later used, which neither this function nor
/// `crates/hir/src/lower.rs`'s own call-type recovery (`generic_container_ret`)
/// can see. Since the parameter is never read (the whole point of `const`),
/// this leaves it uniformly erased (`Ty::Var(0)` → `void*`) rather than
/// guessing — safe specifically *because* neither the trampoline's own real
/// C signature nor whatever the call site resolves for `B` can ever
/// disagree: with no way to learn `B`'s real type, both consistently fall
/// back to the same erased convention (mirrored in `generic_container_ret`'s
/// own `"const"` case, which must produce the identical `Ty::Var(0)` param
/// position for the two to actually match at the call site).
fn lower_const_call(a_expr: &HirExpr, b: &mut Builder) -> Operand {
    let a_ty = a_expr.ty.clone();
    let a_op = lower_value_expr(a_expr, b);
    let env_ty = Ty::Tuple(vec![a_ty.clone()]);
    let env_dest = b.declare_local("_const_env", env_ty);
    b.assign(env_dest, Rvalue::Aggregate(AggregateKind::Tuple, vec![a_op]));

    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__const_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", a_ty.clone());
    let env_param = lb.declare_local("_env", Ty::Error);
    let _ignored_param = lb.declare_local("_ignored", Ty::Var(0));

    let a_local = lb.declare_local("_a", a_ty.clone());
    lb.assign(a_local, Rvalue::Field { base: Operand::Local(env_param), field: "0".into() });
    lb.assign(ret_slot, Rvalue::Use(Operand::Local(a_local)));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: 2,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let closure_ty = Ty::Fn { params: vec![Ty::Var(0)], ret: Box::new(a_ty) };
    make_closure(&wrap_name, Operand::Local(env_dest), closure_ty, b)
}

/// `flip(f)` (BACKLOG item 161, spec §9.1) — `f: A => B => C` is itself a
/// *curried* function value (calling it with one `A` returns another
/// closure `B => C`, real and working via item 140's ordinary closure
/// support — confirmed directly: `fn add(a: Int): Int => Int = (b) => a+b`
/// then `add(5)(3)` already compiles and runs correctly today, with no
/// changes needed here). `flip`'s own result, `B => A => C`, is *also*
/// curried, so this synthesizes two nested functions per call site instead
/// of `lower_compose_call`/`lower_const_call`'s one: an outer one (`y: B`)
/// that builds and returns a closure over `{f, y}`, and an inner one
/// (`x: A`) that calls `f(x)` to get the intermediate `B => C` closure,
/// then calls *that* with `y`. A, B, C are all recoverable directly from
/// `f`'s own already-known type — no unification needed, same as `compose`.
fn lower_flip_call(f_expr: &HirExpr, b: &mut Builder) -> Operand {
    let (a_ty, bc_ty) = match &f_expr.ty {
        Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
        other => (Ty::Error, other.clone()),
    };
    let (b_ty, c_ty) = match &bc_ty {
        Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
        other => (Ty::Error, other.clone()),
    };
    let inner_closure_ty = Ty::Fn { params: vec![a_ty.clone()], ret: Box::new(c_ty.clone()) };

    let f_op = lower_value_expr(f_expr, b);
    let outer_env_ty = Ty::Tuple(vec![f_expr.ty.clone()]);
    let outer_env_dest = b.declare_local("_flip_env", outer_env_ty);
    b.assign(outer_env_dest, Rvalue::Aggregate(AggregateKind::Tuple, vec![f_op]));

    // ---- inner: (env: {f, y}, x: A) -> C ----
    let idx1 = b.lambda_count;
    b.lambda_count += 1;
    let inner_name = format!("__flip_inner_{}_{}", b.fn_name, idx1);
    let mut ib = Builder::new(&inner_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let inner_ret = ib.declare_local("_ret", c_ty.clone());
    let inner_env_param = ib.declare_local("_env", Ty::Error);
    let x_param = ib.declare_local("_x", a_ty.clone());

    let f_local2 = ib.declare_local("_f", f_expr.ty.clone());
    ib.assign(f_local2, Rvalue::Field { base: Operand::Local(inner_env_param), field: "0".into() });
    let y_local2 = ib.declare_local("_y", b_ty.clone());
    ib.assign(y_local2, Rvalue::Field { base: Operand::Local(inner_env_param), field: "1".into() });

    let intermediate = ib.declare_local("_inter", bc_ty.clone());
    let next1 = ib.new_block();
    ib.terminate(Terminator::Call { func: Operand::Local(f_local2), args: vec![Operand::Local(x_param)], dest: intermediate, next: next1 });
    ib.switch_to(next1);

    let result = ib.declare_local("_result", c_ty.clone());
    let next2 = ib.new_block();
    ib.terminate(Terminator::Call { func: Operand::Local(intermediate), args: vec![Operand::Local(y_local2)], dest: result, next: next2 });
    ib.switch_to(next2);

    ib.assign(inner_ret, Rvalue::Use(Operand::Local(result)));
    emit_defers_then_return(Operand::Local(inner_ret), &mut ib);
    let inner_fn = MirFn { name: inner_name.clone(), param_count: 2, locals: ib.locals, blocks: ib.blocks };
    b.lifted_fns.extend(ib.lifted_fns);
    b.lifted_fns.push(inner_fn);

    // ---- outer: (env: {f}, y: B) -> (A => C) closure ----
    let idx2 = b.lambda_count;
    b.lambda_count += 1;
    let outer_name = format!("__flip_outer_{}_{}", b.fn_name, idx2);
    let mut ob = Builder::new(&outer_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let outer_ret = ob.declare_local("_ret", inner_closure_ty.clone());
    let outer_env_param = ob.declare_local("_env", Ty::Error);
    let y_param = ob.declare_local("_y", b_ty.clone());

    let f_local1 = ob.declare_local("_f", f_expr.ty.clone());
    ob.assign(f_local1, Rvalue::Field { base: Operand::Local(outer_env_param), field: "0".into() });

    let inner_env_ty = Ty::Tuple(vec![f_expr.ty.clone(), b_ty.clone()]);
    let inner_env_dest = ob.declare_local("_inner_env", inner_env_ty);
    ob.assign(inner_env_dest, Rvalue::Aggregate(AggregateKind::Tuple, vec![Operand::Local(f_local1), Operand::Local(y_param)]));

    let inner_closure_op = make_closure(&inner_name, Operand::Local(inner_env_dest), inner_closure_ty.clone(), &mut ob);
    ob.assign(outer_ret, Rvalue::Use(inner_closure_op));
    emit_defers_then_return(Operand::Local(outer_ret), &mut ob);
    let outer_fn = MirFn { name: outer_name.clone(), param_count: 2, locals: ob.locals, blocks: ob.blocks };
    b.lifted_fns.extend(ob.lifted_fns);
    b.lifted_fns.push(outer_fn);

    let flip_result_ty = Ty::Fn { params: vec![b_ty], ret: Box::new(inner_closure_ty) };
    make_closure(&outer_name, Operand::Local(outer_env_dest), flip_result_ty, b)
}

/// `List.sortBy(list, key)` (BACKLOG item 162b) — synthesizes a comparator
/// per call site and delegates to the already-real, already-working
/// `List.sort`. The comparator can't be an ordinary hand-written C runtime
/// function the way `List.map`'s callback dispatch is, for two independent
/// reasons discovered while investigating this item: (1) the *key* type
/// `K` is only known concretely at each call site (typeck's own
/// `is_supported_key_type` check has already ruled out anything `<`/`>`
/// aren't correct for by the time this runs); (2) `certo_list_sort`'s C
/// implementation (`crates/stdlib/src/collections.rs`) casts the
/// comparator's `.fn` slot directly to `CertoCmp = int64_t(*)(void*,void*,
/// void*)` — a fixed, fully-erased signature — so the synthesized
/// comparator's own two element params must be `void*`-boxed exactly like
/// an ordinary user lambda passed to a `BOXED_ABI_CALLEES` function (see
/// `lower_lambda_boxed`), even though `List.sort` itself isn't in that
/// list (confirmed directly: a hand-written comparator over a *struct*-
/// element list already fails to compile today, a real, separate,
/// pre-existing gap in `List.sort` — not something this item touches,
/// since building the comparator with the boxed shape from the start
/// sidesteps it entirely rather than depending on it being fixed).
fn lower_sort_by_call(list_expr: &HirExpr, key_expr: &HirExpr, b: &mut Builder) -> Operand {
    let list_op = lower_value_expr(list_expr, b);
    let key_op = lower_value_expr(key_expr, b);
    // `key_expr.ty` itself is always `Ty::Error` for an inline lambda —
    // `lower_lambda_with_param_hints` only ever fixes the lambda's *own*
    // internal param type, never its outer `HirExpr.ty` field. The real,
    // fully-resolved `Ty::Fn{params,ret}` only exists on the *lowered*
    // closure value's own declared local type (set by `make_closure` in
    // the `HirExprKind::Lambda` arm above), so it must be recovered via
    // `infer_operand_ty` after lowering, not read off the HIR node.
    let key_ty = infer_operand_ty(&key_op, b);
    let (t_ty, k_ty) = match &key_ty {
        Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
        other => (other.clone(), Ty::Error),
    };
    let env_ty = Ty::Tuple(vec![key_ty.clone()]);
    let env_dest = b.declare_local("_sortby_env", env_ty);
    b.assign(env_dest, Rvalue::Aggregate(AggregateKind::Tuple, vec![key_op]));

    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__sort_by_cmp_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    // `CertoCmp` returns a plain `int64_t`, not the generic boxed-`Var(0)`
    // convention `lower_lambda_boxed`'s own return uses — a real, narrower
    // type is fine here since `certo_list_sort`'s C body only ever treats
    // this return value as a raw comparison result, never round-trips it
    // through a generic slot.
    let ret_slot = lb.declare_local("_ret", Ty::Int);
    let env_param = lb.declare_local("_env", Ty::Error);
    // The two element params are always `void*`-boxed (see doc comment
    // above) regardless of whether `T` itself would otherwise fit a raw
    // pointer-sized slot — matching exactly how `lower_lambda_boxed`
    // declares its own callback params, for the identical reason.
    let a_raw = lb.declare_local("_a_boxed", Ty::Var(0));
    let b_raw = lb.declare_local("_b_boxed", Ty::Var(0));

    let key_local = lb.declare_local("_key", key_ty.clone());
    lb.assign(key_local, Rvalue::Field { base: Operand::Local(env_param), field: "0".into() });

    let a_local = lb.declare_local("_a", t_ty.clone());
    lb.assign(a_local, Rvalue::Unbox { value: Operand::Local(a_raw), ty: t_ty.clone() });
    let b_local = lb.declare_local("_b", t_ty.clone());
    lb.assign(b_local, Rvalue::Unbox { value: Operand::Local(b_raw), ty: t_ty.clone() });

    let ka_local = lb.declare_local("_ka", k_ty.clone());
    let next1 = lb.new_block();
    lb.terminate(Terminator::Call { func: Operand::Local(key_local), args: vec![Operand::Local(a_local)], dest: ka_local, next: next1 });
    lb.switch_to(next1);

    let kb_local = lb.declare_local("_kb", k_ty.clone());
    let next2 = lb.new_block();
    lb.terminate(Terminator::Call { func: Operand::Local(key_local), args: vec![Operand::Local(b_local)], dest: kb_local, next: next2 });
    lb.switch_to(next2);

    // if ka < kb then -1 else if ka > kb then 1 else 0
    let lt_local = lb.declare_local("_lt", Ty::Bool);
    lb.assign(lt_local, Rvalue::BinOp { op: certo_hir::BinOp::Lt, lhs: Operand::Local(ka_local), rhs: Operand::Local(kb_local) });
    let lt_bb = lb.new_block();
    let ge_bb = lb.new_block();
    lb.terminate(Terminator::If { cond: Operand::Local(lt_local), true_bb: lt_bb, false_bb: ge_bb });

    lb.switch_to(lt_bb);
    lb.assign(ret_slot, Rvalue::Use(Operand::Const(MirConst::Int(-1))));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    lb.switch_to(ge_bb);
    let gt_local = lb.declare_local("_gt", Ty::Bool);
    lb.assign(gt_local, Rvalue::BinOp { op: certo_hir::BinOp::Gt, lhs: Operand::Local(ka_local), rhs: Operand::Local(kb_local) });
    let gt_bb = lb.new_block();
    let eq_bb = lb.new_block();
    lb.terminate(Terminator::If { cond: Operand::Local(gt_local), true_bb: gt_bb, false_bb: eq_bb });

    lb.switch_to(gt_bb);
    lb.assign(ret_slot, Rvalue::Use(Operand::Const(MirConst::Int(1))));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    lb.switch_to(eq_bb);
    lb.assign(ret_slot, Rvalue::Use(Operand::Const(MirConst::Int(0))));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn { name: wrap_name.clone(), param_count: 3, locals: lb.locals, blocks: lb.blocks };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    // Opaque, matching `lower_lambda_boxed`'s own choice: this closure is
    // only ever consumed by `certo_list_sort`'s own raw `.fn`/`.env` cast,
    // never called via `emit_callee`'s locally-typed-cast path.
    let cmp_op = make_closure(&wrap_name, Operand::Local(env_dest), opaque_fn_ty(), b);

    let dest = b.declare_local("_sorted", Ty::List(Box::new(t_ty)));
    let next = b.new_block();
    b.terminate(Terminator::Call { func: Operand::Global("List.sort".into()), args: vec![list_op, cmp_op], dest, next });
    b.switch_to(next);
    Operand::Local(dest)
}

/// `List.minBy`/`List.maxBy` (BACKLOG item 162b) — `List.sortBy` ascending
/// then take the first/last element. Reuses `lower_sort_by_call` entirely
/// rather than a second, independent comparator synthesis; `List.first`/
/// `List.last` already correctly heap-box-aware-unwrap their `Option<T>`
/// result (see `OPT_UNWRAP_CALLEES` above), so no new unboxing logic is
/// needed here either.
fn lower_min_max_by_call(list_expr: &HirExpr, key_expr: &HirExpr, take_last: bool, b: &mut Builder) -> Operand {
    let sorted_op = lower_sort_by_call(list_expr, key_expr, b);
    // Recover `T` from the sorted list's own real declared type rather than
    // `key_expr.ty` (always `Ty::Error` for an inline lambda — see the
    // identical note in `lower_sort_by_call`).
    let t_ty = match infer_operand_ty(&sorted_op, b) {
        Ty::List(inner) => *inner,
        other => other,
    };

    let needs_unwrap = t_ty.needs_heap_box();
    let raw_dest = b.declare_local("_minmax_raw", if needs_unwrap { Ty::Option(Box::new(Ty::Error)) } else { Ty::Option(Box::new(t_ty.clone())) });
    let next = b.new_block();
    let fn_name = if take_last { "List.last" } else { "List.first" };
    b.terminate(Terminator::Call { func: Operand::Global(fn_name.into()), args: vec![sorted_op], dest: raw_dest, next });
    b.switch_to(next);
    if needs_unwrap {
        let real = b.declare_local("_minmax", Ty::Option(Box::new(t_ty.clone())));
        b.assign(real, Rvalue::UnwrapOptStructBox { value: Operand::Local(raw_dest), ty: Ty::Option(Box::new(t_ty)) });
        Operand::Local(real)
    } else {
        Operand::Local(raw_dest)
    }
}

/// `List.sumBy(list, key)` (BACKLOG item 162b) — synthesizes a dedicated
/// named summation function per call site rather than reusing
/// `List.fold`: it's called directly (`Operand::Global`, never passed
/// around as a closure *value*), so unlike `List.sortBy`'s comparator it
/// needs none of the generic `certo_fn_t`/`void*` boxing — its real,
/// concrete `(List<T>, T=>N) -> N` signature is used as-is, exactly like
/// any other ordinary top-level function `certo_list_get_or_panic`/etc are
/// called with. Manually loops (mirroring `HirExprKind::For`'s own
/// hand-rolled loop lowering just above, including its identical
/// `List.getOrPanic` unboxing for a `Float`/heap-boxed element type)
/// rather than delegating to `List.fold`, since accumulating through
/// *that* function's own generic `void*` accumulator slot would reintroduce
/// exactly the boxing question this function exists to avoid — `N` is
/// real and already known to be a plain numeric C type at this point
/// (typeck's `is_supported_key_type`), so a raw `+` and a real zero
/// literal are both always correct here.
fn lower_sum_by_call(list_expr: &HirExpr, key_expr: &HirExpr, b: &mut Builder) -> Operand {
    let list_op = lower_value_expr(list_expr, b);
    let key_op = lower_value_expr(key_expr, b);
    // See the identical note in `lower_sort_by_call`: `key_expr.ty` is
    // always `Ty::Error` for an inline lambda, so the real type must come
    // from the lowered closure value's own declared local type.
    let key_ty = infer_operand_ty(&key_op, b);
    let (t_ty, n_ty) = match &key_ty {
        Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
        other => (other.clone(), Ty::Error),
    };

    let idx = b.lambda_count;
    b.lambda_count += 1;
    let wrap_name = format!("__sum_by_{}_{}", b.fn_name, idx);

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", n_ty.clone());
    let list_param = lb.declare_local("_list", Ty::List(Box::new(t_ty.clone())));
    let key_param = lb.declare_local("_key", key_ty.clone());

    let acc_local = lb.declare_local("_acc", n_ty.clone());
    match &n_ty {
        Ty::Float | Ty::Float32 => lb.assign(acc_local, Rvalue::Use(Operand::Const(MirConst::Float(0.0)))),
        // BACKLOG item 282 — a `Decimal` accumulator's zero must be a real
        // `certo_decimal_t` value (via `certo_decimal_parse`, the same
        // runtime path any other `Decimal` literal goes through), not a
        // bit-pattern `MirConst::Int(0)` this struct-typed accumulator's
        // own `Rvalue::BinOp{Add}` (routed to `certo_decimal_add` by
        // `emit_binop`) would otherwise misread.
        Ty::Decimal(_) => lb.assign(acc_local, Rvalue::Use(Operand::Const(MirConst::Decimal("0".to_string())))),
        // BACKLOG item 311 — a user struct type (e.g. `Money`) has no
        // literal/bit-pattern "zero" at all; typeck/HIR's own
        // `is_supported_sum_type`/`is_supported_sum_ty` already validated
        // this type declares a real `{Type}.zero(): {Type}` function
        // (mirroring `Decimal.add`'s own naming convention), so call it
        // directly to seed the accumulator instead.
        Ty::Named { name, args } if args.is_empty() => {
            let next = lb.new_block();
            lb.terminate(Terminator::Call { func: Operand::Global(format!("{name}.zero")), args: vec![], dest: acc_local, next });
            lb.switch_to(next);
        }
        _ => lb.assign(acc_local, Rvalue::Use(Operand::Const(MirConst::Int(0)))),
    }

    let len_local = lb.declare_local("_len", Ty::Int);
    let len_done = lb.new_block();
    lb.terminate(Terminator::Call { func: Operand::Global("List.len".into()), args: vec![Operand::Local(list_param)], dest: len_local, next: len_done });
    lb.switch_to(len_done);

    let i_local = lb.declare_local("_i", Ty::Int);
    lb.assign(i_local, Rvalue::Use(Operand::Const(MirConst::Int(0))));

    let test_bb = lb.new_block();
    let body_bb = lb.new_block();
    let exit_bb = lb.new_block();
    lb.terminate(Terminator::Goto(test_bb));

    lb.switch_to(test_bb);
    let cmp = lb.declare_local("_cmp", Ty::Bool);
    lb.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::Lt, lhs: Operand::Local(i_local), rhs: Operand::Local(len_local) });
    lb.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: body_bb, false_bb: exit_bb });

    lb.switch_to(body_bb);
    // Same `List.getOrPanic` raw-`void*`-return unboxing `HirExprKind::For`
    // above already needs for exactly the same reason (Float bit-pattern,
    // or a struct element too wide for a pointer-sized slot).
    let elem_needs_unbox = matches!(t_ty, Ty::Float) || t_ty.needs_heap_box();
    let elem_raw = lb.declare_local("_elem_raw", if elem_needs_unbox { Ty::Var(0) } else { t_ty.clone() });
    let elem_done = lb.new_block();
    lb.terminate(Terminator::Call { func: Operand::Global("List.getOrPanic".into()), args: vec![Operand::Local(list_param), Operand::Local(i_local)], dest: elem_raw, next: elem_done });
    lb.switch_to(elem_done);
    let elem_local = if elem_needs_unbox {
        let real = lb.declare_local("_elem", t_ty.clone());
        lb.assign(real, Rvalue::Unbox { value: Operand::Local(elem_raw), ty: t_ty.clone() });
        real
    } else {
        elem_raw
    };

    let key_val = lb.declare_local("_kval", n_ty.clone());
    let key_done = lb.new_block();
    lb.terminate(Terminator::Call { func: Operand::Local(key_param), args: vec![Operand::Local(elem_local)], dest: key_val, next: key_done });
    lb.switch_to(key_done);

    let acc_new = lb.declare_local("_acc_new", n_ty.clone());
    // BACKLOG item 311 — same idea as the zero-seed above: a user struct
    // type has no bare `+` (no operator overloading in Certo), so call its
    // own already-validated `{Type}.add(a, b): {Type}` instead of emitting
    // an `Rvalue::BinOp{Add}` a struct operand could never satisfy at the
    // C level.
    match &n_ty {
        Ty::Named { name, args } if args.is_empty() => {
            let next = lb.new_block();
            lb.terminate(Terminator::Call {
                func: Operand::Global(format!("{name}.add")),
                args: vec![Operand::Local(acc_local), Operand::Local(key_val)],
                dest: acc_new,
                next,
            });
            lb.switch_to(next);
        }
        _ => lb.assign(acc_new, Rvalue::BinOp { op: certo_hir::BinOp::Add, lhs: Operand::Local(acc_local), rhs: Operand::Local(key_val) }),
    }
    lb.assign(acc_local, Rvalue::Use(Operand::Local(acc_new)));

    let i_new = lb.declare_local("_i_new", Ty::Int);
    lb.assign(i_new, Rvalue::BinOp { op: certo_hir::BinOp::Add, lhs: Operand::Local(i_local), rhs: Operand::Const(MirConst::Int(1)) });
    lb.assign(i_local, Rvalue::Use(Operand::Local(i_new)));
    lb.terminate(Terminator::Goto(test_bb));

    lb.switch_to(exit_bb);
    lb.assign(ret_slot, Rvalue::Use(Operand::Local(acc_local)));
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn { name: wrap_name.clone(), param_count: 2, locals: lb.locals, blocks: lb.blocks };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    let dest = b.declare_local("_sum", n_ty);
    let next = b.new_block();
    b.terminate(Terminator::Call { func: Operand::Global(wrap_name), args: vec![list_op, key_op], dest, next });
    b.switch_to(next);
    Operand::Local(dest)
}

/// Wrap a lifted lambda's generated global function and its closure
/// environment into a `certo_fn_t { fn, env }` value in the *enclosing*
/// builder `b` — BACKLOG item 140. `ty` is the closure's own declared
/// `Ty::Fn { params, ret }` — load-bearing whenever the resulting value
/// might later be *called* through a local (`emit_callee` reads the real
/// param/return types straight off the destination local's own declared
/// type to build its cast signature); a placeholder `Ty::Fn` is harmless
/// wherever the result is only ever consumed as an opaque argument (the
/// `BOXED_ABI_CALLEES` callback paths, which are never themselves called
/// via ordinary Certo call syntax).
fn make_closure(lam_name: &str, env: Operand, ty: Ty, b: &mut Builder) -> Operand {
    let dest = b.declare_local("_closure", ty);
    b.assign(dest, Rvalue::Aggregate(
        AggregateKind::Record(vec!["fn".into(), "env".into()]),
        vec![Operand::Global(lam_name.to_string()), env],
    ));
    Operand::Local(dest)
}

/// Placeholder `Ty::Fn` for a closure value that will only ever be consumed
/// as an opaque callback argument, never called directly through
/// `emit_callee` — see `make_closure`.
fn opaque_fn_ty() -> Ty {
    Ty::Fn { params: Vec::new(), ret: Box::new(Ty::Error) }
}

/// Marker type for a lifted spawn worker's hidden cancel-token parameter
/// (BACKLOG item 186) — an opaque `__certo_task_hdr_t*`, compiled to `void*`
/// (`crates/codegen/src/ty_to_c.rs`), same convention as `__CertoTask`. Not
/// `Ty::Error`: that already means "raw `int64_t`" elsewhere in this
/// pipeline (the exact confusion behind BACKLOG items 179/189), which would
/// be wrong for a pointer-typed parameter.
fn cancel_token_ty() -> Ty {
    Ty::Named { name: "__CertoCancelToken".into(), args: Vec::new() }
}

/// Lift a lambda literal that's being passed directly as the callback
/// argument to one of `BOXED_ABI_CALLEES`. Every param is unboxed on entry
/// and the return value boxed on exit, so the lambda's C signature is
/// uniformly `void* (*)(void*, ...)` — matching the generic function
/// pointer type those C runtime functions declare, instead of the lambda's
/// real native signature (e.g. `double(double)`), which is what caused a
/// `Float`-returning callback to silently misread the wrong return
/// register (BACKLOG item 112). Its own closure environment (BACKLOG item
/// 140) is always its first real parameter, ahead of the boxed callback
/// params — a real capture, if any, is unboxed from it exactly like an
/// ordinary parameter, just recovered from the enclosing scope instead of
/// the call site.
fn lower_lambda_boxed(params: &[certo_hir::HirParam], body: &HirExpr, captures: &[LocalId], param_ty_hint: Option<&Ty>, b: &mut Builder) -> Operand {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let lam_name = format!("__lam_{}_{}_boxed", b.fn_name, idx);

    let capture_tys = capture_types(captures, b);
    let env = build_capture_env(captures, &capture_tys, b);

    let mut lb = Builder::new(&lam_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Var(0));
    let env_param = lb.declare_local("_env", Ty::Error);

    // The C signature's params (locals 1..=param_count) are always void* —
    // declare all of them first, then the "real" typed locals the body
    // actually uses, unboxed from the raw params. Lambda params are almost
    // always unannotated (`(x) => ...`), so `p.ty` is `Ty::Error`; fall back
    // to `param_ty_hint` (the callee's scrutinee element type, known at the
    // call site) rather than mis-unboxing as a raw pointer cast.
    let raw_locals: Vec<MirLocal> = params.iter()
        .map(|p| lb.declare_local(&format!("{}_boxed", p.name), Ty::Var(0)))
        .collect();
    bind_captures(captures, &capture_tys, env_param, &mut lb);
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
        param_count: params.len() + 1,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(lam_fn);

    make_closure(&lam_name, env, opaque_fn_ty(), b)
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

    let mut lb = Builder::new(&wrap_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Var(0));
    // A named function reference never captures anything, but the
    // generated wrapper's own signature must still match every other
    // BOXED_ABI_CALLEES callback's uniform `(env, ...)` shape (BACKLOG item
    // 140) — declared and otherwise unused.
    let _env_param = lb.declare_local("_env", Ty::Error);

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
    emit_wrapped_call(name, &real_locals, real_param_tys, call_dest, &mut lb);

    if matches!(real_ret_ty, Ty::Unit) {
        lb.locals[ret_slot as usize].ty = Ty::Var(0);
    } else {
        lb.assign(ret_slot, Rvalue::Box { value: Operand::Local(call_dest), ty: real_ret_ty.clone() });
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let wrap_fn = MirFn {
        name: wrap_name.clone(),
        param_count: real_param_tys.len() + 1,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(wrap_fn);

    make_closure(&wrap_name, Operand::Const(MirConst::Unit), opaque_fn_ty(), b)
}

/// Lift a `spawn` body that isn't a direct call into a synthesized
/// top-level function taking its captured locals as ordinary real-typed
/// parameters — BACKLOG item 141. Unlike a lambda's `certo_fn_t` closure,
/// this needs no env/boxing indirection at all: `Rvalue::Spawn`'s own
/// existing codegen already threads each of its `args` through its own
/// real-typed field on the per-site context struct (`emit_spawn_support`),
/// so treating the captures as if they were an ordinary call's arguments
/// reuses that mechanism directly — spawning the lifted function is
/// identical to spawning a direct call to any other named function.
fn lift_spawn_body(body: &HirExpr, captures: &[LocalId], b: &mut Builder) -> (String, Vec<Operand>, Ty) {
    let idx = b.lambda_count;
    b.lambda_count += 1;
    let fn_name = format!("__spawn_{}_{}", b.fn_name, idx);

    let capture_tys = capture_types(captures, b);
    let arg_ops: Vec<Operand> = captures.iter().map(|cid| Operand::Local(b.get_local(*cid))).collect();

    let mut lb = Builder::new(&fn_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
    let ret_slot = lb.declare_local("_ret", Ty::Error);
    for (cid, ty) in captures.iter().zip(&capture_tys) {
        lb.map_hir_local(*cid, "_cap", ty.clone());
    }
    // Hidden trailing parameter (BACKLOG item 186) — this task's own header
    // pointer, appended by codegen's spawn trampoline (`emit_spawn_support`),
    // not part of `captures`/`arg_ops` below since it isn't a value known at
    // spawn time, only once the worker's context struct exists. Every
    // `While`/`For` loop lowered directly into this function's own control
    // flow (via `lower_expr(body, &mut lb)` just below) checks it on each
    // back-edge to exit early once a `withTimeout` abandons this task.
    let cancel_local = lb.declare_local("_cancel_hdr", cancel_token_ty());
    lb.cancel_check_local = Some(cancel_local);
    lb.current_span = body.span;
    let result = lower_expr(body, &mut lb);
    let ret_ty = infer_operand_ty(&result, &lb);
    lb.locals[ret_slot as usize].ty = ret_ty.clone();
    if !matches!(ret_ty, Ty::Unit) {
        lb.assign(ret_slot, Rvalue::Use(result));
    }
    emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

    let lifted_fn = MirFn {
        name: fn_name.clone(),
        // +1 for the hidden trailing cancel-token parameter, which is NOT
        // reflected in `arg_ops` below (`Rvalue::Spawn`'s own `args`) — it's
        // supplied by the trampoline at call time, not stored in the
        // per-site context struct like a real capture.
        param_count: captures.len() + 1,
        locals: lb.locals,
        blocks: lb.blocks,
    };
    b.lifted_fns.extend(lb.lifted_fns);
    b.lifted_fns.push(lifted_fn);

    (fn_name, arg_ops, ret_ty)
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
                "HttpRequest" | "HttpResponse" | "Bytes" | "DbResult" | "Query" | "Mutation" |
                "__CertoTask" | "Host" | "HostPlugin" | "HostContext" | "ServiceKey" |
                "RestartPolicy" | "HostStatusSnapshot" | "HostWorkerStatus" |
                "HostMetricSnapshot" | "HostState" | "HostWorkerState" |
                "HostLifecycleError" | "HostFailureKind" | "HostLogEvent" | "HostLogField" |
                "HostLogOverflowPolicy" | "HostLogFailurePolicy" | "HostMetric"))
        // BACKLOG item 251 — a generic `Ok(v)`/`Err(e)` construction always
        // heap-boxes `v`/`e` (it's a bare, opaque type-param value at that
        // construction site — item 119/120's own established convention),
        // but a directly-concrete `Ok(5)` previously stored these scalar
        // types inline instead, since neither was in this set — the exact
        // same resolved type (`Ty::Int`) ended up with two different
        // runtime representations depending on whether the value flowed
        // through a generic boundary before reaching the match site that
        // consumes it (BACKLOG item 247's own type-recovery fix correctly
        // resolves the *type*, but this separate boxing decision, keyed
        // only on that resolved type, had no way to know whether the
        // specific value now matched was actually boxed upstream).
        // Always boxing them here — this same function gates *both* the
        // `Ok`/`Err` construction site (just below) and the `?`/match-arm
        // consumption site (`unwrap_result_into`) — eliminates the
        // ambiguity by making every construction site agree, matching
        // `Decimal`/`UUID`'s own already-consistent always-boxed treatment.
        // `Float` (8-byte double) is deliberately excluded — it has its
        // own bit-reinterpretation path (`__certo_f2i`/`__certo_i2f`, a
        // cheaper cast handled before this check is ever consulted at the
        // `Ok`/`Err` construction site below) rather than a real heap
        // allocation. `Float32` has no such mechanism of its own, so it's
        // included here instead, same treatment as `Int`. `Text`/
        // `BoundedText` are already pointer-sized and need no box either
        // way, at either construction site.
        || matches!(ty, Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt | Ty::Float32 | Ty::Bool | Ty::Char | Ty::Unit)
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
    let ml = b.bind_pattern_local(hir_local, name, payload_ty.clone());
    unwrap_result_into(b, scrut, payload_ty, ml);
}

/// Check that the value in `op` (of type `ty`) matches `pat`, branching to
/// `next_arm_bb` if it doesn't, and binding any names `pat` introduces on
/// success. Used to check a field/element sub-pattern nested inside a
/// `Record`/`Constructor`/`Tuple`/`List` pattern — BACKLOG item 305: those
/// field/element loops previously only recognized a plain `HirPat::Bind`
/// sub-pattern and silently emitted no check at all for anything else (a
/// nested constructor like `Active`, a literal like `30`), so an arm like
/// `{ status: Banned } => ...` matched regardless of the field's actual
/// value. Recurses for a sub-pattern nested more than one level deep
/// (`Wrap(Some(x))`, `(Some(x), y)`, etc.), reusing the exact same
/// discriminant-check/unboxing rules the top-level arm dispatch above uses
/// for the analogous top-level pattern kind.
///
/// Leaves the builder positioned at the block where matching should
/// continue: the current block, unchanged, for a pattern that always
/// matches (`Bind`/`Wildcard`), or a fresh block reached only once the
/// check (and any further-nested checks) succeeded otherwise. Callers that
/// need control to reach a specific downstream block regardless of how
/// many intermediate blocks this introduces (the built-in `Some` case,
/// which must branch to `next_arm_bb` on a null scrutinee *before*
/// attempting to unbox its payload) must issue that final `Goto`
/// themselves — see the built-in `Constructor("Some", ...)` arm above.
fn check_nested_pattern(b: &mut Builder, pat: &HirPat, op: &Operand, ty: &Ty, next_arm_bb: BlockId) {
    match pat {
        HirPat::Wildcard => {}
        HirPat::Bind { local, name } => {
            let ml = b.bind_pattern_local(*local, name, ty.clone());
            b.assign(ml, Rvalue::Use(op.clone()));
        }
        HirPat::Lit(lit) => {
            let expected = match lit {
                HirLitPat::Int(n)  => Operand::Const(MirConst::Int(*n)),
                HirLitPat::Bool(v) => Operand::Const(MirConst::Bool(*v)),
                HirLitPat::Str(s)  => Operand::Const(MirConst::Str(s.clone())),
            };
            let cmp = b.declare_local("_cmp", Ty::Bool);
            b.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::Eq, lhs: op.clone(), rhs: expected });
            let cont_bb = b.new_block();
            b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: cont_bb, false_bb: next_arm_bb });
            b.switch_to(cont_bb);
        }
        HirPat::Constructor { name, fields, field_names, field_types } => {
            match name.as_str() {
                "None" => {
                    let cmp = b.declare_local("_cmp", Ty::Bool);
                    b.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::Eq, lhs: op.clone(), rhs: Operand::Global("__NULL".into()) });
                    let cont_bb = b.new_block();
                    b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: cont_bb, false_bb: next_arm_bb });
                    b.switch_to(cont_bb);
                }
                "Some" => {
                    let cmp = b.declare_local("_cmp", Ty::Bool);
                    b.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::NotEq, lhs: op.clone(), rhs: Operand::Global("__NULL".into()) });
                    let cont_bb = b.new_block();
                    b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: cont_bb, false_bb: next_arm_bb });
                    b.switch_to(cont_bb);
                    if let Some(inner_pat) = fields.first() {
                        let payload_ty = match ty { Ty::Option(inner) => (**inner).clone(), _ => Ty::Error };
                        let payload = b.declare_local("_payload", payload_ty.clone());
                        b.assign(payload, Rvalue::UnboxSome { opt: op.clone(), ty: payload_ty.clone() });
                        check_nested_pattern(b, inner_pat, &Operand::Local(payload), &payload_ty, next_arm_bb);
                    }
                }
                "Ok" | "Err" => {
                    let is_ok = b.declare_local("_is_ok", Ty::Bool);
                    let next_bb = b.new_block();
                    b.terminate(Terminator::Call { func: Operand::Global("__result_is_ok".into()), args: vec![op.clone()], dest: is_ok, next: next_bb });
                    b.switch_to(next_bb);
                    let (ok_ty, err_ty) = match ty { Ty::Result(t, e) => ((**t).clone(), (**e).clone()), _ => (Ty::Error, Ty::Error) };
                    let want_ok = name == "Ok";
                    let payload_ty = if want_ok { ok_ty } else { err_ty };
                    let payload = fields.first().map(|inner_pat| {
                        let dest = b.declare_local("_payload", payload_ty.clone());
                        unwrap_result_into(b, op, &payload_ty, dest);
                        (inner_pat, dest)
                    });
                    let cond_local = if want_ok {
                        is_ok
                    } else {
                        let not_ok = b.declare_local("_not_ok", Ty::Bool);
                        b.assign(not_ok, Rvalue::UnOp { op: certo_hir::UnOp::Not, arg: Operand::Local(is_ok) });
                        not_ok
                    };
                    let cont_bb = b.new_block();
                    b.terminate(Terminator::If { cond: Operand::Local(cond_local), true_bb: cont_bb, false_bb: next_arm_bb });
                    b.switch_to(cont_bb);
                    if let Some((inner_pat, dest)) = payload {
                        check_nested_pattern(b, inner_pat, &Operand::Local(dest), &payload_ty, next_arm_bb);
                    }
                }
                _ => {
                    // User-defined sum type: compare .tag, then recurse into fields —
                    // mirrors the top-level arm dispatch's own identical-shape handling.
                    let tag_local = b.declare_local("_tag", Ty::Int);
                    b.assign(tag_local, Rvalue::Field { base: op.clone(), field: "tag".into() });
                    let variant = name.rsplit("__").next().unwrap_or(name).to_lowercase();
                    let enclosing_type_name = name.split("__").next().unwrap_or(name);
                    let ty_args: &[Ty] = match ty { Ty::Named { args, .. } => args, _ => &[] };
                    let cmp = b.declare_local("_cmp", Ty::Bool);
                    let variant_tag = Operand::Global(format!("__tag__{}", name));
                    b.assign(cmp, Rvalue::BinOp { op: certo_hir::BinOp::Eq, lhs: Operand::Local(tag_local), rhs: variant_tag });
                    let cont_bb = b.new_block();
                    b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: cont_bb, false_bb: next_arm_bb });
                    b.switch_to(cont_bb);

                    enum FieldUnbox { None, OptionBoxed, SelfRefBoxed }
                    for (i, field_pat) in fields.iter().enumerate() {
                        let declared = field_types.get(i).cloned().unwrap_or(Ty::Error);
                        let (real_ty, unbox) = match &declared {
                            Ty::Var(_) => {
                                let concrete = ty_args.first().cloned().unwrap_or(Ty::Error);
                                let unbox = if !matches!(concrete, Ty::Var(_)) { FieldUnbox::OptionBoxed } else { FieldUnbox::None };
                                (concrete, unbox)
                            }
                            // BACKLOG item 308 — a *generic* self-referential
                            // field (`Tree<T>`'s own `left: Tree<T>`) needs the
                            // identical box/unbox treatment as the non-generic
                            // case item 277 built (dropped `args.is_empty()`,
                            // previously the only gate): the recursive-storage
                            // problem a self-referential field poses (an
                            // inline-by-value field would be an infinite-size
                            // struct) is about the field's own *name* matching
                            // its enclosing type, independent of whatever type
                            // arguments it (or its enclosing type) carries.
                            Ty::Named { name: field_ty_name, .. } if field_ty_name == enclosing_type_name => {
                                (declared.clone(), FieldUnbox::SelfRefBoxed)
                            }
                            other => (other.clone(), FieldUnbox::None),
                        };
                        let field_name = field_names.get(i).cloned().unwrap_or_else(|| format!("f{i}"));
                        let field_op = match unbox {
                            FieldUnbox::OptionBoxed => {
                                let raw = b.declare_local("_field_raw", Ty::Var(0));
                                b.assign(raw, Rvalue::Field { base: op.clone(), field: format!("{}.{}", variant, field_name) });
                                let unboxed = b.declare_local("_field_val", real_ty.clone());
                                b.assign(unboxed, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: real_ty.clone() });
                                Operand::Local(unboxed)
                            }
                            FieldUnbox::SelfRefBoxed => {
                                let raw = b.declare_local("_field_raw", Ty::Var(0));
                                b.assign(raw, Rvalue::Field { base: op.clone(), field: format!("{}.{}", variant, field_name) });
                                let unboxed = b.declare_local("_field_val", real_ty.clone());
                                b.assign(unboxed, Rvalue::Unbox { value: Operand::Local(raw), ty: real_ty.clone() });
                                Operand::Local(unboxed)
                            }
                            FieldUnbox::None => {
                                let val = b.declare_local("_field_val", real_ty.clone());
                                b.assign(val, Rvalue::Field { base: op.clone(), field: format!("{}.{}", variant, field_name) });
                                Operand::Local(val)
                            }
                        };
                        check_nested_pattern(b, field_pat, &field_op, &real_ty, next_arm_bb);
                    }
                }
            }
        }
        HirPat::Record { fields, field_names, field_types } => {
            let ty_args: &[Ty] = match ty { Ty::Named { args, .. } => args, _ => &[] };
            for (i, field_pat) in fields.iter().enumerate() {
                let declared = field_types.get(i).cloned().unwrap_or(Ty::Error);
                let (real_ty, needs_unbox) = match &declared {
                    Ty::Var(_) => {
                        let concrete = ty_args.first().cloned().unwrap_or(Ty::Error);
                        let needs_unbox = !matches!(concrete, Ty::Var(_));
                        (concrete, needs_unbox)
                    }
                    other => (other.clone(), false),
                };
                let field_name = field_names.get(i).cloned().unwrap_or_default();
                let field_op = if needs_unbox {
                    let raw = b.declare_local(&format!("_field_{}", field_name), Ty::Var(0));
                    b.assign(raw, Rvalue::Field { base: op.clone(), field: field_name.clone() });
                    let unboxed = b.declare_local("_field_val", real_ty.clone());
                    b.assign(unboxed, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: real_ty.clone() });
                    Operand::Local(unboxed)
                } else {
                    let val = b.declare_local("_field_val", real_ty.clone());
                    b.assign(val, Rvalue::Field { base: op.clone(), field: field_name.clone() });
                    Operand::Local(val)
                };
                check_nested_pattern(b, field_pat, &field_op, &real_ty, next_arm_bb);
            }
        }
        HirPat::Tuple(fields) => {
            for (i, field_pat) in fields.iter().enumerate() {
                let elem_local = b.declare_local("_tuple_elem", Ty::Error);
                let next_bb = b.new_block();
                b.terminate(Terminator::Call {
                    func: Operand::Global("__tuple_get".into()),
                    args: vec![op.clone(), Operand::Const(MirConst::Int(i as i64))],
                    dest: elem_local,
                    next: next_bb,
                });
                b.switch_to(next_bb);
                check_nested_pattern(b, field_pat, &Operand::Local(elem_local), &Ty::Error, next_arm_bb);
            }
        }
        HirPat::List { head, tail, elem_ty } => {
            let len_local = b.declare_local("_len", Ty::Int);
            let after_len_bb = b.new_block();
            b.terminate(Terminator::Call { func: Operand::Global("List.len".into()), args: vec![op.clone()], dest: len_local, next: after_len_bb });
            b.switch_to(after_len_bb);
            let cmp = b.declare_local("_cmp", Ty::Bool);
            b.assign(cmp, Rvalue::BinOp {
                op: if tail.is_some() { certo_hir::BinOp::GtEq } else { certo_hir::BinOp::Eq },
                lhs: Operand::Local(len_local),
                rhs: Operand::Const(MirConst::Int(head.len() as i64)),
            });
            let bind_bb = b.new_block();
            b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: bind_bb, false_bb: next_arm_bb });
            b.switch_to(bind_bb);
            for (i, field_pat) in head.iter().enumerate() {
                let elem_needs_unbox = matches!(elem_ty, Ty::Float) || elem_ty.needs_heap_box();
                let raw_ty = if elem_needs_unbox { Ty::Var(0) } else { elem_ty.clone() };
                let elem_raw = b.declare_local("_elem_raw", raw_ty);
                let next_bb = b.new_block();
                b.terminate(Terminator::Call {
                    func: Operand::Global("List.getOrPanic".into()),
                    args: vec![op.clone(), Operand::Const(MirConst::Int(i as i64))],
                    dest: elem_raw,
                    next: next_bb,
                });
                b.switch_to(next_bb);
                let elem_op = if elem_needs_unbox {
                    let unboxed = b.declare_local("_elem_val", elem_ty.clone());
                    b.assign(unboxed, Rvalue::Unbox { value: Operand::Local(elem_raw), ty: elem_ty.clone() });
                    Operand::Local(unboxed)
                } else {
                    Operand::Local(elem_raw)
                };
                check_nested_pattern(b, field_pat, &elem_op, elem_ty, next_arm_bb);
            }
            if let Some(tail_pat) = tail {
                let list_ty = Ty::List(Box::new(elem_ty.clone()));
                let tail_local = b.declare_local("_tail", list_ty.clone());
                let next_bb = b.new_block();
                b.terminate(Terminator::Call {
                    func: Operand::Global("List.slice".into()),
                    args: vec![op.clone(), Operand::Const(MirConst::Int(head.len() as i64)), Operand::Local(len_local)],
                    dest: tail_local,
                    next: next_bb,
                });
                b.switch_to(next_bb);
                check_nested_pattern(b, tail_pat, &Operand::Local(tail_local), &list_ty, next_arm_bb);
            }
        }
        HirPat::Or(l, r) => {
            let after_or = b.new_block();
            let try_right_bb = b.new_block();
            check_nested_pattern(b, l, op, ty, try_right_bb);
            b.terminate(Terminator::Goto(after_or));
            b.switch_to(try_right_bb);
            check_nested_pattern(b, r, op, ty, next_arm_bb);
            b.terminate(Terminator::Goto(after_or));
            b.switch_to(after_or);
        }
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
            if matches!(op, certo_hir::BinOp::And | certo_hir::BinOp::Or) {
                let dest = b.declare_local("_short_circuit", Ty::Bool);
                let rhs_bb = b.new_block();
                let short_bb = b.new_block();
                let join_bb = b.new_block();
                let (true_bb, false_bb, short_value) = match op {
                    certo_hir::BinOp::And => (rhs_bb, short_bb, false),
                    certo_hir::BinOp::Or => (short_bb, rhs_bb, true),
                    _ => unreachable!(),
                };
                b.terminate(Terminator::If {
                    cond: l,
                    true_bb,
                    false_bb,
                });

                b.switch_to(short_bb);
                b.assign(dest, Rvalue::Use(Operand::Const(MirConst::Bool(short_value))));
                b.terminate(Terminator::Goto(join_bb));

                b.switch_to(rhs_bb);
                let r = lower_expr(rhs, b);
                b.assign(dest, Rvalue::Use(r));
                b.terminate(Terminator::Goto(join_bb));

                b.switch_to(join_bb);
                return Operand::Local(dest);
            }
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
                    // BACKLOG item 251 — a still-generic payload (`Ty::Var`,
                    // e.g. `v` inside `fn wrapSome<T>(v: T): Option<T> =
                    // Some(v)`) is *already* a boxed pointer at this
                    // construction site (item 119/120's own established
                    // convention for a bare type-param value) — boxing it
                    // again here double-boxes: `Some`'s own representation
                    // ends up pointing at a pointer-to-the-real-value, not
                    // the value itself, while the eventual match site
                    // (once T resolves to something concrete, e.g. `Ty::Int`
                    // via item 247's own type-recovery fix) only
                    // dereferences once — reading the inner pointer's own
                    // bit pattern as if it were the payload. Use the
                    // already-boxed pointer directly instead of re-boxing.
                    if matches!(payload_ty, certo_typeck::Ty::Var(_)) {
                        b.assign(dest, Rvalue::Use(value));
                    } else {
                        b.assign(dest, Rvalue::BoxSome { value, ty: payload_ty });
                    }
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
                // `compose(f, g)` — BACKLOG item 161. See `lower_compose_call`'s
                // own doc comment for why this needs per-call-site synthesis
                // rather than an ordinary hand-written C runtime function.
                if name == "compose" && args.len() == 2 {
                    return lower_compose_call(&args[0], &args[1], b);
                }
                // `const(a)` — BACKLOG item 161. See `lower_const_call`'s own
                // doc comment for the erased-parameter design.
                if name == "const" && args.len() == 1 {
                    return lower_const_call(&args[0], b);
                }
                // `flip(f)` — BACKLOG item 161. See `lower_flip_call`'s own
                // doc comment for the nested-closure design.
                if name == "flip" && args.len() == 1 {
                    return lower_flip_call(&args[0], b);
                }
                // `List.sortBy`/`minBy`/`maxBy`/`sumBy` — BACKLOG item 162b.
                // See `lower_sort_by_call`'s own doc comment for why these
                // need per-call-site synthesis rather than an ordinary
                // hand-written C runtime function, same reasoning as
                // `compose`/`const`/`flip` above.
                if name == "List.sortBy" && args.len() == 2 {
                    return lower_sort_by_call(&args[0], &args[1], b);
                }
                if (name == "List.minBy" || name == "List.maxBy") && args.len() == 2 {
                    return lower_min_max_by_call(&args[0], &args[1], name == "List.maxBy", b);
                }
                if name == "List.sumBy" && args.len() == 2 {
                    return lower_sum_by_call(&args[0], &args[1], b);
                }
            }
            // `Option.map`'s receiver crosses two genuinely different box
            // conventions depending on its payload type: a scalar payload's
            // `Some(x)` cell holds the raw value itself (needs one `int64_t`
            // deref to reach the bit pattern `f` expects — `certo_option_map`
            // does that), but a struct-shaped payload (record/Decimal/UUID/
            // Fn — anything `needs_heap_box()`) is *already* stored as a
            // direct pointer to its own C representation (`Rvalue::BoxSome`
            // mallocs `sizeof(cty)`, not a fixed `int64_t` cell, for those),
            // which is exactly the pointer `f`'s own boxed-ABI param already
            // expects — dereferencing it as `int64_t` first reads garbage
            // and segfaults. Route to a second runtime function that skips
            // that deref entirely, chosen here since only this call site
            // still has the receiver's real `Ty::Option(inner)`.
            let func_op = if let HirExprKind::Global(name) = &func.kind {
                if name == "Option.map"
                    && args.first().is_some_and(|a| matches!(&a.ty, Ty::Option(inner) if inner.needs_heap_box()))
                {
                    Operand::Global("Option.mapBoxed".to_string())
                } else {
                    lower_expr(func, b)
                }
            } else {
                lower_expr(func, b)
            };
            // A lambda literal passed directly to one of BOXED_ABI_CALLEES
            // needs the boxed-ABI lift instead of the normal native one —
            // see lower_lambda_boxed's doc comment (BACKLOG item 112).
            let needs_boxed_callback = matches!(&func.kind, HirExprKind::Global(name) if BOXED_ABI_CALLEES.contains(&name.as_str()));
            // The callback's param type is almost never annotated in source
            // (`(x) => ...`) — recover it from the scrutinee's own known
            // element type instead (every BOXED_ABI_CALLEES entry takes
            // `(List<T>, T => ...)` or `(Option<T>, T => ...)`, so it's
            // always the first argument).
            let elem_ty_hint: Option<Ty> = if needs_boxed_callback {
                args.first().and_then(|a| match &a.ty {
                    Ty::List(inner) | Ty::Option(inner) => Some((**inner).clone()),
                    _ => None,
                })
            } else {
                None
            };
            // A generic sum-type variant constructor call (e.g. `Secret(42)`)
            // must heap-box any argument whose corresponding *declared*
            // field type is a bare type parameter (`Ty::Var(_)`) — its C
            // storage is `void*` regardless of the concrete type instantiated
            // here, mirroring how `Some`/`Ok`/`Err` box their own payload
            // above — BACKLOG item 119.
            // BACKLOG item 262 — a positional record constructor call
            // (`Money(d"10.00", USD)`) needs the identical boxing treatment
            // for a generic record (`type Box<T> = {value: T}`) as a sum
            // variant does — `record_field_types` (from HIR, already in
            // declaration order) is the record equivalent of
            // `variant_field_types` here, checked as a fallback since a
            // callee name is never registered in both tables at once.
            let variant_field_types: Option<Vec<Ty>> = match &func.kind {
                HirExprKind::Global(name) => b.variant_field_types.get(name).cloned()
                    .or_else(|| b.record_field_types.get(name).cloned()),
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
            // BACKLOG item 277 — this call's own enclosing sum type, if
            // `func` is a variant constructor (e.g. `Node` -> `Tree`). Lets
            // the arg-lowering closure below detect a *direct* self-
            // referential field position (a declared field typed as the
            // same type currently being constructed) and heap-box it —
            // it's stored as a pointer, not inline, in C, since an inline
            // self-referential field would be an infinite-size struct.
            let enclosing_type: Option<String> = match &func.kind {
                HirExprKind::Global(name) => b.variant_to_type.get(name).cloned(),
                _ => None,
            };
            let arg_ops: Vec<Operand> = args.iter().enumerate().map(|(i, a)| {
                if needs_boxed_callback {
                    if let HirExprKind::Lambda { params, body, captures, .. } = &a.kind {
                        return lower_lambda_boxed(params, body, captures, elem_ty_hint.as_ref(), b);
                    }
                    // A bare reference to `Some`/`Ok`/`Err` (`List.map(xs,
                    // Some)`, BACKLOG item 339) — unlike an ordinary named
                    // `fn`, these compiler intrinsics have no single fixed
                    // real signature (they're polymorphic constructors: the
                    // same `Some` reference means `Int -> Option<Int>` for a
                    // `List<Int>` and `Item -> Option<Item>` for a
                    // `List<Item>`), so — like an inline lambda, and unlike a
                    // real named `fn` — this needs the call site's own
                    // `elem_ty_hint` for the real concrete payload type. HIR
                    // has no such context at the bare-reference lowering
                    // site itself, so it only ever produces a uniformly
                    // erased `Ty::Var(0)` shape for these three names —
                    // correct for a still-generic list (the payload is
                    // already a boxed pointer, matching item 251) but wrong
                    // for a concrete one (a raw bit-packed `Int`, or a
                    // struct pointer needing `sizeof(Item)`-aware boxing,
                    // would otherwise be passed straight through as if
                    // already a valid `Option` pointer and segfault the
                    // first time something unboxed it).
                    if let HirExprKind::Global(fn_name) = &a.kind {
                        if matches!(fn_name.as_str(), "Some" | "Ok" | "Err") {
                            let payload_ty = elem_ty_hint.clone().unwrap_or(Ty::Var(0));
                            let ret_ty = match fn_name.as_str() {
                                "Some" => Ty::Option(Box::new(payload_ty.clone())),
                                "Ok"   => Ty::Result(Box::new(payload_ty.clone()), Box::new(Ty::Error)),
                                _      => Ty::Result(Box::new(Ty::Error), Box::new(payload_ty.clone())),
                            };
                            return lower_named_fn_boxed(fn_name, std::slice::from_ref(&payload_ty), &ret_ty, b);
                        }
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
                let declared = variant_field_types.as_ref().and_then(|tys| tys.get(i))
                    .or_else(|| fn_param_tys.as_ref().and_then(|tys| tys.get(i)));
                // A bare named-function reference passed where a
                // higher-kinded function's own declared parameter is itself
                // an *erased* `Ty::Fn` (e.g. `f: A => B`, BACKLOG item 76) —
                // its real native signature (`certo_double: Int -> Int`)
                // must be wrapped to match that erased shape, unboxing/
                // boxing at the boundary, not just wrapped 1:1 like an
                // ordinary named-function value (`wrap_named_fn_as_closure`,
                // which assumes the declared position wants the function's
                // own real types, true for an *ordinary* generic function
                // like item 108's `applyOne` but not for `F<_>`'s uniformly
                // erased signature).
                if let (HirExprKind::Global(fn_name), Ty::Fn { params: real_params, ret: real_ret }) = (&a.kind, &a.ty) {
                    if let Some(Ty::Fn { params: decl_params, ret: decl_ret }) = declared {
                        let erased = decl_params.iter().any(|t| matches!(t, Ty::Var(_))) || matches!(**decl_ret, Ty::Var(_));
                        if erased {
                            return wrap_named_fn_as_erased_closure(fn_name, real_params, real_ret, b);
                        }
                    }
                }
                let value_op = lower_value_expr(a, b);
                // Box only when the declared param/field is a bare type-param
                // AND the argument's own type is a *known concrete* type —
                // if the argument is itself `Ty::Var(_)` (e.g. `v` inside a
                // generic `fn wrap<T>(v: T) = Secret(v)`, where `v` is
                // already an opaque, already-boxed `void*` coming from
                // `wrap`'s own caller), boxing it again would wrap an extra,
                // spurious level of pointer indirection around a value MIR
                // has no way to interpret — the same double-boxing failure
                // class item 134 fixed for `List.first`/etc. — BACKLOG item 120.
                if matches!(declared, Some(Ty::Var(_))) && !matches!(a.ty, Ty::Var(_)) {
                    let boxed = b.declare_local("_boxed_arg", Ty::Var(0));
                    b.assign(boxed, Rvalue::BoxSome { value: value_op, ty: a.ty.clone() });
                    return Operand::Local(boxed);
                }
                // BACKLOG item 277 — a *direct* self-referential variant
                // field (e.g. `Node(left: Tree, right: Tree)`, where `left`'s
                // own declared type is `Tree`, the same type `Node` itself
                // belongs to) is stored as a pointer in C (`crates/codegen/
                // src/emit_module.rs`'s `field_c_ty`), never inline-by-value
                // — an inline `Tree` field inside `Node`'s own struct would
                // be an infinite-size type. Heap-box the argument here (the
                // plain, already-generic `Rvalue::Box`/`box_value`, not the
                // Option-specific `BoxSome`) so the constructor receives the
                // pointer its own C signature now expects.
                // BACKLOG item 308 — a *generic* self-referential field
                // (`Tree<T>`'s own `left: Tree<T>`) needs the same boxing as
                // the non-generic case just above's own comment describes;
                // dropped the `field_ty_args.is_empty()` gate that previously
                // limited this to a non-generic self-reference only.
                if let Some(Ty::Named { name: field_ty_name, .. }) = declared {
                    if Some(field_ty_name.as_str()) == enclosing_type.as_deref() {
                        let boxed = b.declare_local("_boxed_self_field", Ty::Var(0));
                        b.assign(boxed, Rvalue::Box { value: value_op, ty: a.ty.clone() });
                        return Operand::Local(boxed);
                    }
                }
                // The inverse direction (BACKLOG item 76): the argument is
                // itself still an erased `Ty::Var(0)` value (e.g. a
                // higher-kinded `F<A>` parameter, unlike an ordinary bare
                // `T` — Certo has no monomorphization, so `F` being unknown
                // means the *whole* value is opaque, not just one field of a
                // known struct), but the callee's *own* declared param is a
                // real, concrete type (`Box<T>`, passed by value as its own
                // struct — never pointer-sized on its own). Must unbox
                // (mirroring `BoxSome`'s own unconditional malloc-and-copy on
                // the way in) before the value can be used as that concrete
                // type at all.
                if let Some(concrete) = declared {
                    if matches!(a.ty, Ty::Var(_)) && !matches!(concrete, Ty::Var(_) | Ty::Error) {
                        let unboxed = b.declare_local("_unboxed_arg", concrete.clone());
                        b.assign(unboxed, Rvalue::UnboxSome { opt: value_op, ty: concrete.clone() });
                        return Operand::Local(unboxed);
                    }
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
            let stdlib_raw_return = matches!(&func.kind, HirExprKind::Global(name) if RAW_RETURN_CALLEES.contains(&name.as_str()))
                && (matches!(expr.ty, Ty::Float) || expr.ty.needs_heap_box());
            // Same idea, generalized to any user-defined generic function
            // whose *declared* return is a bare type param (`Ty::Var(_)` in
            // `fn_ret_tys`) — its C implementation always returns a raw
            // `void*` too. Only fires once HIR has actually resolved this
            // call's own type to something concrete (`resolve_bare_generic_
            // return`, BACKLOG item 135); an unresolved `Ty::Var`/`Ty::Error`
            // here means HIR already rejected the call outright, so nothing
            // to unbox.
            let user_generic_return = match &func.kind {
                HirExprKind::Global(name) => b.fn_ret_tys.get(name),
                _ => None,
            };
            let user_raw_return = matches!(user_generic_return, Some(Ty::Var(_)))
                && !matches!(expr.ty, Ty::Var(_) | Ty::Error);
            // Same idea again, generalized one step further: a call through
            // a *locally held* closure value (not a named global) whose own
            // declared return is erased (`Ty::Fn{ret: Ty::Var(_), ..}`) —
            // this is exactly the shape of the row-bound field-accessor
            // closures BACKLOG item 200 synthesizes (`crates/hir/src/
            // lower.rs`'s `Expr::Field` row-accessor arm calls one to read
            // `record.name`), boxed the same unconditional way `user_raw_
            // return`'s own callees are (via `Rvalue::BoxSome` in the
            // Lambda-lowering arm below, whenever `ret_hint` is `Ty::Var(_)`
            // — item 76's own HKT-closure convention), so it needs the
            // identical `UnboxSome` pairing, not `user_raw_return`'s
            // name-keyed lookup (which can't apply here at all — there's no
            // name to key on).
            let local_closure_raw_return = matches!(&func.kind, HirExprKind::Local(_))
                && matches!(&func.ty, Ty::Fn { ret, .. } if matches!(ret.as_ref(), Ty::Var(_)))
                && !matches!(expr.ty, Ty::Var(_) | Ty::Error);
            let needs_return_unbox = stdlib_raw_return || user_raw_return || local_closure_raw_return;

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
                &["List.first", "List.last", "List.get", "List.find", "Map.get", "Query.first",
                  // `Option.map` (both `certo_option_map` and
                  // `certo_option_map_boxed`) always returns via
                  // `certo_some`'s own always-`int64_t`-cell `__certo_
                  // opt_box`, same as the callees above — double-boxes
                  // identically when the *result* type needs heap-boxing.
                  "Option.map", "HostContext.service"];
            let needs_opt_unwrap = matches!(&func.kind, HirExprKind::Global(name) if OPT_UNWRAP_CALLEES.contains(&name.as_str()))
                && matches!(&expr.ty, Ty::Option(inner) if inner.needs_heap_box()
                    && !matches!(inner.as_ref(), Ty::Named { name, .. } if matches!(name.as_str(),
                        "Host" | "HostPlugin" | "HostContext" | "ServiceKey" | "RestartPolicy" |
                        "HostStatusSnapshot" | "HostWorkerStatus" | "HostMetricSnapshot" |
                        "HostState" | "HostWorkerState" | "HostLifecycleError" | "HostFailureKind")));

            let dest = b.declare_local("_call", if needs_return_unbox { Ty::Var(0) } else { expr.ty.clone() });
            let next = b.new_block();
            b.terminate(Terminator::Call { func: func_op, args: arg_ops, dest, next });
            b.switch_to(next);
            if needs_return_unbox {
                let real = b.declare_local("_call_unboxed", expr.ty.clone());
                // `stdlib_raw_return` (List.getOrPanic) and `user_raw_return`
                // (BACKLOG item 135) box their payload via two genuinely
                // different, incompatible schemes: `Rvalue::Unbox`/
                // `unbox_value` bit-packs pointer-sized scalars directly
                // into the slot with no allocation at all (only real
                // structs get malloc'd) — but a user-defined generic
                // function's bare-`T` value was boxed via item 120's
                // `Rvalue::BoxSome` at its *argument* site (or transitively
                // carries that same already-boxed pointer through a variant
                // payload/pattern-match, untouched), which *always* mallocs
                // regardless of the concrete type. Confirmed by direct
                // testing: using `Unbox`'s bit-pattern cast on a `BoxSome`'d
                // pointer printed the heap address itself instead of the
                // int it pointed to. `UnboxSome` always dereferences,
                // matching `BoxSome` unconditionally — the correct pairing
                // here.
                if user_raw_return || local_closure_raw_return {
                    b.assign(real, Rvalue::UnboxSome { opt: Operand::Local(dest), ty: expr.ty.clone() });
                } else {
                    b.assign(real, Rvalue::Unbox { value: Operand::Local(dest), ty: expr.ty.clone() });
                }
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
            // BACKLOG item 193 — every block gets its own defer scope, not
            // just the enclosing function's own top-level body. Pushed
            // before this block's own statements run (any `defer` among
            // them pushes onto *this* frame, via `HirStmt::Defer`'s own
            // lowering), popped and drained right here once the block's
            // own value is known — its natural exit — so a `defer` inside
            // an `if`/`match` arm fires when *that* arm's block ends, not
            // only once, whenever the enclosing function eventually
            // returns. Cheap even for the overwhelming majority of blocks
            // that never contain a `defer`: an empty frame's drain is a
            // zero-iteration loop, no generated-code difference at all.
            b.defer_stack.push(Vec::new());
            for stmt in stmts { lower_stmt(stmt, b); }
            let result = lower_expr(tail, b);
            let frame = b.defer_stack.pop().expect("Block must pop the frame it just pushed");
            for body in frame.iter().rev() {
                b.current_span = body.span;
                lower_expr(body, b);
            }
            result
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
                        let mir_local = b.bind_pattern_local(*local, name, Ty::Error);
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
                                // Dereference the payload only in the matched (non-null)
                                // block — never on the `None`/NULL path. A non-`Bind`
                                // payload sub-pattern (BACKLOG item 305 — e.g.
                                // `Some(0)`/`Some(Some(x))`) may itself branch further,
                                // so extract into its own block and explicitly
                                // `Goto(after_pat)` once done, rather than assuming
                                // `after_pat` is still `current` when extraction finishes.
                                let fields_bb = b.new_block();
                                b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: fields_bb, false_bb: next_arm_bb });
                                b.switch_to(fields_bb);
                                match fields.first() {
                                    Some(HirPat::Bind { local, name: fname }) => {
                                        let ml = b.bind_pattern_local(*local, fname, opt_payload.clone());
                                        b.assign(ml, Rvalue::UnboxSome {
                                            opt: scrut_op.clone(),
                                            ty:  opt_payload.clone(),
                                        });
                                    }
                                    Some(HirPat::Wildcard) | None => {}
                                    Some(field_pat) => {
                                        let payload = b.declare_local("_payload", opt_payload.clone());
                                        b.assign(payload, Rvalue::UnboxSome {
                                            opt: scrut_op.clone(),
                                            ty:  opt_payload.clone(),
                                        });
                                        check_nested_pattern(b, field_pat, &Operand::Local(payload), &opt_payload, next_arm_bb);
                                    }
                                }
                                b.terminate(Terminator::Goto(after_pat));
                                if arm.guard.is_some() { b.switch_to(after_pat); }
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
                                let mut pending_check = None;
                                for field_pat in fields.iter() {
                                    match field_pat {
                                        HirPat::Bind { local, name: fname } => {
                                            unwrap_result_payload(b, &scrut_op, *local, fname, &ok_ty);
                                        }
                                        HirPat::Wildcard => {}
                                        other => {
                                            let payload = b.declare_local("_payload", ok_ty.clone());
                                            unwrap_result_into(b, &scrut_op, &ok_ty, payload);
                                            pending_check = Some((other, payload));
                                        }
                                    }
                                }
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                // BACKLOG item 305 — a non-`Bind` payload sub-pattern
                                // (`Ok(0)`, say) needs its own check *after* confirming
                                // this is really the `Ok` arm, so it can't share
                                // `after_pat` as `is_ok`'s own `true_bb` directly (that
                                // would run it, if guard-less, in the very block that
                                // decides `is_ok`, which already has its terminator set).
                                match pending_check {
                                    None => {
                                        b.terminate(Terminator::If { cond: Operand::Local(is_ok), true_bb: after_pat, false_bb: next_arm_bb });
                                    }
                                    Some((pat, payload)) => {
                                        let check_bb = b.new_block();
                                        b.terminate(Terminator::If { cond: Operand::Local(is_ok), true_bb: check_bb, false_bb: next_arm_bb });
                                        b.switch_to(check_bb);
                                        check_nested_pattern(b, pat, &Operand::Local(payload), &ok_ty, next_arm_bb);
                                        b.terminate(Terminator::Goto(after_pat));
                                    }
                                }
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
                                let mut pending_check = None;
                                for field_pat in fields.iter() {
                                    match field_pat {
                                        HirPat::Bind { local, name: fname } => {
                                            unwrap_result_payload(b, &scrut_op, *local, fname, &err_ty);
                                        }
                                        HirPat::Wildcard => {}
                                        other => {
                                            let payload = b.declare_local("_payload", err_ty.clone());
                                            unwrap_result_into(b, &scrut_op, &err_ty, payload);
                                            pending_check = Some((other, payload));
                                        }
                                    }
                                }
                                let not_ok = b.declare_local("_not_ok", Ty::Bool);
                                b.assign(not_ok, Rvalue::UnOp { op: certo_hir::UnOp::Not, arg: Operand::Local(is_ok) });
                                let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                                // BACKLOG item 305 — see the identical "Ok" case above
                                // for why a non-`Bind` payload sub-pattern needs its own
                                // intermediate block rather than sharing `after_pat` as
                                // `not_ok`'s own `true_bb` directly.
                                match pending_check {
                                    None => {
                                        b.terminate(Terminator::If { cond: Operand::Local(not_ok), true_bb: after_pat, false_bb: next_arm_bb });
                                    }
                                    Some((pat, payload)) => {
                                        let check_bb = b.new_block();
                                        b.terminate(Terminator::If { cond: Operand::Local(not_ok), true_bb: check_bb, false_bb: next_arm_bb });
                                        b.switch_to(check_bb);
                                        check_nested_pattern(b, pat, &Operand::Local(payload), &err_ty, next_arm_bb);
                                        b.terminate(Terminator::Goto(after_pat));
                                    }
                                }
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
                                // BACKLOG item 277 — the enclosing sum type's
                                // own name (the `Tree` half of `Tree__Node`),
                                // needed below to detect a *direct* self-
                                // referential field (one whose own declared
                                // type is this same enclosing type) — such a
                                // field is stored as a pointer in C, never
                                // inline, so reading it needs an unbox, just
                                // like a bare-type-param field does.
                                let enclosing_type_name = name.split("__").next().unwrap_or(name);
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
                                // BACKLOG item 277 — three field-read shapes:
                                // no unboxing (an ordinary, inline-by-value
                                // field), `UnboxSome` (a bare type-param
                                // field, item 119's existing convention), or
                                // `Unbox` (a *direct* self-referential field
                                // — stored as a plain heap pointer, not an
                                // Option-style box, so it needs the plain
                                // unbox pairing that matches how the
                                // constructor call site boxed it).
                                enum FieldUnbox { None, OptionBoxed, SelfRefBoxed }
                                for (i, field_pat) in fields.iter().enumerate() {
                                    // BACKLOG item 305 — a non-`Bind` field sub-pattern
                                    // (a nested constructor like `Active`, a literal like
                                    // `30`) previously fell through this `if let Bind`
                                    // guard entirely: no read, no check, so the arm
                                    // matched unconditionally regardless of the field's
                                    // real value. `Wildcard` still needs nothing (matches
                                    // anything); anything else is checked recursively via
                                    // `check_nested_pattern`, same as `Bind`'s read below.
                                    if matches!(field_pat, HirPat::Wildcard) { continue; }
                                    let declared = &field_types[i];
                                    let (real_ty, unbox) = match declared {
                                        Ty::Var(_) => {
                                            let concrete = scrut_args.first().cloned().unwrap_or(Ty::Error);
                                            let unbox = if !matches!(concrete, Ty::Var(_)) { FieldUnbox::OptionBoxed } else { FieldUnbox::None };
                                            (concrete, unbox)
                                        }
                                        // BACKLOG item 308 — see the identical
                                        // comment on the sibling check in
                                        // `check_nested_pattern` above: dropped
                                        // the `args.is_empty()` gate so a
                                        // *generic* self-referential field
                                        // (`Tree<T>`'s own `left: Tree<T>`) gets
                                        // the same box/unbox treatment as the
                                        // non-generic case.
                                        Ty::Named { name: field_ty_name, .. } if field_ty_name == enclosing_type_name => {
                                            (declared.clone(), FieldUnbox::SelfRefBoxed)
                                        }
                                        other => (other.clone(), FieldUnbox::None),
                                    };
                                    let dest = match field_pat {
                                        HirPat::Bind { local, name: fname } => b.bind_pattern_local(*local, fname, real_ty.clone()),
                                        _ => b.declare_local("_field_val", real_ty.clone()),
                                    };
                                    // Read the payload directly via a nested path
                                    // `scrut.<variant>.<field>` — avoids an intermediate
                                    // local whose (anonymous struct) type we can't name.
                                    match unbox {
                                        FieldUnbox::OptionBoxed => {
                                            let raw = b.declare_local("_variant_field_raw", Ty::Var(0));
                                            b.assign(raw, Rvalue::Field {
                                                base: scrut_op.clone(),
                                                field: format!("{}.{}", variant, field_names[i]),
                                            });
                                            b.assign(dest, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: real_ty.clone() });
                                        }
                                        FieldUnbox::SelfRefBoxed => {
                                            let raw = b.declare_local("_variant_field_raw", Ty::Var(0));
                                            b.assign(raw, Rvalue::Field {
                                                base: scrut_op.clone(),
                                                field: format!("{}.{}", variant, field_names[i]),
                                            });
                                            b.assign(dest, Rvalue::Unbox { value: Operand::Local(raw), ty: real_ty.clone() });
                                        }
                                        FieldUnbox::None => {
                                            b.assign(dest, Rvalue::Field {
                                                base: scrut_op.clone(),
                                                field: format!("{}.{}", variant, field_names[i]),
                                            });
                                        }
                                    }
                                    if !matches!(field_pat, HirPat::Bind { .. }) {
                                        check_nested_pattern(b, field_pat, &Operand::Local(dest), &real_ty, next_arm_bb);
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
                    HirPat::Record { fields, field_names, field_types } => {
                        // A record has exactly one shape — always matches,
                        // no tag check needed (unlike `Constructor`'s
                        // sum-type tag comparison above). Each bound field
                        // is read directly off the scrutinee's own real C
                        // struct member (`Rvalue::Field`), with the same
                        // BACKLOG item 119/120 generic-unboxing guard
                        // `Constructor`'s own variant-field extraction uses:
                        // a field declared as a bare type param (`Ty::Var`)
                        // only gets unboxed when the scrutinee's own
                        // instantiation argument is itself concrete.
                        let scrut_args: &[Ty] = match &scrutinee.ty { Ty::Named { args, .. } => args, _ => &[] };
                        for (i, field_pat) in fields.iter().enumerate() {
                            // BACKLOG item 305 — see the identical comment on the
                            // `Constructor` field loop above: a non-`Bind` sub-pattern
                            // used to be silently skipped (no read, no check), so e.g.
                            // `{ status: Banned }` matched regardless of `status`'s
                            // real value.
                            if matches!(field_pat, HirPat::Wildcard) { continue; }
                            let declared = &field_types[i];
                            let (real_ty, needs_unbox) = match declared {
                                Ty::Var(_) => {
                                    let concrete = scrut_args.first().cloned().unwrap_or(Ty::Error);
                                    let needs_unbox = !matches!(concrete, Ty::Var(_));
                                    (concrete, needs_unbox)
                                }
                                other => (other.clone(), false),
                            };
                            let dest = match field_pat {
                                HirPat::Bind { local, name: fname } => b.bind_pattern_local(*local, fname, real_ty.clone()),
                                _ => b.declare_local("_field_val", real_ty.clone()),
                            };
                            if needs_unbox {
                                let raw = b.declare_local(&format!("_field_{}", field_names[i]), Ty::Var(0));
                                b.assign(raw, Rvalue::Field { base: scrut_op.clone(), field: field_names[i].clone() });
                                b.assign(dest, Rvalue::UnboxSome { opt: Operand::Local(raw), ty: real_ty.clone() });
                            } else {
                                b.assign(dest, Rvalue::Field { base: scrut_op.clone(), field: field_names[i].clone() });
                            }
                            if !matches!(field_pat, HirPat::Bind { .. }) {
                                check_nested_pattern(b, field_pat, &Operand::Local(dest), &real_ty, next_arm_bb);
                            }
                        }
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    HirPat::Tuple(fields) => {
                        // Tuple is stored as a CertoList*. Extract each field by index.
                        for (i, field_pat) in fields.iter().enumerate() {
                            // BACKLOG item 305 — same fix as `Record`/`Constructor`
                            // above: a non-`Bind` element sub-pattern (`(Some(x), y)`
                            // vs `(None, y)`) used to be silently skipped.
                            if matches!(field_pat, HirPat::Wildcard) { continue; }
                            let elem_local = match field_pat {
                                HirPat::Bind { local, name: fname } => b.bind_pattern_local(*local, fname, Ty::Error),
                                _ => b.declare_local("_tuple_elem", Ty::Error),
                            };
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
                            if !matches!(field_pat, HirPat::Bind { .. }) {
                                check_nested_pattern(b, field_pat, &Operand::Local(elem_local), &Ty::Error, next_arm_bb);
                            }
                        }
                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    HirPat::List { head, tail, elem_ty } => {
                        // BACKLOG item 195 — `[head, ...tail]`. Unlike `Tuple`
                        // (fixed arity, always matches) a list's length isn't
                        // known statically, so this is the first genuinely
                        // *fallible* structural pattern in this match
                        // compiler: a real runtime length check, branching to
                        // `next_arm_bb` (try the next arm) on failure, not
                        // just unconditional field extraction.
                        let len_local = b.declare_local("_len", Ty::Int);
                        let after_len_bb = b.new_block();
                        b.terminate(Terminator::Call {
                            func: Operand::Global("List.len".into()),
                            args: vec![scrut_op.clone()],
                            dest: len_local,
                            next: after_len_bb,
                        });
                        b.switch_to(after_len_bb);

                        // With a `...tail`, any length >= head.len() matches
                        // (the rest becomes tail); with no `...tail`, the
                        // length must match exactly.
                        let cmp = b.declare_local("_cmp", Ty::Bool);
                        b.assign(cmp, Rvalue::BinOp {
                            op:  if tail.is_some() { certo_hir::BinOp::GtEq } else { certo_hir::BinOp::Eq },
                            lhs: Operand::Local(len_local),
                            rhs: Operand::Const(MirConst::Int(head.len() as i64)),
                        });
                        let bind_bb = b.new_block();
                        b.terminate(Terminator::If { cond: Operand::Local(cmp), true_bb: bind_bb, false_bb: next_arm_bb });
                        b.switch_to(bind_bb);

                        for (i, field_pat) in head.iter().enumerate() {
                            if let HirPat::Bind { local, name: fname } = field_pat {
                                // Declared as the real element type, not
                                // `Ty::Error` — an `int64_t`-typed local
                                // holding e.g. a raw `Int` value is fine for
                                // arithmetic, but f-string interpolation and
                                // other type-directed codegen need to *know*
                                // it's `Int` to call `certo_int_to_text`
                                // rather than passing the raw value straight
                                // into a function expecting a real `Text`
                                // pointer — the exact "known type thrown
                                // away" bug class items 179/180/182/189/191
                                // already fixed elsewhere, caught here by a
                                // real end-to-end run before this shipped.
                                //
                                // A `Float` (or anything else too wide for a
                                // pointer-sized generic slot) additionally
                                // needs a real unbox after `getOrPanic`
                                // returns its raw `void*` — the exact same
                                // `List.getOrPanic`-then-`Rvalue::Unbox` idiom
                                // `HirExprKind::For`'s own MIR lowering
                                // already uses just above for this identical
                                // reason (see its own comment, ~line 888).
                                let elem_needs_unbox = matches!(elem_ty, Ty::Float) || elem_ty.needs_heap_box();
                                let raw_ty = if elem_needs_unbox { Ty::Var(0) } else { elem_ty.clone() };
                                let elem_raw = b.declare_local("_elem_raw", raw_ty);
                                let next_bb = b.new_block();
                                b.terminate(Terminator::Call {
                                    func: Operand::Global("List.getOrPanic".into()),
                                    args: vec![scrut_op.clone(), Operand::Const(MirConst::Int(i as i64))],
                                    dest: elem_raw,
                                    next: next_bb,
                                });
                                b.switch_to(next_bb);

                                let elem_local = b.bind_pattern_local(*local, fname, elem_ty.clone());
                                if elem_needs_unbox {
                                    b.assign(elem_local, Rvalue::Unbox { value: Operand::Local(elem_raw), ty: elem_ty.clone() });
                                } else {
                                    b.assign(elem_local, Rvalue::Use(Operand::Local(elem_raw)));
                                }
                            }
                        }
                        if let Some(tail_pat) = tail {
                            if let HirPat::Bind { local, name: fname } = tail_pat.as_ref() {
                                // Declared as the real `List<elem_ty>` (a
                                // `CertoList*` in C), not `Ty::Error`
                                // (`int64_t`) — `tail` is itself a real list,
                                // and an unsafe int64_t/pointer round-trip
                                // through a mis-declared local segfaulted the
                                // moment `tail` was passed to another List
                                // function, caught by a real end-to-end run
                                // before this shipped.
                                let list_ty = Ty::List(Box::new(elem_ty.clone()));
                                let tail_local = b.bind_pattern_local(*local, fname, list_ty);
                                let next_bb = b.new_block();
                                b.terminate(Terminator::Call {
                                    func: Operand::Global("List.slice".into()),
                                    args: vec![
                                        scrut_op.clone(),
                                        Operand::Const(MirConst::Int(head.len() as i64)),
                                        Operand::Local(len_local),
                                    ],
                                    dest: tail_local,
                                    next: next_bb,
                                });
                                b.switch_to(next_bb);
                            }
                        }

                        let after_pat = if arm.guard.is_some() { b.new_block() } else { arm_bb };
                        b.terminate(Terminator::Goto(after_pat));
                        if arm.guard.is_some() { b.switch_to(after_pat); }
                    }
                    // BACKLOG item 343 — an or-pattern (`1 | 2 => ..`,
                    // `Circle(_) | Square(_) => ..`) used to fall into the
                    // catch-all below, which jumps straight to the arm body
                    // with no test at all, so a top-level `|` arm matched
                    // *every* scrutinee. `check_nested_pattern` already
                    // implements the real "try left, else try right, else
                    // fall to the next arm" test (it's how a `|` nested
                    // inside `Some(..)`/a tuple always worked) — reuse it,
                    // then continue into the arm exactly like every other
                    // pattern kind.
                    HirPat::Or(..) => {
                        check_nested_pattern(b, &arm.pat, &scrut_op, &scrutinee.ty, next_arm_bb);
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
                let value_op = lower_value_expr(v, b);
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
            let ops: Vec<Operand> = elems.iter().map(|e| lower_value_expr(e, b)).collect();
            let dest = b.declare_local("_tup", expr.ty.clone());
            b.assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            Operand::Local(dest)
        }

        HirExprKind::List(elems) => {
            let ops: Vec<Operand> = elems.iter().map(|e| lower_value_expr(e, b)).collect();
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

        HirExprKind::Lambda { params, body, captures, ret_hint } => {
            // Lift the lambda to a top-level MIR function named `__lam_<parent>_N`.
            let idx = b.lambda_count;
            b.lambda_count += 1;
            let lam_name = format!("__lam_{}_{}", b.fn_name, idx);

            // Build the closure environment in the *enclosing* builder,
            // before lowering the lambda's own body — BACKLOG item 140.
            let capture_tys = capture_types(captures, b);
            let env = build_capture_env(captures, &capture_tys, b);

            // Build MIR for the lambda body using a fresh builder.
            let mut lb = Builder::new(&lam_name, b.record_field_types.clone(), b.variant_field_types.clone(), b.fn_param_tys.clone(), b.fn_ret_tys.clone(), b.variant_to_type.clone());
            let ret_slot = lb.declare_local("_ret", Ty::Error);
            // The env parameter always comes first, ahead of the lambda's
            // own real parameters — every generated lambda function agrees
            // on this shape uniformly, whether or not it actually captures
            // anything (BACKLOG item 140; see `emit_callee`).
            let env_param = lb.declare_local("_env", Ty::Error);
            for p in params {
                lb.map_hir_local(p.local, &p.name, p.ty.clone());
            }
            bind_captures(captures, &capture_tys, env_param, &mut lb);
            lb.current_span = body.span;
            let lam_result = lower_expr(body, &mut lb);
            let inferred_ret_ty = infer_operand_ty(&lam_result, &lb);
            // The declared position this lambda was passed into expects an
            // erased result (e.g. a higher-kinded `F<B>` return, BACKLOG
            // item 76 — `wrap: B => F<B>`'s real body, `Box.wrap(x)`,
            // returns the concrete `Box` struct by value, which can't
            // possibly satisfy the closure's uniform `void*`-returning ABI
            // without boxing first, mirroring `BoxSome`'s own unconditional
            // malloc-and-copy already used for every other value crossing
            // into an erased slot).
            let needs_return_box = matches!(ret_hint, Ty::Var(_)) && !matches!(inferred_ret_ty, Ty::Var(_));
            let lam_ret_ty = if needs_return_box { Ty::Var(0) } else { inferred_ret_ty.clone() };
            lb.locals[ret_slot as usize].ty = lam_ret_ty.clone();
            if needs_return_box {
                lb.assign(ret_slot, Rvalue::BoxSome { value: lam_result, ty: inferred_ret_ty });
            } else if !matches!(lam_ret_ty, Ty::Unit) {
                lb.assign(ret_slot, Rvalue::Use(lam_result));
            }
            emit_defers_then_return(Operand::Local(ret_slot), &mut lb);

            let lam_fn = MirFn {
                name: lam_name.clone(),
                param_count: params.len() + 1,
                locals: lb.locals,
                blocks: lb.blocks,
            };
            // Propagate any nested lambdas lifted inside this lambda.
            b.lifted_fns.extend(lb.lifted_fns);
            b.lifted_fns.push(lam_fn);

            // The closure's own declared `Ty::Fn` must match the generated
            // function's *real* native param/return types exactly — unlike
            // `lower_lambda_boxed`'s result, this value may later be called
            // directly through a local (BACKLOG item 108's `emit_callee`
            // path), which reads its cast signature straight off this type.
            let lam_ty = Ty::Fn {
                params: params.iter().map(|p| p.ty.clone()).collect(),
                ret:    Box::new(lam_ret_ty),
            };
            make_closure(&lam_name, env, lam_ty, b)
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
            //
            // BACKLOG item 321 — `e?` on an `Option<T>` operand takes a
            // different branch entirely: only reachable when typeck
            // confirmed the enclosing function itself also returns
            // `Option<_>` (`crates/typeck/src/infer_expr.rs`'s own
            // `Expr::Try` arm). `Option<T>`'s runtime shape is a raw `void*`
            // (`None` = NULL, `Some(v)` a boxed pointer, per
            // `Rvalue::BoxSome`/`UnboxSome`) — there is no `__result_is_ok`-
            // style tag call to make here; branch directly on a null check,
            // mirroring `check_nested_pattern`'s own `Constructor("None",
            // ...)` arm exactly. Propagating `_result` itself on the `None`
            // path is correct regardless of this function's own payload
            // type, since NULL already *is* a valid `Option<_>` for any `_`.

            let inner_op = lower_expr(inner, b);

            // Store the result value so we can reference it in both branches.
            let result_local = b.declare_local("_result", Ty::Error); // void*
            b.assign(result_local, Rvalue::Use(inner_op));

            if matches!(inner.ty, Ty::Option(_)) {
                let is_none_local = b.declare_local("_is_none", Ty::Bool);
                b.assign(is_none_local, Rvalue::BinOp {
                    op:  certo_hir::BinOp::Eq,
                    lhs: Operand::Local(result_local),
                    rhs: Operand::Global("__NULL".into()),
                });
                let ok_block  = b.new_block();
                let err_block = b.new_block();
                b.terminate(Terminator::If {
                    cond:     Operand::Local(is_none_local),
                    true_bb:  err_block,
                    false_bb: ok_block,
                });

                // err_block: run defers then propagate the None.
                b.switch_to(err_block);
                emit_defers_then_return(Operand::Local(result_local), b);

                // ok_block: unbox the Some payload.
                b.switch_to(ok_block);
                let val_local = b.declare_local("_try_val", expr.ty.clone());
                b.assign(val_local, Rvalue::UnboxSome { opt: Operand::Local(result_local), ty: expr.ty.clone() });
                return Operand::Local(val_local);
            }

            // Result<T, E> — call __result_is_ok → bool
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

        // Early exit from the function — BACKLOG item 342 (`guard cond else e`).
        // Mirrors `lower_fn`'s own final return exactly: assign the value into
        // the return slot (local 0, always the first local declared — skipped
        // for a Unit value, which has no `_l0` to write), then run every
        // open `defer` scope and return, same as `?`'s propagation branch.
        // Typeck (E0222) guarantees this never appears inside a lambda or
        // task body, whose own builders have a differently-typed/boxed return
        // slot this would bypass. Everything lowered after it in the same
        // block is unreachable, so continue into a fresh dead block.
        HirExprKind::Return(inner) => {
            let value = lower_value_expr(inner, b);
            if !matches!(infer_operand_ty(&value, b), Ty::Unit) {
                b.assign(0, Rvalue::Use(value));
            }
            emit_defers_then_return(Operand::Local(0), b);
            let dead = b.new_block();
            b.switch_to(dead);
            Operand::Const(MirConst::Unit)
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
            //
            // BACKLOG item 253 — `certo_list_get_or_panic` always returns a
            // raw `void*` list-element slot; assigning it straight to
            // `binding_local` only compiles when the element's own C type is
            // itself pointer/integer-sized and implicitly convertible from
            // `void*` (fine for Int/Bool/etc, a hard C compile error for any
            // record/sum-type struct, which C has no implicit conversion
            // for at all). Mirrors `lower_sum_by_call`'s own identical
            // `elem_needs_unbox` handling just below in this same file
            // (added for item 162b, whose own doc comment already
            // — incorrectly, until this fix — claimed this `For` arm handled
            // the general case too): for anything `Ty::needs_heap_box()`
            // (records, sum types, `Decimal`, `Uuid`, `Fn`) the raw slot is
            // first captured in a `Ty::Var(0)` (real C type `void*`) local,
            // then `Rvalue::Unbox` recovers the real value via the same
            // general-purpose `unbox_value` helper `emit_mir.rs`'s own
            // tuple-field-access path already uses — a genuine heap
            // dereference for these, not the bit-reinterpretation the old
            // code wrongly also applied to `Decimal` here (a real, distinct,
            // latent bug this fix also happens to close: `Decimal`'s C
            // representation is a `{value: int64_t, scale: int8_t}` struct,
            // not a bit-castable scalar `certo_i2f` could ever validly
            // handle). `Ty::Float` keeps its own existing, correct
            // bit-reinterpretation path — `unbox_value` already special-
            // cases it identically to what this arm did directly before.
            b.switch_to(loop_body_bb);
            let binding_local = b.map_hir_local(*binding, binding_name, elem_ty.clone());
            let elem_done = b.new_block();
            let elem_needs_unbox = matches!(elem_ty, Ty::Float) || elem_ty.needs_heap_box();
            if elem_needs_unbox {
                let raw = b.declare_local("_elem_raw", Ty::Var(0));
                b.terminate(Terminator::Call {
                    func: Operand::Global("List.getOrPanic".into()),
                    args: vec![iter_op.clone(), Operand::Local(i_local)],
                    dest: raw,
                    next: elem_done,
                });
                b.switch_to(elem_done);
                b.assign(binding_local, Rvalue::Unbox { value: Operand::Local(raw), ty: elem_ty.clone() });
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
            loop_back_edge(b, loop_test_bb, loop_exit_bb);

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
            loop_back_edge(b, loop_header_bb, loop_exit_bb);

            b.switch_to(loop_exit_bb);
            Operand::Const(MirConst::Unit)
        }

        HirExprKind::Spawn { fn_name: _, args, captures } => {
            // `spawn f(a, b)` — run f on a new OS thread, evaluating the arguments
            // eagerly in the current thread first (a direct call to a named
            // function needs no lifting at all: its own argument expressions
            // already run in the *current* thread before the worker starts,
            // exactly matching ordinary eager call-argument evaluation).
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
                    b.assign(dest, Rvalue::Spawn { func: func_op, args: arg_ops, ret_ty: ret_ty.clone(), is_lifted: false });
                    return Operand::Local(dest);
                }
            }
            // Any other body shape (a block, `if`, `while`, ...) must run in
            // full on the worker thread — lift it into a synthesized
            // top-level function taking its captures as ordinary real-typed
            // parameters, then spawn a call to *that* (BACKLOG item 141;
            // this replaces the old "evaluate eagerly, not actually
            // concurrent" fallback, which silently hung the program for any
            // spawn body with no natural exit, e.g. `while true { ... }`).
            if let Some(body) = inner {
                let (lifted_name, arg_ops, ret_ty) = lift_spawn_body(body, captures, b);
                let task_ty = certo_typeck::Ty::Named { name: "__CertoTask".into(), args: vec![ret_ty.clone()] };
                let dest = b.declare_local("__task", task_ty);
                b.assign(dest, Rvalue::Spawn { func: Operand::Global(lifted_name), args: arg_ops, ret_ty, is_lifted: true });
                return Operand::Local(dest);
            }
            // `args` is always exactly one element in practice — no real
            // spawn body to run.
            let dest = b.declare_local("__task", certo_typeck::Ty::Error);
            b.assign(dest, Rvalue::Use(Operand::Const(MirConst::Int(0))));
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

        HirExprKind::JoinTimedCancel { task, deadline } => {
            let task_op = lower_expr(task, b);
            let deadline_op = Operand::Local(b.get_local(*deadline));
            // Same real-task-vs-sequential-fallback split as `AwaitTimed`.
            if let certo_typeck::Ty::Named { name, args } = infer_operand_ty(&task_op, b) {
                if name == "__CertoTask" {
                    let ret_ty = args.into_iter().next().unwrap_or(certo_typeck::Ty::Error);
                    let opt_ty = certo_typeck::Ty::Option(Box::new(ret_ty.clone()));
                    let dest = b.declare_local("__with_timeout_result", opt_ty);
                    b.assign(dest, Rvalue::JoinTimedCancel { task: task_op, deadline: deadline_op, ret_ty });
                    return Operand::Local(dest);
                }
            }
            // Fallback: sequential-eval handle already holds the value
            // directly — nothing was actually spawned or waited on, so it
            // unconditionally "made the deadline": box it as `Some(value)`
            // rather than treating the moot deadline as a timeout.
            let val_ty = infer_operand_ty(&task_op, b);
            let opt_ty = certo_typeck::Ty::Option(Box::new(val_ty.clone()));
            let dest = b.declare_local("__with_timeout_result", opt_ty);
            b.assign(dest, Rvalue::BoxSome { value: task_op, ty: val_ty });
            Operand::Local(dest)
        }
    }
}

fn lower_stmt(stmt: &HirStmt, b: &mut Builder) {
    match stmt {
        HirStmt::Let { local, name, ty, init } => {
            b.current_span = init.span;
            let init_op  = lower_value_expr(init, b);
            // When the declared type is unknown (Ty::Error), recover it from the
            // initialiser. This preserves e.g. a `__CertoTask<R>` spawn handle so
            // a later `await` can join it with the right result type.
            let decl_ty = if matches!(ty, Ty::Error) { infer_operand_ty(&init_op, b) } else { ty.clone() };
            let mir_local = b.map_hir_local(*local, name, decl_ty);
            b.assign(mir_local, Rvalue::Use(init_op));
        }
        HirStmt::Assign { local, value } => {
            b.current_span = value.span;
            let val_op   = lower_value_expr(value, b);
            let mir_local = b.get_local(*local);
            b.assign(mir_local, Rvalue::Use(val_op));
        }
        HirStmt::Expr(e) => {
            b.current_span = e.span;
            lower_expr(e, b);
        }
        HirStmt::Defer { body } => {
            // Pushes onto the *innermost* open scope frame (BACKLOG item
            // 193) — the `HirExprKind::Block` currently being lowered,
            // which always has a frame open by the time any of its own
            // statements (including this one) are reached; see that arm's
            // own push/pop.
            b.defer_stack.last_mut()
                .expect("HirStmt::Defer reached with no open defer scope — every defer sits inside some Block, which always pushes a frame first")
                .push(body.clone());
        }
    }
}
