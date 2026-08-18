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
