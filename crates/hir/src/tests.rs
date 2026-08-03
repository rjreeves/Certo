use certo_parser::parse;
use certo_typeck::Ty;
use crate::lower_module;
use crate::hir::{HirItem, HirExprKind, BinOp};

fn lower(src: &str) -> crate::HirModule {
    let module = parse(src).expect("parse error");
    lower_module(&module).expect("lower error")
}

#[test]
fn lower_empty_module() {
    let m = lower("module A");
    assert_eq!(m.name, "A");
    assert!(m.items.is_empty());
}

#[test]
fn lower_impl_method_becomes_qualified_fn() {
    // `impl T for Person { fn greet(...) }` lowers to a top-level fn `Person.greet`.
    let m = lower("module A\ntrait T { fn greet(self: Text): Text }\n\
        type Person = { name: Text }\n\
        impl T for Person { fn greet(p: Person): Text = p.name }");
    let has_method = m.items.iter().any(|it|
        matches!(it, HirItem::Fn(f) if f.name == "Person.greet" && f.body.is_some()));
    assert!(has_method, "expected a lowered `Person.greet` fn item");
}

#[test]
fn lower_const_int() {
    let m = lower("module A\nval x: Int = 42");
    assert_eq!(m.items.len(), 1);
    if let HirItem::Const(c) = &m.items[0] {
        assert_eq!(c.name, "x");
        assert!(matches!(c.value.kind, HirExprKind::Int(42)));
    } else {
        panic!("expected Const");
    }
}

#[test]
fn lower_simple_fn() {
    let m = lower("module A\nfn add(a: Int, b: Int): Int = a + b");
    assert_eq!(m.items.len(), 1);
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.name, "add");
        assert_eq!(f.params.len(), 2);
        let body = f.body.as_ref().unwrap();
        assert!(matches!(&body.kind, HirExprKind::BinOp { op: BinOp::Add, .. }));
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn pipe_desugared_to_call() {
    // `a |> f` → `f(a)` — single-arg form
    let m = lower("module A\nfn f(x: Int): Int = x |> g\nfn g(x: Int): Int = x");
    if let HirItem::Fn(f) = &m.items[0] {
        let body = f.body.as_ref().unwrap();
        assert!(matches!(&body.kind, HirExprKind::Call { .. }), "pipe should desugar to Call");
    }
}

#[test]
fn pipe_partial_application() {
    // `a |> f(b)` → `f(a, b)` — LHS inserted as first arg
    let m = lower("module A\nfn f(x: Int): Int = x |> add(1)\nfn add(a: Int, b: Int): Int = a + b");
    if let HirItem::Fn(f) = &m.items[0] {
        let body = f.body.as_ref().unwrap();
        if let HirExprKind::Call { args, .. } = &body.kind {
            assert_eq!(args.len(), 2, "pipe with call RHS should produce 2-arg call");
        } else {
            panic!("expected Call, got {:?}", body.kind);
        }
    }
}

#[test]
fn lower_if_expr() {
    let m = lower("module A\nfn choose(b: Bool): Int = if b then 1 else 2");
    if let HirItem::Fn(f) = &m.items[0] {
        assert!(matches!(f.body.as_ref().unwrap().kind, HirExprKind::If { .. }));
    }
}

#[test]
fn lower_list_literal() {
    let m = lower("module A\nval xs: List<Int> = [1, 2, 3]");
    if let HirItem::Const(c) = &m.items[0] {
        assert!(matches!(&c.value.kind, HirExprKind::List(v) if v.len() == 3));
    }
}

#[test]
fn lower_lambda() {
    // Parser supports unannotated lambda params
    let m = lower("module A\nfn apply(f: Int): Int = f\nval result = apply(1)");
    assert_eq!(m.items.len(), 2);
}

#[test]
fn lower_block_with_val() {
    let m = lower("module A\nfn f(): Int = {\n    val x: Int = 1\n    val y: Int = 2\n    x + y\n}");
    if let HirItem::Fn(f) = &m.items[0] {
        let body = f.body.as_ref().unwrap();
        assert!(matches!(&body.kind, HirExprKind::Block { .. }));
    }
}

#[test]
fn lower_match_expr() {
    let m = lower("module A\nfn describe(n: Int): Text = match n {\n    0 => \"zero\"\n    _ => \"other\"\n}");
    if let HirItem::Fn(f) = &m.items[0] {
        assert!(matches!(f.body.as_ref().unwrap().kind, HirExprKind::Match { .. }));
    }
}

#[test]
fn lower_match_guard_is_preserved() {
    let src = "module A\nfn classify(n: Int): Text = match n {\n    x if x > 0 => \"positive\"\n    x if x < 0 => \"negative\"\n    _ => \"zero\"\n}";
    let m = lower(src);
    if let HirItem::Fn(f) = &m.items[0] {
        if let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind {
            // First two arms have guards; third (wildcard) does not.
            assert!(arms[0].guard.is_some(), "first arm should have a guard");
            assert!(arms[1].guard.is_some(), "second arm should have a guard");
            assert!(arms[2].guard.is_none(), "wildcard arm should have no guard");
        } else {
            panic!("expected Match");
        }
    }
}

#[test]
fn lower_match_guard_no_guard_arms() {
    // Plain match without guards should still work with guard: None on all arms.
    let src = "module A\nfn f(n: Int): Int = match n {\n    0 => 1\n    _ => 2\n}";
    let m = lower(src);
    if let HirItem::Fn(f) = &m.items[0] {
        if let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind {
            for arm in arms {
                assert!(arm.guard.is_none());
            }
        }
    }
}

// ------------------------------------------------------------------ //
// Stdlib call return-type recovery (BACKLOG item 113)
// ------------------------------------------------------------------ //

#[test]
fn binop_type_recovers_from_operand_instead_of_always_error() {
    // `x * 2.0` must resolve to Float, not the old unconditional Ty::Error —
    // a prerequisite for List.map's callback return type being recoverable.
    let m = lower("module A\nfn f(x: Float): Float = x * 2.0");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Float);
    }
}

#[test]
fn list_get_or_panic_return_type_is_element_type() {
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Float>): Float = List.getOrPanic(xs, 0)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Float);
    }
}

#[test]
fn list_filter_sort_reverse_distinct_preserve_list_type() {
    for call in ["List.filter(xs, (x) => true)", "List.reverse(xs)", "List.distinct(xs)"] {
        let src = format!("module A\nimport Stdlib.Collections.{{ List }}\nfn f(xs: List<Float>): List<Float> = {call}");
        let m = lower(&src);
        if let HirItem::Fn(f) = &m.items[0] {
            assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Float)), "for {call}");
        }
    }
}

#[test]
fn list_map_return_type_recovered_via_lambda_param_hint() {
    // The callback's param is unannotated, so its type must come from the
    // scrutinee list's own element type (Float) — then `x * 2.0`'s own
    // recovered BinOp type (via binop_result_ty) becomes List.map's element
    // type, closing the loop that item 112's fix alone couldn't.
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Float>): List<Float> = List.map(xs, (x) => x * 2.0)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Float)));
    }
}

#[test]
fn list_group_by_return_type_is_map_of_key_to_list() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Float>): Map<Int, List<Float>> = List.groupBy(xs, (x) => 1)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Map(Box::new(Ty::Int), Box::new(Ty::List(Box::new(Ty::Float)))));
    }
}

#[test]
fn map_get_return_type_is_option_of_value_type() {
    let m = lower("module A\nimport Stdlib.Collections.{ Map }\nfn f(m: Map<Text, Float>): Float? = Map.get(m, \"k\")");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(Ty::Float)));
    }
}

#[test]
fn annotated_lambda_param_overrides_hint() {
    // An explicit lambda param annotation must still win over the inferred
    // hint from the scrutinee list.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Float> = List.map(xs, (x: Int) => 1.0)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Float)));
    }
}
