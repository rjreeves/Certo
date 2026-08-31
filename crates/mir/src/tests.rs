use certo_parser::parse;
use certo_hir::{lower_module, HirItem};
use crate::{lower_fn, Terminator, Rvalue, Operand, MirConst, MirStmt};

fn mir_fn(src: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let HirItem::Fn(f) = &hir.items[0] else { panic!("expected fn"); };
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys, &hir.variant_to_type).0
}

fn mir_fn_named(src: &str, name: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let f = hir.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn named {name}"));
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys, &hir.variant_to_type).0
}

fn mir_fn_and_lifted_named(src: &str, name: &str) -> (crate::MirFn, Vec<crate::MirFn>) {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let f = hir.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn named {name}"));
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys, &hir.fn_ret_tys, &hir.variant_to_type)
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
fn row_bound_field_accessor_call_gets_unbox_some_not_unbox() {
    // BACKLOG item 200 — the accessor's own return is boxed via `Rvalue::
    // BoxSome` (item 76's HKT-closure convention, since its `ret_hint` is
    // `Ty::Var(0)`), so calling it needs the unconditional-dereference
    // `UnboxSome` pairing, not `Unbox`'s bit-pattern cast — using `Unbox`
    // here would read the malloc'd cell's own heap address as if it were
    // the field's raw bits (confirmed directly: this exact mismatch
    // segfaulted before `local_closure_raw_return` was added).
    let mf = mir_fn_named(
        "module A\nfn getName<R: { name: Text }>(record: R): Text = record.name",
        "getName");
    let has_unbox_some = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::UnboxSome { .. }, .. }
    )));
    assert!(has_unbox_some, "expected the accessor call's result to be unboxed via UnboxSome");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(!has_unbox, "expected no plain Unbox for the accessor call — that's the wrong pairing for BoxSome");
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

// ------------------------------------------------------------------ //
// BACKLOG item 262 — positional record construction call-site boxing.
// ------------------------------------------------------------------ //

#[test]
fn positional_record_constructor_boxes_concrete_argument_into_bare_field() {
    // `Box(42)` is a positional call to the record type's own new
    // constructor — its declared field (`value: T`) is a bare type param,
    // so the concrete `42` argument must be heap-boxed exactly like a
    // generic sum-variant constructor call already is (see
    // `generic_call_boxes_concrete_argument` above), via the new
    // `record_field_types` fallback in the same boxing check.
    let mf = mir_fn_named(
        "module A\ntype Box<T> = { value: T }\nfn f(): Box<Int> = Box(42)",
        "f");
    let has_boxsome = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::BoxSome { .. }, .. }
    )));
    assert!(has_boxsome, "expected a BoxSome for the concrete argument passed into Box's bare-T field");
}

// ------------------------------------------------------------------ //
// BACKLOG item 277 — recursive sum type construction/pattern-match boxing.
// ------------------------------------------------------------------ //

#[test]
fn self_referential_constructor_call_boxes_the_recursive_field_argument() {
    // `Node(1, Leaf, Leaf)` — `left`/`right`'s own declared type is `Tree`
    // itself (Node's own enclosing type), a *direct* self-reference. Their
    // C storage is a pointer (see the codegen test for the struct layout
    // itself), so the constructor call must heap-box each argument via the
    // plain `Rvalue::Box` (not the Option-specific `BoxSome`, which is only
    // for the bare-type-param case).
    let mf = mir_fn_named(
        "module A\ntype Tree = | Leaf | Node(value: Int, left: Tree, right: Tree)\n\
         fn f(): Tree = Node(1, Leaf, Leaf)",
        "f");
    let box_count = mf.blocks.iter().flat_map(|bb| bb.stmts.iter()).filter(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Box { .. }, .. }
    )).count();
    assert_eq!(box_count, 2, "expected exactly 2 Rvalue::Box (one for `left`, one for `right`), got {box_count}");
}

#[test]
fn self_referential_constructor_call_does_not_use_boxsome() {
    // Regression guard: a direct self-referential field must use the plain
    // `Rvalue::Box`, not `BoxSome` (that pairing is reserved for the bare
    // type-param/Option-erasure case and would mismatch what the pattern-
    // match side unboxes with — see the paired `Unbox` test below).
    let mf = mir_fn_named(
        "module A\ntype Tree = | Leaf | Node(value: Int, left: Tree, right: Tree)\n\
         fn f(): Tree = Node(1, Leaf, Leaf)",
        "f");
    let has_boxsome = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::BoxSome { .. }, .. }
    )));
    assert!(!has_boxsome, "a direct self-referential field must not use BoxSome");
}

#[test]
fn pattern_match_on_a_self_referential_field_unboxes_via_plain_unbox() {
    // `match t { Node(v, l, r) => ... }` — `l`/`r` must be read as plain
    // `Tree` values, dereferencing the pointer the constructor boxed,
    // mirroring the plain `Rvalue::Unbox` pairing (not `UnboxSome`, which
    // stays reserved for the bare-type-param case).
    let mf = mir_fn_named(
        "module A\ntype Tree = | Leaf | Node(value: Int, left: Tree, right: Tree)\n\
         fn depth(t: Tree): Int = match t {\n Leaf => 0\n Node(v, l, r) => 1\n}",
        "depth");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(has_unbox, "expected an Rvalue::Unbox reading the self-referential `left`/`right` fields");
    let has_unboxsome = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::UnboxSome { .. }, .. }
    )));
    assert!(!has_unboxsome, "a direct self-referential field must not use UnboxSome");
}

#[test]
fn non_recursive_field_construction_is_unaffected_by_the_self_ref_boxing_check() {
    // Regression guard: an ordinary sum-type constructor call with no
    // self-referential field must not spuriously box anything.
    let mf = mir_fn_named(
        "module A\ntype Shape = | Circle(radius: Float) | Square(side: Float)\n\
         fn f(): Shape = Circle(1.0)",
        "f");
    let has_box = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        crate::MirStmt::Assign { rvalue: Rvalue::Box { .. }, .. }
    )));
    assert!(!has_box, "an ordinary, non-recursive constructor call must not box anything");
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

// ------------------------------------------------------------------ //
// `for x in <list>` element unboxing — BACKLOG item 253
// ------------------------------------------------------------------ //

#[test]
fn for_loop_over_record_list_uses_unbox() {
    // Regression for the confirmed crash: a record-element list's own
    // `certo_list_get_or_panic` result (a raw `void*`) was assigned directly
    // to the struct-typed loop variable with no cast at all — a hard C
    // compile error (`assigning to 'Widget' from incompatible type 'void *'`),
    // unlike `Ty::Float`, whose bit-reinterpretation this arm already
    // special-cased separately.
    let mf = mir_fn("module A\ntype Widget = { name: Text }\nfn f(xs: List<Widget>): Unit = {\n  for w in xs { println(w.name) }\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { ty: certo_typeck::Ty::Named { name, .. }, .. }, .. } if name == "Widget"
    )));
    assert!(has_unbox, "expected an Unbox for the Widget record element");
}

#[test]
fn for_loop_over_int_list_does_not_use_unbox() {
    // Regression guard: the ordinary pointer/integer-sized element case
    // (already working before this fix) must stay a direct assignment, not
    // gain a needless Unbox.
    let mf = mir_fn("module A\nfn f(xs: List<Int>): Unit = {\n  for x in xs { println(intToText(x)) }\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { .. }, .. }
    )));
    assert!(!has_unbox, "an Int element needs no Unbox");
}

#[test]
fn for_loop_over_decimal_list_uses_unbox_not_bitcast() {
    // A second, latent bug this same fix closes: `Decimal`'s C
    // representation is a `{value: int64_t, scale: int8_t}` struct, not a
    // bit-castable scalar — the old code wrongly ran it through the same
    // `__certo_i2f` bit-reinterpretation path as `Float`, which only
    // happens to "work" for `Float` because a `double`'s bit pattern really
    // does fit in the pointer-sized slot untouched.
    let mf = mir_fn("module A\nfn f(xs: List<Decimal>): Unit = {\n  for d in xs { println(d.toText()) }\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { ty: certo_typeck::Ty::Decimal(_), .. }, .. }
    )));
    let has_i2f = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Call { func: Operand::Global(name), .. }, .. } if name == "__certo_i2f"
    )));
    assert!(has_unbox, "expected a real Unbox for the Decimal element");
    assert!(!has_i2f, "Decimal must not be bit-reinterpreted via __certo_i2f");
}

#[test]
fn for_loop_over_float_list_still_unboxes_correctly() {
    // Behavior-preservation guard: Float must still route through Unbox
    // (which itself special-cases Float via __certo_i2f at the codegen
    // layer) after unifying this arm's element-recovery logic.
    let mf = mir_fn("module A\nfn f(xs: List<Float>): Unit = {\n  for v in xs { println(intToText(0)) }\n}");
    let has_unbox = mf.blocks.iter().any(|bb| bb.stmts.iter().any(|s| matches!(s,
        MirStmt::Assign { rvalue: Rvalue::Unbox { ty: certo_typeck::Ty::Float, .. }, .. }
    )));
    assert!(has_unbox, "expected an Unbox with Ty::Float for the Float element");
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

// BACKLOG item 238 — an `impl` method or `computed` record property gets a
// dot-qualified HIR name (`"Cart.total"`, see hir/src/lower.rs's own
// `qname = format!("{}.{}", type_name, m.name.node)`). When such a method's
// body passes an inline lambda to a stdlib higher-order function (`sumBy`,
// `map`, etc.), MIR lifts that lambda to a synthesized top-level helper
// whose name is built from the enclosing function's own name (`b.fn_name`).
// Every one of those synthesized names is prefixed with `__`, which makes
// codegen's `c_fn_name` (emit_mir.rs) skip its own sanitization entirely
// (`if name.starts_with("__") { return name.to_string(); }`) — so a raw dot
// in `b.fn_name` would flow straight through into an illegal C identifier
// (`__lam_Cart.total_0`). The fix sanitizes once, at `Builder::new` itself.
#[test]
fn impl_method_qualified_name_dot_is_sanitized_in_lifted_lambda_helper_name() {
    let (_mf, lifted) = mir_fn_and_lifted_named(
        "module A\n\
         type Item = { price: Int, qty: Int }\n\
         type Cart = { items: List<Item> }\n\
         impl Cart {\n  fn total(self): Int = self.items.sumBy(fn(i: Item): Int = i.price * i.qty)\n}\n\
         fn f(c: Cart): Int = c.total()",
        "Cart.total");
    assert!(!lifted.is_empty(), "expected sumBy's lambda to be lifted to a top-level helper");
    for lf in &lifted {
        assert!(!lf.name.contains('.'),
            "lifted helper name must not contain a raw '.', it becomes an illegal C identifier: {:?}", lf.name);
    }
}

#[test]
fn computed_property_qualified_name_dot_is_sanitized_in_lifted_lambda_helper_name() {
    let (_mf, lifted) = mir_fn_and_lifted_named(
        "module A\n\
         type Item = { price: Int, qty: Int }\n\
         type Cart = { items: List<Item>, computed total: Int = items.sumBy(fn(i: Item): Int = i.price * i.qty) }\n\
         fn f(c: Cart): Int = c.total",
        "Cart.total");
    assert!(!lifted.is_empty(), "expected sumBy's lambda to be lifted to a top-level helper");
    for lf in &lifted {
        assert!(!lf.name.contains('.'),
            "lifted helper name must not contain a raw '.', it becomes an illegal C identifier: {:?}", lf.name);
    }
}
