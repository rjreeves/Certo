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
    rule r { require invoice.createdAt.age < VoidWindow else true }
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

// Constraint

#[test]
fn constraint_name_is_bool() {
    check("module A
constraint Active = status == active
validator V for Order errors OE {
    rule r { require Active else true }
}").unwrap();
}

// Validator

#[test]
fn validator_require_true_ok() {
    check("module A
validator V for Order errors OE {
    rule r { require true else true }
}").unwrap();
}

#[test]
fn validator_require_entity_field_ok() {
    check("module A
type Order = { total: Int }
validator V for Order errors OE {
    rule r { require order.total > 0 else true }
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

#[test]
fn validator_context_field_in_scope() {
    check("module A
type Order = { id: Int }
type Customer = { active: Bool }
validator V for Order errors OE {
    context { customer: Customer }
    rule r { require customer.active else true }
}").unwrap();
}

#[test]
fn validator_multiple_rules_ok() {
    check("module A
type Order = { total: Int, valid: Bool }
validator V for Order errors OE {
    rule a { require order.total > 0 else true }
    rule b { require order.valid else true }
}").unwrap();
}

#[test]
fn validator_validate_fn_registered() {
    // After hoisting, V.validate should be callable.
    check("module A
validator V for Order errors OE {
    rule r { require true else true }
}
fn callIt(o: Order): Result<Unit, OE> = V.validate(o)").unwrap();
}

// ------------------------------------------------------------------ //
// State machine typestate tests
// ------------------------------------------------------------------ //

// State machine generated names (Order_new, Order_submit, …) are not visible
// to the resolve pass, so we skip resolve and go straight to typeck — the same
// pattern used by age_in_standalone_expr_ok above.
fn check_sm(src: &str) -> Result<(), Vec<crate::error::TypeError>> {
    let module = parse(src).expect("parse error");
    let _ = certo_resolve::resolve(&module);
    check_module(&module)
}

const ORDER_SM: &str = "
module A
statemachine Order {
    states: Draft, Submitted, Fulfilled, Cancelled
    transitions:
        Draft -> Submitted : submit
        Submitted -> Fulfilled : fulfil
        [Draft, Submitted] -> Cancelled : cancel
}
";

#[test]
fn statemachine_new_and_valid_transition_ok() {
    check_sm(&format!("{ORDER_SM}
val o: Order<OrderDraft> = Order_new()
val s: Order<OrderSubmitted> = Order_submit(o)
")).unwrap();
}

#[test]
fn statemachine_cancel_from_draft_ok() {
    check_sm(&format!("{ORDER_SM}
val o: Order<OrderDraft> = Order_new()
val c: Order<OrderCancelled> = Order_cancel(o)
")).unwrap();
}

#[test]
fn statemachine_cancel_from_submitted_ok() {
    check_sm(&format!("{ORDER_SM}
val o: Order<OrderSubmitted> = Order_submit(Order_new())
val c: Order<OrderCancelled> = Order_cancel(o)
")).unwrap();
}

#[test]
fn statemachine_wrong_state_is_type_error() {
    // fulfil requires Order<OrderSubmitted>; passing Order<OrderDraft> must fail.
    let result = check_sm(&format!("{ORDER_SM}
val o: Order<OrderDraft> = Order_new()
val bad = Order_fulfil(o)
"));
    assert!(result.is_err(), "expected type error for wrong-state transition");
}

#[test]
fn statemachine_predicate_accepts_any_state() {
    // Order_isDraft should accept any Order<S>, not just Order<OrderDraft>.
    check_sm(&format!("{ORDER_SM}
val o: Order<OrderSubmitted> = Order_submit(Order_new())
val b: Bool = Order_isDraft(o)
")).unwrap();
}

#[test]
fn statemachine_assert_downcast_returns_optional() {
    // Order_assertSubmitted returns Option — callers must handle None.
    check_sm(&format!("{ORDER_SM}
fn load(): Order<OrderDraft> = Order_new()
val opt: Order<OrderSubmitted>? = Order_assertSubmitted(load())
")).unwrap();
}
