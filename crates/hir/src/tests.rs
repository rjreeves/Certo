use certo_parser::parse;
use certo_typeck::Ty;
use crate::lower_module;
use crate::hir::{HirItem, HirExprKind, BinOp};

fn lower(src: &str) -> crate::HirModule {
    let module = parse(src).expect("parse error");
    lower_module(&module).expect("lower error")
}

fn try_lower(src: &str) -> Result<crate::HirModule, Vec<crate::LowerError>> {
    let module = parse(src).expect("parse error");
    lower_module(&module)
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
fn int_to_float_call_type_is_float_not_error() {
    // BACKLOG item 129: intToFloat/floatToInt were never in the old hand-
    // maintained stdlib_ret_type table, so the call's HirExpr.ty defaulted
    // to Ty::Error, which codegen maps to int64_t — a `val` bound to the
    // call's result got wrongly declared int64_t, and any further float
    // arithmetic on it silently ran as truncating integer math instead of
    // real Float division. Now derived mechanically from `seed_stdlib`.
    let m = lower("module A\nfn f(n: Int): Float = intToFloat(n)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Float);
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn float_to_int_call_type_is_int_not_error() {
    let m = lower("module A\nfn f(x: Float): Int = floatToInt(x)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int);
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn stdlib_ret_types_covers_qualified_and_bare_names_mechanically() {
    // A spot-check that the mechanically-derived table (crate::lower::stdlib_ret_types)
    // picks up both bare top-level names and qualified `Type.method` names
    // straight from seed_stdlib, without any name needing a hand-added entry.
    let m = lower("module A\nimport Stdlib.Text.{ Char }\nfn f(c: Char): Int = Char.toInt(c)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int);
    } else {
        panic!("expected Fn");
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

// ------------------------------------------------------------------ //
// Named function reference typing + DB-query mapper return type
// recovery (BACKLOG item 134)
// ------------------------------------------------------------------ //

#[test]
fn bare_named_fn_reference_gets_real_fn_type() {
    // A bare reference to a top-level `fn` (as opposed to a call) — e.g.
    // passed as a callback argument — must resolve to the function's real
    // `Ty::Fn{params, ret}`, not the old unconditional `Ty::Error` (which
    // `global_types` alone left it as, since that table is never populated
    // for ordinary `fn` declarations).
    let m = lower("module A\nfn double(x: Int): Int = x * 2\nfn useIt(): Int = 1\nval ref = double");
    let const_item = m.items.iter().find_map(|it| match it {
        HirItem::Const(c) if c.name == "ref" => Some(c),
        _ => None,
    }).expect("expected a Const named `ref`");
    assert_eq!(const_item.value.ty, Ty::Fn { params: vec![Ty::Int], ret: Box::new(Ty::Int) });
}

#[test]
fn db_query_typed_return_type_is_list_of_mapper_return_type() {
    // `dbQueryTyped` is registered as a `Forall` generic
    // (`(Int, Text, List<Text>, List<Text?> -> T) -> List<T>`), so it's
    // filtered out of the mechanical `stdlib_ret_types()` table (its return
    // type has free vars) and must instead be recovered structurally via
    // `generic_container_ret`, using the mapper argument's own `Ty::Fn`
    // return type as `T`.
    let m = lower(
        "module A\nimport Stdlib.Db\ntype Widget = { id: Int }\n\
         fn fromRow(row: List<Text?>): Widget = Widget { id: 0 }\n\
         fn f(conn: Int): List<Widget> = dbQueryTyped(conn, \"SELECT 1\", [], fromRow)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let widget = Ty::Named { name: "Widget".to_string(), args: vec![] };
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(widget)));
}

// ------------------------------------------------------------------ //
// Generic function boxing prerequisites (BACKLOG item 120)
// ------------------------------------------------------------------ //

#[test]
fn impl_method_own_param_is_locally_typed_not_error() {
    // Unlike a top-level `fn`, an impl method's own params were never
    // inserted into `cx.local_types` — so a *reference* to the method's own
    // param inside its own body (`v` in `Wrap(v)`) fell back to `Ty::Error`,
    // even though the param's own declared type (`Ty::Var(0)` for a bare
    // type-param) was already computed correctly. This broke the "is this
    // argument already opaque" check MIR needs to avoid double-boxing.
    let m = lower(
        "module A\ntype Box<T> = | Wrap(T)\nimpl<T> Box { fn wrap(v: T): Box<T> = Wrap(v) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "Box.wrap" => Some(f),
        _ => None,
    }).expect("expected Box.wrap");
    if let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind {
        assert_eq!(args[0].ty, Ty::Var(0), "argument `v` should be Ty::Var(0), not Ty::Error");
    } else {
        panic!("expected a Call (Wrap(v))");
    }
}

#[test]
fn match_arm_binding_substitutes_concrete_scrutinee_instantiation() {
    // A sum-type variant pattern's field binding must substitute the
    // scrutinee's own concrete instantiation argument for a bare-type-param
    // declared field, not leave the local (and the whole match expression's
    // own inferred type) as `Ty::Var(0)` — otherwise a perfectly ordinary,
    // non-generic function's own return type is silently miscomputed as
    // void* instead of the real concrete type.
    let m = lower(
        "module A\ntype Box<T> = | Wrap(T)\nfn unwrapAsInt(b: Box<Int>): Int = match b { Wrap(v) => v }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "unwrapAsInt" => Some(f),
        _ => None,
    }).expect("expected unwrapAsInt");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int);
}

#[test]
fn query_first_return_type_is_option_of_mapper_return_type() {
    // Same mechanism as `dbQueryTyped`, but `Query.first`'s stdlib shape is
    // `(Query, Int, List<Text?> -> T) -> T?` — the recovered type must be
    // `Option<T>`, not `List<T>`.
    let m = lower(
        "module A\nimport Stdlib.Db\ntype Widget = { id: Int }\n\
         fn fromRow(row: List<Text?>): Widget = Widget { id: 0 }\n\
         fn f(conn: Int): Widget? = Query.first(Query.from(\"SELECT 1\"), conn, fromRow)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let widget = Ty::Named { name: "Widget".to_string(), args: vec![] };
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(widget)));
}

// ------------------------------------------------------------------ //
// Lambda closure-capture rejection (BACKLOG item 136)
// ------------------------------------------------------------------ //

#[test]
fn lambda_capturing_outer_local_is_rejected() {
    // Closures were never actually implemented — a lambda body referencing an
    // outer local previously compiled silently and read garbage memory at
    // runtime (confirmed via direct testing: a different wrong value on every
    // run). Must now be a real, reported HIR lowering error, not a panic or
    // a silent Ok.
    let result = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): List<Int> = { val outer = 1\n List.map(xs, (x) => x + outer) }");
    let errs = result.expect_err("expected a lowering error for a capturing lambda");
    assert!(errs.iter().any(|e| matches!(&e.kind, crate::LowerErrorKind::Unsupported(msg) if msg.contains("closures"))));
}

#[test]
fn lambda_without_capture_still_compiles() {
    // Sanity check: a lambda referencing only its own parameter (no outer
    // scope) must still compile cleanly — this check must not be over-broad.
    let result = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = List.map(xs, (x) => x + 1)");
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

#[test]
fn lambda_referencing_own_let_binding_is_fine() {
    // A `let` bound *inside* the lambda body is not a capture — must not be
    // flagged (the LocalId threshold check must distinguish "defined inside
    // the lambda" from "defined in an enclosing scope").
    let result = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): List<Int> = List.map(xs, (x) => { val y = x + 1\n y })");
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}
