use certo_parser::parse;
use certo_resolve::resolve;
use crate::infer_decl::check_module;
use crate::error::TypeErrorKind;

fn check(src: &str) -> Result<(), Vec<crate::error::TypeError>> {
    let module = parse(src).expect("parse error");
    resolve(&module).expect("resolve error");
    check_module(&module)
}

fn check_err(src: &str) -> Vec<crate::error::TypeError> {
    let module = parse(src).expect("parse error");
    let _ = resolve(&module); // may have errors for unknown names
    check_module(&module).unwrap_err()
}

fn first_error_kind(src: &str) -> TypeErrorKind {
    check_err(src).into_iter().next().expect("expected at least one error").kind
}

fn has_kind_typeck(errs: &[crate::error::TypeError], f: impl Fn(&TypeErrorKind) -> bool) -> bool {
    errs.iter().any(|e| f(&e.kind))
}

/// BACKLOG item 230 — `check_module_warnings` runs directly against the
/// un-expanded AST (no typeck/resolve dependency at all, mirroring
/// `check_constraint_scope`'s own design), so this helper just parses.
fn warnings(src: &str) -> Vec<crate::error::Warning> {
    let module = parse(src).expect("parse error");
    crate::infer_decl::check_module_warnings(&module)
}

// ------------------------------------------------------------------ //
// Happy-path tests
// ------------------------------------------------------------------ //

#[test]
fn int_literal() {
    check("module A\nval x: Int = 42").unwrap();
}

#[test]
fn bool_literal() {
    check("module A\nval x: Bool = true").unwrap();
}

#[test]
fn text_literal() {
    check("module A\nval x: Text = \"hello\"").unwrap();
}

#[test]
fn simple_fn() {
    check(
        "module A
fn add(a: Int, b: Int): Int = a + b"
    ).unwrap();
}

#[test]
fn identity_fn() {
    // Polymorphic identity — return type annotation needed for now
    check(
        "module A
fn identity(x: Int): Int = x"
    ).unwrap();
}

#[test]
fn if_expr() {
    check(
        "module A
fn choose(cond: Bool, a: Int, b: Int): Int = if cond then a else b"
    ).unwrap();
}

#[test]
fn list_literal() {
    check("module A\nval xs: List<Int> = [1, 2, 3]").unwrap();
}

#[test]
fn tuple_literal() {
    check("module A\nval t: (Int, Bool) = (1, true)").unwrap();
}

#[test]
fn val_block() {
    check(
        "module A
fn f(): Int = {
    val x: Int = 1
    val y: Int = 2
    x + y
}"
    ).unwrap();
}

// ------------------------------------------------------------------ //
// Error tests
// ------------------------------------------------------------------ //

#[test]
fn type_mismatch() {
    let errs = check_err("module A\nval x: Bool = 42");
    assert!(!errs.is_empty(), "expected a type error");
    assert!(errs[0].message().contains("E0200"), "expected E0200, got: {}", errs[0].message());
}

#[test]
fn arity_mismatch_is_mismatch_error() {
    // Calling a 2-arg fn with 1 arg should produce a unification error
    let errs = check_err(
        "module A
fn add(a: Int, b: Int): Int = a + b
val z: Int = add(1)"
    );
    assert!(!errs.is_empty(), "expected a type error");
}

#[test]
fn recursive_fn_needs_annotation() {
    let errs = check_err(
        "module A
fn fact(n: Int) = if n == 0 then 1 else n * fact(n - 1)"
    );
    assert!(!errs.is_empty(), "expected E0203");
    assert!(errs[0].message().contains("E0203"), "got: {}", errs[0].message());
}

// ------------------------------------------------------------------ //
// Phase 4 — validator feature type checking
// ------------------------------------------------------------------ //

// Temporal

#[test]
fn temporal_duration_days_ok() {
    check("module A\ntemporal GracePeriod = Duration.days(30)").unwrap();
}

#[test]
fn temporal_duration_hours_ok() {
    check("module A\ntemporal Window = Duration.hours(48)").unwrap();
}

#[test]
fn temporal_int_body_e0708() {
    let kind = first_error_kind("module A\ntemporal Bad = 42");
    assert!(matches!(kind, TypeErrorKind::TemporalNotDuration { .. }), "expected E0708, got {kind:?}");
}

#[test]
fn temporal_text_body_e0708() {
    let kind = first_error_kind("module A\ntemporal Bad = \"30 days\"");
    assert!(matches!(kind, TypeErrorKind::TemporalNotDuration { .. }), "expected E0708, got {kind:?}");
}

#[test]
fn temporal_name_has_duration_type() {
    // Once declared, a temporal name is usable as Duration in expressions.
    check("module A
temporal GracePeriod = Duration.days(30)
fn uses(d: Duration): Bool = true").unwrap();
}

// .age

#[test]
fn age_on_timestamp_field_ok() {
    // Validator with a Timestamp field — .age should typecheck and produce Duration.
    check("module A
type Invoice = { createdAt: Timestamp }
type IE = | X
temporal VoidWindow = Duration.days(30)
validator V for Invoice errors IE {
    rule r { require invoice.createdAt.age < VoidWindow else IE.X }
}").unwrap();
}

#[test]
fn age_on_int_field_e0709() {
    // .age on an Int field must be E0709.
    let kind = first_error_kind("module A
type Invoice = { total: Int }
type IE = | X
validator V for Invoice errors IE {
    rule r { require invoice.total.age < invoice.total else true }
}");
    assert!(matches!(kind, TypeErrorKind::AgeOnNonTimestamp { .. }), "expected E0709, got {kind:?}");
}

#[test]
fn age_in_standalone_expr_ok() {
    // .age on an unresolved base — optimistic: the type checker should not emit E0709.
    // (resolve will flag the unknown name; we only care the typeck itself doesn't add E0709)
    let module = parse("module A\nfn f(ts: Timestamp): Duration = ts.age").expect("parse error");
    let _ = resolve(&module);
    check_module(&module).unwrap();
}

// `expect(x).toBeXxx(...)` — BACKLOG item 165
//
// `certo_resolve::resolve()` (unlike the full `certo check` CLI pipeline,
// which seeds it with the whole stdlib via `resolve_seeded` — see that
// function's own doc comment) only knows a small hardcoded builtin list of
// its own and has no idea `expect` is a real stdlib function; a bare
// `expect(x)` call in one of these snippets would fail to *resolve*, not
// fail typeck, so these tests declare a same-shaped local `expect` inside
// the test source itself (an ordinary top-level `fn`, resolved by the
// normal hoisting pass, no different from declaring any other local
// helper a test needs) rather than reaching into `certo_resolve`'s builtin
// list for something that's really a stdlib registration concern — already
// verified for real via the actual compiled `certo.exe`, not just here.
const EXPECT_SHIM: &str = "fn expect<T>(x: T): T = x\n";

#[test]
fn to_be_matching_scalar_types_ok() {
    check(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(1 + 1).toBe(2)")).unwrap();
}

#[test]
fn to_be_mismatched_types_is_e0200() {
    // Same unify path plain `==` uses — a real type mismatch, not silently accepted.
    let kind = first_error_kind(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(1).toBe(\"x\")"));
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

#[test]
fn to_be_true_false_require_bool() {
    check(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(1 == 1).toBeTrue()")).unwrap();
    check(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(1 == 2).toBeFalse()")).unwrap();
}

#[test]
fn to_be_true_on_non_bool_is_e0200() {
    let kind = first_error_kind(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(5).toBeTrue()"));
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

#[test]
fn to_be_some_none_require_option() {
    check(&format!("module A\n{EXPECT_SHIM}fn f(o: Int?): Unit = expect(o).toBeSome()")).unwrap();
    check(&format!("module A\n{EXPECT_SHIM}fn f(o: Int?): Unit = expect(o).toBeNone()")).unwrap();
}

#[test]
fn to_be_some_on_non_option_is_e0200() {
    let kind = first_error_kind(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(5).toBeSome()"));
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

#[test]
fn to_be_ok_err_require_result() {
    check(&format!("module A\n{EXPECT_SHIM}fn f(r: Result<Int, Text>): Unit = expect(r).toBeOk()")).unwrap();
    check(&format!("module A\n{EXPECT_SHIM}fn f(r: Result<Int, Text>): Unit = expect(r).toBeErr()")).unwrap();
}

#[test]
fn to_be_ok_on_non_result_is_e0200() {
    let kind = first_error_kind(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = expect(5).toBeOk()"));
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

#[test]
fn expect_assertion_expression_type_is_unit() {
    // Whole `expect(x).toBe(y)` expression must itself be Unit — same as `assert`.
    check(&format!("module A\n{EXPECT_SHIM}fn f(): Unit = {{\n    val u: Unit = expect(1).toBe(1)\n}}")).unwrap();
}

#[test]
fn matcher_works_without_a_literal_expect_receiver() {
    // Not gated behind expect(...) specifically (see the parser's own test
    // of the same name) — typeck only cares about the base's type.
    check("module A\nfn f(): Unit = (3).toBe(3)").unwrap();
}

// `List.sortBy`/`minBy`/`maxBy`/`sumBy` key/numeric projection restriction
// (E0710, BACKLOG item 162b) — needs the *real* `List.sortBy`-shaped
// `Ty::Forall` registration (these are module-qualified stdlib functions,
// not covered by `seed_builtins`'s small hardcoded set), so these use
// `check_module_seeded` with a hand-registered env rather than the plain
// `check`/`check_err` helpers above. Registered directly against
// `certo_typeck`'s own `Ty`/`TypeEnv` (mirroring `crates/stdlib/src/
// seed.rs`'s real registration exactly) rather than depending on the
// `certo-stdlib` crate itself — that crate depends on `certo-typeck`, so
// pulling it in as a dev-dependency here builds a second, incompatible
// copy of this very crate (confirmed directly: `expected TypeEnv, found
// TypeEnv` from two distinct compilations of the same source).
fn seeded_check(src: &str) -> Result<(), Vec<crate::error::TypeError>> {
    use crate::Ty;
    let module = parse(src).expect("parse error");
    resolve(&module).expect("resolve error");
    let mut env = crate::TypeEnv::new();
    let mut counter = 0u32;
    let mut fresh = || { counter += 1; counter };

    let a = fresh();
    let k = fresh();
    let list_a = Ty::List(Box::new(Ty::Var(a)));
    let key_fn = Ty::Fn { params: vec![Ty::Var(a)], ret: Box::new(Ty::Var(k)) };
    env.define("List.sortBy", Ty::Forall {
        vars: vec![a, k],
        body: Box::new(Ty::Fn { params: vec![list_a.clone(), key_fn.clone()], ret: Box::new(list_a.clone()) }),
    });
    env.define_param_meta("List.sortBy", vec![("list".into(), false), ("key".into(), false)]);

    for name in ["minBy", "maxBy"] {
        let a = fresh();
        let k = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        let key_fn = Ty::Fn { params: vec![Ty::Var(a)], ret: Box::new(Ty::Var(k)) };
        let full = format!("List.{name}");
        env.define(full.as_str(), Ty::Forall {
            vars: vec![a, k],
            body: Box::new(Ty::Fn { params: vec![list_a, key_fn], ret: Box::new(Ty::Option(Box::new(Ty::Var(a)))) }),
        });
        env.define_param_meta(full.as_str(), vec![("list".into(), false), ("key".into(), false)]);
    }

    {
        let a = fresh();
        let n = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        let key_fn = Ty::Fn { params: vec![Ty::Var(a)], ret: Box::new(Ty::Var(n)) };
        env.define("List.sumBy", Ty::Forall {
            vars: vec![a, n],
            body: Box::new(Ty::Fn { params: vec![list_a, key_fn], ret: Box::new(Ty::Var(n)) }),
        });
        env.define_param_meta("List.sumBy", vec![("list".into(), false), ("key".into(), false)]);
    }

    // `List.slice` (BACKLOG item 171's own regression coverage) — real
    // signature per `crates/stdlib/src/seed.rs`: `List.slice<T>(list:
    // List<T>, from: Int, to: Int): List<T>`.
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.slice", Ty::Forall {
            vars: vec![a],
            body: Box::new(Ty::Fn { params: vec![list_a.clone(), Ty::Int, Ty::Int], ret: Box::new(list_a) }),
        });
        env.define_param_meta("List.slice", vec![("list".into(), false), ("from".into(), false), ("to".into(), false)]);
    }

    // `List.map`/`Option.isSome` (BACKLOG item 162's own dot-call
    // coverage) — real signatures per `crates/stdlib/src/seed.rs`.
    {
        let a = fresh();
        let b = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        let f_ty = Ty::Fn { params: vec![Ty::Var(a)], ret: Box::new(Ty::Var(b)) };
        env.define("List.map", Ty::Forall {
            vars: vec![a, b],
            body: Box::new(Ty::Fn { params: vec![list_a, f_ty], ret: Box::new(Ty::List(Box::new(Ty::Var(b)))) }),
        });
        env.define_param_meta("List.map", vec![("list".into(), false), ("f".into(), false)]);
    }
    {
        let a = fresh();
        env.define("Option.isSome", Ty::Forall {
            vars: vec![a],
            body: Box::new(Ty::Fn { params: vec![Ty::Option(Box::new(Ty::Var(a)))], ret: Box::new(Ty::Bool) }),
        });
        env.define_param_meta("Option.isSome", vec![("opt".into(), false)]);
    }

    crate::infer_decl::check_module_seeded(&module, env, counter)
}

fn seeded_first_error_kind(src: &str) -> TypeErrorKind {
    seeded_check(src).unwrap_err().into_iter().next().expect("expected at least one error").kind
}

#[test]
fn dot_call_ufcs_smoke_test() {
    seeded_check("module A\nfn f(xs: List<Int>): List<Int> = xs.map((x) => x)").unwrap();
}

#[test]
fn dot_call_on_option_receiver() {
    seeded_check("module A\nfn f(o: Option<Int>): Bool = o.isSome()").unwrap();
}

#[test]
fn dot_call_labeled_args_reorder_correctly() {
    // `List.slice(list, from, to)` called via dot-call with the trailing
    // two args out of order and labeled — the receiver is spliced in as
    // the implicit `list` slot, and the existing labeled-arg reordering
    // logic (unmodified, operating on the rewritten `args`) must still
    // place `from`/`to` correctly despite the extra leading argument.
    seeded_check("module A\nfn f(xs: List<Int>): List<Int> = xs.slice(to: 5, from: 1)").unwrap();
}

#[test]
fn dot_call_qualified_form_is_equivalent_to_dot_form() {
    // Same call, written both ways, must produce the same (successful) result —
    // proves the UFCS rewrite doesn't diverge from the pre-existing qualified path.
    seeded_check("module A\nfn f(xs: List<Int>): List<Int> = List.map(xs, (x) => x)").unwrap();
    seeded_check("module A\nfn f(xs: List<Int>): List<Int> = xs.map((x) => x)").unwrap();
}

#[test]
fn dot_call_on_unknown_method_is_still_e0205() {
    // A genuine field/method-name typo with no matching `"Order.frobnicate"`
    // registered anywhere must still fail as an ordinary unknown-field
    // access, not be silently swallowed by the UFCS rewrite attempt.
    let kind = seeded_first_error_kind(
        "module A\n\
         type Order = { total: Int }\n\
         fn f(o: Order): Int = o.frobnicate()"
    );
    assert!(matches!(kind, TypeErrorKind::UnknownField { .. }), "expected E0205, got {kind:?}");
}

#[test]
fn sortby_int_key_ok() {
    seeded_check("module A\nfn f(xs: List<Int>): List<Int> = List.sortBy(xs, (x) => x)").unwrap();
}

#[test]
fn sortby_float_key_ok() {
    seeded_check("module A\nfn f(xs: List<Float>): List<Float> = List.sortBy(xs, (x) => x)").unwrap();
}

#[test]
fn sortby_text_key_is_e0710() {
    let kind = seeded_first_error_kind(
        "module A\nfn f(xs: List<Text>): List<Text> = List.sortBy(xs, (x) => x)");
    assert!(matches!(kind, TypeErrorKind::UnsupportedKeyType { .. }), "expected E0710, got {kind:?}");
}

#[test]
fn minby_maxby_sumby_text_key_is_e0710() {
    for fname in ["minBy", "maxBy", "sumBy"] {
        let kind = seeded_first_error_kind(&format!(
            "module A\nfn f(xs: List<Text>): Unit = {{\n    val r = List.{fname}(xs, (x) => x)\n}}"));
        assert!(matches!(kind, TypeErrorKind::UnsupportedKeyType { .. }),
            "expected E0710 for List.{fname}, got {kind:?}");
    }
}

#[test]
fn sortby_struct_float_field_key_does_not_false_positive() {
    // Regression test: a key projection that does field access on a struct
    // element (`(p) => p.price`) can't be resolved by typeck's own
    // `Expr::Field` inference at all — it returns a disconnected fresh
    // `Ty::Var`, never `Ty::Float`, no matter how the rest of unification
    // resolves (`resolve_field_ty`'s `Ty::Var(_) => ctx.fresh()` branch
    // records no constraint linking it back to the field name). Confirmed
    // directly: before this test existed, this exact snippet was wrongly
    // rejected as E0710 ("resolved to `T`") the moment the check was fixed
    // to actually run at all (it originally never fired for *any*
    // module-qualified call — see the check site's own doc comment). The
    // fix is for typeck to stay silent on an unresolved `Ty::Var` here and
    // defer to HIR's own, later, correctly-hinted check instead (E0601,
    // see `certo-hir`'s tests).
    seeded_check(
        "module A\ntype Product = { name: Text, price: Float }\n\
         fn f(ps: List<Product>): List<Product> = List.sortBy(ps, (p) => p.price)"
    ).unwrap();
}

// Labeled-argument reordering for module-qualified stdlib calls (BACKLOG
// item 171). `List.slice(list, from, to)` parses its callee as
// `Expr::Field { expr: Path("List"), field: "slice" }`, not `Expr::Path` —
// before this fix, `Expr::App`'s `fn_name` derivation only ever handled
// `Expr::Path`, so it was silently `None` for every such call, and the
// labeled-arg-reordering branch (gated on `fn_name`) never ran — a
// positionally-mismatched labeled call like the one below type-checked as
// if the labels didn't exist, comparing `to`'s value against the `from`
// slot and vice versa. Confirmed directly via the real compiled `certo.exe`
// this was reachable in practice: `List.groupBy(key: ..., list: ...)`
// type-checked fine (no typeck error) but then *lowered* its arguments in
// written order regardless (a separate, HIR-side reordering gap — BACKLOG
// item 170's corrected fix, `crates/hir/src/tests.rs`).
#[test]
fn labeled_args_reorder_for_module_qualified_call() {
    // Written out of order and with mismatched types per position
    // (`to`/`from` swapped from their declared order) — only type-checks
    // if the labels are actually used to reorder them.
    seeded_check(
        "module A\nfn f(xs: List<Int>): List<Int> = List.slice(to: 3, list: xs, from: 1)"
    ).unwrap();
}

#[test]
fn labeled_args_wrong_type_after_reordering_is_still_caught() {
    // `from`/`to` must still be `Int` after reordering — this isn't just
    // "labels are ignored and it happens to type-check anyway".
    let kind = seeded_first_error_kind(
        "module A\nfn f(xs: List<Int>): List<Int> = List.slice(to: 3, list: xs, from: \"x\")");
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

// `computed name: Ty = expr` field access (BACKLOG item 143). No stdlib
// seeding needed here — these bodies only use plain comparisons, so the
// plain `check`/`check_err` helpers (backed by `seed_builtins`) suffice.

#[test]
fn computed_field_read_type_checks_ok() {
    check(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Bool = o.isPositive"
    ).unwrap();
}

#[test]
fn computed_field_read_wrong_expected_type_is_e0200() {
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Text = o.isPositive"
    );
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected E0200, got {kind:?}");
}

#[test]
fn setting_a_computed_field_in_a_literal_is_e0217() {
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(): Order = Order { total: 5, isPositive: true }"
    );
    assert!(matches!(kind, TypeErrorKind::ComputedFieldNotSettable { .. }), "expected E0217, got {kind:?}");
}

#[test]
fn setting_a_computed_field_via_with_is_e0217() {
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Order = o.with(isPositive: false)"
    );
    assert!(matches!(kind, TypeErrorKind::ComputedFieldNotSettable { .. }), "expected E0217, got {kind:?}");
}

#[test]
fn a_real_stored_field_of_the_same_shape_is_unaffected() {
    // Regression: the computed-field check must only fire for an actual
    // computed name, never suppress the ordinary field-set path.
    check(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(): Order = Order { total: 5 }"
    ).unwrap();
}

#[test]
fn accessing_a_genuinely_unknown_field_on_a_type_with_computed_fields_is_still_e0205() {
    // Regression: adding the computed-fields fallback lookup must not mask
    // a real "no such field at all" error.
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int, computed isPositive: Bool = total > 0 }\n\
         fn f(o: Order): Int = o.nonexistent"
    );
    assert!(matches!(kind, TypeErrorKind::UnknownField { .. }), "expected E0205, got {kind:?}");
}

// Unknown field names in construction (BACKLOG item 174) — a direct
// literal or `.with(...)` (item 151) setting a name that isn't a real
// field of the type at all (not even a `computed` one) must be a real
// E0205, not silently accepted.

#[test]
fn unknown_field_in_a_literal_is_e0205() {
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int }\n\
         fn f(): Order = Order { total: 5, totall: 10 }"
    );
    assert!(matches!(kind, TypeErrorKind::UnknownField { .. }), "expected E0205, got {kind:?}");
}

#[test]
fn unknown_field_via_with_is_e0205() {
    // Previously this didn't error at all — `.with(...)`'s own field-name
    // validation silently dropped any name it didn't recognize.
    let kind = first_error_kind(
        "module A\n\
         type Order = { total: Int }\n\
         fn f(o: Order): Order = o.with(totall: 10)"
    );
    assert!(matches!(kind, TypeErrorKind::UnknownField { .. }), "expected E0205, got {kind:?}");
}

#[test]
fn partial_with_update_of_real_fields_is_still_ok() {
    // Regression: the new check must not require every field to be
    // present — `.with(...)`'s whole point is a partial update.
    check(
        "module A\n\
         type Order = { total: Int, status: Text }\n\
         fn f(o: Order): Order = o.with(status: \"closed\")"
    ).unwrap();
}

#[test]
fn full_literal_construction_with_only_real_fields_is_still_ok() {
    check(
        "module A\n\
         type Order = { total: Int, status: Text }\n\
         fn f(): Order = Order { total: 5, status: \"open\" }"
    ).unwrap();
}

// Constraint

#[test]
fn constraint_name_is_bool() {
    check("module A
type OE = | X
constraint Active = status == active
validator V for Order errors OE {
    rule r { require Active else OE.X }
}").unwrap();
}

// Validator

#[test]
fn validator_require_true_ok() {
    check("module A
type OE = | X
validator V for Order errors OE {
    rule r { require true else OE.X }
}").unwrap();
}

#[test]
fn validator_require_entity_field_ok() {
    check("module A
type Order = { total: Int }
type OE = | X
validator V for Order errors OE {
    rule r { require order.total > 0 else OE.X }
}").unwrap();
}

#[test]
fn validator_require_non_bool_e0200() {
    let errs = check_err("module A
validator V for Order errors OE {
    rule r { require 42 else true }
}");
    assert!(!errs.is_empty(), "expected type error");
    assert!(errs[0].message().contains("E020"), "expected E020x, got: {}", errs[0].message());
}

// BACKLOG item 230 — W0100/W0101/W0102 validator warnings.
use crate::error::WarningKind;

#[test]
fn w0100_fires_for_an_overrides_relationship_with_no_priority_on_either_rule() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule base { require order.status == Draft else OE.X }
    rule bypass { overrides base require order.status == Approved else OE.X }
}");
    assert!(ws.iter().any(|w| matches!(&w.kind,
        WarningKind::AmbiguousOverridePriority { rule_name, overridden_name }
            if rule_name == "bypass" && overridden_name == "base")),
        "expected W0100, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0100_does_not_fire_when_the_overriding_rule_has_an_explicit_priority() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule base { require order.status == Draft else OE.X }
    rule bypass { overrides base priority 100 require order.status == Approved else OE.X }
}");
    assert!(!ws.iter().any(|w| matches!(&w.kind, WarningKind::AmbiguousOverridePriority { .. })),
        "did not expect W0100 when priority is explicit, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0100_does_not_fire_for_a_rule_with_no_overrides_at_all() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule r { require order.status == Draft else OE.X }
}");
    assert!(ws.is_empty(), "a rule with no `overrides` at all must never warn, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0100_skips_an_overrides_reference_to_a_rule_that_does_not_exist() {
    // A separate concern (E0702) — not this check's job to flag.
    let ws = warnings("module A
validator V for Order errors OE {
    rule r { overrides doesNotExist priority 100 require true else OE.X }
}");
    assert!(!ws.iter().any(|w| matches!(&w.kind, WarningKind::AmbiguousOverridePriority { .. })),
        "an overrides reference to a non-existent rule must be skipped, not warned about");
}

#[test]
fn w0101_fires_when_the_overriding_rules_own_condition_is_a_bare_true() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule base { require order.status == Draft else OE.X }
    rule bypass { overrides base priority 100 require true else OE.X }
}");
    assert!(ws.iter().any(|w| matches!(&w.kind,
        WarningKind::UnreachableOverriddenRule { overriding_name, overridden_name }
            if overriding_name == "bypass" && overridden_name == "base")),
        "expected W0101, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0101_does_not_fire_for_a_non_tautological_condition() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule base { require order.status == Draft else OE.X }
    rule bypass { overrides base priority 100 require order.status == Approved else OE.X }
}");
    assert!(!ws.iter().any(|w| matches!(&w.kind, WarningKind::UnreachableOverriddenRule { .. })),
        "a non-tautological override condition must not warn, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0102_fires_for_a_context_field_with_no_loaded_by() {
    let ws = warnings("module A
validator V for Order errors OE {
    context { note: Text }
    rule r { require true else OE.X }
}");
    assert!(ws.iter().any(|w| matches!(&w.kind,
        WarningKind::ContextFieldMissingLoadedBy { validator_name, field_name }
            if validator_name == "V" && field_name == "note")),
        "expected W0102, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0102_does_not_fire_when_the_field_has_loaded_by() {
    let ws = warnings("module A
fn findNote(id: Int): Text = \"n\"
validator V for Order errors OE {
    context { note: Text loaded by findNote(1) }
    rule r { require true else OE.X }
}");
    assert!(!ws.iter().any(|w| matches!(&w.kind, WarningKind::ContextFieldMissingLoadedBy { .. })),
        "a context field with loaded by must not warn, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

#[test]
fn w0102_does_not_fire_for_a_validator_with_no_context_at_all() {
    let ws = warnings("module A
validator V for Order errors OE {
    rule r { require true else OE.X }
}");
    assert!(ws.is_empty(), "a validator with no context block at all must never warn, got: {:?}", ws.iter().map(|w| w.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// `else` branch error-type mismatch (E0703) — BACKLOG item 219
// ------------------------------------------------------------------ //

#[test]
fn else_producing_the_declared_errors_type_is_ok() {
    check("module A
type OE = | TooHigh
validator V for Order errors OE {
    rule r { require order.total > 0 else OE.TooHigh }
}").unwrap();
}

#[test]
fn else_producing_a_different_type_is_e0703() {
    let kind = first_error_kind("module A
type OE = | TooHigh
type BillingError = | CreditExceeded
validator V for Order errors OE {
    rule r { require order.total > 0 else BillingError.CreditExceeded }
}");
    assert!(matches!(kind, TypeErrorKind::ElseTypeMismatch { .. }), "expected ElseTypeMismatch (E0703), got {kind:?}");
}

#[test]
fn else_type_mismatch_names_the_offending_rule() {
    let kind = first_error_kind("module A
type OE = | TooHigh
type BillingError = | CreditExceeded
validator V for Order errors OE {
    rule creditCheck { require order.total > 0 else BillingError.CreditExceeded }
}");
    let TypeErrorKind::ElseTypeMismatch { rule_name, .. } = kind else {
        panic!("expected ElseTypeMismatch, got {kind:?}");
    };
    assert_eq!(rule_name, "creditCheck");
}

#[test]
fn else_type_mismatch_message_names_both_types() {
    let errs = check_err("module A
type OE = | TooHigh
type BillingError = | CreditExceeded
validator V for Order errors OE {
    rule r { require order.total > 0 else BillingError.CreditExceeded }
}");
    let msg = errs[0].message();
    assert!(msg.contains("E0703"), "expected E0703, got: {msg}");
    assert!(msg.contains("OE"), "expected the declared errors type named, got: {msg}");
    assert!(msg.contains("BillingError"), "expected the actual else-type named, got: {msg}");
}

#[test]
fn a_bool_else_placeholder_is_also_e0703_not_silently_accepted() {
    // The exact gap this item closes: before this fix, `else true` (a
    // bare Bool, not a real error variant) compiled clean against any
    // declared `errors` type.
    let kind = first_error_kind("module A
type OE = | TooHigh
validator V for Order errors OE {
    rule r { require order.total > 0 else true }
}");
    assert!(matches!(kind, TypeErrorKind::ElseTypeMismatch { .. }), "expected ElseTypeMismatch (E0703), got {kind:?}");
}

#[test]
fn validator_context_field_in_scope() {
    check("module A
type Order = { id: Int }
type Customer = { active: Bool }
type OE = | X
validator V for Order errors OE {
    context { customer: Customer }
    rule r { require customer.active else OE.X }
}").unwrap();
}

#[test]
fn validator_multiple_rules_ok() {
    check("module A
type Order = { total: Int, valid: Bool }
type OE = | X
validator V for Order errors OE {
    rule a { require order.total > 0 else OE.X }
    rule b { require order.valid else OE.X }
}").unwrap();
}

#[test]
fn validator_validate_fn_registered() {
    // After hoisting, V.validate should be callable.
    check("module A
type OE = | X
validator V for Order errors OE {
    rule r { require true else OE.X }
}
fn callIt(o: Order): Result<Unit, OE> = V.validate(o)").unwrap();
}

#[test]
fn validator_with_context_validate_takes_entity_and_ctx() {
    // BACKLOG item 224 — a validator with a `context` block's real generated
    // function takes `(entity, ctx)`, but `hoist_decl`'s own registration
    // previously always registered a 1-parameter signature regardless,
    // rejecting the correct, spec-shaped 2-argument call with a hard type
    // error. Resolved nominally (`Ty::Named("{Name}Context")`) — `VContext`
    // is never declared in this isolated `check(...)` module, so this also
    // exercises the "undeclared capitalized name is an accepted opaque
    // nominal type" leniency `certo check` alone already relies on
    // elsewhere (real end-to-end verification, where `expand_validators`
    // really does splice `type VContext = {...}` in first, lives in
    // `crates/cli`'s own integration coverage, not here).
    check("module A
type OE = | X
validator V for Order errors OE {
    context { customer: Customer }
    rule r { require true else OE.X }
}
fn callIt(o: Order, c: VContext): Result<Unit, OE> = V.validate(o, c)").unwrap();
}

#[test]
fn validator_with_context_validate_all_takes_entity_and_ctx() {
    // Same fix, `validateAll`'s own signature.
    check("module A
type OE = | X
validator V for Order errors OE {
    context { customer: Customer }
    rule r { require true else OE.X }
}
fn callIt(o: Order, c: VContext): List<OE> = V.validateAll(o, c)").unwrap();
}

#[test]
fn validator_with_context_validate_rejects_single_arg() {
    // Regression guard: a context-bearing validator's `validate` must
    // actually *require* the ctx argument now, not just tolerate it —
    // confirms this isn't accidentally loosened to "any arity".
    let kind = first_error_kind("module A
type OE = | X
validator V for Order errors OE {
    context { customer: Customer }
    rule r { require true else OE.X }
}
fn callIt(o: Order): Result<Unit, OE> = V.validate(o)");
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected a type mismatch, got {kind:?}");
}

#[test]
fn validator_without_context_validate_still_takes_only_entity() {
    // Regression guard: a context-*free* validator's signature must stay
    // exactly 1 parameter, not silently grow a phantom ctx arg.
    check("module A
type OE = | X
validator V for Order errors OE {
    rule r { require true else OE.X }
}
fn callIt(o: Order): Result<Unit, OE> = V.validate(o)").unwrap();
}

// ------------------------------------------------------------------ //
// E0700/E0701/E0702 — rule dependency validation (BACKLOG item 252)
//
// Ported from `crates/resolve/src/tests.rs`, which exercises the same
// logic through `certo_resolve::resolve` — dead code for the real CLI
// binary (its only real consumer is the LSP). These versions exercise
// the port living in `check_module_seeded` itself, which is what
// `certo check`/`certo build` actually run.
// ------------------------------------------------------------------ //

#[test]
fn validator_after_missing_rule_e0701() {
    let kind = first_error_kind("module A
type OE = | X
validator V for Order errors OE {
    rule b { after nonexistent  require true else OE.X }
}");
    assert!(matches!(kind, TypeErrorKind::AfterRuleNotFound { ref after_name, .. } if after_name == "nonexistent"));
}

#[test]
fn validator_overrides_missing_rule_e0702() {
    let kind = first_error_kind("module A
type OE = | X
validator V for Order errors OE {
    rule b { overrides ghost  require true else OE.X }
}");
    assert!(matches!(kind, TypeErrorKind::OverridesRuleNotFound { ref overrides_name, .. } if overrides_name == "ghost"));
}

#[test]
fn validator_rule_cycle_e0700() {
    let kind = first_error_kind("module A
type OE = | X
validator V for Order errors OE {
    rule a { after b  require true else OE.X }
    rule b { after a  require true else OE.X }
}");
    assert!(matches!(kind, TypeErrorKind::RuleCycle { ref validator, .. } if validator == "V"));
}

#[test]
fn validator_no_cycle_chain_ok() {
    // a -> b -> c is a valid chain with no cycle.
    check("module A
type OE = | X
validator V for Order errors OE {
    rule a { require true else OE.X }
    rule b { after a  require true else OE.X }
    rule c { after b  require true else OE.X }
}").unwrap();
}

// ------------------------------------------------------------------ //
// `db.<table>.<method>(...)` sugar — BACKLOG item 226
//
// Real usage relies on `certo db pull`-generated functions (`{table}
// FindById`/`{table}FindAll`/`{table}DeleteById`) being declared elsewhere
// and imported — these tests declare small stand-in stubs directly in the
// same module instead, exactly mirroring what a real `db/schema.cto` import
// would provide. `db` is deliberately never seeded in `crates/typeck`
// itself (see `try_db_accessor_call`'s own doc comment,
// `crates/typeck/src/infer_expr.rs`), so these tests exercise the real,
// structural recognition path, not a bound-global lookup.
// ------------------------------------------------------------------ //

fn with_db_stubs(body: &str) -> String {
    format!("module A
type Customer = {{ id: Int, name: Text }}
fn __certo_db_conn(): Int = 0
fn customersFindById(conn: Int, id: Int): Customer? = None
fn customersFindAll(conn: Int): List<Customer> = []
fn customersDeleteById(conn: Int, id: Int): Int = 0
{body}")
}

#[test]
fn db_accessor_find_resolves_to_generated_function_return_type() {
    check(&with_db_stubs("fn f(id: Int): Customer? = db.customers.find(id)")).unwrap();
}

#[test]
fn db_accessor_all_resolves_to_generated_function_return_type() {
    check(&with_db_stubs("fn f(): List<Customer> = db.customers.all()")).unwrap();
}

#[test]
fn db_accessor_delete_resolves_to_generated_function_return_type() {
    check(&with_db_stubs("fn f(id: Int): Int = db.customers.delete(id)")).unwrap();
}

#[test]
fn db_accessor_wrong_arity_is_a_type_error() {
    // The rewritten call is reused, unmodified, by the exact same ordinary
    // call-checking machinery a hand-written `customersFindById(conn, ...)`
    // call would go through — a wrong-arity call surfaces as whatever kind
    // of error that machinery already produces for this shape (a `Fn`-type
    // mismatch, not a dedicated `ArityMismatch`), confirming no bespoke
    // arity/unify logic was reimplemented here at all.
    let kind = first_error_kind(&with_db_stubs("fn f(): Customer? = db.customers.find()"));
    assert!(matches!(kind, TypeErrorKind::Mismatch { .. }), "expected a type mismatch, got {kind:?}");
}

#[test]
fn db_accessor_unknown_table_is_e0711() {
    // No `widgetsFindById` stub declared anywhere.
    let kind = first_error_kind(&with_db_stubs("fn f(id: Int): Customer? = db.widgets.find(id)"));
    assert!(
        matches!(kind, TypeErrorKind::DbAccessorNotFound { ref table, ref method, .. }
            if table == "widgets" && method == "find"),
        "expected DbAccessorNotFound, got {kind:?}"
    );
}

#[test]
fn db_accessor_unrecognized_method_is_e0711() {
    // `.save` isn't one of the three sugared methods at all.
    let kind = first_error_kind(&with_db_stubs("fn f(id: Int): Int = db.customers.save(id)"));
    assert!(
        matches!(kind, TypeErrorKind::DbAccessorNotFound { ref method, .. } if method == "save"),
        "expected DbAccessorNotFound, got {kind:?}"
    );
}

#[test]
fn db_accessor_local_param_named_db_shadows_the_sugar() {
    // A real local named `db` must win — the call falls through to ordinary
    // field-access resolution instead of the sugar, and errors accordingly
    // (there is no real `.customers` field on `db`'s own declared type).
    let kind = first_error_kind(&with_db_stubs(
        "fn f(db: Customer): Int = db.customers.find(1)"));
    assert!(
        !matches!(kind, TypeErrorKind::DbAccessorNotFound { .. }),
        "shadowed `db` must not trigger the db-accessor sugar at all, got {kind:?}"
    );
}

// ------------------------------------------------------------------ //
// E0704 — named-constraint field scope (BACKLOG item 225)
// ------------------------------------------------------------------ //

#[test]
fn constraint_referencing_a_field_not_in_context_is_e0704() {
    // Spec §16.8's own exact repro shape: `user.role` inside a constraint,
    // but `user` was never declared in the validator's `context` block —
    // this direct check (not the generated/expanded code's own inlined
    // version) must catch it under `certo check` alone, without ever
    // running `expand_validators`.
    let kind = first_error_kind("module A
type UserRole = | Admin | Regular
type User = { role: UserRole }
type OE = | NotAuthorised
constraint UserIsAdmin = user.role == Admin
validator V for Order errors OE {
    rule r { require UserIsAdmin else OE.NotAuthorised }
}");
    match kind {
        TypeErrorKind::ConstraintFieldNotInScope { constraint_name, field_name } => {
            assert_eq!(constraint_name, "UserIsAdmin");
            assert_eq!(field_name, "user");
        }
        other => panic!("expected ConstraintFieldNotInScope (E0704), got {other:?}"),
    }
}

#[test]
fn constraint_referencing_a_declared_context_field_is_ok() {
    // The exact same constraint is fine once `user` is actually declared.
    check("module A
type UserRole = | Admin | Regular
type User = { role: UserRole }
type OE = | NotAuthorised
constraint UserIsAdmin = user.role == Admin
validator V for Order errors OE {
    context { user: User }
    rule r { require UserIsAdmin else OE.NotAuthorised }
}").unwrap();
}

#[test]
fn constraint_referencing_the_entity_variable_itself_is_ok() {
    // A constraint may also reference the validator's own primary entity
    // variable directly (not just a `context` field) — `order` here, from
    // `for Order`, not a declared context field at all.
    check("module A
type Order = { total: Int }
type OE = | TooLow
constraint OrderHasTotal = order.total > 0
validator V for Order errors OE {
    rule r { require OrderHasTotal else OE.TooLow }
}").unwrap();
}

#[test]
fn plain_bool_expression_without_any_named_constraint_is_unaffected() {
    // Regression guard: a rule with no named-constraint reference at all
    // (just an ordinary boolean expression) must not spuriously trigger
    // E0704 — the whole pass is a no-op when there are no constraints in
    // the module (an early return), and this confirms that.
    check("module A
type Order = { total: Int }
type OE = | TooLow
validator V for Order errors OE {
    rule r { require order.total > 0 else OE.TooLow }
}").unwrap();
}

#[test]
fn result_rewrap_in_match_different_payload() {
    // Regression: `Ok`/`Err` must quantify BOTH type variables, so matching a
    // `Result<Int, Text>` and re-wrapping into `Result<Text, Text>` type-checks.
    // Previously the unquantified var leaked the scrutinee's payload type.
    check("module A
fn validate(n: Int): Result<Int, Text> = if n > 0 then Ok(n) else Err(\"bad\")
fn process(n: Int): Result<Text, Text> = match validate(n) {
  Ok(_) => Ok(\"good\")
  Err(e) => Err(e)
}").unwrap();
}

#[test]
fn ok_err_independent_across_uses() {
    // Two unrelated Results built with Ok/Err must not share type variables.
    check("module A
fn a(): Result<Int, Text> = Ok(1)
fn b(): Result<Bool, Int> = Err(7)").unwrap();
}

// ------------------------------------------------------------------ //
// Core type system — containers, generics, operators
// ------------------------------------------------------------------ //

#[test]
fn option_some_constructor_ok() {
    check("module A\nfn f(): Option<Int> = Some(1)").unwrap();
}

#[test]
fn option_question_suffix_ok() {
    check("module A\nfn f(): Int? = Some(2)").unwrap();
}

#[test]
fn result_ok_constructor_ok() {
    check("module A\nfn f(): Result<Int, Text> = Ok(1)").unwrap();
}

#[test]
fn result_err_constructor_ok() {
    check("module A\nfn f(): Result<Int, Text> = Err(\"boom\")").unwrap();
}

#[test]
fn list_literal_homogeneous_ok() {
    check("module A\nfn f(): List<Int> = [1, 2, 3]").unwrap();
}

#[test]
fn list_literal_heterogeneous_mismatch() {
    let errs = check_err("module A\nfn f(): List<Int> = [1, \"two\"]");
    assert!(!errs.is_empty(), "expected a type error for mixed-type list");
}

#[test]
fn string_concat_is_text() {
    check("module A\nfn f(a: Text, b: Text): Text = a ++ b").unwrap();
}

#[test]
fn string_concat_on_int_mismatch() {
    let errs = check_err("module A\nfn f(): Text = 1 ++ 2");
    assert!(!errs.is_empty(), "expected a type error for `++` on Int");
}

#[test]
fn pipeline_threads_value_ok() {
    check("module A\nfn inc(x: Int): Int = x + 1\nfn f(): Int = 5 |> inc").unwrap();
}

#[test]
fn unknown_field_e0205() {
    let kind = first_error_kind(
        "module A\ntype Rec = { a: Int }\nfn f(r: Rec): Int = r.missing");
    assert!(matches!(kind, TypeErrorKind::UnknownField { .. }), "expected E0205, got {kind:?}");
}

#[test]
fn explicit_arity_mismatch_e0204_or_mismatch() {
    // Calling a 2-arg fn with 3 args.
    let errs = check_err(
        "module A\nfn add(a: Int, b: Int): Int = a + b\nval z: Int = add(1, 2, 3)");
    assert!(!errs.is_empty(), "expected an arity/type error");
    assert!(errs.iter().any(|e|
        matches!(e.kind, TypeErrorKind::ArityMismatch { .. } | TypeErrorKind::Mismatch { .. })),
        "expected E0204 or E0200, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn comparison_yields_bool() {
    check("module A\nfn f(a: Int, b: Int): Bool = a < b").unwrap();
}

#[test]
fn boolean_ops_typecheck() {
    check("module A\nfn f(a: Bool, b: Bool): Bool = a and b or not a").unwrap();
}

#[test]
fn nested_block_lets_ok() {
    check("module A\nfn f(): Int = {\n  val a = 1\n  val b = a + 1\n  val c = b + a\n  c\n}").unwrap();
}

#[test]
fn match_arms_must_agree() {
    // One arm Int, one arm Text — match result cannot unify.
    let errs = check_err(
        "module A\nfn f(n: Int): Int = match n {\n  0 => 1\n  _ => \"other\"\n}");
    assert!(!errs.is_empty(), "expected a type error for divergent match arms");
}

// ------------------------------------------------------------------ //
// Pattern-to-scrutinee type checking
// ------------------------------------------------------------------ //

#[test]
fn match_literal_pattern_wrong_type_e0200() {
    // A Text literal pattern matched against an Int scrutinee.
    let errs = check_err(
        "module A\nfn f(n: Int): Text = match n {\n  \"hello\" => \"a\"\n  _ => \"b\"\n}");
    assert!(!errs.is_empty(), "expected a type error for a Text pattern against an Int scrutinee");
    assert!(has_kind_typeck(&errs, |k| matches!(k, TypeErrorKind::Mismatch { .. })),
        "expected E0200, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn match_bool_literal_patterns_ok() {
    check("module A\nfn f(b: Bool): Text = match b {\n  true => \"yes\"\n  false => \"no\"\n}").unwrap();
}

#[test]
fn match_constructor_from_wrong_type_e0200() {
    // `Ok(x)` (a Result constructor) matched against a plain Int scrutinee.
    let errs = check_err(
        "module A\nfn f(n: Int): Int = match n {\n  Ok(x) => x\n  _ => 0\n}");
    assert!(!errs.is_empty(), "expected a type error matching a Result constructor against an Int scrutinee");
}

#[test]
fn match_constructor_arity_mismatch_e0204() {
    // `Some` takes exactly one field.
    let errs = check_err(
        "module A\nfn f(o: Option<Int>): Int = match o {\n  Some(a, b) => a\n  _ => 0\n}");
    assert!(has_kind_typeck(&errs, |k| matches!(k, TypeErrorKind::ArityMismatch { .. })),
        "expected E0204, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn match_binds_precise_type_from_scrutinee() {
    // `x` bound from `Some(x)` against Option<Int> must be usable as an Int —
    // this only works if check_pattern gave it Int, not a totally free var.
    check("module A\nfn f(o: Option<Int>): Int = match o {\n  Some(x) => x + 1\n  _ => 0\n}").unwrap();
}

#[test]
fn match_tuple_pattern_binds_precise_types() {
    // `a` bound from the tuple pattern must be usable as an Int and `b` as a
    // Bool — this only works if check_pattern gave each element its precise
    // positional type, not a shared/unconstrained fresh var.
    check("module A\nfn f(p: (Int, Bool)): Int = match p {\n  (a, b) => if b then a + 1 else a\n}").unwrap();
}

#[test]
fn match_record_pattern_binds_precise_types() {
    // A single record-pattern arm doesn't count as an exhaustiveness catch-all
    // (see is_catch_all's doc comment — a known v1 conservative limitation),
    // so a trailing wildcard is required here.
    check("module A\ntype Order = { id: Int, total: Int }\n\
           fn f(o: Order): Int = match o {\n  Order { id, total } => id + total\n  _ => 0\n}").unwrap();
}

#[test]
fn val_destructure_tuple_binds_precise_types() {
    check("module A\nfn f(): Int = {\n  val (a, b) = (1, 2)\n  a + b\n}").unwrap();
}

#[test]
fn val_destructure_wrong_type_e0200() {
    let errs = check_err(
        "module A\nfn f(): Int = {\n  val (a, b): (Int, Int) = (1, \"x\")\n  a + b\n}");
    assert!(!errs.is_empty(), "expected a type error destructuring a Text into an Int-typed tuple slot");
}

// ------------------------------------------------------------------ //
// FFI safety — extern calls must be inside `unsafe { }`
// ------------------------------------------------------------------ //

#[test]
fn extern_call_outside_unsafe_errors() {
    let kinds: Vec<_> = check_err(
        "module A\nextern \"C\" {\n  fn rustAdd(a: Int, b: Int): Int\n}\nfn main(): Int = rustAdd(2, 3)")
        .into_iter().map(|e| e.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TypeErrorKind::FfiCallOutsideUnsafe { .. })),
        "expected FfiCallOutsideUnsafe, got {:?}", kinds);
}

#[test]
fn extern_call_inside_unsafe_ok() {
    check(
        "module A\nextern \"C\" {\n  fn rustAdd(a: Int, b: Int): Int\n}\nfn main(): Int = unsafe { rustAdd(2, 3) }").unwrap();
}

// ------------------------------------------------------------------ //
// f-string interpolation is type-checked (E0200 / E0211)
// ------------------------------------------------------------------ //

#[test]
fn fstring_displayable_ok() {
    check("module A\nfn f(i: Int, s: Text): Text = f\"{i} and {s}\"").unwrap();
}

#[test]
fn fstring_type_mismatch_caught() {
    let kinds: Vec<_> = check_err(
        "module A\nfn add(a: Int, b: Int): Int = a + b\nfn f(): Text = f\"x: {add(1, true)}\"")
        .into_iter().map(|e| e.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TypeErrorKind::Mismatch { .. })),
        "expected a Mismatch inside the f-string, got {:?}", kinds);
}

#[test]
fn fstring_non_displayable_errors() {
    let kinds: Vec<_> = check_err(
        "module A\nfn f(): Text = {\n  val xs = [1, 2, 3]\n  f\"list: {xs}\"\n}")
        .into_iter().map(|e| e.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TypeErrorKind::NonDisplayableInterpolation { .. })),
        "expected NonDisplayableInterpolation, got {:?}", kinds);
}

// ------------------------------------------------------------------ //
// Row polymorphism — `<R: { field: Ty }>` bounds (E0212)
// ------------------------------------------------------------------ //

#[test]
fn row_bound_exact_match_ok() {
    check(
        "module A
type Widget = { name: Text }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = getName(Widget { name: \"Bolt\" })"
    ).unwrap();
}

#[test]
fn row_bound_superset_fields_ok() {
    check(
        "module A
type Order = { id: Int, name: Text, total: Int }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = getName(Order { id: 1, name: \"Alice\", total: 100 })"
    ).unwrap();
}

#[test]
fn row_bound_via_pipe_ok() {
    check(
        "module A
type Order = { id: Int, name: Text, total: Int }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = Order { id: 1, name: \"Alice\", total: 100 } |> getName"
    ).unwrap();
}

#[test]
fn row_bound_missing_field_e0212() {
    let kind = first_error_kind(
        "module A
type NoName = { id: Int }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = getName(NoName { id: 1 })"
    );
    assert!(matches!(kind, TypeErrorKind::MissingRowField { .. }), "expected E0212, got {kind:?}");
}

#[test]
fn row_bound_wrong_field_type_mismatch() {
    let kinds: Vec<_> = check_err(
        "module A
type WrongType = { name: Int }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = getName(WrongType { name: 42 })"
    ).into_iter().map(|e| e.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TypeErrorKind::Mismatch { .. })),
        "expected a Mismatch for wrong field type, got {:?}", kinds);
}

#[test]
fn row_bound_non_record_argument_errors() {
    let errs = check_err(
        "module A
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = getName(42)"
    );
    assert!(!errs.is_empty(), "expected a type error passing a non-record to a row-bounded param");
}

#[test]
fn row_bound_missing_field_via_pipe_e0212() {
    let kind = first_error_kind(
        "module A
type NoName = { id: Int }
fn getName<R: { name: Text }>(record: R): Text = record.name
fn f(): Text = NoName { id: 1 } |> getName"
    );
    assert!(matches!(kind, TypeErrorKind::MissingRowField { .. }), "expected E0212, got {kind:?}");
}

// ------------------------------------------------------------------ //
// `??` chaining with an optional fallback (right-associative parse)
// ------------------------------------------------------------------ //

#[test]
fn null_coalesce_chain_with_optional_middle_value_ok() {
    // `a ?? b ?? c` must parse/typecheck as `a ?? (b ?? c)` so a chain of
    // fallbacks works even when the middle value is itself optional.
    check(
        "module A
fn maybeA(): Text? = None
fn maybeB(): Text? = None
fn f(): Text = maybeA() ?? maybeB() ?? \"default\""
    ).unwrap();
}

// ------------------------------------------------------------------ //
// Match exhaustiveness checking (E0213)
// ------------------------------------------------------------------ //

#[test]
fn bool_match_missing_false_e0213() {
    let kind = first_error_kind("module A\nfn f(b: Bool): Text = match b {\n  true => \"y\"\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn bool_match_both_arms_ok() {
    check("module A\nfn f(b: Bool): Text = match b {\n  true => \"y\"\n  false => \"n\"\n}").unwrap();
}

#[test]
fn bool_match_wildcard_catch_all_ok() {
    check("module A\nfn f(b: Bool): Text = match b {\n  true => \"y\"\n  _ => \"n\"\n}").unwrap();
}

#[test]
fn option_match_missing_none_e0213() {
    let kind = first_error_kind("module A\nfn f(o: Option<Int>): Int = match o {\n  Some(x) => x\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn option_match_both_arms_ok() {
    check("module A\nfn f(o: Option<Int>): Int = match o {\n  Some(x) => x\n  None => 0\n}").unwrap();
}

#[test]
fn result_match_missing_err_e0213() {
    let kind = first_error_kind("module A\nfn f(r: Result<Int, Text>): Int = match r {\n  Ok(x) => x\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn result_match_both_arms_ok() {
    check("module A\nfn f(r: Result<Int, Text>): Int = match r {\n  Ok(x) => x\n  Err(_) => 0\n}").unwrap();
}

#[test]
fn sum_type_match_missing_variant_e0213() {
    let kind = first_error_kind(
        "module A
type Shape =
    | Circle(radius: Float)
    | Square(side: Float)
fn area(s: Shape): Float = match s {\n  Circle(r) => r\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn sum_type_match_all_variants_ok() {
    check(
        "module A
type Shape =
    | Circle(radius: Float)
    | Square(side: Float)
fn area(s: Shape): Float = match s {\n  Circle(r) => r\n  Square(side) => side\n}"
    ).unwrap();
}

#[test]
fn sum_type_match_wildcard_catch_all_ok() {
    check(
        "module A
type Shape =
    | Circle(radius: Float)
    | Square(side: Float)
fn area(s: Shape): Float = match s {\n  Circle(r) => r\n  _ => 0.0\n}"
    ).unwrap();
}

#[test]
fn or_pattern_covers_both_branches_ok() {
    // `true | false` in one arm should count as covering both bool values.
    check("module A\nfn f(b: Bool): Text = match b {\n  true | false => \"either\"\n}").unwrap();
}

#[test]
fn guarded_arm_does_not_count_as_coverage_e0213() {
    // A guard could fail at runtime, so `true if cond` must not satisfy
    // exhaustiveness on its own.
    let kind = first_error_kind(
        "module A\nfn f(b: Bool): Text = match b {\n  true if b => \"y\"\n  false => \"n\"\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn tuple_pattern_of_irrefutable_elements_is_exhaustive_ok() {
    // A tuple type has exactly one shape, so `(a, b)` alone is genuinely
    // exhaustive — no wildcard arm required.
    check("module A\nfn f(p: (Int, Bool)): Int = match p {\n  (a, b) => a\n}").unwrap();
}

#[test]
fn int_match_requires_wildcard_e0213() {
    // Literal-only patterns can never be exhaustive over an infinite domain.
    let kind = first_error_kind("module A\nfn f(n: Int): Text = match n {\n  0 => \"zero\"\n  1 => \"one\"\n}");
    assert!(matches!(kind, TypeErrorKind::NonExhaustiveMatch { .. }), "expected E0213, got {kind:?}");
}

#[test]
fn int_match_with_wildcard_ok() {
    check("module A\nfn f(n: Int): Text = match n {\n  0 => \"zero\"\n  _ => \"other\"\n}").unwrap();
}

// ------------------------------------------------------------------ //
// Smart constructors — `type X = priv X(...)` (item 74)
// ------------------------------------------------------------------ //

#[test]
fn priv_ctor_call_outside_impl_is_e0214() {
    let kind = first_error_kind(
        "module A
type Email = priv Email(Text)
fn f(): Email = Email(\"x\")"
    );
    assert!(matches!(kind, TypeErrorKind::PrivConstructorCall { ref type_name } if type_name == "Email"),
        "expected E0214, got {kind:?}");
}

#[test]
fn priv_ctor_call_in_own_impl_ok() {
    check(
        "module A
type Email = priv Email(Text)
impl Email {
    fn new(raw: Text): Email = Email(raw)
}"
    ).unwrap();
}

#[test]
fn priv_ctor_call_in_other_types_impl_is_e0214() {
    // Being inside *some* impl block isn't enough — it must be an impl for
    // the same type as the constructor.
    let kind = first_error_kind(
        "module A
type Email = priv Email(Text)
type Widget = { id: Int }
impl Widget {
    fn make(): Email = Email(\"x\")
}"
    );
    assert!(matches!(kind, TypeErrorKind::PrivConstructorCall { .. }), "expected E0214, got {kind:?}");
}

#[test]
fn priv_ctor_call_in_top_level_val_is_e0214() {
    // The spec's own motivating example: a bare top-level `val` binding
    // constructing directly, not just a function body.
    let kind = first_error_kind(
        "module A
type Email = priv Email(Text)
val bad: Email = Email(\"bad\")"
    );
    assert!(matches!(kind, TypeErrorKind::PrivConstructorCall { .. }), "expected E0214, got {kind:?}");
}

#[test]
fn non_priv_sum_type_constructor_call_anywhere_ok() {
    // Sanity: a plain (non-priv) sum type's constructor is unrestricted —
    // this check must not fire for ordinary sum types.
    check(
        "module A
type Shape = | Circle(radius: Float) | Square(side: Float)
fn f(): Shape = Circle(1.0)"
    ).unwrap();
}

// ------------------------------------------------------------------ //
// Impl method body checking + generic impls (found while implementing
// item 78's Secret<T> prerequisite — see BACKLOG)
// ------------------------------------------------------------------ //

#[test]
fn impl_method_body_type_mismatch_is_caught() {
    // Previously impl method bodies were never checked at all — only the
    // hoisted signature existed, for callers to unify against. A body
    // returning the wrong type must now be rejected.
    let errs = check_err(
        "module A
type Point = { x: Int, y: Int }
impl Point {
    fn wrongBody(p: Point): Int = \"not an int\"
}"
    );
    assert!(!errs.is_empty(), "expected a type error from the impl method's own body");
    assert!(errs[0].message().contains("E0200"), "got: {}", errs[0].message());
}

#[test]
fn impl_method_body_correct_type_ok() {
    check(
        "module A
type Point = { x: Int, y: Int }
impl Point {
    fn sum(p: Point): Int = p.x + p.y
}"
    ).unwrap();
}

#[test]
fn generic_impl_type_param_threads_through_method_body() {
    // `impl<T> Box { ... }` — the impl's own type param `T` must be in scope
    // for every method's params, return type, and body, resolving to the
    // *same* type variable as the type's own declared `T`, not a bogus
    // rigid `Ty::Named("T")`.
    check(
        "module A
type Box<T> = priv Box(T)
impl<T> Box {
    fn wrap(v: T): Box<T> = Box(v)
    fn unwrap(b: Box<T>): T = match b { Box(v) => v }
}
fn f(): Text = Box.unwrap(Box.wrap(\"hi\"))"
    ).unwrap();
}

#[test]
fn generic_impl_wrong_body_type_is_caught() {
    // The generic-impl fix must not accidentally bypass body-checking —
    // a genuinely wrong body inside a generic impl is still an error.
    let errs = check_err(
        "module A
type Box<T> = priv Box(T)
impl<T> Box {
    fn wrap(v: T): Box<T> = 42
}"
    );
    assert!(!errs.is_empty(), "expected a type error");
}

#[test]
fn generic_sum_type_unit_variant_ok() {
    // A generic sum type can still have non-generic unit variants alongside
    // a payload-carrying one referencing the type param.
    check(
        "module A
type Maybe<T> = | Just(T) | Nothing
fn f(): Maybe<Int> = Just(1)
fn g(): Maybe<Int> = Nothing"
    ).unwrap();
}

#[test]
fn parallel_timeout_accepts_duration() {
    // BACKLOG item 81: `timeout:` was parsed but never actually type-checked
    // against `Duration` at all before this.
    let src = "module A
fn work(): Int = 1
fn run(): (Int, Int) = await parallel(timeout: Duration.seconds(5)) { work(), work() }";
    assert!(check(src).is_ok(), "{:?}", check(src).err());
}

#[test]
fn parallel_timeout_rejects_non_duration() {
    let src = "module A
fn work(): Int = 1
fn run(): (Int, Int) = await parallel(timeout: 5) { work(), work() }";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected a type mismatch for a non-Duration timeout, got: {:?}", kind
    );
}

// `withTimeout(d) { body }` (BACKLOG item 122) — cooperative-cancellation
// timeout: type-checks like `parallel(timeout:)`'s own duration clause, but
// the whole expression's type is `Option<T>` (`T` being `body`'s own type),
// not `body`'s type directly — `Some` on completion, `None` on timeout.

#[test]
fn with_timeout_result_is_option_of_body_type() {
    let src = "module A
fn work(): Int = 1
fn run(): Int? = withTimeout(Duration.seconds(5)) { work() }";
    assert!(check(src).is_ok(), "{:?}", check(src).err());
}

#[test]
fn with_timeout_rejects_non_duration() {
    let src = "module A
fn run(): Int? = withTimeout(5) { 1 }";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected a type mismatch for a non-Duration duration, got: {:?}", kind
    );
}

#[test]
fn with_timeout_body_type_mismatch_is_rejected() {
    // The declared return type must be `Option<Int>`, not bare `Int` —
    // confirms `withTimeout` really does wrap in `Option`, not pass through.
    let src = "module A
fn run(): Int = withTimeout(Duration.seconds(5)) { 1 }";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected a type mismatch (Option<Int> vs Int), got: {:?}", kind
    );
}

// ------------------------------------------------------------------ //
// Float32 / Char — BACKLOG item 75
// ------------------------------------------------------------------ //

#[test]
fn float32_param_and_return_type_check() {
    check("module A\nfn f(x: Float32): Float32 = x").unwrap();
}

#[test]
fn float32_arithmetic_type_checks() {
    // BinOp::Add etc. just unify(lt, rt) and return lt — generic over any
    // type that unifies with itself, so this should work with zero special
    // casing once the Ty variant exists (confirmed by reading infer_binop).
    check("module A\nfn f(x: Float32, y: Float32): Float32 = x + y").unwrap();
}

#[test]
fn float32_does_not_unify_with_float() {
    // Mirrors Int8's existing exclusivity — Float32 must be a fully
    // distinct type, not silently coercible from/to Float.
    let src = "module A\nfn f(x: Float): Float32 = x";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected Float32/Float to be exclusive, got: {:?}", kind
    );
}

#[test]
fn char_param_and_return_type_check() {
    check("module A\nfn f(c: Char): Char = c").unwrap();
}

#[test]
fn char_does_not_unify_with_text() {
    let src = "module A\nfn f(s: Text): Char = s";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected Char/Text to be exclusive, got: {:?}", kind
    );
}

#[test]
fn char_does_not_unify_with_int() {
    let src = "module A\nfn f(n: Int): Char = n";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected Char/Int to be exclusive, got: {:?}", kind
    );
}

// ------------------------------------------------------------------ //
// Decimal(p, s) — BACKLOG item 128
// ------------------------------------------------------------------ //

#[test]
fn decimal_param_type_checks() {
    check("module A\nfn f(x: Decimal(19, 4)): Decimal(19, 4) = x").unwrap();
}

#[test]
fn decimal_param_arithmetic_type_checks() {
    // BinOp::Add etc. just unify(lt, rt) and return lt — generic over any
    // type that unifies with itself, so Decimal(p,s) needs zero special
    // casing in the operator itself (mirrors float32_arithmetic_type_checks).
    check("module A\nfn f(x: Decimal(19, 4), y: Decimal(19, 4)): Decimal(19, 4) = x + y").unwrap();
}

#[test]
fn bare_decimal_unifies_with_parameterized_decimal() {
    // The core design decision: a bare `Decimal` value can be passed where
    // `Decimal(p, s)` is expected, and vice versa — same runtime
    // representation, the parameter is a compile-time-only refinement.
    check("module A\nfn f(x: Decimal): Decimal(19, 4) = x").unwrap();
    check("module A\nfn g(x: Decimal(19, 4)): Decimal = x").unwrap();
}

#[test]
fn different_decimal_params_do_not_unify() {
    // Decimal(10,2) and Decimal(19,4) are NOT interchangeable — only a bare
    // Decimal on one side makes them compatible.
    let src = "module A\nfn f(x: Decimal(10, 2)): Decimal(19, 4) = x";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected Decimal(10,2)/Decimal(19,4) to be exclusive, got: {:?}", kind
    );
}

#[test]
fn decimal_param_display_shows_precision_scale() {
    let src = "module A\nfn f(x: Decimal(10, 2)): Decimal(19, 4) = x";
    let kind = first_error_kind(src);
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    // Don't assume which side unify labels "expected" vs "found" — just
    // confirm both precisions appear, correctly formatted, in the error.
    let both = format!("{} {}", expected.display(), found.display());
    assert!(both.contains("Decimal(19, 4)"), "got: {both}");
    assert!(both.contains("Decimal(10, 2)"), "got: {both}");
}

// ------------------------------------------------------------------ //
// BoundedText(n) — BACKLOG item 147
// ------------------------------------------------------------------ //

#[test]
fn bounded_text_param_type_checks() {
    check("module A\nfn f(x: BoundedText(255)): BoundedText(255) = x").unwrap();
}

#[test]
fn bounded_text_concat_type_checks() {
    // `++` unifies both operands against Text — BoundedText must flow
    // through it exactly like plain Text (compile-time-only refinement,
    // zero special-casing needed anywhere Text is already accepted).
    check("module A\nfn f(x: BoundedText(255), y: BoundedText(255)): Text = x ++ y").unwrap();
}

#[test]
fn bare_text_unifies_with_bounded_text_both_ways() {
    // The core design decision: a bare `Text` value can be passed where
    // `BoundedText(n)` is expected, and vice versa — same runtime
    // representation, the length is a compile-time-only refinement.
    check("module A\nfn f(x: Text): BoundedText(255) = x").unwrap();
    check("module A\nfn g(x: BoundedText(255)): Text = x").unwrap();
}

#[test]
fn different_bounded_text_lengths_do_not_unify() {
    // BoundedText(10) and BoundedText(255) are NOT interchangeable — only a
    // bare Text on one side makes them compatible.
    let src = "module A\nfn f(x: BoundedText(10)): BoundedText(255) = x";
    let kind = first_error_kind(src);
    assert!(
        matches!(kind, TypeErrorKind::Mismatch { .. } | TypeErrorKind::CannotUnify { .. }),
        "expected BoundedText(10)/BoundedText(255) to be exclusive, got: {:?}", kind
    );
}

#[test]
fn bounded_text_param_display_shows_max_len() {
    let src = "module A\nfn f(x: BoundedText(10)): BoundedText(255) = x";
    let kind = first_error_kind(src);
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    let both = format!("{} {}", expected.display(), found.display());
    assert!(both.contains("BoundedText(255)"), "got: {both}");
    assert!(both.contains("BoundedText(10)"), "got: {both}");
}

// ------------------------------------------------------------------ //
// `E0200` expected/found direction — BACKLOG item 196. `unify()`'s own
// ~50 call sites overwhelmingly pass (the value's actual/inferred type,
// the declared/required type) in that order, but every `Mismatch`
// construction used to unconditionally label the *first* argument
// "expected" — silently swapping the two fields backwards on nearly
// every basic type error. These pin the corrected, real-world-readable
// direction: `expected` names what the surrounding context requires,
// `found` names what the offending expression's own type actually is.
// ------------------------------------------------------------------ //

#[test]
fn val_annotation_mismatch_reports_the_annotation_as_expected() {
    let kind = first_error_kind("module A\nfn f(): Unit = {\n val x: Int = \"hello\"\n}");
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    assert_eq!(expected.display(), "Int", "the val's own declared type must be `expected`, got: {}", expected.display());
    assert_eq!(found.display(), "Text", "the literal's real type must be `found`, got: {}", found.display());
}

#[test]
fn call_argument_mismatch_reports_the_parameter_type_as_expected() {
    let kind = first_error_kind("module A\nfn addOne(x: Int): Int = x + 1\nfn f(): Unit = {\n val r = addOne(\"hello\")\n}");
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    assert_eq!(expected.display(), "Int", "the parameter's declared type must be `expected`, got: {}", expected.display());
    assert_eq!(found.display(), "Text", "the argument's real type must be `found`, got: {}", found.display());
}

#[test]
fn named_type_mismatch_reports_the_required_type_as_expected() {
    let kind = first_error_kind(
        "module A\ntype Dog = { name: Text }\ntype Cat = { name: Text }\nfn petDog(d: Dog): Unit = {}\nfn f(): Unit = {\n val c: Cat = Cat { name: \"Tom\" }\n petDog(c)\n}"
    );
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    assert_eq!(expected.display(), "Dog", "the parameter's own declared type must be `expected`, got: {}", expected.display());
    assert_eq!(found.display(), "Cat", "the argument's real type must be `found`, got: {}", found.display());
}

#[test]
fn fn_arity_mismatch_reports_the_annotated_signature_as_expected() {
    let kind = first_error_kind(
        "module A\nfn twoParams(a: Int, b: Int): Int = a + b\nfn f(): Unit = {\n val g: (Int) => Int = twoParams\n}"
    );
    let TypeErrorKind::Mismatch { expected, found } = kind else {
        panic!("expected Mismatch, got {:?}", kind);
    };
    assert_eq!(expected.display(), "(Int) => Int", "the val's own annotation must be `expected`, got: {}", expected.display());
    assert_eq!(found.display(), "(Int, Int) => Int", "the referenced function's real type must be `found`, got: {}", found.display());
}

// ------------------------------------------------------------------ //
// Secret<T> not-Loggable/Serializable check (BACKLOG item 78)
// ------------------------------------------------------------------ //

const SECRET_SRC: &str = "\
type Secret<T> = priv Secret(T)
impl<T> Secret {
    fn wrap(v: T): Secret<T> = Secret(v)
    fn expose(s: Secret<T>): T = match s { Secret(v) => v }
}
";

#[test]
fn passing_secret_directly_to_println_is_rejected() {
    let src = format!(
        "module A\n{}fn f(): Unit [io] = {{\n val p: Secret<Text> = Secret.wrap(\"x\")\n println(p)\n}}",
        SECRET_SRC
    );
    let errs = check_err(&src);
    assert!(
        errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SecretInSensitiveContext { .. })),
        "expected a SecretInSensitiveContext error, got: {:?}", errs
    );
}

#[test]
fn exposed_secret_is_allowed() {
    // `.expose()`'s result is a plain Text, not Secret<Text> anymore —
    // logging it is exactly the spec's own documented escape hatch and
    // must not be rejected. Uses `check_err` like its siblings, not the
    // strict `check()` — `crates_resolve::resolve()` doesn't know about
    // *any* stdlib builtin (confirmed: even a bare `println("x")` with no
    // Secret involved fails resolve() on its own, a real pre-existing gap
    // unrelated to this item), so `check()`'s `.expect("resolve error")`
    // would panic regardless of whether the Secret check itself is correct.
    let src = format!(
        "module A\n{}fn f(): Unit [io] = {{\n val p: Secret<Text> = Secret.wrap(\"x\")\n println(Secret.expose(p))\n}}",
        SECRET_SRC
    );
    let errs = check_err(&src);
    assert!(
        !errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SecretInSensitiveContext { .. })),
        "exposing a Secret before logging must not trigger the not-Loggable check, got: {:?}", errs
    );
}

#[test]
fn secret_passed_to_eprint_is_also_rejected() {
    // Same check must apply to every sensitive sink, not just println.
    let src = format!(
        "module A\n{}fn f(): Unit [io] = {{\n val p: Secret<Text> = Secret.wrap(\"x\")\n eprint(p)\n}}",
        SECRET_SRC
    );
    let errs = check_err(&src);
    assert!(
        errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SecretInSensitiveContext { .. })),
        "expected a SecretInSensitiveContext error for eprint, got: {:?}", errs
    );
}

// ------------------------------------------------------------------ //
// Raw-SQL interpolation rejection (BACKLOG item 159)
// ------------------------------------------------------------------ //

#[test]
fn interpolated_fstring_sql_in_dbquery_is_rejected() {
    let src = "module A\nfn f(): Unit [io] = {\n \
        val conn = dbConnect(\"x\")\n \
        val userInput = \"attacker\"\n \
        dbQuery(conn, f\"SELECT * FROM users WHERE email = '{userInput}'\", [])\n\
    }";
    let errs = check_err(src);
    assert!(
        errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::SqlInjectionRisk { fn_name } if fn_name == "dbQuery")),
        "expected a SqlInjectionRisk error for dbQuery, got: {:?}", errs
    );
}

#[test]
fn interpolated_fstring_sql_in_dbexec_is_rejected() {
    let src = "module A\nfn f(): Unit [io] = {\n \
        val conn = dbConnect(\"x\")\n \
        val tableName = \"sessions\"\n \
        dbExec(conn, f\"DELETE FROM {tableName} WHERE expired = true\", [])\n\
    }";
    let errs = check_err(src);
    assert!(
        errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::SqlInjectionRisk { fn_name } if fn_name == "dbExec")),
        "expected a SqlInjectionRisk error for dbExec, got: {:?}", errs
    );
}

#[test]
fn parameterized_query_with_placeholders_is_allowed() {
    // The safe alternative the error message itself recommends must not
    // be flagged: a plain (non-interpolated) SQL literal with values
    // passed through `params`.
    let src = "module A\nfn f(): Unit [io] = {\n \
        val conn = dbConnect(\"x\")\n \
        val userInput = \"attacker\"\n \
        dbQuery(conn, \"SELECT * FROM users WHERE email = ?\", [userInput])\n\
    }";
    let errs = check_err(src);
    assert!(
        !errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SqlInjectionRisk { .. })),
        "a parameterized, non-interpolated query must not be rejected, got: {:?}", errs
    );
}

#[test]
fn fstring_with_no_interpolation_passed_as_sql_is_allowed() {
    // An f-string literal with no `{ }` holes at all carries no injection
    // risk — only live interpolation should trip the check.
    let src = "module A\nfn f(): Unit [io] = {\n \
        val conn = dbConnect(\"x\")\n \
        dbExec(conn, f\"DELETE FROM sessions WHERE expired = true\", [])\n\
    }";
    let errs = check_err(src);
    assert!(
        !errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SqlInjectionRisk { .. })),
        "a non-interpolated f-string must not be rejected, got: {:?}", errs
    );
}

#[test]
fn interpolated_fstring_passed_to_unrelated_function_is_allowed() {
    // The check is scoped to the known raw-SQL sinks only — an
    // interpolated f-string passed to an ordinary function (or println)
    // is unrelated to SQL injection and must not be flagged.
    let src = "module A\nfn f(): Unit [io] = {\n \
        val userInput = \"attacker\"\n \
        println(f\"hello {userInput}\")\n\
    }";
    let errs = check_err(src);
    assert!(
        !errs.iter().any(|e| matches!(e.kind, TypeErrorKind::SqlInjectionRisk { .. })),
        "a non-sink call must not trigger the SQL injection check, got: {:?}", errs
    );
}

// ------------------------------------------------------------------ //
// Generic type-argument soundness (non-transitive substitution fix,
// found while implementing BACKLOG item 79 / Permission<T>)
// ------------------------------------------------------------------ //

const BOX_SRC: &str = "\
type Box<T> = priv Box(T)
impl<T> Box {
    fn wrap(v: T): Box<T> = Box(v)
}
";

#[test]
fn wrong_generic_type_argument_is_rejected() {
    // Real, confirmed pre-existing soundness bug: a `Box<Text>` local
    // (bound via a `val` with its own concrete annotation, so its type
    // passes through a "let"-generalization step) satisfied a `Box<Int>`
    // parameter with no error at all — `TypeEnv::generalise`'s free-var
    // check relied on `apply_subst`'s single-hop substitution, which left
    // the variable chain a generic call's own return-type unification
    // creates only half-resolved, making a fully-concrete type look "still
    // free" and get wrongly re-generalized. Confirmed directly: this exact
    // program used to type-check clean (`certo check` reported `ok`).
    let src = format!(
        "module A\n{}fn needsIntBox(b: Box<Int>): Text = \"x\"\nfn f(): Text = {{\n val textBox: Box<Text> = Box.wrap(\"hello\")\n needsIntBox(textBox)\n}}",
        BOX_SRC
    );
    let errs = check_err(&src);
    assert!(!errs.is_empty(), "Box<Text> must not satisfy a Box<Int> parameter");
}

#[test]
fn matching_generic_type_argument_is_still_accepted() {
    // The fix must not become overly strict — the correct instantiation
    // still has to type-check.
    let src = format!(
        "module A\n{}fn needsIntBox(b: Box<Int>): Text = \"x\"\nfn f(): Text = {{\n val intBox: Box<Int> = Box.wrap(42)\n needsIntBox(intBox)\n}}",
        BOX_SRC
    );
    assert!(check(&src).is_ok(), "Box<Int> must still satisfy a Box<Int> parameter");
}

// ------------------------------------------------------------------ //
// Permission<T> phantom-type role authorization (BACKLOG item 79)
// ------------------------------------------------------------------ //

const PERMISSION_SRC: &str = "\
type Admin = | Admin
type Viewer = | Viewer
type Permission<T> = priv Permission(T)
impl<T> Permission {
    fn wrap(v: T): Permission<T> = Permission(v)
    fn expose(p: Permission<T>): T = match p { Permission(v) => v }
}
";

#[test]
fn matching_permission_role_is_accepted() {
    let src = format!(
        "module A\n{}fn deleteUser(id: Int, _auth: Permission<Admin>): Text = \"deleted\"\nfn f(): Text = {{\n val adminAuth: Permission<Admin> = Permission.wrap(Admin)\n deleteUser(42, adminAuth)\n}}",
        PERMISSION_SRC
    );
    assert!(check(&src).is_ok(), "Permission<Admin> must satisfy a Permission<Admin> parameter");
}

#[test]
fn wrong_permission_role_is_rejected() {
    // The actual point of the feature: a Permission<Viewer> must not
    // satisfy a Permission<Admin> parameter — this is exactly the case
    // that was silently accepted before the apply_subst chain fix above.
    let src = format!(
        "module A\n{}fn deleteUser(id: Int, _auth: Permission<Admin>): Text = \"deleted\"\nfn f(): Text = {{\n val viewerAuth: Permission<Viewer> = Permission.wrap(Viewer)\n deleteUser(42, viewerAuth)\n}}",
        PERMISSION_SRC
    );
    let errs = check_err(&src);
    assert!(!errs.is_empty(), "Permission<Viewer> must not satisfy a Permission<Admin> parameter");
}

// ------------------------------------------------------------------ //
// Higher-kinded types — `F<_>` type-constructor parameters (BACKLOG item 76)
// ------------------------------------------------------------------ //

const HKT_MAP_SRC: &str = "
type Box<T> = { value: T }
type Pair<A, B> = { fst: A, snd: B }
fn boxUnwrap<T>(b: Box<T>): T = b.value
fn boxWrap<T>(v: T): Box<T> = Box { value: v }
fn isPos(n: Int): Bool = n > 0
fn hktMap<F<_>, A, B>(fa: F<A>, unwrap: F<A> => A, wrap: B => F<B>, f: A => B): F<B> =
    wrap(f(unwrap(fa)))
";

#[test]
fn hkt_generic_fn_declaration_type_checks() {
    // Just the declaration — no call site — sanity-checks that a
    // constructor-kind type param (`F<_>`) doesn't break ordinary
    // generalisation/instantiation of the rest of the signature.
    check(&format!("module A\n{}", HKT_MAP_SRC)).unwrap();
}

#[test]
fn hkt_call_site_infers_constructor_and_result_type() {
    // The real point: at a call site, `F` unifies with the `Box` type
    // constructor recovered from the argument's own type (`Box<Int>`),
    // `A`/`B` unify with the concrete element types on either side of `f`
    // (Int, then Bool), and the call's own result type comes out as
    // `F<B>` = `Box<Bool>` — a *different* instantiation of the same
    // constructor, not `Box<Int>` again and not an unresolved variable.
    let src = format!(
        "module A\n{}fn run(): Box<Bool> = {{\n val b: Box<Int> = Box {{ value: 5 }}\n hktMap(b, boxUnwrap, boxWrap, isPos)\n}}",
        HKT_MAP_SRC
    );
    assert!(check(&src).is_ok(), "expected hktMap's inferred F<B> to unify with the declared Box<Bool> return type");
}

#[test]
fn hkt_call_site_with_wrong_return_type_is_rejected() {
    // The inverse of the test above — pins that the inferred `F<B>` is
    // actually checked against the declared return type, not silently
    // accepted regardless (e.g. by falling back to an unconstrained var).
    let src = format!(
        "module A\n{}fn run(): Box<Int> = {{\n val b: Box<Int> = Box {{ value: 5 }}\n hktMap(b, boxUnwrap, boxWrap, isPos)\n}}",
        HKT_MAP_SRC
    );
    let errs = check_err(&src);
    assert!(!errs.is_empty(), "hktMap's real result type is Box<Bool>, not Box<Int> — must be rejected");
}

#[test]
fn hkt_rejects_a_2ary_type_for_a_1ary_constructor_param() {
    // `Pair<A, B>` is 2-ary — it can never satisfy `F<_>`, which only ever
    // has one type argument. This is a real kind mismatch, not something
    // to silently coerce or ignore.
    let src = format!(
        "module A\n{}fn pairUnwrap(p: Pair<Int, Int>): Int = p.fst\nfn pairWrap(n: Int): Pair<Int, Int> = Pair {{ fst: n, snd: n }}\nfn run(): Unit = {{\n val p: Pair<Int, Int> = Pair {{ fst: 1, snd: 2 }}\n hktMap(p, pairUnwrap, pairWrap, isPos)\n}}",
        HKT_MAP_SRC
    );
    let errs = check_err(&src);
    assert!(!errs.is_empty(), "Pair<Int, Int> (2-ary) must not satisfy a 1-ary F<_> parameter");
}

#[test]
fn hkt_two_occurrences_of_same_generic_record_with_different_types_both_work() {
    // Regression for the real, independent, pre-existing bug this item's
    // own test surfaced: a generic record type's own type-param vars were
    // minted once at hoisting and reused, unresolved, by every occurrence
    // of `TypeName { .. }` — forcing two different concrete instantiations
    // to unify with each other. Must be instantiated fresh per occurrence.
    check("module A
type Box<T> = { value: T }
fn intBox(): Box<Int> = Box { value: 1 }
fn boolBox(): Box<Bool> = Box { value: true }").unwrap();
}

// ------------------------------------------------------------------ //
// Safe field access `?.` (BACKLOG item 146)
// ------------------------------------------------------------------ //

const SAFE_FIELD_SRC: &str = "\
type Address = { city: Text, zip: Text }
type User = { name: Text, address: Address? }
";

#[test]
fn safe_field_access_on_a_real_optional_field_type_checks() {
    // This is the exact case the operator exists for and the exact case
    // that previously always failed with a false E0205 (field resolution
    // ran directly against the still-Option-wrapped base type, which has
    // no fields of its own).
    let src = format!(
        "module A\n{}fn f(u: User): Text? = u.address?.city",
        SAFE_FIELD_SRC
    );
    check(&src).expect("u.address?.city must type-check: address is Address?, city: Text");
}

#[test]
fn safe_field_access_result_type_is_option_of_field_type() {
    // Pins the *result* type, not just that it compiles — `?.`'s own
    // result must be `Option<field type>`, not the field type itself.
    let src = format!(
        "module A\n{}fn f(u: User): Text = u.address?.city",
        SAFE_FIELD_SRC
    );
    let errs = check_err(&src);
    assert!(
        !errs.is_empty(),
        "u.address?.city is Text?, not Text — must be rejected against a bare Text return type"
    );
}

#[test]
fn safe_field_access_on_unknown_field_is_still_rejected() {
    // Confirms the fix didn't just make the check permissive — a real
    // typo on the unwrapped payload's own fields must still be caught.
    let src = format!(
        "module A\n{}fn f(u: User): Text? = u.address?.country",
        SAFE_FIELD_SRC
    );
    let errs = check_err(&src);
    assert!(
        !errs.is_empty(),
        "Address has no field 'country' — must be rejected even through `?.`"
    );
}

#[test]
fn safe_field_access_on_a_non_optional_base_is_rejected() {
    // `?.` unifies its base against Option<fresh> — a genuinely
    // non-Optional base must fail to unify, not be silently accepted.
    let src = format!(
        "module A\n{}fn f(a: Address): Text? = a?.city",
        SAFE_FIELD_SRC
    );
    let errs = check_err(&src);
    assert!(
        !errs.is_empty(),
        "Address is not an Option — `a?.city` must be rejected"
    );
}

// ------------------------------------------------------------------ //
// Generic record patterns (BACKLOG item 177) — `check_pattern`'s
// `Pattern::Record` arm previously unified against a permanently
// arg-less `Ty::Named`, so any pattern on a generic record failed to
// unify against the scrutinee's own real instantiation.
// ------------------------------------------------------------------ //

#[test]
fn val_destructure_of_a_generic_record_type_checks() {
    check(
        "module A\n\
         type Box<T> = { value: T }\n\
         fn f(b: Box<Float>): Float = {\n\
         \x20   val Box { value: v } = b\n\
         \x20   v\n\
         }"
    ).unwrap();
}

#[test]
fn match_arm_pattern_on_a_generic_record_type_checks() {
    check(
        "module A\n\
         type Box<T> = { value: T }\n\
         fn f(b: Box<Float>): Float = match b {\n\
         \x20   Box { value: v } => v,\n\
         \x20   _ => 0.0\n\
         }"
    ).unwrap();
}

#[test]
fn bare_pattern_on_a_generic_record_type_checks() {
    check(
        "module A\n\
         type Box<T> = { value: T }\n\
         fn f(b: Box<Float>): Float = match b {\n\
         \x20   { value: v } => v,\n\
         \x20   _ => 0.0\n\
         }"
    ).unwrap();
}

#[test]
fn generic_record_pattern_still_catches_a_real_type_mismatch() {
    // Regression: fixing the false-positive unify failure must not turn
    // into a false negative — using the bound field as the wrong type is
    // still a real error.
    let errs = check_err(
        "module A\n\
         type Box<T> = { value: T }\n\
         fn f(b: Box<Float>): Text = {\n\
         \x20   val Box { value: v } = b\n\
         \x20   v\n\
         }"
    );
    assert!(!errs.is_empty(), "returning a Float as Text must still be rejected");
}

#[test]
fn non_generic_record_pattern_is_unaffected_by_the_generic_fix() {
    check(
        "module A\n\
         type User = { name: Text, age: Int }\n\
         fn f(u: User): Text = {\n\
         \x20   val User { name: n, age: a } = u\n\
         \x20   n\n\
         }"
    ).unwrap();
}

// ------------------------------------------------------------------ //
// Opaque date/time arithmetic operators (E0218) — BACKLOG item 214
// ------------------------------------------------------------------ //

#[test]
fn timestamp_minus_timestamp_is_e0218() {
    let kind = first_error_kind(
        "module A\nfn f(a: Timestamp, b: Timestamp): Timestamp = a - b"
    );
    assert!(matches!(kind, TypeErrorKind::OpaqueTemporalArithmetic { .. }),
        "expected OpaqueTemporalArithmetic (E0218), got {kind:?}");
}

#[test]
fn datetime_plus_datetime_is_e0218() {
    let kind = first_error_kind(
        "module A\nfn f(a: DateTime, b: DateTime): DateTime = a + b"
    );
    assert!(matches!(kind, TypeErrorKind::OpaqueTemporalArithmetic { .. }),
        "expected OpaqueTemporalArithmetic (E0218), got {kind:?}");
}

#[test]
fn date_times_date_is_e0218() {
    let kind = first_error_kind(
        "module A\nfn f(a: Date, b: Date): Date = a * b"
    );
    assert!(matches!(kind, TypeErrorKind::OpaqueTemporalArithmetic { .. }),
        "expected OpaqueTemporalArithmetic (E0218), got {kind:?}");
}

#[test]
fn e0218_message_names_the_type_and_operator() {
    let errs = check_err("module A\nfn f(a: Timestamp, b: Timestamp): Timestamp = a - b");
    let msg = errs[0].message();
    assert!(msg.contains("E0218"), "expected E0218, got: {msg}");
    assert!(msg.contains("Timestamp"), "expected the type named, got: {msg}");
    assert!(msg.contains('-'), "expected the operator named, got: {msg}");
}

#[test]
fn duration_arithmetic_is_not_flagged() {
    // Distinct from Timestamp/DateTime/Date — summing two spans is sensible;
    // only exposed via `.add`/`.sub` rather than an operator, but the bare
    // operator isn't the "nonsensical same-type result" bug this item is
    // about, so it's deliberately not rejected here.
    check("module A\nfn f(a: Duration, b: Duration): Duration = a - b").unwrap();
}

#[test]
fn ordinary_int_arithmetic_is_unaffected() {
    check("module A\nfn f(a: Int, b: Int): Int = a - b").unwrap();
}

#[test]
fn timestamp_comparison_is_not_flagged() {
    // E0218 is specifically about arithmetic (+/-/*//%/**) producing a
    // nonsensical same-typed result — comparisons return Bool and are fine.
    check("module A\nfn f(a: Timestamp, b: Timestamp): Bool = a == b").unwrap();
}

// ------------------------------------------------------------------ //
// `unreachable()`/`todo()` zero-arg arity — BACKLOG item 210
// ------------------------------------------------------------------ //

#[test]
fn unreachable_with_no_args_typechecks() {
    check("module A\nfn f(): Int = unreachable()").unwrap();
}

#[test]
fn todo_with_no_args_typechecks() {
    check("module A\nfn f(): Int = todo()").unwrap();
}

#[test]
fn unreachable_with_a_text_arg_is_rejected() {
    // The spec's own table (§9.1) documents `unreachable`/`todo` as
    // zero-arg (`fn(): Nothing`) — distinct from `panic(msg: Text)`, which
    // does take one. Before this fix, this was the *only* form that
    // typechecked at all (and it then failed the C compile stage).
    let errs = check_err("module A\nfn f(): Int = unreachable(\"oops\")");
    assert!(!errs.is_empty(), "unreachable(\"msg\") should be rejected — it takes no arguments");
}

#[test]
fn todo_with_a_text_arg_is_rejected() {
    let errs = check_err("module A\nfn f(): Int = todo(\"not done\")");
    assert!(!errs.is_empty(), "todo(\"msg\") should be rejected — it takes no arguments");
}

#[test]
fn panic_still_requires_a_text_arg() {
    // panic's own signature is unaffected by this item — only
    // unreachable/todo changed.
    check("module A\nfn f(): Int = panic(\"real message\")").unwrap();
    let errs = check_err("module A\nfn f(): Int = panic()");
    assert!(!errs.is_empty(), "panic() with no message should still be rejected");
}

// BACKLOG item 235 — fixed-width numeric literal inference. A bare int/float
// literal at an *annotated* position (`val`/`var`/fn-return) types directly
// as the declared fixed-width type instead of the rigid default `Int`/`Float`.
#[test]
fn int_literal_types_as_each_fixed_width_int_annotation() {
    check("module A\nfn f(): Unit = { val a: Int8 = 100 }").unwrap();
    check("module A\nfn f(): Unit = { val a: Int16 = 100 }").unwrap();
    check("module A\nfn f(): Unit = { val a: Int32 = 100 }").unwrap();
    check("module A\nfn f(): Unit = { val a: UInt = 100 }").unwrap();
}

#[test]
fn float_literal_types_as_float32_annotation() {
    check("module A\nfn f(): Unit = { val a: Float32 = 3.5 }").unwrap();
}

#[test]
fn negative_int_literal_types_as_signed_fixed_width_annotation() {
    check("module A\nfn f(): Unit = { val a: Int32 = -5000 }").unwrap();
}

#[test]
fn negative_int_literal_against_uint_is_still_rejected() {
    // Unsigned — a negated literal must not silently widen into it.
    let errs = check_err("module A\nfn f(): Unit = { val a: UInt = -5 }");
    assert!(!errs.is_empty(), "-5 should not typecheck as UInt");
}

#[test]
fn float_literal_against_int8_is_still_rejected() {
    // No cross-matching between the int and float literal families.
    let errs = check_err("module A\nfn f(): Unit = { val a: Int8 = 3.14 }");
    assert!(!errs.is_empty(), "3.14 should not typecheck as Int8");
}

#[test]
fn var_declaration_also_gets_fixed_width_literal_inference() {
    check("module A\nfn f(): Unit = { var a: Int16 = 200 }").unwrap();
}

#[test]
fn fn_return_position_also_gets_fixed_width_literal_inference() {
    check("module A\nfn f(): Int8 = 42").unwrap();
    check("module A\nfn g(): Int32 = -5000").unwrap();
}

#[test]
fn non_literal_expression_does_not_get_fixed_width_inference() {
    // The narrow fix is literal-only — an ordinary Int-typed expression
    // must still be rejected against a fixed-width annotation, not
    // silently truncated.
    let errs = check_err("module A\nfn f(): Unit = { val x = 100\n val a: Int8 = x }");
    assert!(!errs.is_empty(), "a non-literal Int value must not silently coerce to Int8");
}

// BACKLOG item 248 — `UnOp::Neg` previously unified its operand against
// Ty::Int unconditionally, so negating any other numeric type (most
// surprisingly plain Float) never typechecked at all.
#[test]
fn negating_a_float_typechecks() {
    check("module A\nfn f(): Unit = { val y: Float = -3.5 }").unwrap();
    check("module A\nfn f(): Unit = { val z = -3.5 }").unwrap();
}

#[test]
fn negating_each_fixed_width_numeric_type_typechecks() {
    check("module A\nfn f(a: Int8): Unit = { val n: Int8 = -a }").unwrap();
    check("module A\nfn f(a: Int16): Unit = { val n: Int16 = -a }").unwrap();
    check("module A\nfn f(a: Int32): Unit = { val n: Int32 = -a }").unwrap();
    check("module A\nfn f(a: Float32): Unit = { val n: Float32 = -a }").unwrap();
}

#[test]
fn negating_a_decimal_now_typechecks() {
    // Originally a clean rejection (BACKLOG item 248's own scope, when
    // Decimal negation had no codegen support at all) — item 249 gave
    // `UnOp::Neg` on Decimal a real codegen path (`certo_decimal_negate`),
    // so this now correctly typechecks instead.
    check("module A\nfn f(): Unit = { val d: Decimal = d\"1.5\"\n val n = -d }").unwrap();
}

#[test]
fn negating_a_text_still_cleanly_rejected() {
    let errs = check_err("module A\nfn f(): Unit = { val n = -\"hello\" }");
    assert!(!errs.is_empty(), "negating a Text should still be rejected");
}

#[test]
fn negating_an_unannotated_literal_still_defaults_to_int() {
    // Preserves the pre-existing default behaviour for the common case —
    // an unannotated negative literal with nothing else pinning its type.
    check("module A\nfn f(): Int = -5").unwrap();
}

// BACKLOG item 245 — `EXPR in LIST-EXPR` / `EXPR not in LIST-EXPR`.
#[test]
fn membership_in_a_matching_element_type_typechecks_as_bool() {
    check("module A\nfn f(): Bool = 2 in [1, 2, 3]").unwrap();
}

#[test]
fn membership_not_in_also_typechecks_as_bool() {
    check("module A\nfn f(): Bool = 2 not in [1, 2, 3]").unwrap();
}

#[test]
fn membership_element_type_mismatch_is_rejected() {
    let errs = check_err("module A\nfn f(): Unit = { val x = \"hello\"\n val y = x in [1, 2, 3] }");
    assert!(!errs.is_empty(), "a Text left side against a List<Int> right side should be rejected");
}

#[test]
fn membership_right_side_must_be_a_list() {
    let errs = check_err("module A\nfn f(): Unit = { val y = 5 in 5 }");
    assert!(!errs.is_empty(), "the right side of `in` must be a list");
}

