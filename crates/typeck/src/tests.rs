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
