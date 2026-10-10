use certo_parser::parse;
use certo_typeck::Ty;
use crate::lower_module;
use crate::hir::{HirItem, HirExpr, HirExprKind, HirFn, HirStmt, BinOp, HirPat};

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

// ------------------------------------------------------------------ //
// `e?` on Result/Option — BACKLOG item 321 (Option-`?`)
// ------------------------------------------------------------------ //

fn first_let_init(f: &HirFn) -> &HirExpr {
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a Block body") };
    let HirStmt::Let { init, .. } = &stmts[0] else { panic!("expected first stmt to be a Let") };
    init
}

#[test]
fn try_on_result_still_resolves_to_the_ok_payload_type() {
    // Regression guard: the pre-existing Result-`?` path is unaffected by
    // the new Option-`?` arm.
    let m = lower("module A\nfn g(): Result<Int, Text> = Ok(1)\n\
        fn f(): Result<Int, Text> = { val x = g()?\n Ok(x) }");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "f" { Some(f) } else { None } } else { None }).unwrap();
    let init = first_let_init(f);
    assert!(matches!(&init.kind, HirExprKind::Try(_)), "expected HirExprKind::Try, got {:?}", init.kind);
    assert_eq!(init.ty, Ty::Int, "e?'s type should be Result's Ok payload (Int), got {:?}", init.ty);
}

#[test]
fn try_on_option_resolves_to_the_some_payload_type() {
    let m = lower("module A\nfn g(): Int? = Some(1)\n\
        fn f(): Int? = { val x = g()?\n Some(x) }");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "f" { Some(f) } else { None } } else { None }).unwrap();
    let init = first_let_init(f);
    assert!(matches!(&init.kind, HirExprKind::Try(_)), "expected HirExprKind::Try, got {:?}", init.kind);
    assert_eq!(init.ty, Ty::Int, "e?'s type should be Option's Some payload (Int), got {:?}", init.ty);
}

// ------------------------------------------------------------------ //
// `val`/`var` explicit annotation resolving a bare type-param name inside
// a generic function's own body — BACKLOG item 335 (investigation)
// ------------------------------------------------------------------ //

#[test]
fn var_annotation_with_bare_type_param_resolves_to_erased_var_not_a_literal_name() {
    // Before this fix, `Stmt::Var`'s own explicit-annotation conversion
    // (`ast_ty_to_ty_with_params(&t.node, &[], ...)`) had no knowledge of
    // the enclosing generic fn's own type params, so a bare `T` fell
    // through to an opaque `Ty::Named { name: "T", .. }` — codegen then
    // emitted the literal, undeclared C type name `T` (confirmed via a
    // live repro: `T* _ob = (T*)malloc(sizeof(T));`). Now it must resolve
    // to the same `Ty::Var(0)` erasure sentinel a function's own param/
    // return-type annotations already get.
    let m = lower("module A\nfn f<T>(): Unit = { var xs: List<T> = [] }");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { Some(f) } else { None }).unwrap();
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a Block body") };
    let HirStmt::Let { ty, .. } = &stmts[0] else { panic!("expected first stmt to be a Let") };
    assert_eq!(*ty, Ty::List(Box::new(Ty::Var(0))),
        "expected the empty list's element type to resolve to the Var(0) erasure sentinel, got {:?}", ty);
}

#[test]
fn val_annotation_with_bare_type_param_is_rejected_not_silently_miscompiled() {
    // The `Stmt::Val` sibling of the test above: annotating a bare-generic
    // stdlib return with the *same* still-abstract type parameter can't
    // actually resolve anything concrete (this compiler never
    // monomorphizes — `T` stays erased for the whole function). Before
    // this fix, the identical `&[]`-blind conversion wrongly produced a
    // *different-looking* type (`Ty::Named("T")` vs the return's own
    // `Ty::Var(0)` sentinel), which accidentally satisfied
    // `resolve_bare_generic_return`'s "was this actually resolved?" check
    // and let broken code (the same literal-`T`-as-a-C-type bug) through
    // to codegen. Now that both sides correctly agree it's still `Var(0)`,
    // this is correctly recognized as unresolvable and rejected here
    // instead — matching this compiler's own established "hard error
    // instead of silent miscompile" precedent (BACKLOG item 135) for
    // exactly this situation.
    let err = try_lower("module A\nfn f<T>(items: List<T>): T? = { val item: T = List.getOrPanic(items, 0)\n Some(item) }")
        .expect_err("expected this to be rejected, not silently lowered");
    assert!(err.iter().any(|e| e.message().contains("cannot determine the concrete type")),
        "expected a 'cannot determine the concrete type' error, got {:?}", err);
}

// ------------------------------------------------------------------ //
// `db.transaction {}` / `db.<table>.<method>(...)` — BACKLOG item 226
// ------------------------------------------------------------------ //

#[test]
fn db_transaction_lowers_to_a_call_to_db_transaction() {
    let m = lower("module A\nfn f(): Unit = db.transaction {\n println(\"in txn\")\n}");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { Some(f) } else { None }).unwrap();
    let body = f.body.as_ref().unwrap();
    let HirExprKind::Call { func, args } = &body.kind else { panic!("expected Call, got {:?}", body.kind) };
    assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "__db_transaction"));
    assert_eq!(args.len(), 1, "expected the zero-arg thunk as the sole argument");
    assert!(matches!(&args[0].kind, HirExprKind::Lambda { .. }), "expected a lambda thunk, got {:?}", args[0].kind);
}

#[test]
fn db_accessor_find_lowers_to_generated_fn_with_ambient_conn_prepended() {
    let m = lower("module A
type Customer = { id: Int, name: Text }
fn customersFindById(conn: Int, id: Int): Customer? = None
fn f(id: Int): Customer? = db.customers.find(id)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f), _ => None,
    }).unwrap();
    let body = f.body.as_ref().unwrap();
    let HirExprKind::Call { func, args } = &body.kind else { panic!("expected Call, got {:?}", body.kind) };
    assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "customersFindById"),
        "expected a call to the real generated function, got {:?}", func.kind);
    assert_eq!(args.len(), 2, "expected [__certo_db_conn(), id]");
    let HirExprKind::Call { func: conn_func, args: conn_args } = &args[0].kind else {
        panic!("expected the leading arg to itself be a Call, got {:?}", args[0].kind)
    };
    assert!(matches!(&conn_func.kind, HirExprKind::Global(name) if name == "__certo_db_conn"));
    assert!(conn_args.is_empty());
}

#[test]
fn db_accessor_local_param_named_db_does_not_get_rewritten() {
    // A real local `db` must shadow the sugar at the HIR layer too, matching
    // typeck's own shadowing check — confirmed here by asserting the call
    // does NOT lower to a synthesized call to some generated function.
    let m = lower("module A
type Customer = { id: Int, name: Text }
fn f(db: Customer): Int = db.id");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f), _ => None,
    }).unwrap();
    let body = f.body.as_ref().unwrap();
    assert!(!matches!(&body.kind, HirExprKind::Call { func, .. }
        if matches!(&func.kind, HirExprKind::Global(name) if name.starts_with("__certo_db_conn"))),
        "a shadowed `db` local must not trigger the db-accessor rewrite, got {:?}", body.kind);
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
// Safe field access `?.` desugaring (BACKLOG item 146)
// ------------------------------------------------------------------ //

const SAFE_FIELD_SRC: &str = "\
type Address = { city: Text, zip: Text }
type User = { name: Text, address: Address? }
";

#[test]
fn safe_field_desugars_to_a_real_some_constructor_pattern_not_a_bare_bind() {
    // `e?.f` lowers to `match e { Some(v) => Some(v.f), None => None }`.
    // The `Some` arm's pattern used to be a bare `HirPat::Bind`, which is
    // irrefutable — it matched unconditionally regardless of whether the
    // base was actually `Some` or `None`, making the `None` arm dead code
    // and binding `v` to the whole still-wrapped Option instead of its
    // payload. Must be a real `Constructor("Some", [Bind(..)])` pattern.
    let src = format!(
        "module A\n{}fn f(u: User): Text? = u.address?.city",
        SAFE_FIELD_SRC
    );
    let m = lower(&src);
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Match expression");
    };
    assert_eq!(arms.len(), 2);
    match &arms[0].pat {
        crate::hir::HirPat::Constructor { name, fields, .. } => {
            assert_eq!(name, "Some");
            assert_eq!(fields.len(), 1, "Some arm should bind exactly one payload local");
        }
        other => panic!("expected Some arm to be a real Constructor pattern, got {other:?}"),
    }
    match &arms[1].pat {
        crate::hir::HirPat::Constructor { name, .. } => assert_eq!(name, "None"),
        other => panic!("expected None arm to be a Constructor pattern, got {other:?}"),
    }
}

#[test]
fn safe_field_result_type_is_option_of_the_field_type() {
    // `u.address?.city` — address: Address?, city: Text — must lower with
    // an overall HIR type of Option<Text>, not the previous unconditional
    // Ty::Error.
    let src = format!(
        "module A\n{}fn f(u: User): Text? = u.address?.city",
        SAFE_FIELD_SRC
    );
    let m = lower(&src);
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(Ty::Text)));
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
fn list_len_return_type_is_int_not_error() {
    // BACKLOG item 179 — `List.len<T>(list: List<T>): Int` is registered as
    // a `Ty::Forall` (generic in its *parameter*), which `stdlib_ret_types()`
    // used to skip entirely (it only ever matched a direct, non-generic
    // `Ty::Fn`) even though the *return* type (`Int`) is fully concrete
    // regardless of `T`. Left unresolved, this silently fell back to
    // `Ty::Error`'s `int64_t` layout-compatible default — indistinguishable
    // in the emitted C from a real `Int` until something (an f-string
    // interpolation choosing its `*_to_text` conversion) actually needed to
    // know the difference, at which point it read a `Text` pointer's bits
    // out of a raw `int64_t` and segfaulted.
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): Int = List.len(xs)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int);
    }
}

#[test]
fn option_is_some_return_type_is_bool_not_error() {
    // BACKLOG item 182 — `List.first(xs)` chained with `Option.isSome(...)`
    // segfaulted at runtime, but investigation found `certo_list_first`,
    // `certo_option_is_some`, and their call-site codegen were all already
    // correct. The real cause was the *same* bug item 179 fixed for
    // `List.len`: `Option.isSome<T>(opt: Option<T>): Bool` is also a
    // `Ty::Forall`-wrapped stdlib signature whose *return* type (`Bool`) is
    // fully concrete regardless of `T` — `stdlib_ret_types()`'s old
    // direct-`Ty::Fn`-only match excluded it too, leaving its call's HIR
    // type `Ty::Error` and corrupting the downstream `certo_bool_to_text`
    // call. No separate code change was needed here — confirmed fixed by
    // item 179's `Ty::Forall`-unwrap alone; this test exists so the fix
    // stays pinned for this call site specifically, not just `List.len`.
    let m = lower("module A\nfn f(o: Option<Int>): Bool = Option.isSome(o)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Bool);
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
fn list_for_each_callback_param_hinted_from_element_type() {
    // BACKLOG item 266 — `List.forEach`'s own callback param must be
    // hinted from the receiver list's element type the same way
    // `List.map`'s is, so a method call or field access inside the
    // callback body resolves correctly rather than staying `Ty::Error`.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         type Coupon = { name: Text }\n\
         fn f(xs: List<Coupon>): Unit = List.forEach(xs) { x => println(x.name) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected f's body to be a Call");
    };
    let HirExprKind::Lambda { body, .. } = &args[1].kind else {
        panic!("expected the 2nd arg to be the callback lambda, got {:?}", args[1].kind);
    };
    // `println(x.name)`'s own argument type resolving to Text (not
    // Ty::Error) proves `x` was correctly hinted to `Coupon`.
    let HirExprKind::Call { args: println_args, .. } = &body.kind else {
        panic!("expected the callback body to be a Call to println, got {:?}", body.kind);
    };
    assert_eq!(println_args[0].ty, Ty::Text, "x.name must resolve to Text, not Ty::Error");
}

#[test]
fn list_flat_map_return_type_recovered_via_lambda_param_hint() {
    // BACKLOG item 267 — `List.flatMap` had no `generic_container_ret` arm
    // and no `needs_lambda_hint` entry at all: the callback's own param
    // (`x`) stayed unhinted (`Ty::Error`), so `[x, x]`'s own body type
    // resolved to `List<Error>`, not `List<Int>` — a real segfault once the
    // result was consumed downstream (e.g. a `for` loop + f-string
    // interpolation misreading a raw Int as a Text pointer), confirmed by a
    // direct repro before this fix.
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = List.flatMap(xs, (x) => [x, x])");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Int)),
            "List.flatMap's own call type must fully resolve to List<Int>, not collapse to Ty::Error/List<Error>");
    }
}

#[test]
fn list_map_return_type_recovered_via_named_function_callback() {
    // BACKLOG item 180 — the lambda-only recovery above left a *named*
    // function callback (`xs.map(double)`) silently `Ty::Error`-typed
    // instead of `List<Int>`, since `generic_container_ret`'s `List.map`
    // arm only ever matched a `HirExprKind::Lambda`. A bare reference to a
    // top-level `fn` already carries its real `Ty::Fn{params, ret}` (see
    // `Expr::Path`'s own lowering), so the fix reads the return type from
    // there when the callback isn't an inline lambda at all.
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn double(x: Int): Int = x * 2\nfn f(xs: List<Int>): List<Int> = List.map(xs, double)");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "f" { Some(f) } else { None } } else { None }).unwrap();
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Int)));
}

#[test]
fn list_group_by_and_sum_by_recover_return_type_from_named_function_callback() {
    // Same fix, same class of bug, for List.groupBy's key type and
    // List.sumBy's numeric type.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn keyOf(x: Int): Int = x % 2\n\
         fn f(xs: List<Int>): Map<Int, List<Int>> = List.groupBy(xs, keyOf)");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "f" { Some(f) } else { None } } else { None }).unwrap();
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Map(Box::new(Ty::Int), Box::new(Ty::List(Box::new(Ty::Int)))));

    let m2 = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn identity(x: Int): Int = x\n\
         fn g(xs: List<Int>): Int = List.sumBy(xs, identity)");
    let g = m2.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "g" { Some(f) } else { None } } else { None }).unwrap();
    assert_eq!(g.body.as_ref().unwrap().ty, Ty::Int);
}

// ------------------------------------------------------------------ //
// Option.map (BACKLOG item 256) — same hint/return-type-recovery
// machinery as List.map just above, sourced from the receiver's
// `Option<A>` instead of a list's element type.
// ------------------------------------------------------------------ //

#[test]
fn option_map_return_type_recovered_via_lambda_param_hint() {
    let m = lower("module A\nfn f(o: Option<Float>): Option<Float> = Option.map(o, (x) => x * 2.0)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(Ty::Float)));
    }
}

#[test]
fn option_map_dot_call_lowers_to_same_shape_as_qualified_call() {
    let qualified = lower(
        "module A\nfn f(o: Option<Int>): Option<Int> = Option.map(o, (x) => x)");
    let dotted = lower(
        "module A\nfn f(o: Option<Int>): Option<Int> = o.map((x) => x)");
    let get_call = |m: &crate::HirModule| -> (String, usize) {
        if let HirItem::Fn(f) = &m.items[0] {
            if let HirExprKind::Call { func, args } = &f.body.as_ref().unwrap().kind {
                if let HirExprKind::Global(name) = &func.kind {
                    return (name.clone(), args.len());
                }
            }
        }
        panic!("expected a Call to a Global callee");
    };
    assert_eq!(get_call(&qualified), ("Option.map".to_string(), 2));
    assert_eq!(get_call(&dotted), ("Option.map".to_string(), 2));
}

#[test]
fn option_map_callback_can_resolve_a_method_call_on_its_element() {
    // Regression test for a real bug found verifying item 256 end-to-end:
    // `Option.map` was listed in `needs_lambda_hint`'s match arms but never
    // registered in `stdlib_param_names()`, so `cx.stdlib_params.get(
    // "Option.map")` was always `None` and the entire hint-seeding branch
    // (gated on `stdlib_names` being `Some`) never ran — the callback
    // param's type never got seeded from the receiver's `Option<A>`. Bare
    // field access on the param happened to keep working regardless (MIR's
    // own separate, independent `elem_ty_hint` recovers it for unboxing
    // purposes), which masked the bug until a *method call* inside the
    // callback (needing the dot-call UFCS rewrite's own type lookup) was
    // tried — that produced field-access-then-call-as-function-pointer
    // codegen instead of a real call, segfaulting/failing to compile.
    let m = lower(
        "module A\n\
         type Coupon = { discount: Int }\n\
         impl Coupon {\n  fn applyTo(self, subtotal: Int): Int = subtotal - self.discount\n}\n\
         fn f(o: Option<Coupon>, subtotal: Int): Option<Int> = Option.map(o, (c) => c.applyTo(subtotal))");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { if f.name == "f" { Some(f) } else { None } } else { None }).unwrap();
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Option(Box::new(Ty::Int)));
    if let HirExprKind::Call { args, .. } = &body.kind {
        let HirExprKind::Lambda { body: lam_body, .. } = &args[1].kind else {
            panic!("expected the 2nd Option.map argument to be a Lambda");
        };
        // The regression: without the fix, this call never resolves to
        // a real callee — it stays a plain Field-access expression.
        assert!(matches!(&lam_body.kind, HirExprKind::Call { .. }),
            "expected a resolved Call to Coupon.applyTo, got {:?}", lam_body.kind);
    } else {
        panic!("expected Option.map's own Call node");
    }
}

// ------------------------------------------------------------------ //
// Dot-call UFCS (BACKLOG item 162) — `xs.map(f)` must lower to the exact
// same `HirExprKind::Call{ func: Global("List.map"), args: [xs, f] }`
// shape the already-working qualified form `List.map(xs, f)` produces,
// with identical return-type recovery and lambda-param hinting (both of
// which run entirely downstream of this rewrite, unmodified).
// ------------------------------------------------------------------ //

#[test]
fn dot_call_lowers_to_same_shape_as_qualified_call() {
    let qualified = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = List.map(xs, (x) => x)");
    let dotted = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = xs.map((x) => x)");
    let get_call = |m: &crate::HirModule| -> (String, usize) {
        if let HirItem::Fn(f) = &m.items[0] {
            if let HirExprKind::Call { func, args } = &f.body.as_ref().unwrap().kind {
                if let HirExprKind::Global(name) = &func.kind {
                    return (name.clone(), args.len());
                }
            }
        }
        panic!("expected a Call to a Global callee");
    };
    assert_eq!(get_call(&qualified), ("List.map".to_string(), 2));
    assert_eq!(get_call(&dotted), ("List.map".to_string(), 2));
}

#[test]
fn dot_call_return_type_recovered_via_lambda_param_hint() {
    // Same as `list_map_return_type_recovered_via_lambda_param_hint` above,
    // written as a dot-call — the lambda-hint machinery only ever looks at
    // the (post-rewrite) `args`, so it must work identically either way.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Float>): List<Float> = xs.map((x) => x * 2.0)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Float)));
    }
}

#[test]
fn dot_call_on_option_lowers_to_qualified_global() {
    let m = lower(
        "module A\nfn f(o: Option<Int>): Bool = o.isSome()");
    if let HirItem::Fn(f) = &m.items[0] {
        if let HirExprKind::Call { func, args } = &f.body.as_ref().unwrap().kind {
            assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Option.isSome"));
            assert_eq!(args.len(), 1);
        } else {
            panic!("expected Call, got {:?}", f.body.as_ref().unwrap().kind);
        }
    }
}

// BACKLOG item 307 — `Ty::qualifying_name` (shared by this file's own UFCS
// rewrite and typeck's identical one) never handled any primitive scalar
// type, so a dot-call on an `Int`/`Char`/etc. receiver could never even
// reach this rewrite — confirmed live before this fix (`n.toFloat()` and
// `c.toText()` both failed `E0205`, the latter despite 10 already-
// registered `Char.*` stdlib functions).
#[test]
fn dot_call_on_int_lowers_to_qualified_global() {
    let m = lower("module A\nfn f(n: Int): Float = n.toFloat()");
    if let HirItem::Fn(f) = &m.items[0] {
        if let HirExprKind::Call { func, args } = &f.body.as_ref().unwrap().kind {
            assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Int.toFloat"));
            assert_eq!(args.len(), 1);
        } else {
            panic!("expected Call, got {:?}", f.body.as_ref().unwrap().kind);
        }
    }
}

#[test]
fn dot_call_on_char_lowers_to_qualified_global() {
    let m = lower("module A\nfn f(c: Char): Text = c.toText()");
    if let HirItem::Fn(f) = &m.items[0] {
        if let HirExprKind::Call { func, args } = &f.body.as_ref().unwrap().kind {
            assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Char.toText"));
            assert_eq!(args.len(), 1);
        } else {
            panic!("expected Call, got {:?}", f.body.as_ref().unwrap().kind);
        }
    }
}

#[test]
fn dot_call_field_access_typo_is_unaffected() {
    // A genuine field-access typo (no matching `"Order.frobnicate"` global
    // anywhere) must still lower as an ordinary (erroring) field access,
    // not be swallowed by the UFCS rewrite attempt.
    let m = lower(
        "module A\ntype Order = { total: Int }\nfn f(o: Order): Int = o.nonexistent");
    if let HirItem::Fn(f) = &m.items[0] {
        assert!(matches!(&f.body.as_ref().unwrap().kind, HirExprKind::Field { .. }));
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

// `List.sortBy`/`minBy`/`maxBy`/`sumBy` (BACKLOG item 162b) — same
// `generic_container_ret` return-type recovery mechanism as `List.map`/
// `groupBy` above, plus the E0601 key/numeric projection restriction for
// the cases typeck's own `Expr::Field` inference can't see through
// (struct-element field-access keys — see `crates/typeck/src/tests.rs`'s
// `sortby_struct_float_field_key_does_not_false_positive` for the typeck
// half of this same restriction).

#[test]
fn sortby_return_type_is_list_of_element_type() {
    let m = lower("module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = List.sortBy(xs, (x) => x)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Int)));
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn minby_maxby_return_type_is_option_of_element_type() {
    for name in ["minBy", "maxBy"] {
        let m = lower(&format!(
            "module A\nimport Stdlib.Collections.{{ List }}\nfn f(xs: List<Int>): Int? = List.{name}(xs, (x) => x)"));
        if let HirItem::Fn(f) = &m.items[0] {
            assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(Ty::Int)), "for List.{name}");
        } else {
            panic!("expected Fn");
        }
    }
}

#[test]
fn sumby_return_type_is_projected_key_type() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): Float = List.sumBy(ps, (p) => p.price)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Float);
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn sortby_struct_text_field_key_is_e0601() {
    // The genuinely-unsupported case: typeck can't see through the field
    // access to know it's Text (see the typeck-side regression test this
    // mirrors), so this is the *only* place it's actually caught.
    let err = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): List<Product> = List.sortBy(ps, (p) => p.name)"
    ).unwrap_err();
    let msg = match &err[0].kind {
        crate::error::LowerErrorKind::Unsupported(m) => m.clone(),
        other => panic!("expected LowerErrorKind::Unsupported (E0601), got {other:?}"),
    };
    assert!(msg.contains("List.sortBy") && msg.contains("Text"), "unexpected message: {msg}");
}

#[test]
fn sortby_struct_float_field_key_lowers_ok() {
    // Regression: a struct-element key that *does* project to a supported
    // type (Float) must not be rejected just because typeck itself
    // couldn't resolve it — this is exactly the false-positive the typeck-
    // side fix (`crate::infer_expr`'s `!matches!(resolved_key, Ty::Var(_))`
    // guard) exists to avoid; this HIR-level check is what actually
    // validates it correctly, using the post-hint type.
    lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): List<Product> = List.sortBy(ps, (p) => p.price)"
    );
}

// BACKLOG item 172 — the per-call-site lambda hint above only ever applied
// on the *positional* argument-lowering branch; a labeled call's inline
// lambda stayed `Ty::Error` regardless of how useful its body's own
// inferred type would otherwise have been. These mirror the tests above,
// just with the arguments labeled and written out of declared order.

#[test]
fn labeled_groupby_call_with_key_before_list_gets_lambda_hint() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Float>): Map<Int, List<Float>> = List.groupBy(key: (x) => 1, list: xs)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Map(Box::new(Ty::Int), Box::new(Ty::List(Box::new(Ty::Float)))));
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn labeled_sortby_call_with_struct_float_key_before_list_lowers_ok() {
    // The E0601 struct-field-key check only ever fires against the
    // correctly-hinted lambda body — before the fix, a labeled call's
    // lambda body stayed `Ty::Error`, so this legitimate case and the
    // genuinely-unsupported case below were indistinguishable.
    lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): List<Product> = List.sortBy(key: (p) => p.price, list: ps)"
    );
}

#[test]
fn labeled_sortby_call_with_struct_text_key_before_list_is_e0601() {
    let err = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): List<Product> = List.sortBy(key: (p) => p.name, list: ps)"
    ).unwrap_err();
    let msg = match &err[0].kind {
        crate::error::LowerErrorKind::Unsupported(m) => m.clone(),
        other => panic!("expected LowerErrorKind::Unsupported (E0601), got {other:?}"),
    };
    assert!(msg.contains("List.sortBy") && msg.contains("Text"), "unexpected message: {msg}");
}

#[test]
fn labeled_map_call_with_f_before_list_gets_lambda_hint() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Float>): List<Float> = List.map(f: (x) => x * 2.0, list: xs)");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Float)));
    } else {
        panic!("expected Fn");
    }
}

#[test]
fn map_get_return_type_is_option_of_value_type() {
    let m = lower("module A\nimport Stdlib.Collections.{ Map }\nfn f(m: Map<Text, Float>): Float? = Map.get(m, \"k\")");
    if let HirItem::Fn(f) = &m.items[0] {
        assert_eq!(f.body.as_ref().unwrap().ty, Ty::Option(Box::new(Ty::Float)));
    }
}

// BACKLOG item 338 — `Map.insert`'s own return type used to just blindly
// copy its first (input-map) argument's type unchanged. For
// `Map.insert(Map.empty(), key, value)` that input map is itself
// unresolvable (`Map.empty()` has no arguments to recover K/V from), so the
// *whole chain* — including a later `Map.get` on the result — could never
// recover a real K/V either. Confirmed as a real runtime crash, not just a
// missing type: `Map.get`'s own C implementation correctly extracted the
// right bits, but downstream code (an f-string interpolation) trusted the
// wrong (`Ty::Error`) type for the extracted value and skipped a needed
// Int-to-Text conversion, segfaulting in certo_text_concat.
#[test]
fn map_insert_on_a_fresh_empty_map_recovers_key_value_types_from_the_insert_itself() {
    let m = lower("module A\nimport Stdlib.Collections.{ Map }\n\
        fn f(): Int? = { val m = Map.insert(Map.empty(), \"k\", 42)\n Map.get(m, \"k\") }");
    let f = m.items.iter().find_map(|it| if let HirItem::Fn(f) = it { Some(f) } else { None }).unwrap();
    let init = first_let_init(f);
    assert_eq!(init.ty, Ty::Map(Box::new(Ty::Text), Box::new(Ty::Int)),
        "expected Map.insert(Map.empty(), ...) to resolve to Map<Text, Int>, got {:?}", init.ty);
    // The whole point: the *tail* Map.get call built on top of that `val`
    // must now also correctly resolve, not just the intermediate `val` itself.
    let HirExprKind::Block { tail, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a Block body") };
    assert_eq!(tail.ty, Ty::Option(Box::new(Ty::Int)),
        "expected the tail Map.get(...) to resolve to Option<Int>, got {:?}", tail.ty);
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

#[test]
fn query_first_consumed_inline_via_match_resolves_field_access() {
    // BACKLOG item 207 — filed as "Query.first fails to compile when consumed
    // inline in the same function, not through an intermediate named wrapper
    // function". Re-investigated: doesn't reproduce against a live Postgres
    // connection (4 separate real end-to-end round trips all worked), and
    // this HIR-level check confirms why — `Query.first`'s recovered
    // `Option<Widget>` return type (via `generic_container_ret`) already
    // flows correctly into a `match` consuming it in the very same function,
    // with no intermediate `val` boundary at all: the `Some(x) => x.name`
    // arm's field access resolves to `Text`, not `Ty::Error`. Likely already
    // fixed as a side effect of the many other "known type thrown away"
    // fixes landed this session (items 179/180/182/189/191/199/211) — none
    // of which existed when item 207 was originally filed.
    let m = lower(
        "module A\nimport Stdlib.Db\ntype Widget = { id: Int, name: Text }\n\
         fn fromRow(row: List<Text?>): Widget = Widget { id: 0, name: \"\" }\n\
         fn f(conn: Int): Text = match Query.first(Query.from(\"SELECT 1\"), conn, fromRow) {\
         Some(x) => x.name, None => \"none\" }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Text);
}

#[test]
fn query_list_and_list_first_consumed_inline_with_no_intermediate_val() {
    // BACKLOG item 207 — the file's own "Query.list + List.first" half of the
    // filed repro, nested directly (`List.first(Query.list(...))`) with zero
    // intermediate `val` bindings at all, inside a `match` in the same
    // function. Confirms the same recovery chain (`Query.list`'s `List<T>` ->
    // `List.first`'s `Option<T>`) composes correctly end-to-end.
    let m = lower(
        "module A\nimport Stdlib.Db\ntype Widget = { id: Int, name: Text }\n\
         fn fromRow(row: List<Text?>): Widget = Widget { id: 0, name: \"\" }\n\
         fn f(conn: Int): Text = match List.first(Query.list(Query.from(\"SELECT 1\"), conn, fromRow)) {\
         Some(x) => x.name, None => \"none\" }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Text);
}

// ------------------------------------------------------------------ //
// `Ok`/`Err` as a bare lambda body (BACKLOG item 227)
// ------------------------------------------------------------------ //

#[test]
fn bare_ok_call_recovers_success_type_leaving_error_side_as_error() {
    // A direct HIR probe (not just an end-to-end run) confirmed `Ok(x + 1)`
    // alone — with no enclosing context at all — used to come out as plain
    // `Ty::Error` in its entirety, not "a `Result` with one bad slot". The
    // success side (known from the argument) must now resolve; the error
    // side genuinely can't be known from `Ok(...)` alone, so it stays
    // `Ty::Error` here (this exact case is what `flatMap`'s own arm exists
    // to backfill — see the next test).
    let m = lower("module A\nfn f(x: Int): Result<Int, Text> = Ok(x + 1)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    match &f.body.as_ref().unwrap().ty {
        Ty::Result(ok, _) => assert_eq!(**ok, Ty::Int, "success side should recover to Int"),
        other => panic!("expected Ty::Result, got {:?} (Ok(x+1) collapsed to Ty::Error again)", other),
    }
}

#[test]
fn flatmap_with_bare_ok_lambda_body_resolves_full_result_type() {
    // BACKLOG item 227's own filed repro: `r.flatMap((x) => Ok(x + 1))`
    // segfaulted on consumption because the whole call's type silently
    // stayed `Ty::Error`. `flatMap`'s error type never changes from the
    // receiver's own (by its real signature), so the fix backfills it
    // from `r`'s own known `Text` — not a guess, a correct derivation.
    let m = lower(
        "module A\nfn f(r: Result<Int, Text>): Result<Int, Text> = r.flatMap((x) => Ok(x + 1))");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(
        f.body.as_ref().unwrap().ty,
        Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)),
        "flatMap's own call type must fully resolve, not collapse to Ty::Error"
    );
}

#[test]
fn flatmap_with_bare_err_lambda_body_leaves_the_genuinely_unknowable_success_side_as_error() {
    // A callback whose tail is *unconditionally* `Err("bad")`, never
    // referencing its own param at all, has no argument, branch, or
    // operation anywhere for the success side to be structurally
    // recovered from — unlike every other case here (arithmetic on the
    // param, an `if`/`else` where the *other* branch is `Ok(x)`), this one
    // is a genuine, honest structural-recovery limit, not a bug: real
    // programs write this shape when the callback's success type is
    // truly irrelevant to a call that always errors (e.g. a validation
    // step that never succeeds on its own). Documented here so a future
    // change can't silently start returning something *wrong* instead of
    // just incomplete.
    let m = lower(
        "module A\nfn f(r: Result<Int, Text>): Result<Int, Text> = r.flatMap((x) => Err(\"bad\"))");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(
        f.body.as_ref().unwrap().ty,
        Ty::Result(Box::new(Ty::Error), Box::new(Ty::Text))
    );
}

#[test]
fn flatmap_with_a_conditional_ok_or_err_lambda_body_resolves_full_result_type() {
    // A slightly less trivial callback body (an `if`, not a bare tail call)
    // — both branches independently need the same backfill treatment.
    let m = lower(
        "module A\nfn f(r: Result<Int, Text>): Result<Int, Text> = \
         r.flatMap((x) => if x > 3 then Err(\"too big\") else Ok(x))");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(
        f.body.as_ref().unwrap().ty,
        Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text))
    );
}

#[test]
fn flatmap_with_named_function_callback_is_unaffected() {
    // Regression guard: the already-working named-function-callback path
    // (item 199's own precedent, `callback_ret_ty`'s `Ty::Fn{ret,..}` arm)
    // must still resolve correctly, not be shadowed by the new backfill.
    let m = lower(
        "module A\nfn addOne(x: Int): Result<Int, Text> = Ok(x + 1)\n\
         fn f(r: Result<Int, Text>): Result<Int, Text> = r.flatMap(addOne)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(
        f.body.as_ref().unwrap().ty,
        Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text))
    );
}

// ------------------------------------------------------------------ //
// Lambda closure-capture rejection (BACKLOG item 136)
// ------------------------------------------------------------------ //

#[test]
fn lambda_capturing_outer_local_is_accepted_and_recorded() {
    // BACKLOG item 140: real closure capture superseded item 136's interim
    // "reject any capture" compile error — a lambda referencing an outer
    // local must now lower cleanly, with the captured local's `LocalId`
    // recorded on the `HirExprKind::Lambda` node so MIR can box it into the
    // lambda's own environment.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): List<Int> = { val outer = 1\n List.map(xs, (x) => x + outer) }");
    if let HirItem::Fn(f) = &m.items[0] {
        let body = f.body.as_ref().unwrap();
        let HirExprKind::Block { tail, .. } = &body.kind else { panic!("expected a block body") };
        let HirExprKind::Call { args, .. } = &tail.kind else { panic!("expected a call tail") };
        let HirExprKind::Lambda { captures, .. } = &args[1].kind else { panic!("expected a lambda argument") };
        assert_eq!(captures.len(), 1, "expected exactly one captured local (`outer`), got {:?}", captures);
    } else {
        panic!("expected a fn item");
    }
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

// ------------------------------------------------------------------ //
// Bare-generic-return resolution (BACKLOG item 135)
// ------------------------------------------------------------------ //

const BOX_SRC: &str = "\
type Box<T> = priv Box(T)
impl<T> Box {
    fn wrap(v: T): Box<T> = Box(v)
    fn unwrap(b: Box<T>): T = match b { Box(v) => v }
}
";

fn first_let_ty(m: &crate::HirModule, fn_name: &str) -> Ty {
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == fn_name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn `{}`", fn_name));
    match &f.body.as_ref().unwrap().kind {
        HirExprKind::Block { stmts, .. } => match &stmts[0] {
            crate::hir::HirStmt::Let { ty, .. } => ty.clone(),
            other => panic!("expected a Let statement, got {:?}", other),
        },
        other => panic!("expected a Block body, got {:?}", other),
    }
}

#[test]
fn val_annotation_resolves_bare_generic_return() {
    // `Box.unwrap`'s declared return is bare `T` (`Ty::Var(0)`) — the `val`'s
    // own `: Int` annotation must resolve the call's HIR type to the real
    // `Ty::Int`, not leave it as the unresolved sentinel.
    let src = format!("module A\n{}fn f(): Int = {{\n val x: Int = Box.unwrap(Box.wrap(42))\n x\n}}", BOX_SRC);
    let m = lower(&src);
    assert_eq!(first_let_ty(&m, "f"), Ty::Int);
}

#[test]
fn unannotated_val_with_bare_generic_return_is_an_error() {
    // No declared type anywhere to resolve `Ty::Var(0)` against — must be a
    // hard compile error, not a silently-wrong `void*` flowing through.
    let src = format!("module A\n{}fn f(): Int = {{\n val x = Box.unwrap(Box.wrap(42))\n x\n}}", BOX_SRC);
    let result = try_lower(&src);
    assert!(result.is_err(), "expected an error for an unannotated bare-generic-return val");
}

#[test]
fn argument_position_resolves_bare_generic_return_via_concrete_param() {
    // `addOne`'s declared param is concrete (`Int`) — a bare-generic-return
    // call passed directly as that argument should resolve through it, same
    // as the `val`-annotation case.
    let src = format!(
        "module A\n{}fn addOne(n: Int): Int = n + 1\nfn f(): Int = {{\n val x: Int = addOne(Box.unwrap(Box.wrap(42)))\n x\n}}",
        BOX_SRC
    );
    let m = lower(&src);
    // The outer `val`'s own type must be Int; if the inner call weren't
    // resolved, typeck-independent HIR lowering would still leave it as
    // Ty::Var, but this at least confirms the whole call lowers without
    // erroring and produces the expected outer type.
    assert_eq!(first_let_ty(&m, "f"), Ty::Int);
}

#[test]
fn argument_to_another_generic_param_does_not_resolve_and_errors() {
    // Passing a bare-generic-return call into *another* still-generic
    // function's param (declared type is itself `Ty::Var`, not concrete)
    // gives no usable expected type — must still error rather than silently
    // leaving `Ty::Var(0)` to flow through unchecked.
    let src = format!(
        "module A\n{}fn identity<T>(v: T): T = v\nfn f(): Int = {{\n val x: Int = identity(Box.unwrap(Box.wrap(42)))\n x\n}}",
        BOX_SRC
    );
    let result = try_lower(&src);
    assert!(result.is_err(), "expected an error: identity's param is itself generic, no concrete type to resolve against");
}

#[test]
fn argument_position_resolves_via_stdlib_param_type_not_just_user_defined() {
    // Real bug found via end-to-end testing (BACKLOG item 78's own
    // verification): the argument-position resolution above only checked
    // `cx.fn_param_tys`, which is populated *exclusively* from user-defined
    // `fn`/`impl` declarations — a stdlib callee like `println` was never in
    // it. `println(Box.unwrap(Box.wrap(...)))` silently stayed as the raw
    // unboxed pointer (no resolution, no error) and printed garbage at
    // runtime instead of either resolving or failing to compile. Fixed by
    // also consulting `stdlib_param_types()` (mirroring the existing
    // `stdlib_ret_types()`, built from the same real `TypeEnv` seeding).
    let src = format!(
        "module A\n{}fn f(): Unit [io] = {{\n println(intToText(Box.unwrap(Box.wrap(42))))\n}}",
        BOX_SRC
    );
    let result = try_lower(&src);
    assert!(result.is_ok(), "expected `intToText`'s stdlib param type (Int) to resolve the bare-T return, got: {:?}", result.err());
}

// `expect(x).toBeXxx(...)` desugaring — BACKLOG item 165

fn lower_f_body(src: &str) -> crate::hir::HirExpr {
    let m = lower(src);
    match &m.items[0] {
        HirItem::Fn(f) => f.body.as_ref().expect("fn body").clone(),
        _ => panic!("expected Fn"),
    }
}

#[test]
fn to_be_desugars_to_assert_of_binop_eq() {
    let body = lower_f_body("module A\nfn f(): Unit = (1 + 1).toBe(2)");
    match &body.kind {
        HirExprKind::Call { func, args } => {
            assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "assert"));
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0].kind, HirExprKind::BinOp { op: BinOp::Eq, .. }), "cond must be a == comparison, got {:?}", args[0].kind);
            assert!(matches!(&args[1].kind, HirExprKind::Str(_)), "msg must be a Text literal");
        }
        other => panic!("expected Call to assert, got {:?}", other),
    }
}

#[test]
fn to_be_true_passes_the_condition_through_unwrapped() {
    // No redundant `== true` — the already-Bool expression is the condition directly.
    let body = lower_f_body("module A\nfn f(cond: Bool): Unit = cond.toBeTrue()");
    match &body.kind {
        HirExprKind::Call { args, .. } => {
            assert!(matches!(&args[0].kind, HirExprKind::Local(_)), "expected the bare local, not a BinOp wrapper, got {:?}", args[0].kind);
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn to_be_false_negates_with_unop_not() {
    let body = lower_f_body("module A\nfn f(cond: Bool): Unit = cond.toBeFalse()");
    match &body.kind {
        HirExprKind::Call { args, .. } => {
            assert!(matches!(&args[0].kind, HirExprKind::UnOp { op: crate::hir::UnOp::Not, .. }));
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn to_be_some_calls_option_is_some() {
    let body = lower_f_body("module A\nfn f(o: Int?): Unit = o.toBeSome()");
    match &body.kind {
        HirExprKind::Call { args, .. } => match &args[0].kind {
            HirExprKind::Call { func, .. } => assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Option.isSome")),
            other => panic!("expected a Call to Option.isSome, got {:?}", other),
        },
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn to_be_err_calls_result_is_err() {
    let body = lower_f_body("module A\nfn f(r: Result<Int, Text>): Unit = r.toBeErr()");
    match &body.kind {
        HirExprKind::Call { args, .. } => match &args[0].kind {
            HirExprKind::Call { func, .. } => assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Result.isErr")),
            other => panic!("expected a Call to Result.isErr, got {:?}", other),
        },
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn expect_assertion_result_type_is_unit() {
    let body = lower_f_body("module A\nfn f(): Unit = (1).toBe(1)");
    assert_eq!(body.ty, Ty::Unit);
}

// `computed name: Ty = expr` field access (BACKLOG item 143) — field
// access on a computed name must lower to a `Call` to the synthesized
// accessor (`Type.name`, registered exactly like any other in-body `fn`
// method — BACKLOG item 150), not a struct-field `Field` read.

#[test]
fn computed_field_access_lowers_to_a_call_not_a_field_read() {
    let m = lower(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Bool = o.isPositive"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let body = f.body.as_ref().expect("expected a body");
    match &body.kind {
        HirExprKind::Call { func, args } => {
            assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Order.isPositive"));
            assert_eq!(args.len(), 1, "the receiver is the call's sole argument");
        }
        other => panic!("expected a Call to Order.isPositive, got {other:?}"),
    }
}

#[test]
fn computed_field_access_return_type_is_the_declared_type() {
    let m = lower(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Bool = o.isPositive"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let body = f.body.as_ref().expect("expected a body");
    assert_eq!(body.ty, Ty::Bool);
}

#[test]
fn ordinary_field_access_on_a_type_with_computed_fields_is_still_a_field_read() {
    // Regression: adding the computed-name fallback must not turn a real
    // stored-field read into a call.
    let m = lower(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Int = o.total"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let body = f.body.as_ref().expect("expected a body");
    assert!(matches!(&body.kind, HirExprKind::Field { field, .. } if field == "total"),
        "expected a Field read of `total`, got {:?}", body.kind);
}

// ------------------------------------------------------------------ //
// Record patterns (BACKLOG item 145) — both the type-prefixed
// (`TypeName { ... }`) and bare (`{ ... }`) forms share this lowering.
// ------------------------------------------------------------------ //

#[test]
fn val_record_destructure_gives_each_field_its_real_declared_type() {
    // Regression for the confirmed silent-truncation bug: field bindings
    // used to always get `Ty::Error`, which codegen maps to `int64_t`,
    // reinterpreting a `Float` field's raw bits as an integer.
    let m = lower(
        "module A\n\
         type Product = { name: Text, price: Float }\n\
         fn f(p: Product): Float = {\n\
         \x20   val { name, price } = p\n\
         \x20   price\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a block body") };
    // stmts[0] = `_rec` temp, stmts[1] = `name`, stmts[2] = `price`.
    let price_ty = stmts.iter().find_map(|s| match s {
        crate::hir::HirStmt::Let { name, ty, .. } if name == "price" => Some(ty.clone()),
        _ => None,
    }).expect("expected a `price` binding");
    assert_eq!(price_ty, Ty::Float);
}

// ------------------------------------------------------------------ //
// List rest-pattern `[head, ...tail]` — BACKLOG item 195
// ------------------------------------------------------------------ //

#[test]
fn val_list_rest_pattern_binds_each_head_element_and_tail_with_real_types() {
    // Regression for the confirmed crash: this used to fall into the
    // generic "complex pattern" stub, which dropped every binding —
    // `head`/`tail` referenced further down had no local at all,
    // producing an undeclared-identifier error in the generated C.
    let m = lower(
        "module A\n\
         fn f(xs: List<Float>): Float = {\n\
         \x20   val [head, ...tail] = xs\n\
         \x20   head\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a block body") };
    let head_ty = stmts.iter().find_map(|s| match s {
        crate::hir::HirStmt::Let { name, ty, .. } if name == "head" => Some(ty.clone()),
        _ => None,
    }).expect("expected a `head` binding");
    let tail_ty = stmts.iter().find_map(|s| match s {
        crate::hir::HirStmt::Let { name, ty, .. } if name == "tail" => Some(ty.clone()),
        _ => None,
    }).expect("expected a `tail` binding");
    // `head`'s real element type (`Float`), not the generic `Ty::Error`
    // fallback that would silently reinterpret its raw bits as an Int.
    assert_eq!(head_ty, Ty::Float);
    // `tail` is the *same* `List<Float>` type as the whole value, not a
    // smaller/different type.
    assert_eq!(tail_ty, Ty::List(Box::new(Ty::Float)));
}

#[test]
fn val_list_pattern_with_no_tail_still_binds_head_elements() {
    let m = lower(
        "module A\n\
         fn f(xs: List<Int>): Int = {\n\
         \x20   val [a, b] = xs\n\
         \x20   a + b\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a block body") };
    for name in ["a", "b"] {
        let ty = stmts.iter().find_map(|s| match s {
            crate::hir::HirStmt::Let { name: n, ty, .. } if n == name => Some(ty.clone()),
            _ => None,
        }).unwrap_or_else(|| panic!("expected a `{name}` binding"));
        assert_eq!(ty, Ty::Int, "`{name}` should have Int's real type, not Ty::Error");
    }
}

#[test]
fn list_rest_pattern_in_match_arm_lowers_to_hirpat_list_with_real_elem_ty() {
    let m = lower(
        "module A\n\
         fn f(xs: List<Float>): Float = match xs {\n\
         \x20   [a, ...rest] => a,\n\
         \x20   _ => 0.0\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    match &arms[0].pat {
        crate::hir::HirPat::List { head, tail, elem_ty } => {
            assert_eq!(elem_ty, &Ty::Float);
            assert_eq!(head.len(), 1);
            assert!(matches!(&head[0], crate::hir::HirPat::Bind { name, .. } if name == "a"));
            let Some(tail_pat) = tail else { panic!("expected a bound tail pattern") };
            assert!(matches!(tail_pat.as_ref(), crate::hir::HirPat::Bind { name, .. } if name == "rest"));
        }
        other => panic!("expected HirPat::List, got {other:?}"),
    }
}

#[test]
fn list_pattern_with_no_rest_lowers_with_no_tail() {
    let m = lower(
        "module A\n\
         fn f(xs: List<Int>): Int = match xs {\n\
         \x20   [a, b] => a + b,\n\
         \x20   _ => 0\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    match &arms[0].pat {
        crate::hir::HirPat::List { head, tail, .. } => {
            assert_eq!(head.len(), 2);
            assert!(tail.is_none(), "no `...rest` in the pattern — tail must be None");
        }
        other => panic!("expected HirPat::List, got {other:?}"),
    }
}

#[test]
fn type_prefixed_record_pattern_in_match_arm_lowers_to_hirpat_record() {
    let m = lower(
        "module A\n\
         type User = { name: Text, age: Int }\n\
         fn f(u: User): Text = match u {\n\
         \x20   User { name: n, age: a } => n,\n\
         \x20   _ => \"?\"\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    match &arms[0].pat {
        crate::hir::HirPat::Record { fields, field_names, field_types } => {
            assert_eq!(field_names, &vec!["name".to_string(), "age".to_string()]);
            assert_eq!(field_types, &vec![Ty::Text, Ty::Int]);
            assert_eq!(fields.len(), 2);
        }
        other => panic!("expected HirPat::Record (not Wildcard), got {other:?}"),
    }
}

#[test]
fn bare_record_pattern_in_match_arm_resolves_type_structurally() {
    // No leading type name — must resolve the same `field_names`/
    // `field_types` as the prefixed form, using the scrutinee's own type.
    let m = lower(
        "module A\n\
         type User = { name: Text, age: Int }\n\
         fn f(u: User): Text = match u {\n\
         \x20   { name: n, age: a } => n,\n\
         \x20   _ => \"?\"\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    match &arms[0].pat {
        crate::hir::HirPat::Record { field_names, field_types, .. } => {
            assert_eq!(field_names, &vec!["name".to_string(), "age".to_string()]);
            assert_eq!(field_types, &vec![Ty::Text, Ty::Int]);
        }
        other => panic!("expected HirPat::Record (not Wildcard), got {other:?}"),
    }
}

#[test]
fn bare_record_pattern_shorthand_field_binds_a_local() {
    let m = lower(
        "module A\n\
         type User = { name: Text, age: Int }\n\
         fn f(u: User): Text = match u {\n\
         \x20   { name, age } => name,\n\
         \x20   _ => \"?\"\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    match &arms[0].pat {
        crate::hir::HirPat::Record { fields, .. } => {
            assert!(matches!(&fields[0], crate::hir::HirPat::Bind { name, .. } if name == "name"));
            assert!(matches!(&fields[1], crate::hir::HirPat::Bind { name, .. } if name == "age"));
        }
        other => panic!("expected HirPat::Record, got {other:?}"),
    }
}

#[test]
fn generic_record_match_arm_binding_gets_the_real_substituted_type_not_ty_var() {
    // Regression (found while verifying BACKLOG item 177): a bound field's
    // *local* must carry the scrutinee's real instantiation type (`Float`),
    // not the record's raw, unsubstituted declared type (`Ty::Var(0)`) —
    // otherwise a later reference to it (here, the arm's own body) infers
    // the wrong HIR type and codegen mismatches a boxed `void*` against a
    // real unboxed `double`.
    let m = lower(
        "module A\n\
         type Box<T> = { value: T }\n\
         fn f(b: Box<Float>): Float = match b {\n\
         \x20   Box { value: v } => v,\n\
         \x20   _ => 0.0\n\
         }"
    );
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Match { arms, .. } = &f.body.as_ref().unwrap().kind else { panic!("expected a match expr") };
    assert_eq!(arms[0].body.ty, Ty::Float, "arm body (the bound `v`) must resolve to Float, not {:?}", arms[0].body.ty);
}

// ------------------------------------------------------------------ //
// `withTimeout(duration) { body }` — BACKLOG item 122
// ------------------------------------------------------------------ //

#[test]
fn with_timeout_desugars_to_deadline_spawn_and_join_timed_cancel() {
    use crate::hir::HirStmt;
    let m = lower(
        "module A\nfn work(): Int = 1\nasync fn f(): Int? = withTimeout(Duration.seconds(5)) { work() }");
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Block { stmts, tail } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block, got {:?}", f.body.as_ref().unwrap().kind)
    };
    assert_eq!(stmts.len(), 2, "expected [let deadline = ..., let task = spawn ...], got {:?}", stmts);
    let HirStmt::Let { init, .. } = &stmts[0] else { panic!("expected a Let stmt") };
    assert!(matches!(&init.kind, HirExprKind::BinOp { .. }), "expected the deadline math, got {:?}", init.kind);
    let HirStmt::Let { init, .. } = &stmts[1] else { panic!("expected a Let stmt") };
    assert!(matches!(&init.kind, HirExprKind::Spawn { .. }), "expected a Spawn, got {:?}", init.kind);
    assert!(matches!(&tail.kind, HirExprKind::JoinTimedCancel { .. }), "expected JoinTimedCancel, got {:?}", tail.kind);
}

#[test]
fn with_timeout_task_references_the_spawned_local() {
    use crate::hir::HirStmt;
    let m = lower(
        "module A\nfn work(): Int = 1\nasync fn f(): Int? = withTimeout(Duration.seconds(5)) { work() }");
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let HirExprKind::Block { stmts, tail } = &f.body.as_ref().unwrap().kind else { panic!("expected a Block") };
    let HirStmt::Let { local: task_local, .. } = &stmts[1] else { panic!("expected the task Let stmt") };
    let HirExprKind::JoinTimedCancel { task, .. } = &tail.kind else { panic!("expected JoinTimedCancel") };
    assert!(matches!(&task.kind, HirExprKind::Local(id) if id == task_local),
        "JoinTimedCancel's task must reference the spawned local, got {:?}", task.kind);
}

// Bare-interpolation f-string (`f"{n}"`, no surrounding literal text) — found
// while looking at item 183, unrelated to it. `parse_fstring_parts`
// (`crates/parser/src/parse_expr.rs`) yields a single-element `parts` list
// for this shape (no leading/trailing `Literal("")`), and the old lowering
// took the first segment as-is whenever nothing else needed folding into it,
// so the f-string's *own* type became whatever the interpolated expression's
// type was (e.g. `Int`) instead of `Text` — never passing through the
// `Concat` node that codegen's `coerce_to_text` (`crates/codegen/src/
// emit_mir.rs`) relies on to convert a non-Text value. A `println(f"{n}")`
// with `n: Int` compiled cleanly but segfaulted at runtime, reading the raw
// `int64_t` as a `certo_text_t` pointer.

#[test]
fn bare_interpolation_f_string_has_text_type_not_the_interpolated_expr_type() {
    let body = lower_f_body("module A\nfn f(n: Int): Text = f\"{n}\"");
    assert_eq!(body.ty, Ty::Text, "f\"{{n}}\"'s own type must be Text, not Int, got {:?}", body.ty);
}

#[test]
fn bare_interpolation_f_string_lowers_to_a_concat_not_a_bare_local() {
    // The fix routes even a single segment through `BinOp::Concat` (seeded
    // with an empty Text literal) so codegen's per-operand `coerce_to_text`
    // always runs — asserting the *shape*, not just the final `Ty`, is what
    // actually pins the fix: a bare `Local(n)` could still incorrectly carry
    // a forged `Ty::Text` without ever being converted at the value level.
    let body = lower_f_body("module A\nfn f(n: Int): Text = f\"{n}\"");
    assert!(matches!(&body.kind, HirExprKind::BinOp { op: BinOp::Concat, .. }),
        "expected a Concat chain even for a single interpolation, got {:?}", body.kind);
}

#[test]
fn multi_part_f_string_still_lowers_to_a_left_folded_concat_chain() {
    // No-regression check: `f"hello {name}!"` (literal, interpolation,
    // literal — three parts) must still fold left-to-right into nested
    // Concat nodes, same as before this fix, just now seeded with an extra
    // leading empty-string Concat rather than starting from the first
    // segment directly.
    let body = lower_f_body("module A\nfn f(name: Text): Text = f\"hello {name}!\"");
    let HirExprKind::BinOp { op: BinOp::Concat, rhs, .. } = &body.kind else {
        panic!("expected outermost Concat, got {:?}", body.kind)
    };
    assert!(matches!(&rhs.kind, HirExprKind::Str(s) if s == "!"), "expected trailing literal \"!\", got {:?}", rhs.kind);
    assert_eq!(body.ty, Ty::Text);
}

// `await <plain call>` used to hardcode its own HIR type to `Ty::Error`
// regardless of the awaited expression's own already-resolved type — the
// same "known type discarded" failure shape as items 179/180/182, just in
// `Expr::Await`'s own lowering this time. Found while verifying item 186's
// checkpoint fix didn't break withTimeout's successful (non-abandoned)
// path: `withTimeout(d) { await namedFn() }` compiled cleanly but
// segfaulted on success, because the match-arm binding unwrapping its
// `Option<T>` result inherited `Ty::Error` for `T`, so an f-string
// interpolating it skipped `certo_int_to_text` and read the raw int as a
// text pointer — BACKLOG item 191.

#[test]
fn await_of_plain_call_has_the_calls_own_type_not_error() {
    let body = lower_f_body("module A\nasync fn work(): Int = 1\nasync fn f(): Int = await work()");
    assert_eq!(body.ty, Ty::Int, "await <plain call>'s own type must be the call's real type, got {:?}", body.ty);
}

#[test]
fn with_timeout_await_body_result_type_is_option_of_the_awaited_calls_type() {
    // The exact original repro: withTimeout's own Option<T> must resolve T
    // from the awaited call's real type, not silently fall back to Error.
    let m = lower(
        "module A\nasync fn work(): Int = 1\nasync fn f(): Int? = withTimeout(Duration.seconds(5)) { await work() }");
    let HirItem::Fn(f) = m.items.iter().find(|it| matches!(it, HirItem::Fn(f) if f.name == "f")).unwrap() else { unreachable!() };
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Option(Box::new(Ty::Int)),
        "withTimeout(d) {{ await work() }}'s own type must be Option<Int>, got {:?}", body.ty);
}

// ------------------------------------------------------------------ //
// Dot-call vs. underscore-named real function (BACKLOG item 224)
// ------------------------------------------------------------------ //

#[test]
fn dot_qualified_call_to_an_underscore_named_function_resolves_its_real_return_type() {
    // Mirrors exactly what `expand_validators` (`crates/cli/src/main.rs`)
    // actually produces: a real function named with an underscore
    // (`V_validate`, so the generated source parses as an ordinary
    // function — `crates/codegen/src/emit_validator.rs`'s own
    // `build_fn_sig`), called at a dot-qualified site (`V.validate(...)`,
    // matching every validator call site users actually write). `c_fn_name`
    // (`crates/codegen/src/emit_mir.rs`) already normalizes both spellings
    // to the identical C symbol, so the call itself always linked and ran
    // correctly — only the HIR-level *return type* lookup (`cx.fn_ret_types`,
    // a literal-string `HashMap` with no dot/underscore normalization of
    // its own) silently missed and fell all the way through to `Ty::Error`,
    // confirmed directly via a real segfault-class UFCS dot-call failure
    // (`V.validate(entity).isOk()` failed to compile even though `match`-ing
    // the identical value already worked).
    let m = lower("module A\nfn V_validate(x: Int): Result<Int, Text> = Ok(x)\nfn f(x: Int): Result<Int, Text> = V.validate(x)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(
        f.body.as_ref().unwrap().ty,
        Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)),
        "dot-qualified call to an underscore-named real function must resolve its real return type, not Ty::Error"
    );
}

#[test]
fn plain_dot_qualified_call_matching_the_real_name_is_unaffected() {
    // Regression guard: the already-working, exact-name-match case (a real
    // user `impl` method, registered under its own literal dotted global
    // name — `Decl::Impl`'s own pre-registration, unlike a validator's
    // underscore-joined function) must not be disturbed by the new
    // underscore-fallback lookup.
    let m = lower(
        "module A\ntype Widget = { n: Int }\nimpl Widget { fn size(w: Widget): Int = w.n }\n\
         fn f(w: Widget): Int = Widget.size(w)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int);
}

// ------------------------------------------------------------------ //
// `for x in a..b` range-loop variable typing (BACKLOG item 231)
// ------------------------------------------------------------------ //

#[test]
fn range_call_resolves_to_list_int_not_error() {
    // `1..5`/`1...5` desugar directly to a `range`/`rangeInclusive` stdlib
    // call — this bypassed the general call-type-recovery machinery
    // entirely (it's a dedicated `Expr::BinOp` desugar, not a real call
    // expression), so it always hardcoded `Ty::Error` regardless of the
    // stdlib's own always-correct `(Int, Int) -> List<Int>` signature.
    let m = lower("module A\nfn f(): List<Int> = 1..5");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::List(Box::new(Ty::Int)));
}

#[test]
fn for_loop_binding_ty_resolves_to_int_not_error() {
    // The loop variable's own recorded type (`binding_ty`, MIR's fallback
    // in `crates/mir/src/lower.rs`) used to stay `Ty::Error` unconditionally
    // — confirmed via a real segfault: `for i in 1..5 { f"{i}" }`
    // interpolated the loop variable's raw bits as if already `Text`.
    let m = lower("module A\nfn f(): Unit = for i in 1..3 { println(f\"{i}\") }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::For { binding_ty, iter, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a top-level For expression");
    };
    assert_eq!(*binding_ty, Ty::Int, "for-loop binding_ty must resolve to Int, not Ty::Error");
    assert_eq!(iter.ty, Ty::List(Box::new(Ty::Int)), "the range iterator's own type must resolve too");
}

#[test]
fn for_loop_variable_reference_inside_the_body_resolves_to_int() {
    // The deeper half of the same bug: even with `binding_ty` fixed, the
    // loop variable's *local type table entry* (`cx.local_types`) must
    // also be populated *before* the body is lowered, since every
    // `Expr::Path` reference to `i` inside the loop reads from that table
    // directly, independent of `binding_ty`. Checks a real in-body
    // reference (`i + 1`) resolves to `Int`, not `Ty::Error`.
    let m = lower("module A\nfn f(): Unit = for i in 1..3 { val n = i + 1\nprintln(f\"{n}\") }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::For { body, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a top-level For expression");
    };
    let HirExprKind::Block { stmts, .. } = &body.kind else { panic!("expected a Block loop body") };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val n = i + 1`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Int, "`i + 1` inside the loop body must resolve to Int, not Ty::Error");
}

// BACKLOG item 235 — a bare literal at an annotated fixed-width position
// must get the *declared* type (`Ty::Int8` etc.), not `lower_lit`'s own
// rigid default (`Ty::Int`/`Ty::Float`) — checked directly on the lowered
// HIR node's own `.ty` field, since typeck accepting the annotation alone
// doesn't guarantee HIR's independent lowering agrees (MIR derives a
// function's real C return type from the body's own computed operand type,
// not from `HirFn.ret_ty`, so a wrong `.ty` here would still emit the wrong
// C type even though typeck no longer rejects the program).
fn let_ty_of(m: &crate::HirModule, fn_name: &str) -> Ty {
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == fn_name => Some(f),
        _ => None,
    }).unwrap_or_else(|| panic!("expected fn `{fn_name}`"));
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { ty, .. } = stmts.first().expect("expected a `val`/`var` statement") else {
        panic!("expected the first statement to be a Let");
    };
    ty.clone()
}

#[test]
fn val_literal_resolves_to_each_declared_fixed_width_type() {
    let m = lower("module A\nfn f(): Unit = { val a: Int8 = 100 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int8);
    let m = lower("module A\nfn f(): Unit = { val a: Int16 = 100 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int16);
    let m = lower("module A\nfn f(): Unit = { val a: Int32 = 100 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int32);
    let m = lower("module A\nfn f(): Unit = { val a: UInt = 100 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::UInt);
    let m = lower("module A\nfn f(): Unit = { val a: Float32 = 3.5 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Float32);
}

// BACKLOG item 306 — `Int64` (spec §4.1: "Same as Int") was never mapped to
// any `Ty` in `ast_ty_to_ty_with_params_depth` (this function's own
// duplicate of typeck's identical conversion table), unlike its sibling
// fixed-width names just above — checked directly on HIR's own lowered
// type, same rationale as `let_ty_of`'s own doc comment: typeck accepting
// an annotation doesn't guarantee HIR's independent conversion agrees.
#[test]
fn int64_resolves_to_plain_int_not_a_dead_type_name() {
    let m = lower("module A\nfn f(): Unit = { val a: Int64 = 100 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int);
}

#[test]
fn var_literal_also_resolves_to_the_declared_fixed_width_type() {
    // `Stmt::Var` previously discarded its own type annotation entirely in
    // HIR (`Stmt::Var { name, value, .. }` — the `ty` field was never even
    // destructured), unlike `Stmt::Val`.
    let m = lower("module A\nfn f(): Unit = { var a: Int16 = 200 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int16);
}

#[test]
fn negative_int_literal_resolves_to_the_signed_fixed_width_type() {
    // `-5000` parses as `UnOp::Neg` wrapping a bare `Lit::Int` — must see
    // through the negation to still recognize the literal.
    let m = lower("module A\nfn f(): Unit = { val a: Int32 = -5000 }");
    assert_eq!(let_ty_of(&m, "f"), Ty::Int32);
}

#[test]
fn fn_return_position_literal_resolves_to_the_declared_fixed_width_type() {
    let m = lower("module A\nfn f(): Int8 = 42");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Int8,
        "the bare-literal function body's own HIR type must resolve to Int8, not Ty::Int — \
         MIR derives the C return type from this, not from HirFn.ret_ty");
}

// ------------------------------------------------------------------ //
// BACKLOG item 322 — an empty list literal (`[]`) has no elements of its
// own to infer an element type from (`Expr::List`'s own lowering always
// leaves it `Ty::List(Ty::Error)`); a `val`/`var`'s own declared
// annotation must patch that in, the same role `literal_matches_fixed_
// width` already plays for an under-determined numeric literal just
// above. Confirmed live before this fix: `val empty: List<Item> = []`
// followed by `List.sumBy(empty, (i) => i.price)` miscompiled to invalid
// C (the lambda parameter's own type stayed `Ty::Error`/`int64_t`), even
// though passing the *same* `[]` literal directly as an ordinary function
// argument (governed by that function's own declared param type, a
// wholly different code path) already worked correctly.
// ------------------------------------------------------------------ //

#[test]
fn val_empty_list_literal_resolves_element_type_from_its_own_annotation() {
    let m = lower("module A\ntype Item = { price: Decimal }\nfn f(): Unit = { val empty: List<Item> = [] }");
    assert_eq!(let_ty_of(&m, "f"), Ty::List(Box::new(Ty::Named { name: "Item".into(), args: vec![] })),
        "an empty list literal's own type must come from its val's declared annotation, not stay List<Ty::Error>");
}

#[test]
fn var_empty_list_literal_resolves_element_type_from_its_own_annotation() {
    let m = lower("module A\ntype Item = { price: Decimal }\nfn f(): Unit = { var empty: List<Item> = [] }");
    assert_eq!(let_ty_of(&m, "f"), Ty::List(Box::new(Ty::Named { name: "Item".into(), args: vec![] })),
        "an empty list literal's own type must come from its var's declared annotation, not stay List<Ty::Error>");
}

#[test]
fn nonempty_list_literal_element_type_is_unaffected_by_the_annotation_patch() {
    // Regression guard: a *non*-empty list literal already correctly infers
    // its own element type from its real elements — the annotation-based
    // patch above must only ever fire for the genuinely under-determined
    // (empty) case, never override an already-correct inferred type.
    let m = lower("module A\nfn f(): Unit = { val xs: List<Int> = [1, 2, 3] }");
    assert_eq!(let_ty_of(&m, "f"), Ty::List(Box::new(Ty::Int)));
}

#[test]
fn sumby_over_a_val_annotated_empty_list_resolves_correctly() {
    // The item's own original repro, checked at the actual call site (not
    // just the `val`'s own type) — the lambda parameter and the whole
    // `List.sumBy` call must resolve to `Item`/`Decimal`, not `Ty::Error`.
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         type Item = { price: Decimal }\n\
         fn f(): Decimal = {\n    val empty: List<Item> = []\n    List.sumBy(empty, (i) => i.price)\n}"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { tail, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    assert_eq!(tail.ty, Ty::Decimal(None), "List.sumBy over a val-annotated empty list must still resolve Decimal, got {:?}", tail.ty);
}

// BACKLOG item 248 — `Expr::UnOp` unconditionally hardcoded `ty: Ty::Error`
// regardless of the operand's own type, masked for `-Int` only because
// Ty::Error's own C fallback (int64_t) happens to coincide with Int's;
// negating anything else (Float, a fixed-width int) got silently
// miscompiled once typeck itself started accepting them (this item's own
// typeck-side fix). Negation must preserve the operand's real type.
#[test]
fn negation_hir_node_carries_the_operands_own_type_not_ty_error() {
    let m = lower("module A\nfn f(): Unit = { val y: Float = -3.5 }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val y = -3.5`") else {
        panic!("expected the first statement to be a Let");
    };
    let HirExprKind::UnOp { .. } = &init.kind else { panic!("expected a UnOp node, got {:?}", init.kind) };
    assert_eq!(init.ty, Ty::Float, "negating a Float must carry Ty::Float, not Ty::Error");
}

#[test]
fn not_hir_node_carries_ty_bool_not_ty_error() {
    let m = lower("module A\nfn f(): Unit = { val b = not true }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val b = not true`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Bool, "`not` must carry Ty::Bool, not Ty::Error");
}

// BACKLOG item 246 — `.age` on an `Option<Timestamp>` (`Timestamp?`)
// previously desugared straight-line to `DateTime.diff(DateTime.now(), e)`
// with no `None`/`Some` handling at all, passing the still-boxed Option
// where a raw `CertoDateTime` struct was expected. Must desugar to a real
// match: `None` → `Duration.max` (i64::MAX milliseconds), `Some(x)` →
// the original `DateTime.diff(now, x)` call against the *unwrapped* `x`.
#[test]
fn optional_timestamp_age_desugars_to_a_match_with_a_duration_max_none_arm() {
    let m = lower("module A\nfn f(t: Timestamp?): Duration = t.age");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Named { name: "Duration".into(), args: vec![] });
    let HirExprKind::Match { arms, .. } = &body.kind else {
        panic!("expected `.age` on an Option to desugar to a Match, got {:?}", body.kind);
    };
    assert_eq!(arms.len(), 2);

    let some_arm = arms.iter().find(|a| matches!(&a.pat, HirPat::Constructor { name, .. } if name == "Some"))
        .expect("expected a Some(_) arm");
    let HirExprKind::Call { func, args } = &some_arm.body.kind else {
        panic!("expected the Some arm's body to be a DateTime.diff call, got {:?}", some_arm.body.kind);
    };
    assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "DateTime.diff"));
    assert_eq!(args.len(), 2);
    // The second argument must be the *unwrapped* payload local, not the
    // still-Option-wrapped scrutinee — the whole point of the fix.
    assert!(matches!(&args[1].kind, HirExprKind::Local(_)));
    assert_ne!(args[1].ty, Ty::Option(Box::new(Ty::Named { name: "Timestamp".into(), args: vec![] })),
        "the Some arm's DateTime.diff must receive the unwrapped Timestamp, not the still-optional value");

    let none_arm = arms.iter().find(|a| matches!(&a.pat, HirPat::Constructor { name, .. } if name == "None"))
        .expect("expected a None arm");
    assert!(matches!(none_arm.body.kind, HirExprKind::Int(i64::MAX)),
        "the None arm must produce Duration.max (i64::MAX milliseconds), got {:?}", none_arm.body.kind);
    assert_eq!(none_arm.body.ty, Ty::Named { name: "Duration".into(), args: vec![] });
}

#[test]
fn non_optional_timestamp_age_is_unaffected_by_the_optional_fix() {
    // The ordinary, non-optional `.age` path (a bare `Timestamp`, not
    // `Timestamp?`) must still desugar straight-line, no regression.
    let m = lower("module A\nfn f(t: Timestamp): Duration = t.age");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert!(matches!(&body.kind, HirExprKind::Call { .. }),
        "a non-optional Timestamp's .age must stay a direct call, not a Match, got {:?}", body.kind);
    assert_eq!(body.ty, Ty::Named { name: "Duration".into(), args: vec![] });
}

// BACKLOG item 245 — `a in xs` desugars directly to `List.contains(xs, a)`
// — note the reversed argument order vs. the source's own `item in list`.
#[test]
fn membership_in_desugars_to_list_contains_with_reversed_args() {
    let m = lower("module A\nfn f(): Bool = 2 in [1, 2, 3]");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Bool);
    let HirExprKind::Call { func, args } = &body.kind else {
        panic!("expected `in` to desugar to a Call, got {:?}", body.kind);
    };
    assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "List.contains"));
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].ty, Ty::List(Box::new(Ty::Int)), "first arg must be the list");
    assert_eq!(args[1].ty, Ty::Int, "second arg must be the searched-for item");
}

// BACKLOG item 250 — a bare `Some(x)`/`None` literal previously never
// resolved its own real type at all (`Ty::Error`), even when a `val`'s own
// declared annotation made the concrete type completely unambiguous.
#[test]
fn bare_some_call_resolves_option_type_from_its_own_argument() {
    // `Some(x)` needs no external annotation at all — its own argument
    // already carries a real type, mirroring `Ok`/`Err`'s existing fix
    // (item 227) in `generic_container_ret`.
    let m = lower("module A\nfn f(): Unit = { val x = Some(5) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val x = Some(5)`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Option(Box::new(Ty::Int)),
        "Some(5) must resolve to Option<Int>, not Ty::Error, got {:?}", init.ty);
}

#[test]
fn bare_none_resolves_option_type_from_the_vals_own_declared_annotation() {
    let m = lower("module A\nfn f(): Unit = { val x: Option<Int> = None }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val x: Option<Int> = None`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Option(Box::new(Ty::Int)),
        "None must resolve to the val's own declared Option<Int>, not Ty::Error, got {:?}", init.ty);
}

#[test]
fn bare_none_with_no_annotation_stays_ty_error_not_a_new_hard_error() {
    // Conservative choice: with nothing at all pinning the payload type
    // down, `None` silently stays `Ty::Error` (the pre-fix behavior),
    // rather than a new hard-error path that could break code that
    // happens to work today without ever needing the real type — unlike
    // an unresolved generic call's return (which *always* needs
    // resolving), an unannotated `None` might genuinely never need it.
    let m = try_lower("module A\nfn f(): Unit = { val x = None }");
    assert!(m.is_ok(), "an unannotated bare `None` must not become a new hard lowering error");
    let f = m.unwrap().items.into_iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val x = None`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Error);
}

#[test]
fn some_via_a_function_argument_still_resolves_via_the_callees_declared_param_type() {
    // Regression guard for the *other* call site of
    // `resolve_bare_generic_return` (function-call-argument backfill,
    // used independently of the `val`-annotation path) — must still work
    // for both `None` and `Some(x)` passed directly as arguments.
    let m = lower("module A\nfn ageOf(t: Timestamp?): Duration = t.age\nfn f(): Duration = ageOf(None)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected fn f's body to be a Call");
    };
    assert_eq!(args[0].ty, Ty::Option(Box::new(Ty::Named { name: "Timestamp".into(), args: vec![] })),
        "None passed as ageOf's argument must resolve via the callee's declared param type");
}

// BACKLOG item 247 — a generic function's return type with a `Ty::Var`
// *nested inside* a compound type (e.g. `Result<T, E>`) was never resolved
// at all — `resolve_bare_generic_return` only recognized a *bare*
// `Ty::Var(_)` as the whole return type (`fn identity<T>(x: T): T`), never
// one buried inside `Result`/`Option`/etc. Every declared type param
// collapses to the identical sentinel `Ty::Var(0)` regardless of name, so
// resolution has to be positional (walking both types in lockstep), not
// identity-based.
#[test]
fn generic_result_return_resolves_both_slots_from_a_val_annotation() {
    let m = lower("module A\nfn wrapOk<T, E>(v: T): Result<T, E> = Ok(v)\nfn f(): Unit = { val r: Result<Int, Text> = wrapOk(42) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val r = wrapOk(42)`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)),
        "wrapOk(42)'s call type must fully resolve to Result<Int, Text>, not stay generic/Error, got {:?}", init.ty);
}

#[test]
fn generic_result_return_merges_only_the_still_unresolved_slot() {
    // `merge_var_slots` must keep an already-concrete slot from `actual` as
    // -is, not blindly overwrite the whole type with `expected` — a
    // function generic in only the error type, with a fixed success type.
    let m = lower("module A\nfn okOrErr<E>(e: E): Result<Int, E> = Err(e)\nfn f(): Unit = { val r: Result<Int, Text> = okOrErr(\"boom\") }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val r = okOrErr(\"boom\")`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)));
}

#[test]
fn generic_option_return_also_resolves_via_the_same_mechanism() {
    // The fix generalizes beyond Result — Option<T> (and List<T>/Map<K,V>/
    // Tuple/user-defined Named<Args> generics) share the identical
    // nested-Var-slot shape.
    let m = lower("module A\nfn wrapSome<T>(v: T): Option<T> = Some(v)\nfn f(): Unit = { val o: Option<Int> = wrapSome(7) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val o = wrapSome(7)`") else {
        panic!("expected the first statement to be a Let");
    };
    assert_eq!(init.ty, Ty::Option(Box::new(Ty::Int)));
}

#[test]
fn generic_result_return_still_resolves_via_a_function_argument_too() {
    // Regression guard for the *other* call site of
    // `resolve_bare_generic_return` (function-call-argument backfill).
    let m = lower("module A\nfn wrapOk<T, E>(v: T): Result<T, E> = Ok(v)\n\
                   fn takesResult(r: Result<Int, Text>): Unit = {}\n\
                   fn f(): Unit = { takesResult(wrapOk(1)) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected fn f's body to be a Call");
    };
    assert_eq!(args[0].ty, Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)));
}

#[test]
fn generic_result_return_with_no_expected_type_is_still_a_hard_error() {
    // Preserves the pre-existing behavior for the genuinely-unresolvable
    // case — nothing anywhere provides a concrete type for T/E.
    let m = try_lower("module A\nfn wrapOk<T, E>(v: T): Result<T, E> = Ok(v)\nfn f(): Unit = { val r = wrapOk(42) }");
    assert!(m.is_err(), "a generic Result-returning call with no expected type anywhere must still be a hard error");
}

// BACKLOG item 251 — `await genericCall(...)`'s own bound value is
// `HirExprKind::Await(inner)`, not a bare `Call` — the item 247 fix above
// only ever looked for `Call` directly, so an awaited generic call's
// return type (this item's own original filed repro, `async fn
// withAudit<T,E>`) never got resolved at all, regardless of the item 247
// fix. `resolve_bare_generic_return` now sees through an `Await` wrapper
// to its inner call.
#[test]
fn awaited_generic_result_return_resolves_via_a_val_annotation() {
    let m = lower("module A\nasync fn wrapOk<T, E>(v: T): Result<T, E> = Ok(v)\nfn f(): Unit = { val r: Result<Int, Text> = await wrapOk(42) }");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Block { stmts, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected a Block body");
    };
    let HirStmt::Let { init, .. } = stmts.first().expect("expected `val r = await wrapOk(42)`") else {
        panic!("expected the first statement to be a Let");
    };
    assert!(matches!(&init.kind, HirExprKind::Await(_)), "expected the Let's init to still be an Await node");
    assert_eq!(init.ty, Ty::Result(Box::new(Ty::Int), Box::new(Ty::Text)),
        "await wrapOk(42)'s own type must fully resolve to Result<Int, Text>, got {:?}", init.ty);
}

#[test]
fn awaited_generic_result_return_with_no_expected_type_is_still_a_hard_error() {
    let m = try_lower("module A\nasync fn wrapOk<T, E>(v: T): Result<T, E> = Ok(v)\nfn f(): Unit = { val r = await wrapOk(42) }");
    assert!(m.is_err(), "an awaited generic Result-returning call with no expected type anywhere must still be a hard error");
}

// ------------------------------------------------------------------ //
// Row polymorphism codegen via call-site field accessors — BACKLOG item 200
// ------------------------------------------------------------------ //

#[test]
fn row_bound_fn_gains_an_extra_accessor_param() {
    let m = lower("module A\nfn getName<R: { name: Text }>(record: R): Text = record.name");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "getName" => Some(f),
        _ => None,
    }).expect("expected fn `getName`");
    assert_eq!(f.params.len(), 2, "expected the ordinary `record` param plus one synthesized accessor param, got {:?}", f.params);
    assert!(matches!(&f.params[1].ty, Ty::Fn { params, ret } if params.is_empty() && matches!(ret.as_ref(), Ty::Var(_))),
        "expected the accessor param's type to be an erased zero-arg closure, got {:?}", f.params[1].ty);
}

#[test]
fn row_bound_field_access_lowers_to_a_call_on_the_accessor_not_a_field_node() {
    let m = lower("module A\nfn getName<R: { name: Text }>(record: R): Text = record.name");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "getName" => Some(f),
        _ => None,
    }).expect("expected fn `getName`");
    let body = f.body.as_ref().expect("expected a body");
    assert_eq!(body.ty, Ty::Text, "record.name's own type must resolve to Text (the bound's declared field type), got {:?}", body.ty);
    let HirExprKind::Call { func, args } = &body.kind else {
        panic!("expected record.name to lower to a Call on the accessor, got {:?}", body.kind);
    };
    assert!(args.is_empty(), "the accessor call takes no arguments");
    let accessor_local = f.params[1].local;
    assert!(matches!(&func.kind, HirExprKind::Local(id) if *id == accessor_local),
        "expected the call's own callee to be the synthesized accessor param, got {:?}", func.kind);
}

#[test]
fn row_bound_call_site_synthesizes_one_accessor_arg_per_bound_field() {
    let m = lower(
        "module A\ntype Coupon = { name: Text, discount: Int }\n\
         fn describe<R: { name: Text, discount: Int }>(record: R): Text = record.name\n\
         fn f(c: Coupon): Text = describe(c)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected f's body to be a Call");
    };
    // The ordinary `c` argument, plus one synthesized accessor per bound
    // field (`name`, `discount` — 2 fields on this bound).
    assert_eq!(args.len(), 3, "expected 1 ordinary arg + 2 accessor args, got {:?}", args);
    for a in &args[1..] {
        assert!(matches!(&a.kind, HirExprKind::Lambda { params, ret_hint, .. } if params.is_empty() && matches!(ret_hint, Ty::Var(_))),
            "expected a zero-arg, erased-return accessor lambda, got {:?}", a.kind);
    }
}

#[test]
fn row_bound_call_site_accessor_reads_the_concrete_receivers_real_field() {
    // The accessor's own body must resolve the field against the *concrete*
    // receiver type (Coupon) at this call site, not the erased Ty::Var the
    // generic function's own body sees — confirming the whole point of this
    // item: the accessor closure carries the byte-offset knowledge the
    // generic function body itself can never have.
    let m = lower(
        "module A\ntype Coupon = { name: Text, discount: Int }\n\
         fn getName<R: { name: Text }>(record: R): Text = record.name\n\
         fn f(c: Coupon): Text = getName(c)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected f's body to be a Call");
    };
    let HirExprKind::Lambda { body, .. } = &args[1].kind else {
        panic!("expected the 2nd arg to be the synthesized accessor lambda, got {:?}", args[1].kind);
    };
    assert_eq!(body.ty, Ty::Text, "the accessor's own body must resolve record.name's real type (Text), got {:?}", body.ty);
    assert!(matches!(&body.kind, HirExprKind::Field { field, .. } if field == "name"),
        "expected the accessor body to be an ordinary Field read on the concrete receiver, got {:?}", body.kind);
}

#[test]
fn non_bound_field_access_on_a_row_bound_param_is_unaffected() {
    // A field the bound never declared falls through to the ordinary
    // (pre-existing, separately-tracked) `resolve_field_ty` path unchanged —
    // confirms the accessor rewrite only fires for genuinely bound fields,
    // not any field access on a row-bound-typed local whatsoever.
    let m = lower("module A\nfn getDiscount<R: { name: Text }>(record: R): Int = record.discount");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "getDiscount" => Some(f),
        _ => None,
    }).expect("expected fn `getDiscount`");
    let body = f.body.as_ref().unwrap();
    assert!(matches!(&body.kind, HirExprKind::Field { field, .. } if field == "discount"),
        "a non-bound field access must still lower as an ordinary Field node, got {:?}", body.kind);
}

#[test]
fn row_bound_on_an_impl_method_also_gains_an_accessor_param() {
    let m = lower(
        "module A\ntype Namer = { tag: Text }\n\
         impl Namer {\n  fn greet<R: { name: Text }>(self, record: R): Text = self.tag\n}\n\
         fn f(): Unit = {}");
    let greet = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "Namer.greet" => Some(f),
        _ => None,
    }).expect("expected fn `Namer.greet`");
    // `self` (index 0), `record` (index 1), plus the synthesized accessor.
    assert_eq!(greet.params.len(), 3, "expected self + record + one accessor param, got {:?}", greet.params);
    assert!(matches!(&greet.params[2].ty, Ty::Fn { params, ret } if params.is_empty() && matches!(ret.as_ref(), Ty::Var(_))));
}

// ------------------------------------------------------------------ //
// Trait-bound method dispatch via call-site accessor closures — BACKLOG
// item 309 (the trait-bound sibling of item 200's own row-bound mechanism
// just above — same architecture, keyed by trait method instead of row
// field). Confirmed live before this fix: `value.toJson()` inside
// `fn serialize<T: Serializable>(value: T): Text = value.toJson()` compiled
// and typechecked (item 309's own typeck-side fix, a separate soundness
// gap) but failed at the C stage — `member reference base type 'void *' is
// not a structure or union`.
// ------------------------------------------------------------------ //

#[test]
fn trait_bound_fn_gains_an_extra_accessor_param() {
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         fn serialize<T: Serializable>(value: T): Text = value.toJson()");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "serialize" => Some(f),
        _ => None,
    }).expect("expected fn `serialize`");
    assert_eq!(f.params.len(), 2, "expected the ordinary `value` param plus one synthesized accessor param, got {:?}", f.params);
    assert!(matches!(&f.params[1].ty, Ty::Fn { params, ret } if params.is_empty() && matches!(ret.as_ref(), Ty::Var(_))),
        "expected the accessor param's type to be an erased zero-arg closure (toJson takes no extra params beyond self), got {:?}", f.params[1].ty);
}

#[test]
fn trait_bound_method_call_lowers_to_a_call_on_the_accessor() {
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         fn serialize<T: Serializable>(value: T): Text = value.toJson()");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "serialize" => Some(f),
        _ => None,
    }).expect("expected fn `serialize`");
    let body = f.body.as_ref().expect("expected a body");
    assert_eq!(body.ty, Ty::Text, "value.toJson()'s own type must resolve to Text (the trait's declared return type), not stay erased, got {:?}", body.ty);
    let HirExprKind::Call { func, args } = &body.kind else {
        panic!("expected value.toJson() to lower to a Call on the accessor, got {:?}", body.kind);
    };
    assert!(args.is_empty(), "toJson takes no extra args beyond the receiver, already consumed by the accessor closure itself");
    let accessor_local = f.params[1].local;
    assert!(matches!(&func.kind, HirExprKind::Local(id) if *id == accessor_local),
        "expected the call's own callee to be the synthesized accessor param, got {:?}", func.kind);
}

#[test]
fn trait_bound_call_site_synthesizes_one_accessor_arg_per_bound_method() {
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         trait Loggable { fn describe(self): Text }\n\
         type Widget = { id: Int }\n\
         fn report<T: Serializable + Loggable>(value: T): Text = value.toJson()\n\
         fn f(w: Widget): Text = report(w)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected f's body to be a Call");
    };
    // The ordinary `w` argument, plus one synthesized accessor per bound
    // method (`toJson`, `describe` — 2 methods across the 2 combined bounds).
    assert_eq!(args.len(), 3, "expected 1 ordinary arg + 2 accessor args, got {:?}", args);
    for a in &args[1..] {
        assert!(matches!(&a.kind, HirExprKind::Lambda { params, ret_hint, .. } if params.is_empty() && matches!(ret_hint, Ty::Var(_))),
            "expected a zero-arg, erased-return accessor lambda, got {:?}", a.kind);
    }
}

#[test]
fn trait_bound_call_site_accessor_dispatches_through_the_concrete_impl() {
    // The accessor's own body must resolve to the *concrete* receiver's own
    // real qualified call (`Widget.toJson`) at this call site, not stay
    // erased the way the generic function's own body sees it — confirming
    // the whole point of this item: the accessor closure carries the
    // concrete-dispatch knowledge the generic function body itself can
    // never have (true type erasure — one compiled C body per function,
    // never monomorphized per call site).
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         type Widget = { id: Int }\n\
         impl Serializable for Widget { fn toJson(self): Text = \"widget\" }\n\
         fn serialize<T: Serializable>(value: T): Text = value.toJson()\n\
         fn f(w: Widget): Text = serialize(w)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let HirExprKind::Call { args, .. } = &f.body.as_ref().unwrap().kind else {
        panic!("expected f's body to be a Call");
    };
    let HirExprKind::Lambda { body, .. } = &args[1].kind else {
        panic!("expected the 2nd arg to be the synthesized accessor lambda, got {:?}", args[1].kind);
    };
    assert_eq!(body.ty, Ty::Text, "the accessor's own body must resolve toJson's real return type (Text), got {:?}", body.ty);
    let HirExprKind::Call { func, .. } = &body.kind else {
        panic!("expected the accessor body to be an ordinary Call on the concrete impl method, got {:?}", body.kind);
    };
    assert!(matches!(&func.kind, HirExprKind::Global(name) if name == "Widget.toJson"),
        "expected the accessor body to call the concrete `Widget.toJson`, got {:?}", func.kind);
}

#[test]
fn trait_bound_method_with_extra_args_forwards_them_through_the_accessor() {
    // `compareTo` takes an extra `other: Self` param beyond the receiver —
    // the accessor closure must take one real (synthetic) param per extra
    // arg, not stay zero-arg the way a row-bound field read always is.
    // Both `a` and `b` are bound by `T`, so — mirroring
    // `collect_row_bound_fields`'s identical per-*parameter-position*
    // behavior (not per-type-param) — each gets its *own* independent
    // accessor (2 total), even though this particular body only calls
    // through the one keyed to `a`, the actual receiver; the `b`-keyed one
    // sits unused, exactly as harmless as an unused row-bound accessor
    // would be in the same shape.
    let m = lower(
        "module A\ntrait Comparable { fn compareTo(self, other: Self): Int }\n\
         fn maxOf<T: Comparable>(a: T, b: T): Int = a.compareTo(b)");
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "maxOf" => Some(f),
        _ => None,
    }).expect("expected fn `maxOf`");
    // `a`, `b`, plus one accessor param each for `a.compareTo`/`b.compareTo`.
    assert_eq!(f.params.len(), 4, "expected a + b + one accessor param per bound parameter position, got {:?}", f.params);
    for accessor in &f.params[2..] {
        assert!(matches!(&accessor.ty, Ty::Fn { params, ret } if params.len() == 1 && matches!(params[0], Ty::Var(_)) && matches!(ret.as_ref(), Ty::Var(_))),
            "expected each accessor's own declared shape to take exactly 1 erased extra param (for `other`), got {:?}", accessor.ty);
    }
    let body = f.body.as_ref().unwrap();
    let HirExprKind::Call { func, args } = &body.kind else {
        panic!("expected a.compareTo(b) to lower to a Call on an accessor, got {:?}", body.kind);
    };
    assert_eq!(args.len(), 1, "expected exactly 1 forwarded arg (b) to the accessor, got {:?}", args);
    // The receiver is `a` (param index 0), so the accessor actually called
    // must be the *first* one registered (`f.params[2]`), not `b`'s.
    assert!(matches!(&func.kind, HirExprKind::Local(id) if *id == f.params[2].local),
        "expected the call to go through `a`'s own accessor (params[2]), got {:?}", func.kind);
}

#[test]
fn non_bound_method_call_on_a_trait_bound_param_is_unaffected() {
    // A method name the bound trait never declares falls through to the
    // ordinary (pre-existing, separately-tracked) UFCS/field-access path
    // unchanged — confirms the accessor rewrite only fires for genuinely
    // bound methods, matching item 200's own `non_bound_field_access...`
    // regression guard. `typo` isn't declared anywhere, so this must not
    // panic or silently produce a bogus accessor Call.
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         fn serialize<T: Serializable>(value: T): Text = value.toJson()\n\
         fn other<T: Serializable>(value: T): Text = value.toJson()");
    // Both functions still lower successfully and each keeps exactly its
    // own single accessor param (for `toJson`) — no cross-contamination
    // between two different trait-bounded functions' own accessor tables.
    for name in ["serialize", "other"] {
        let f = m.items.iter().find_map(|it| match it {
            HirItem::Fn(f) if f.name == name => Some(f),
            _ => None,
        }).unwrap_or_else(|| panic!("expected fn `{name}`"));
        assert_eq!(f.params.len(), 2, "expected `value` + one accessor param for fn `{name}`, got {:?}", f.params);
    }
}

#[test]
fn trait_bound_on_an_impl_method_also_gains_an_accessor_param() {
    let m = lower(
        "module A\ntrait Serializable { fn toJson(self): Text }\n\
         type Logger = { tag: Text }\n\
         impl Logger {\n  fn log<T: Serializable>(self, value: T): Text = value.toJson()\n}\n\
         fn f(): Unit = {}");
    let log = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "Logger.log" => Some(f),
        _ => None,
    }).expect("expected fn `Logger.log`");
    // `self` (index 0), `value` (index 1), plus the synthesized accessor.
    assert_eq!(log.params.len(), 3, "expected self + value + one accessor param, got {:?}", log.params);
    assert!(matches!(&log.params[2].ty, Ty::Fn { params, ret } if params.is_empty() && matches!(ret.as_ref(), Ty::Var(_))));
}

// ------------------------------------------------------------------ //
// BACKLOG item 262 — positional record construction (spec §8.4). Mirrors
// a sum-type variant constructor's own HIR-level return-type resolution.
// ------------------------------------------------------------------ //

#[test]
fn positional_record_call_resolves_its_real_return_type_not_ty_error() {
    let m = lower(
        "module A\ntype Money = { amount: Int, currency: Text }\n\
         fn f(): Money = Money(1000, \"USD\")"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Named { name: "Money".into(), args: vec![] },
        "positional record construction must resolve the record's own type, not Ty::Error, got {:?}", body.ty);
    assert!(matches!(&body.kind, HirExprKind::Call { .. }),
        "positional record construction lowers as an ordinary Call, got {:?}", body.kind);
}

// ------------------------------------------------------------------ //
// BACKLOG item 281 — `type X = Y` (spec §3.3) must expand to its real
// target type at the HIR level too, not just in `crates/typeck` — HIR has
// its own, entirely separate AST→Ty conversion (`ast_ty_to_ty_with_params`)
// that previously kept an alias as an opaque `Ty::Named` naming a C type
// codegen never generates a definition for.
// ------------------------------------------------------------------ //

#[test]
fn fn_param_declared_as_an_alias_gets_the_real_underlying_ty() {
    let m = lower(
        "module A\ntype UserId = Text\nfn greet(id: UserId): Text = id"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "greet" => Some(f),
        _ => None,
    }).expect("expected fn `greet`");
    assert_eq!(f.params[0].ty, Ty::Text,
        "an alias-typed param must resolve to its real target Ty, not an opaque Named, got {:?}", f.params[0].ty);
}

#[test]
fn fn_return_declared_as_an_alias_gets_the_real_underlying_ty() {
    let m = lower(
        "module A\ntype UserId = Text\nfn newId(): UserId = \"abc\""
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "newId" => Some(f),
        _ => None,
    }).expect("expected fn `newId`");
    assert_eq!(f.ret_ty, Ty::Text,
        "an alias-typed return must resolve to its real target Ty, not an opaque Named, got {:?}", f.ret_ty);
}

#[test]
fn generic_alias_param_expands_with_substituted_type_args() {
    // The spec's own `type Callback<T> = T => Unit` example, instantiated
    // as `Callback<Int>` — must resolve to a real `Ty::Fn` over `Ty::Int`,
    // not an opaque `Ty::Named { name: "Callback", .. }`.
    let m = lower(
        "module A\ntype Callback<T> = T => Unit\nfn runWith(cb: Callback<Int>): Unit = cb(42)"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "runWith" => Some(f),
        _ => None,
    }).expect("expected fn `runWith`");
    assert_eq!(f.params[0].ty, Ty::Fn { params: vec![Ty::Int], ret: Box::new(Ty::Unit) },
        "Callback<Int> must expand to (Int) => Unit, got {:?}", f.params[0].ty);
}

#[test]
fn record_field_declared_as_an_alias_gets_the_real_underlying_ty() {
    let m = lower(
        "module A\ntype UserId = Text\ntype User = { id: UserId, name: Text }"
    );
    let field_types = m.record_field_types.get("User").expect("expected User's field types registered");
    assert_eq!(field_types[0], Ty::Text,
        "an alias-typed record field must resolve to its real target Ty, got {:?}", field_types[0]);
}

#[test]
fn self_referential_type_alias_does_not_hang_hir_lowering() {
    // `type A = A` must not stack-overflow HIR lowering — bounded by
    // MAX_ALIAS_EXPANSION_DEPTH, degrading to a harmless opaque type.
    let _ = try_lower("module A\ntype A = A\nfn f(x: A): A = x");
}

// ------------------------------------------------------------------ //
// BACKLOG item 284 — piping into a generic stdlib call that returns a
// struct must recover the same real return type an equivalent non-piped
// call already does, not `Ty::Error` (which miscompiled to invalid C:
// `member reference base type 'int64_t' is not a structure or union`).
// ------------------------------------------------------------------ //

#[test]
fn pipe_into_struct_returning_generic_call_resolves_the_real_element_type() {
    let m = lower(
        "module A\ntype Pt = { x: Int, y: Int }\n\
         fn f(): List<Pt> = [1, 2, 3] |> List.map((n) => Pt { x: n, y: n * 2 })"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::List(Box::new(Ty::Named { name: "Pt".into(), args: vec![] })),
        "piped List.map over a struct-returning lambda must resolve List<Pt>, not Ty::Error, got {:?}", body.ty);
}

#[test]
fn pipe_into_struct_returning_call_matches_the_non_piped_equivalent() {
    // The exact same call written with and without the pipe must resolve
    // to the identical type — the pipe form must not lose any of the
    // return-type recovery the ordinary `Expr::App` arm already does.
    let piped = lower(
        "module A\ntype Pt = { x: Int, y: Int }\n\
         fn f(): List<Pt> = [1, 2, 3] |> List.map((n) => Pt { x: n, y: n * 2 })"
    );
    let plain = lower(
        "module A\ntype Pt = { x: Int, y: Int }\n\
         fn f(): List<Pt> = List.map([1, 2, 3], (n) => Pt { x: n, y: n * 2 })"
    );
    let ty_of = |m: &crate::HirModule| m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => f.body.as_ref().map(|b| b.ty.clone()),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(ty_of(&piped), ty_of(&plain), "piped and non-piped forms of the same call must resolve identically");
}

#[test]
fn pipe_with_no_call_args_into_struct_returning_fn_still_resolves() {
    // `a |> f` (bare pipe, no explicit call args) must also recover the
    // real return type, not just the `a |> f(...)` form.
    let m = lower(
        "module A\ntype Pt = { x: Int, y: Int }\n\
         fn makePt(n: Int): Pt = Pt { x: n, y: n }\n\
         fn f(): Pt = 5 |> makePt"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Named { name: "Pt".into(), args: vec![] },
        "bare `a |> f` must resolve f's real return type, not Ty::Error, got {:?}", body.ty);
}

// ------------------------------------------------------------------ //
// BACKLOG item 287 — `parallel { ... }`'s destructured tuple result must
// carry each task's real type, not `Ty::Error` — a real runtime segfault
// (an f-string interpolating a destructured element read the boxed value
// as the wrong C type), not just a type-checking gap. Mirrors the exact
// fix `Expr::WithTimeout` (item 122) already got for its own result, never
// carried over to `parallel` until now.
// ------------------------------------------------------------------ //

#[test]
fn parallel_tuple_result_resolves_real_element_types_not_ty_error() {
    let m = lower(
        "module A\n\
         fn getCount(): Int = 42\n\
         fn getLabel(): Text = \"hi\"\n\
         fn f(): (Int, Text) = parallel { getCount(), getLabel() }"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Tuple(vec![Ty::Int, Ty::Text]),
        "parallel {{ }}'s result must resolve (Int, Text), not Ty::Error, got {:?}", body.ty);
    // The block's own tail (the Tuple node itself, and each awaited
    // element inside it) must carry the same real types, not just the
    // outer wrapper — that's what a `val (a, b) = ...` destructure and an
    // f-string interpolating one of the bound names actually consult.
    if let HirExprKind::Block { tail, .. } = &body.kind {
        assert_eq!(tail.ty, Ty::Tuple(vec![Ty::Int, Ty::Text]));
        if let HirExprKind::Tuple(elems) = &tail.kind {
            assert_eq!(elems[0].ty, Ty::Int, "first awaited element must be Int, got {:?}", elems[0].ty);
            assert_eq!(elems[1].ty, Ty::Text, "second awaited element must be Text, got {:?}", elems[1].ty);
        } else {
            panic!("expected a Tuple tail, got {:?}", tail.kind);
        }
    } else {
        panic!("expected parallel to lower to a Block, got {:?}", body.kind);
    }
}

#[test]
fn parallel_with_timeout_still_resolves_real_element_types() {
    let m = lower(
        "module A\n\
         fn getCount(): Int = 42\n\
         fn getLabel(): Text = \"hi\"\n\
         fn f(): (Int, Text) = parallel(timeout: Duration.seconds(1)) { getCount(), getLabel() }"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    assert_eq!(body.ty, Ty::Tuple(vec![Ty::Int, Ty::Text]),
        "parallel(timeout: ...) {{ }}'s result must also resolve (Int, Text), not Ty::Error, got {:?}", body.ty);
}

// ------------------------------------------------------------------ //
// BACKLOG item 282 — `List.sumBy`/`sortBy`/`minBy`/`maxBy`'s key/numeric
// projection restriction previously excluded `Decimal`, breaking spec
// §8.5's own flagship Aggregate Roots example (summing `Money`-typed
// per-item fields, which wrap `Decimal`). `Decimal`'s `+`/`<`/`>` already
// route through real runtime functions (item 249), so the "struct
// operands don't compile with a bare C operator" reasoning this
// restriction was built on no longer applies.
// ------------------------------------------------------------------ //

#[test]
fn sumby_struct_decimal_field_key_lowers_ok_and_resolves_decimal() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Item = { name: Text, price: Decimal }\n\
         fn f(items: List<Item>): Decimal = List.sumBy(items, (i) => i.price)"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Decimal(None),
        "List.sumBy over a Decimal field must resolve Decimal, got {:?}", f.body.as_ref().unwrap().ty);
}

#[test]
fn sortby_struct_decimal_field_key_lowers_ok() {
    // Must not be rejected as E0601 — Decimal is now a supported key type.
    lower(
        "module A\nimport Stdlib.Collections.{ List }\ntype Item = { name: Text, price: Decimal }\n\
         fn f(items: List<Item>): List<Item> = List.sortBy(items, (i) => i.price)"
    );
}

// ------------------------------------------------------------------ //
// BACKLOG item 311 — `List.sumBy` alone also accepts a non-generic struct
// type declaring both `{Type}.add(a, b): {Type}` and `{Type}.zero(): {Type}`
// — the actual spec §8.5 shape (`items.sumBy(.lineTotal)`, `lineTotal:
// Money`, a `@valueObject` wrapping `Decimal`), unlike item 282's own
// `Decimal`-field verification just above, which sidesteps this case.
// This is the field-access shape typeck's own check can't resolve (see
// `sortby_struct_float_field_key_does_not_false_positive` in
// `crates/typeck`), so it's validated here instead, after the per-call-site
// lambda hint runs.
// ------------------------------------------------------------------ //

#[test]
fn sumby_struct_field_key_with_add_and_zero_lowers_ok() {
    let m = lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         type Money = { amount: Int }\n\
         impl Money {\n  fn add(a: Money, b: Money): Money = Money { amount: a.amount + b.amount }\n  fn zero(): Money = Money { amount: 0 }\n}\n\
         type Item = { lineTotal: Money }\n\
         fn f(items: List<Item>): Money = List.sumBy(items, (i) => i.lineTotal)"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    assert_eq!(f.body.as_ref().unwrap().ty, Ty::Named { name: "Money".into(), args: vec![] },
        "List.sumBy over a Money field must resolve Money, got {:?}", f.body.as_ref().unwrap().ty);
}

#[test]
fn sumby_struct_field_key_without_add_zero_is_e0601() {
    let err = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         type Box = { value: Int }\n\
         type Item = { b: Box }\n\
         fn f(items: List<Item>): Box = List.sumBy(items, (i) => i.b)"
    ).expect_err("a struct field key with no add/zero must be rejected");
    assert!(err.iter().any(|e| matches!(&e.kind, crate::LowerErrorKind::Unsupported(msg) if msg.contains("Box"))),
        "expected an Unsupported error naming Box, got {err:?}");
}

#[test]
fn sortby_struct_field_key_with_add_zero_is_still_e0601() {
    // Regression guard: `sortBy`/`minBy`/`maxBy` stay exactly as
    // restrictive as before, mirroring the identical `crates/typeck` guard.
    let err = try_lower(
        "module A\nimport Stdlib.Collections.{ List }\n\
         type Money = { amount: Int }\n\
         impl Money {\n  fn add(a: Money, b: Money): Money = Money { amount: a.amount + b.amount }\n  fn zero(): Money = Money { amount: 0 }\n}\n\
         type Item = { lineTotal: Money }\n\
         fn f(items: List<Item>): List<Item> = List.sortBy(items, (i) => i.lineTotal)"
    ).expect_err("sortBy must not accept a struct key even with add/zero declared");
    assert!(err.iter().any(|e| matches!(&e.kind, crate::LowerErrorKind::Unsupported(msg) if msg.contains("Money"))),
        "expected an Unsupported error naming Money, got {err:?}");
}

// ------------------------------------------------------------------ //
// BACKLOG item 304 — a bare `Type.method` reference (not called) must
// resolve a real `Ty::Fn`, not `Ty::Error` — the previous unconditional
// `Ty::Error` here made `wrap_named_fn_as_closure` (crates/mir) build a
// wrapper with the wrong C signature, miscompiling to invalid C.
// ------------------------------------------------------------------ //

fn find_call_arg<'a>(body: &'a crate::HirExpr, i: usize) -> &'a crate::HirExpr {
    match &body.kind {
        HirExprKind::Call { args, .. } => &args[i],
        other => panic!("expected a Call, got {:?}", other),
    }
}

#[test]
fn bare_non_generic_impl_method_reference_resolves_a_real_fn_type() {
    let m = lower(
        "module A\ntype Box = { n: Int }\nimpl Box { fn double(x: Int): Int = x * 2 }\n\
         fn apply(f: (Int) => Int, x: Int): Int = f(x)\n\
         fn f(): Int = apply(Box.double, 5)"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let arg = find_call_arg(f.body.as_ref().unwrap(), 0);
    assert_eq!(arg.ty, Ty::Fn { params: vec![Ty::Int], ret: Box::new(Ty::Int) },
        "bare Box.double reference must resolve its real (Int) => Int type, not Ty::Error, got {:?}", arg.ty);
}

#[test]
fn bare_generic_impl_method_reference_resolves_an_erased_fn_type() {
    // The item's own exact repro shape: a generic impl method (Secret.wrap)
    // referenced bare, not called — must resolve to the same erased
    // Ty::Var(0) signature an ordinary generic top-level function already
    // gets when referenced bare, not Ty::Error.
    let m = lower(
        "module A\ntype Secret<T> = priv Secret(T)\n\
         impl<T> Secret { fn wrap(v: T): Secret<T> = Secret(v) }\n\
         fn apply(f: (Int) => Secret<Int>, x: Int): Secret<Int> = f(x)\n\
         fn f(): Secret<Int> = apply(Secret.wrap, 5)"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let arg = find_call_arg(f.body.as_ref().unwrap(), 0);
    assert!(!matches!(arg.ty, Ty::Error), "bare Secret.wrap reference must not resolve to Ty::Error, got {:?}", arg.ty);
    assert!(matches!(&arg.ty, Ty::Fn { .. }), "expected a Ty::Fn, got {:?}", arg.ty);
}

#[test]
fn var_bound_struct_field_access_resolves_real_type_not_ty_error() {
    // Stmt::Var's own lowering (unlike Stmt::Val's identical-shape arm just
    // above it) never called `cx.local_types.insert` for the new local, so
    // any later reference to a `var`-bound record fell back to
    // `cx.local_types.get(&local).unwrap_or(Ty::Error)` for its own type,
    // and a field access on that `Ty::Error` base bailed out to `Ty::Error`
    // too (`resolve_field_ty`'s `let Ty::Named { .. } = base_ty else {
    // return (Ty::Error, false) }`) - which codegen's `ty_to_c.rs` maps to
    // `int64_t`, a hard compile error for a struct-typed field ("assigning
    // to 'int64_t' from incompatible type 'X'"), confirmed live with a
    // minimal repro before this fix. A primitive-typed field (Bool/Int)
    // never surfaced this, since C silently allows assigning those to
    // int64_t - only a struct-typed field access exposes the wrong type.
    // The exact same field access on a `val`-bound record already worked
    // correctly; this test's `var` is the one case that didn't.
    let m = lower(
        "module A\n\
         type Inner = { line: Int }\n\
         type Outer = { found: Bool, name: Inner }\n\
         fn f(): Int = {\n\
         \x20 var result = Outer { found: false, name: Inner { line: 42 } }\n\
         \x20 result.name.line\n\
         }"
    );
    let f = m.items.iter().find_map(|it| match it {
        HirItem::Fn(f) if f.name == "f" => Some(f),
        _ => None,
    }).expect("expected fn `f`");
    let body = f.body.as_ref().unwrap();
    // (The wrapping Block's own `.ty` is a separate, unrelated quirk -
    // `lower_block` hardcodes it to `Ty::Error` whenever any statement
    // precedes the tail, regardless of this fix; nothing downstream reads
    // it, only `tail.ty` matters, same convention every other multi-
    // statement test in this file follows.)
    let HirExprKind::Block { tail, .. } = &body.kind else {
        panic!("expected a Block, got {:?}", body.kind);
    };
    assert_eq!(tail.ty, Ty::Int, "block tail must resolve to Int, not Ty::Error, got {:?}", tail.ty);
    let HirExprKind::Field { base: inner_base, field, .. } = &tail.kind else {
        panic!("expected the tail to be a Field access, got {:?}", tail.kind);
    };
    assert_eq!(field, "line");
    // The intermediate `result.name` access (the base of the outer `.line`
    // read) must resolve to the real `Inner` record type, not `Ty::Error` -
    // this is the field access that was silently corrupted before the fix.
    assert!(!matches!(inner_base.ty, Ty::Error),
        "the intermediate result.name access must not resolve to Ty::Error, got {:?}", inner_base.ty);
    assert!(matches!(&inner_base.ty, Ty::Named { name, .. } if name == "Inner"),
        "expected the intermediate result.name access to resolve to Inner, got {:?}", inner_base.ty);
}

// ------------------------------------------------------------------ //
// `guard cond else e` — BACKLOG item 342
// ------------------------------------------------------------------ //

fn contains_return(e: &HirExpr) -> bool {
    match &e.kind {
        HirExprKind::Return(_) => true,
        HirExprKind::If { cond, then_expr, else_expr } =>
            contains_return(cond) || contains_return(then_expr) || contains_return(else_expr),
        HirExprKind::Block { stmts, tail } =>
            stmts.iter().any(|s| matches!(s, HirStmt::Expr(x) if contains_return(x))) || contains_return(tail),
        _ => false,
    }
}

#[test]
fn guard_lowers_to_a_conditional_early_return_not_a_discarded_expression() {
    // This used to lower to `if !cond { else_expr }; unit` — the else value
    // was evaluated and thrown away, so every `guard` was a silent no-op.
    let m = lower("module A\nfn f(n: Int): Int = {\n guard n > 0 else -1\n n * 2\n}");
    let f = m.items.iter().find_map(|it| match it { HirItem::Fn(f) if f.name == "f" => Some(f), _ => None }).unwrap();
    assert!(contains_return(f.body.as_ref().unwrap()), "expected a HirExprKind::Return inside the guard's lowering");
}

#[test]
fn a_function_without_guard_lowers_with_no_early_return() {
    let m = lower("module A\nfn f(n: Int): Int = {\n val x = n * 2\n x\n}");
    let f = m.items.iter().find_map(|it| match it { HirItem::Fn(f) if f.name == "f" => Some(f), _ => None }).unwrap();
    assert!(!contains_return(f.body.as_ref().unwrap()));
}
