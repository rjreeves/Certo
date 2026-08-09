use certo_parser::parse;
use certo_hir::{lower_module, HirItem};
use crate::{lower_fn, Terminator, Rvalue, Operand, MirConst};

fn mir_fn(src: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let HirItem::Fn(f) = &hir.items[0] else { panic!("expected fn"); };
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys).0
}

fn mir_fn_named(src: &str, name: &str) -> crate::MirFn {
    let module = parse(src).expect("parse error");
    let hir = lower_module(&module).expect("hir error");
    let f = hir.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn named {name}"));
    lower_fn(f, &hir.record_field_types, &hir.variant_field_types, &hir.fn_param_tys).0
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
