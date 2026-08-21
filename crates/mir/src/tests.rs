use certo_parser::parse;
use certo_hir::{lower_module, HirItem};
use crate::{lower_fn, Terminator, Rvalue, Operand, MirConst, MirStmt};

fn mir_fn(src: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let HirItem::Fn(f) = &hir.items[0] else { panic!("expected fn"); };
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys).0
}

fn mir_fn_named(src: &str, name: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let f = hir.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn named {name}"));
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys).0
}

fn mir_fn_and_lifted_named(src: &str, name: &str) -> (crate::MirFn, Vec<crate::MirFn>) {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let f = hir.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn named {name}"));
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys)
}

#[test]
fn literal_return() {
    let mf = mir_fn("module A\nfn answer(): Int = 42");
    // entry block should assign 42 to _ret and return it
    let entry = &mf.blocks[0];
    assert!(entry.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Use(Operand::Const(MirConst::Int(42))), .. }
    )));
    assert!(matches!(entry.terminator, Some(Terminator::Return(_))));
}

#[test]
fn binop_produces_assign() {
    let mf = mir_fn("module A\nfn add(a: Int, b: Int): Int = a + b");
    let has_binop = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::BinOp { .. }, .. }
    )));
    assert!(has_binop, "expected BinOp assign");
}

#[test]
fn if_expr_creates_three_blocks() {
    let mf = mir_fn("module A\nfn f(b: Bool): Int = if b then 1 else 2");
    // entry + then + else + join = at least 4 blocks
    assert!(mf.blocks.len() >= 4, "expected >=4 blocks for if, got {}", mf.blocks.len());
    let has_if_term = mf.blocks.iter().any(|bb| matches!(bb.terminator, Some(Terminator::If { .. })));
    assert!(has_if_term, "expected If terminator");
}

#[test]
fn call_creates_call_terminator() {
    let mf = mir_fn("module A\nfn f(x: Int): Int = g(x)\nfn g(x: Int): Int = x");
    let has_call = mf.blocks.iter().any(|bb| matches!(bb.terminator, Some(Terminator::Call { .. })));
    assert!(has_call, "expected Call terminator");
}

#[test]
fn block_with_let() {
    let mf = mir_fn("module A\nfn f(): Int = {\n    val x: Int = 10\n    x\n}");
    // x = 10 should appear as an Assign in some block
    let has_ten = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Use(Operand::Const(MirConst::Int(10))), .. }
    )));
    assert!(has_ten, "expected assignment of 10");
}

// ------------------------------------------------------------------ //
// Generic function call-site argument boxing (BACKLOG item 120)
// ------------------------------------------------------------------ //

#[test]
fn generic_call_boxes_concrete_argument() {
    // Calling a generic impl method with a concrete argument (`42`) into its
    // bare-`T` parameter must heap-box that argument, since the method's C
    // signature is uniformly `void*` for `T` — previously this argument was
    // passed through as a raw pointer/bit-cast with no boxing at all.
    let mf = mir_fn_named(
        "module A\ntype Box<T> = | Wrap(T)\nimpl<T> Box { fn wrap(v: T): Box<T> = Wrap(v) }\nfn f(): Box<Int> = Box.wrap(42)",
        "f");
    let has_boxsome = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::BoxSome { .. }, .. }
    )));
    assert!(has_boxsome, "expected a BoxSome for the concrete argument passed into Box.wrap's bare-T param");
}

#[test]
fn generic_call_does_not_double_box_already_opaque_argument() {
    // `wrapTwice`'s own body calls `Box.wrap(v)` where `v` is `wrapTwice`'s
    // own bare-`T` parameter — already an opaque, already-boxed `void*`
    // coming from `wrapTwice`'s own caller. Re-boxing it here would wrap an
    // extra, spurious level of pointer indirection (the same double-boxing
    // failure class item 134 fixed for `List.first`/etc.) — so this call
    // site must NOT emit a BoxSome, unlike `generic_call_boxes_concrete_argument`.
    let mf = mir_fn_named(
        "module A\ntype Box<T> = | Wrap(T)\nimpl<T> Box { fn wrap(v: T): Box<T> = Wrap(v)\n fn wrapTwice(v: T): Box<T> = Box.wrap(v) }",
        "Box.wrapTwice");
    let has_boxsome = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::BoxSome { .. }, .. }
    )));
    assert!(!has_boxsome, "wrapTwice's call to Box.wrap(v) must not re-box the already-opaque `v`");
}

#[test]
fn generic_call_unboxes_resolved_bare_return_via_unbox_some() {
    // BACKLOG item 135: a bare-`T`-returning generic function's payload was
    // boxed at its *argument* site via `Rvalue::BoxSome` (see
    // `generic_call_boxes_concrete_argument` above), which always mallocs
    // regardless of the concrete type — so unboxing it back at the call
    // site must use `Rvalue::UnboxSome` (always dereferences) to match, not
    // `Rvalue::Unbox`/`unbox_value` (a *different* scheme that bit-packs
    // pointer-sized scalars directly into the slot, no allocation at all).
    // Confirmed by direct testing that using the wrong one prints the raw
    // heap address instead of the value it points to.
    let mf = mir_fn_named(
        "module A\ntype Box<T> = priv Box(T)\nimpl<T> Box { fn wrap(v: T): Box<T> = Box(v)\n fn unwrap(b: Box<T>): T = match b { Box(v) => v } }\nfn f(): Int = {\n val x: Int = Box.unwrap(Box.wrap(42))\n x\n}",
        "f");
    let has_unbox_some = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::UnboxSome { .. }, .. }
    )));
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(has_unbox_some, "expected an UnboxSome for the resolved bare-T return value");
    assert!(!has_unbox, "must not use the mismatched Unbox/unbox_value scheme here");
}

// ------------------------------------------------------------------ //
// List rest-pattern `[head, ...tail]` — BACKLOG item 195
// ------------------------------------------------------------------ //

fn has_global_call(mf: &crate::MirFn, name: &str) -> bool {
    mf.blocks.iter().any(|bb| matches!(&bb.terminator,
        Some(Terminator::Call { func: Operand::Global(g), .. }) if g == name))
}

#[test]
fn list_pattern_with_tail_calls_len_get_and_slice() {
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Int = match xs {\n [a, b, ...rest] => a,\n _ => 0\n}");
    assert!(has_global_call(&mf, "List.len"), "expected a List.len call for the runtime length check");
    assert!(has_global_call(&mf, "List.getOrPanic"), "expected List.getOrPanic calls for the head elements");
    assert!(has_global_call(&mf, "List.slice"), "expected a List.slice call for the tail binding");
}

#[test]
fn list_pattern_without_tail_never_calls_slice() {
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Int = match xs {\n [a, b] => a,\n _ => 0\n}");
    assert!(has_global_call(&mf, "List.len"), "expected a List.len call for the runtime length check");
    assert!(!has_global_call(&mf, "List.slice"), "no `...rest` in the pattern — must not call List.slice at all");
}

#[test]
fn list_pattern_with_tail_uses_gteq_length_check() {
    // With `...tail`, any length >= head.len() must match (the rest becomes
    // tail) — an exact-length `==` check here would wrongly reject a longer
    // list instead of binding the extra elements to `rest`.
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Int = match xs {\n [a, ...rest] => a,\n _ => 0\n}");
    let has_gteq = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::BinOp { op: certo_hir::BinOp::GtEq, .. }, .. }
    )));
    assert!(has_gteq, "expected a >= length comparison when `...tail` is present");
}

#[test]
fn list_pattern_without_tail_uses_exact_eq_length_check() {
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Int = match xs {\n [a, b] => a,\n _ => 0\n}");
    let has_eq = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::BinOp { op: certo_hir::BinOp::Eq, .. }, .. }
    )));
    let has_gteq = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::BinOp { op: certo_hir::BinOp::GtEq, .. }, .. }
    )));
    assert!(has_eq, "expected an exact == length comparison with no `...tail`");
    assert!(!has_gteq, "must not use >= when the pattern has no `...tail`");
}

#[test]
fn list_pattern_float_head_element_gets_unboxed() {
    // Regression for the confirmed crash: `certo_list_get_or_panic` always
    // returns a raw `void*` — a `Float` element (too wide for a
    // pointer-sized generic slot) needs a real `Rvalue::Unbox` afterward,
    // not a direct assignment, or the C compiler rejects assigning `void*`
    // straight into a `double`-typed local.
    let mf = mir_fn("module A\nfn f(xs: List<Float>): Float = match xs {\n [a, ...rest] => a,\n _ => 0.0\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(has_unbox, "expected an Unbox for the Float head element");
}

#[test]
fn list_pattern_int_head_element_does_not_get_unboxed() {
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Int = match xs {\n [a, ...rest] => a,\n _ => 0\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(!has_unbox, "Int is pointer-sized — no unbox should be needed");
}

// ------------------------------------------------------------------ //
// Cooperative-cancellation checkpoint for spawn worker loops — BACKLOG item 186
// ------------------------------------------------------------------ //

#[test]
fn spawn_block_body_while_loop_gets_cancel_checkpoint() {
    // An inline `spawn { ... }` block body (unlike `spawn f(a, b)`, a direct
    // call to an existing function) is lowered into a dedicated function via
    // `lift_spawn_body`, which now always gets one hidden trailing
    // cancel-token parameter, and every `while`/`for` loop lowered directly
    // into that function's own control flow checks it before looping back.
    let (_, lifted) = mir_fn_and_lifted_named(
        "module A\nfn f(): Unit [async] = spawn {\n  var i = 0\n  while i < 3 {\n    i = i + 1\n  }\n}",
        "f");
    let worker = lifted.iter().find(|lf| lf.name.starts_with("__spawn_"))
        .expect("expected a lifted spawn worker function");
    assert_eq!(worker.param_count, 1, "no real captures here, so param_count must be exactly 1 (the hidden cancel-token param)");
    let has_checkpoint_call = worker.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Call { func: Operand::Global(name), .. }, .. } if name == "__certo_task_hdr_is_abandoned"
    )));
    assert!(has_checkpoint_call, "expected a checkpoint call to __certo_task_hdr_is_abandoned in the loop's lifted body, got: {:?}", worker.blocks);
}

#[test]
fn spawn_direct_named_call_is_not_lifted_and_gets_no_checkpoint() {
    // `spawn f(a, b)` — a direct call to an existing, possibly-shared named
    // function — must NOT be marked lifted, and must NOT synthesize a
    // dedicated `__spawn_*` wrapper: the existing function's own signature
    // (and every other, non-spawn call site of it) must stay untouched.
    let (mf, lifted) = mir_fn_and_lifted_named(
        "module A\nfn work(): Unit = unit\nfn f(): Unit [async] = spawn work()",
        "f");
    let is_lifted = mf.blocks.iter().flat_map(|bb| &bb.stmts).find_map(|s| match s {
        MirStmt::Assign { rvalue: Rvalue::Spawn { is_lifted, .. }, .. } => Some(*is_lifted),
        _ => None,
    });
    assert_eq!(is_lifted, Some(false), "a direct spawn of a named function must not be marked lifted");
    assert!(!lifted.iter().any(|lf| lf.name.starts_with("__spawn_")),
        "a direct named-function spawn must not synthesize a lifted wrapper, got: {:?}", lifted.iter().map(|lf| &lf.name).collect::<Vec<_>>());
}

#[test]
fn ordinary_function_while_loop_has_no_checkpoint() {
    // No-regression check: an ordinary top-level function's own `while`
    // loop (never lifted, never inside a spawn body) must keep its plain
    // unconditional back-edge — no cancel-token param, no checkpoint call.
    let mf = mir_fn_named(
        "module A\nfn f(): Unit = {\n  var i = 0\n  while i < 3 {\n    i = i + 1\n  }\n}",
        "f");
    assert_eq!(mf.param_count, 0);
    let has_checkpoint_call = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Call { func: Operand::Global(name), .. }, .. } if name == "__certo_task_hdr_is_abandoned"
    )));
    assert!(!has_checkpoint_call, "an ordinary function's while loop must not get a cancellation checkpoint");
}

// ------------------------------------------------------------------ //
// Scope-aware `defer` — BACKLOG item 193
// ------------------------------------------------------------------ //

/// Every named-function call target across a `MirFn`, in true control-flow
/// order — a DFS over the block graph (`Goto`/`If`/`Call`'s own `next`/
/// `Switch`), not the raw `blocks` vector order. Block *creation* order
/// doesn't reliably match execution order: `HirExprKind::If`'s own
/// lowering creates `then_bb`/`else_bb`/`join_bb` up front, before either
/// branch is lowered into, so a call inside `then_bb` that itself needs a
/// continuation block gets a *higher* block id than `join_bb` despite
/// running strictly before it — walking `blocks` by index would report
/// that call as happening *after* whatever runs post-join, which is
/// backwards. Local function names only (`Operand::Global`) — MIR keeps
/// the bare Certo name, not codegen's later `certo_`-prefixed C name.
fn call_trace(mf: &crate::MirFn) -> Vec<String> {
    let mut out = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![0usize];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) { continue; }
        let bb = &mf.blocks[id];
        for stmt in &bb.stmts {
            if let MirStmt::Assign { rvalue: Rvalue::Call { func: Operand::Global(name), .. }, .. } = stmt {
                out.push(name.clone());
            }
        }
        match &bb.terminator {
            Some(Terminator::Goto(next)) => stack.push(*next),
            Some(Terminator::If { true_bb, false_bb, .. }) => { stack.push(*false_bb); stack.push(*true_bb); }
            Some(Terminator::Call { func: Operand::Global(name), next, .. }) => {
                out.push(name.clone());
                stack.push(*next);
            }
            Some(Terminator::Switch { targets, otherwise, .. }) => {
                stack.push(*otherwise);
                for (_, bb_id) in targets { stack.push(*bb_id); }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn defer_inside_an_if_branch_fires_before_code_that_follows_the_if_not_at_functions_end() {
    // The exact bug BACKLOG item 193 fixed: a `defer` used to only ever
    // fire once, right before the *enclosing function's* own final
    // return — not at the natural end of whatever block directly
    // contains it. Here, `sideEffect()` (the deferred call) must appear
    // *before* `afterward()` in call order, since the defer sits inside
    // the `if`'s own true-branch and must fire when that branch ends —
    // under the old bug, `sideEffect` would only ever appear at the very
    // end of the trace, after `afterward`.
    let mf = mir_fn_named(
        "module A\nfn other(): Unit = unit\nfn sideEffect(): Unit = unit\nfn afterward(): Unit = unit\n\
         fn f(): Unit = {\n  if true then {\n    defer { sideEffect() }\n    other()\n  } else { unit }\n  afterward()\n}",
        "f");
    let trace = call_trace(&mf);
    let side_effect_pos = trace.iter().position(|n| n == "sideEffect")
        .unwrap_or_else(|| panic!("sideEffect() never called at all, got trace: {:?}", trace));
    let afterward_pos = trace.iter().position(|n| n == "afterward")
        .unwrap_or_else(|| panic!("afterward() never called at all, got trace: {:?}", trace));
    assert!(side_effect_pos < afterward_pos,
        "deferred call must fire before code following the if, got trace: {:?}", trace);
}

#[test]
fn defer_at_the_top_level_of_a_function_still_fires_at_its_own_end_unaffected() {
    // No-regression check: a `defer` that already sat directly in the
    // function's own top-level body (the one shape every pre-193 test
    // happened to use, which is why this bug was invisible until now)
    // must keep working exactly as before.
    let mf = mir_fn_named(
        "module A\nfn cleanup(): Unit = unit\nfn work(): Unit = unit\n\
         fn f(): Unit = {\n  defer { cleanup() }\n  work()\n}",
        "f");
    let trace = call_trace(&mf);
    assert_eq!(trace, vec!["work", "cleanup"],
        "work() must run before the deferred cleanup(), which fires at the function's own end");
}

#[test]
fn defer_inside_an_early_exit_branch_fires_before_the_error_propagates() {
    // The other real exit path (BACKLOG item 193's own scope): a `defer`
    // inside a block that exits early via `?` must fire before that
    // early return actually happens. `?`'s own lowering (`HirExprKind::Try`)
    // always statically emits *both* branches — the never-taken success
    // continuation (`unreachable()` here, since `fail()` always errors)
    // genuinely exists in the compiled MIR even though this specific
    // runtime never reaches it, so a whole-graph call trace correctly
    // contains `unreachable` too; that's not a bug, and the real
    // "unreachable never actually runs" property was already confirmed
    // by a real compiled program (BACKLOG item 193's own end-to-end
    // verification). What a static MIR test *can* pin precisely: the
    // specific block that `?`'s error branch returns from — identified
    // directly, not inferred from trace order — must itself contain the
    // deferred `cleanup()` call before its own `Return`.
    let mf = mir_fn_named(
        "module A\nfn fail(): Result<Unit, Text> = Err(\"boom\")\nfn cleanup(): Unit = unit\nfn unreachable(): Unit = unit\n\
         fn f(): Result<Unit, Text> = {\n  defer { cleanup() }\n  fail()?\n  unreachable()\n}",
        "f");
    // An ordinary user-function call like `cleanup()` lowers via
    // `Terminator::Call` (its own continuation block, `next`), not
    // `Rvalue::Call` (which is reserved for internal helper calls like
    // `__result_unwrap`) — so the block calling `cleanup` and the block
    // that actually returns are two directly-linked blocks, not one.
    let cleanup_call = mf.blocks.iter().find_map(|bb| match &bb.terminator {
        Some(Terminator::Call { func: Operand::Global(name), next, .. }) if name == "cleanup" => Some(*next),
        _ => None,
    });
    let next_id = cleanup_call.unwrap_or_else(|| panic!("no call to cleanup() found at all, got blocks: {:#?}", mf.blocks));
    assert!(matches!(mf.blocks[next_id].terminator, Some(Terminator::Return(_))),
        "the block right after the deferred cleanup() call must return — got: {:#?}", mf.blocks[next_id]);
}
