use crate::format_source;

fn fmt(src: &str) -> String {
    format_source(src).expect("parse error")
}

fn assert_contains(out: &str, needle: &str) {
    assert!(out.contains(needle), "expected {:?} in:\n{}", needle, out);
}

fn assert_not_contains(out: &str, needle: &str) {
    assert!(!out.contains(needle), "did not expect {:?} in:\n{}", needle, out);
}

// ------------------------------------------------------------------ //
// Module header
// ------------------------------------------------------------------ //

#[test]
fn module_header() {
    let out = fmt("module MyApp.Orders");
    assert_contains(&out, "module MyApp.Orders");
}

#[test]
fn imports_formatted() {
    let out = fmt("module A\nimport Stdlib.Collections.{ List, Map }");
    assert_contains(&out, "import Stdlib.Collections.{ List, Map }");
}

#[test]
fn aliased_import() {
    let out = fmt("module A\nimport Stdlib.DateTime as DT");
    assert_contains(&out, "import Stdlib.DateTime as DT");
}

#[test]
fn import_when_condition_round_trips() {
    // BACKLOG item 144 — `import when [target = "wasm"]` must round-trip
    // through the formatter unchanged, not get silently dropped.
    let out = fmt("module A\nimport when [target = \"wasm\"] Stdlib.Text");
    assert_contains(&out, "import when [target = \"wasm\"] Stdlib.Text");
}

// ------------------------------------------------------------------ //
// fn declarations
// ------------------------------------------------------------------ //

#[test]
fn simple_fn() {
    let out = fmt("module A\nfn answer(): Int = 42");
    assert_contains(&out, "fn answer(): Int = 42");
}

#[test]
fn fn_with_params() {
    let out = fmt("module A\nfn add(x: Int, y: Int): Int = x + y");
    assert_contains(&out, "fn add(x: Int, y: Int): Int = x + y");
}

#[test]
fn fn_idempotent() {
    let src  = "module A\n\nfn add(x: Int, y: Int): Int = x + y\n";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "formatter is not idempotent");
}

#[test]
fn fn_with_if_expr() {
    let out = fmt("module A\nfn max(a: Int, b: Int): Int = if a > b then a else b");
    assert_contains(&out, "if a > b then a else b");
}

#[test]
fn fn_with_block_body() {
    let out = fmt("module A\nfn greet(name: Text): Text = {\n    val msg = \"Hello\"\n    msg\n}");
    assert_contains(&out, "val msg");
}

// ------------------------------------------------------------------ //
// type declarations
// ------------------------------------------------------------------ //

#[test]
fn record_type() {
    let out = fmt("module A\ntype Point = { x: Int, y: Int }");
    assert_contains(&out, "type Point = {");
    assert_contains(&out, "x: Int");
    assert_contains(&out, "y: Int");
}

#[test]
fn value_object_annotation_round_trips() {
    // BACKLOG item 149 — `@valueObject`/`@aggregate` are preserved by the
    // formatter, not silently dropped.
    let out = fmt("module A\n@valueObject\ntype Money = { amount: Int }");
    assert_contains(&out, "@valueObject");
    assert_contains(&out, "type Money = {");
}

// BACKLOG item 175 — `certo fmt` previously printed the parser's
// synthesized trailing `impl` block (BACKLOG items 143/150) as a second,
// literal top-level declaration, duplicating in-body `fn` methods and
// `computed` properties instead of round-tripping them from inside the
// `type { ... }` body they were written in.

#[test]
fn in_body_fn_method_round_trips_inside_type_body_no_duplicate_impl() {
    let out = fmt(
        "module A\ntype Cart = {\n    total: Int\n    fn addTotal(c: Cart, amount: Int): Int = c.total + amount\n}"
    );
    assert_contains(&out, "type Cart = {");
    assert_contains(&out, "fn addTotal(c: Cart, amount: Int): Int");
    // The synthesized trailing `impl Cart { ... }` must not also be printed
    // as a separate top-level declaration.
    assert_not_contains(&out, "impl Cart {");
}

#[test]
fn computed_property_round_trips_without_duplicate_impl_block() {
    let out = fmt(
        "module A\ntype Invoice = {\n    paidAt: Text?\n    computed isPaid: Bool = Option.isSome(paidAt)\n}"
    );
    assert_contains(&out, "type Invoice = {");
    assert_contains(&out, "computed isPaid: Bool = Option.isSome(paidAt)");
    // No synthesized `impl Invoice { ... isPaid ... }` block duplicating
    // the accessor — the original bug would have re-printed it there too,
    // producing output that fails to recompile (two `isPaid` definitions).
    assert_not_contains(&out, "impl Invoice {");
}

#[test]
fn type_with_both_in_body_fn_and_computed_round_trips_without_duplicate_impl() {
    let out = fmt(
        "module A\ntype Cart = {\n    total: Int\n    computed isEmpty: Bool = total == 0\n    fn addTotal(c: Cart, amount: Int): Int = c.total + amount\n}"
    );
    assert_contains(&out, "type Cart = {");
    assert_contains(&out, "fn addTotal(c: Cart, amount: Int): Int");
    assert_contains(&out, "computed isEmpty: Bool = total == 0");
    assert_not_contains(&out, "impl Cart {");
}

#[test]
fn hand_written_standalone_impl_block_still_prints_normally() {
    // Regression: only the *synthesized* trailing impl is skipped — a real,
    // hand-written `impl X { ... }` block (not attached to any `type`'s
    // in-body methods) must still round-trip as its own declaration.
    let out = fmt(
        "module A\ntype Box = { value: Int }\n\nimpl Box {\n    fn get(b: Box): Int = b.value\n}"
    );
    assert_contains(&out, "impl Box {");
    assert_contains(&out, "fn get(b: Box): Int");
}

// BACKLOG item 176 — `certo fmt` previously dropped a record literal's type
// name entirely, and formatted a `.with(...)` copy-update using a bare
// `with` keyword that isn't valid expression syntax anywhere in this
// language — both produced output that fails to re-parse.

#[test]
fn record_literal_keeps_its_type_name() {
    let out = fmt("module A\ntype Point = { x: Int, y: Int }\nfn f(): Point = Point { x: 1, y: 2 }");
    assert_contains(&out, "Point { x: 1, y: 2 }");
    assert_not_contains(&out, "= { x: 1, y: 2 }");
}

#[test]
fn with_copy_update_round_trips_as_dot_with_call() {
    let out = fmt(
        "module A\ntype Order = { total: Int, status: Text }\nfn f(o: Order): Order = o.with(status: \"closed\")"
    );
    assert_contains(&out, "o.with(status: \"closed\")");
    assert_not_contains(&out, " with {");
}

#[test]
fn record_spread_with_explicit_type_name_round_trips() {
    let out = fmt(
        "module A\ntype Order = { total: Int, status: Text }\nfn f(o: Order): Order = Order { ..o, status: \"closed\" }"
    );
    assert_contains(&out, "Order { ..o, status: \"closed\" }");
}

#[test]
fn sum_type() {
    let out = fmt("module A\ntype Color = | Red | Green | Blue");
    assert_contains(&out, "type Color =");
    assert_contains(&out, "| Red");
    assert_contains(&out, "| Green");
    assert_contains(&out, "| Blue");
}

#[test]
fn decimal_param_round_trips() {
    // BACKLOG item 128 — `Decimal(19, 4)` must round-trip through the
    // formatter unchanged, not collapse to bare `Decimal`.
    let out = fmt("module A\ntype Money = { amount: Decimal(19, 4) }");
    assert_contains(&out, "amount: Decimal(19, 4)");
}

#[test]
fn bounded_text_param_round_trips() {
    // BACKLOG item 147 — `BoundedText(255)` must round-trip through the
    // formatter unchanged, not collapse to bare `Text`.
    let out = fmt("module A\ntype User = { zip: BoundedText(10) }");
    assert_contains(&out, "zip: BoundedText(10)");
}

#[test]
fn type_alias() {
    let out = fmt("module A\ntype Name = Text");
    assert_contains(&out, "type Name = Text");
}

// ------------------------------------------------------------------ //
// val / var
// ------------------------------------------------------------------ //

#[test]
fn val_decl() {
    let out = fmt("module A\nval pi: Int = 3");
    assert_contains(&out, "val pi: Int = 3");
}

// ------------------------------------------------------------------ //
// match expression
// ------------------------------------------------------------------ //

#[test]
fn match_expr() {
    let out = fmt("module A\nfn describe(n: Int): Text = match n {\n    0 => \"zero\"\n    _ => \"other\"\n}");
    assert_contains(&out, "match n {");
    assert_contains(&out, "0 => \"zero\"");
    assert_contains(&out, "_ => \"other\"");
}

// ------------------------------------------------------------------ //
// Operators
// ------------------------------------------------------------------ //

#[test]
fn binop_spacing() {
    let out = fmt("module A\nfn calc(x: Int): Int = x * 2 + 1");
    assert_contains(&out, "x * 2 + 1");
}

#[test]
fn unop_not() {
    let out = fmt("module A\nfn negate(b: Bool): Bool = not b");
    assert_contains(&out, "not b");
}

// ------------------------------------------------------------------ //
// List / tuple literals
// ------------------------------------------------------------------ //

#[test]
fn list_literal() {
    let out = fmt("module A\nval nums: List<Int> = [1, 2, 3]");
    assert_contains(&out, "[1, 2, 3]");
}

#[test]
fn tuple_literal() {
    let out = fmt("module A\nfn pair(): (Int, Int) = (1, 2)");
    assert_contains(&out, "(1, 2)");
}

// ------------------------------------------------------------------ //
// Idempotency for several constructs
// ------------------------------------------------------------------ //

#[test]
fn record_type_idempotent() {
    let src  = "module A\ntype Point = { x: Int, y: Int }";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "record type formatting not idempotent");
}

#[test]
fn match_idempotent() {
    let src  = "module A\nfn f(n: Int): Text = match n {\n    0 => \"zero\"\n    _ => \"other\"\n}";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "match formatting not idempotent");
}

// ------------------------------------------------------------------ //
// `withTimeout(duration) { body }` — BACKLOG item 122
// ------------------------------------------------------------------ //

#[test]
fn with_timeout_round_trips() {
    let out = fmt("module A\nasync fn f(): Int? = withTimeout(Duration.seconds(5)) {\n  work()\n}");
    assert_contains(&out, "withTimeout(Duration.seconds(5)) {");
    assert_contains(&out, "work()");
}

#[test]
fn with_timeout_idempotent() {
    let src  = "module A\nasync fn f(): Int? = withTimeout(Duration.seconds(5)) {\n  work()\n}";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "withTimeout formatting not idempotent");
}

// ------------------------------------------------------------------ //
// `form` declarations — BACKLOG item 190. `fmt_form` used to be a hardcoded
// stub (`form Name { // ... }`) that discarded every real field, target,
// pk, onSubmit, and onSuccess regardless of what was actually parsed —
// running `certo fmt` in place on a file with a `form` declaration silently
// destroyed its content. These pin the real round-trip.
// ------------------------------------------------------------------ //

#[test]
fn form_flat_fields_round_trip() {
    let out = fmt("module A\nform CreateCustomer -> Customer {\n    custName: Text\n    custPhone: Text\n}");
    assert_contains(&out, "form CreateCustomer -> Customer {");
    assert_contains(&out, "custName: Text");
    assert_contains(&out, "custPhone: Text");
    assert_not_contains(&out, "// ...");
}

#[test]
fn form_without_target_arrow_round_trips_without_one() {
    let out = fmt("module A\nform CreateCustomer {\n    custName: Text\n}");
    assert_contains(&out, "form CreateCustomer {");
    assert_not_contains(&out, "->");
}

#[test]
fn form_pk_on_submit_on_success_round_trip() {
    let out = fmt(
        "module A\nfn createProduct(): Unit = {}\nform ProductForm -> Product {\n    name: Text\n    pk: name\n    onSubmit: createProduct\n}");
    assert_contains(&out, "pk: name");
    assert_contains(&out, "onSubmit: createProduct");
}

#[test]
fn form_empty_body_round_trips_to_empty_braces() {
    let out = fmt("module A\nform Empty -> Customer {}");
    assert_contains(&out, "form Empty -> Customer {}");
    assert_not_contains(&out, "// ...");
}

#[test]
fn form_idempotent() {
    let src  = "module A\nfn createProduct(): Unit = {}\nform ProductForm -> Product {\n    name: Text\n    price: Int\n    pk: name\n    onSubmit: createProduct\n}";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "form formatting not idempotent");
}

// ------------------------------------------------------------------ //
// Nested `field NAME { ... }` blocks — BACKLOG item 166. A field with any
// nested-only metadata (label/placeholder/options/rows) must round-trip
// through the real `field NAME { ... }` block, or that metadata would be
// silently dropped every time `certo fmt` runs on it — the same class of
// bug item 190 fixed for the form declaration as a whole.
// ------------------------------------------------------------------ //

#[test]
fn nested_field_metadata_round_trips() {
    let out = fmt(
        "module A\nform ProductForm -> Product {\n    field price {\n        label: \"Price\"\n        placeholder: \"0.00\"\n        type: CurrencyInput(USD)\n        rows: 3\n    }\n}"
    );
    assert_contains(&out, "field price {");
    assert_contains(&out, "label: \"Price\"");
    assert_contains(&out, "placeholder: \"0.00\"");
    assert_contains(&out, "type: CurrencyInput(USD)");
    assert_contains(&out, "rows: 3");
}

#[test]
fn flat_field_without_nested_metadata_still_prints_the_flat_shorthand() {
    // No regression for the overwhelmingly common case: a field with only
    // a bare type (no label/placeholder/options/rows) must keep printing
    // as `name: Type`, not get needlessly wrapped in a `field { }` block.
    let out = fmt("module A\nform CreateCustomer -> Customer {\n    custName: Text\n}");
    assert_contains(&out, "custName: Text");
    assert_not_contains(&out, "field custName");
}

#[test]
fn flat_and_nested_form_fields_round_trip_together() {
    let out = fmt(
        "module A\nform ProductForm -> Product {\n    name: Text\n    field description {\n        type: RichText\n        rows: 6\n    }\n}"
    );
    assert_contains(&out, "name: Text");
    assert_contains(&out, "field description {");
    assert_contains(&out, "type: RichText");
    assert_contains(&out, "rows: 6");
}

#[test]
fn nested_field_with_no_keys_round_trips_to_empty_braces() {
    let out = fmt("module A\nform ProductForm -> Product {\n    field name {}\n}");
    assert_contains(&out, "field name {}");
}

#[test]
fn nested_field_idempotent() {
    let src  = "module A\nform ProductForm -> Product {\n    field price {\n        label: \"Price\"\n        type: CurrencyInput(USD)\n        rows: 3\n    }\n}";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "nested field formatting not idempotent");
}

// ------------------------------------------------------------------ //
// Leading-dot shorthand round-trips — BACKLOG item 203
// ------------------------------------------------------------------ //

#[test]
fn leading_dot_field_shorthand_round_trips() {
    // Regression for a real, confirmed leak: without this, `.lineTotal`
    // formatted back out as `__dot => __dot.lineTotal`, exposing the
    // parser's own internal synthesized parameter name in the user's source.
    let out = fmt("module A\nfn f(items: List<Item>): Int = items.sumBy(.lineTotal)");
    assert_contains(&out, ".lineTotal");
    assert_not_contains(&out, "__dot");
}

#[test]
fn leading_dot_method_call_shorthand_round_trips() {
    let out = fmt("module A\nfn g(items: List<Item>, subtotal: Int): List<Int> = items.map(.discount(subtotal))");
    assert_contains(&out, ".discount(subtotal)");
    assert_not_contains(&out, "__dot");
}

#[test]
fn leading_dot_shorthand_as_labeled_arg_round_trips() {
    let out = fmt("module A\nfn f(items: List<Item>, newItem: Item): List<Item> = items.upsert(newItem, on: .productId)");
    assert_contains(&out, "on: .productId");
    assert_not_contains(&out, "__dot");
}

#[test]
fn leading_dot_shorthand_idempotent() {
    let src  = "module A\nfn f(items: List<Item>): Int = items.sumBy(.lineTotal)";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "leading-dot shorthand formatting not idempotent");
}

#[test]
fn a_hand_written_lambda_still_prints_normally() {
    // Regression guard: an ordinary, hand-written lambda (not matching the
    // parser's own exact desugar shape) must still print as a lambda.
    let out = fmt("module A\nfn f(items: List<Item>): Int = items.sumBy((item) => item.lineTotal)");
    // The parser has no bare `x => ...` form, so the parameter keeps its parentheses.
    assert_contains(&out, "(item) => item.lineTotal");
}

// ------------------------------------------------------------------ //
// BACKLOG item 222 — `ruleTest`/`validatorTest` real round-tripping.
// Previously formatted as a literal `{ ... }` placeholder, destroying the
// real `entity`/`context`/`expect` body on every `certo fmt` run.
// ------------------------------------------------------------------ //

#[test]
fn rule_test_round_trips_its_real_body_not_a_placeholder() {
    let out = fmt(
        "module A\nruleTest V.someRule \"passes\" {\n    entity: 1\n    context: 2\n    expect: pass\n}"
    );
    assert_not_contains(&out, "{ ... }");
    assert_contains(&out, "entity: 1");
    assert_contains(&out, "context: 2");
    assert_contains(&out, "expect: pass");
}

#[test]
fn validator_test_round_trips_its_real_body_not_a_placeholder() {
    let out = fmt(
        "module A\nvalidatorTest V \"passes\" {\n    entity: 1\n    context: 2\n    expect: pass\n}"
    );
    assert_not_contains(&out, "{ ... }");
    assert_contains(&out, "entity: 1");
    assert_contains(&out, "context: 2");
    assert_contains(&out, "expect: pass");
}

#[test]
fn rule_test_expect_fail_with_value_round_trips() {
    let out = fmt(
        "module A\nruleTest V.someRule \"fails\" {\n    entity: 1\n    context: 2\n    expect: fail with OE.Bad\n}"
    );
    assert_contains(&out, "expect: fail with OE.Bad");
}

#[test]
fn rule_test_expect_fail_without_value_round_trips() {
    let out = fmt(
        "module A\nruleTest V.someRule \"fails\" {\n    entity: 1\n    context: 2\n    expect: fail\n}"
    );
    assert_contains(&out, "expect: fail\n");
    assert_not_contains(&out, "expect: fail with");
}

#[test]
fn rule_test_is_idempotent() {
    let src  = "module A\nruleTest V.someRule \"passes\" {\n    entity: 1\n    context: 2\n    expect: pass\n}";
    let out1 = fmt(src);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "ruleTest formatting not idempotent");
}

// ------------------------------------------------------------------ //
// BACKLOG item 279 — `validator` declaration real round-tripping.
// Previously formatted as `validator V for ... errors ... { rule active
// { ... } }`, destroying the real for/errors types and every rule's
// after/overrides/priority/require/else body on every `certo fmt` run.
// ------------------------------------------------------------------ //

const VALIDATOR_DECL_SRC: &str = "\
    module A\n\
    validator V for Order errors OE {\n\
        context {\n\
            customer: Customer loaded by db.customers.find(order.customerId)\n\
        }\n\
        rule active {\n\
            after other\n\
            overrides legacy\n\
            priority 5\n\
            require customer.status == \"active\"\n\
            else OE.NotActive\n\
        }\n\
        rule creditLimit {\n\
            require order.total <= 100\n\
            else OE.OverLimit\n\
        }\n\
    }";

#[test]
fn validator_decl_round_trips_its_real_for_and_errors_types() {
    let out = fmt(VALIDATOR_DECL_SRC);
    assert_not_contains(&out, "for ... errors ...");
    assert_contains(&out, "validator V for Order errors OE");
}

#[test]
fn validator_decl_round_trips_context_block() {
    let out = fmt(VALIDATOR_DECL_SRC);
    assert_contains(&out, "context {");
    assert_contains(&out, "customer: Customer loaded by db.customers.find(order.customerId)");
}

#[test]
fn validator_decl_round_trips_rule_bodies_not_a_placeholder() {
    let out = fmt(VALIDATOR_DECL_SRC);
    assert_not_contains(&out, "{ ... }");
    assert_contains(&out, "after other");
    assert_contains(&out, "overrides legacy");
    assert_contains(&out, "priority 5");
    assert_contains(&out, "require customer.status == \"active\"");
    assert_contains(&out, "else OE.NotActive");
}

#[test]
fn validator_decl_rule_without_after_overrides_priority_omits_them() {
    let out = fmt(VALIDATOR_DECL_SRC);
    assert_contains(&out, "rule creditLimit {");
    assert_contains(&out, "require order.total <= 100");
    assert_contains(&out, "else OE.OverLimit");
}

#[test]
fn validator_decl_with_trigger_round_trips() {
    let out = fmt(
        "module A\nvalidator V for Order errors OE trigger on Update when status == Submitted {\n    rule active { require true else OE.NotActive }\n}"
    );
    assert_contains(&out, "trigger on Update when status == Submitted");
}

#[test]
fn validator_decl_is_idempotent() {
    let out1 = fmt(VALIDATOR_DECL_SRC);
    let out2 = fmt(&out1);
    assert_eq!(out1, out2, "validator formatting not idempotent");
}

// ------------------------------------------------------------------ //
// Formatting must never change what a program means.
// BACKLOG item 403 (grouping parentheses dropped) and 404 (f-string
// interpolation printed as `${x}`, lambda parameter parentheses stripped).
//
// The strongest check available without a compiler: the formatted source must
// parse to exactly the same program as the original (spans blanked out, since
// layout legitimately changes), and formatting must be stable.
// ------------------------------------------------------------------ //

/// Debug-print the parsed module with every span (`Span { start: 7, end: 8 }`)
/// replaced by `_`, so two parses of the same program compare equal however
/// it's laid out.
fn shape(src: &str) -> String {
    let module = certo_parser::parse(src)
        .unwrap_or_else(|e| panic!("parse error {:?} in:\n{}", e, src));
    let dbg = format!("{:?}", module);
    let mut out = String::new();
    let mut rest = dbg.as_str();
    while let Some(i) = rest.find("Span { start: ") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let close = tail.find(" }").expect("unterminated Span in Debug output") + 2;
        out.push('_');
        rest = &tail[close..];
    }
    out.push_str(rest);
    out
}

/// Where two `shape` strings first differ, with a little context from each.
fn first_difference(a: &str, b: &str) -> String {
    let at = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let ctx = |s: &str| -> String {
        let from = at.saturating_sub(120);
        s.chars().skip(from).take(300).collect()
    };
    format!("first difference at char {}:\n  before: ...{}\n  after:  ...{}", at, ctx(a), ctx(b))
}

/// Formats `src`, requires the result to parse to the same program and to be
/// a fixed point of the formatter, and returns it.
fn assert_preserved(src: &str) -> String {
    let once = fmt(src);
    let (before, after) = (shape(src), shape(&once));
    assert!(
        before == after,
        "formatting changed the program\n--- before ---\n{}\n--- after ---\n{}\n{}", src, once, first_difference(&before, &after)
    );
    assert_eq!(fmt(&once), once, "formatting is not stable for:\n{}", src);
    once
}

/// A function whose body is `expr` (types are irrelevant: only parsing matters).
fn body(expr: &str) -> String {
    format!(
        "module T\n\nfn f(a: Int, b: Int, c: Int, p: Bool, q: Bool, r: Bool, xs: List<Int>): Int = {}\n",
        expr
    )
}

#[test]
fn arithmetic_grouping_parentheses_are_kept() {
    for e in [
        "(a + b) * c", "a * (b + c)", "(a + b) * (c - a)", "a - (b - c)", "a / (b * c)",
        "a % (b + c)", "(a + b) * c - (a - b) / (c + 1)", "(a ++ b) + c", "a ++ (b + c)",
    ] {
        assert_preserved(&body(e));
    }
}

#[test]
fn the_motivating_case_prints_with_its_parentheses() {
    let out = assert_preserved(&body("(a + b) * c"));
    assert_contains(&out, "(a + b) * c");
}

#[test]
fn power_and_unary_minus_keep_the_right_grouping() {
    // `**` is right-associative; unary minus binds tighter than `**`.
    for e in ["(a ** b) ** c", "a ** (b ** c)", "a ** b ** c", "(a + b) ** c", "-(a + b)", "-(-a)", "(-a) ** b", "-(a ** b)"] {
        assert_preserved(&body(e));
    }
}

#[test]
fn logical_grouping_parentheses_are_kept() {
    for e in [
        "(p or q) and r", "p or (q and r)", "p and (q or r)", "(p and q) or r",
        "not (p and q)", "not (p or q)", "not (a > b)", "not (p ?? q)",
        "(p ?? q) ?? r", "p ?? (q ?? r)", "(p or q) ?? r", "(p and q) ?? r",
    ] {
        assert_preserved(&body(e));
    }
}

#[test]
fn comparison_range_and_membership_grouping_is_kept() {
    for e in [
        "(a < b) == (b < c)", "a < (b == c)", "a == (b == c)", "(a == b) == c",
        "a in (b ++ c)", "(a .. b) .. c", "a .. (b .. c)", "(a < b) .. c",
    ] {
        assert_preserved(&body(e));
    }
}

#[test]
fn pipes_keep_the_grouping_they_need() {
    for e in ["(a |> f) |> g", "a |> f |> g", "(a |> f) + 1", "(a |> f).x", "a |> (p or q)"] {
        assert_preserved(&body(e));
    }
}

#[test]
fn open_ended_forms_are_parenthesised_as_operands() {
    // An `if` or a lambda swallows everything to its right, so as an operand
    // it must be wrapped or the following operator moves inside it.
    for e in [
        "(if p then a else b) + c", "c + (if p then a else b)",
        "((x) => x + 1)(a)", "(if p then f else g)(a)",
    ] {
        assert_preserved(&body(e));
    }
}

#[test]
fn postfix_receivers_keep_their_parentheses() {
    for e in [
        "(a + b).x", "(f(a) ?? b).name", "(a + b)?", "(a |> f)?", "(-a).x", "(not p).x",
        "(await f(a)).name", "await (a + b)", "(spawn f(a)).x",
    ] {
        assert_preserved(&body(e));
    }
}

#[test]
fn parentheses_that_are_not_needed_are_dropped() {
    // The AST has no parenthesis node, so redundant ones can't be told apart
    // from needed ones; the printer emits only the ones the grammar requires.
    assert_contains(&assert_preserved(&body("(a * b) + c")), "a * b + c");
    assert_contains(&assert_preserved(&body("(a + b) - c")), "a + b - c");
    assert_contains(&assert_preserved(&body("a ** (b ** c)")), "a ** b ** c");
    assert_contains(&assert_preserved(&body("(p and q) or r")), "p and q or r");
    assert_contains(&assert_preserved(&body("not (a > b)")), "not a > b");
}

#[test]
fn lambda_parameters_always_keep_their_parentheses() {
    // The parser has no bare `x => ...` form (item 404).
    for e in [
        "List.map(xs, (x) => x * 2)", "List.map(xs, (x: Int) => x * 2)",
        "List.fold(xs, 0, (acc, x) => acc + x)", "List.map(xs, (a, b: Int) => a)", "(() => 1)()",
    ] {
        assert_preserved(&body(e));
    }
    let out = assert_preserved(&body("List.map(xs, (x) => x * 2)"));
    assert_contains(&out, "(x) => x * 2");
    assert_not_contains(&out, " x => ");
}

#[test]
fn fstring_interpolation_stays_plain_braces() {
    // Item 404: `{x}` was printed as `${x}`, which prints a literal `$`.
    let src = "module T\n\nfn f(a: Int, b: Int): Text = f\"x is {a} and {a + b}\"\n";
    let out = assert_preserved(src);
    assert_contains(&out, "f\"x is {a} and {a + b}\"");
    assert_not_contains(&out, "${");
}

#[test]
fn fstring_interpolated_expressions_keep_their_own_grouping() {
    let src = "module T\n\nfn f(a: Int, b: Int): Text = f\"{(a + b) * 2} / {-(a - b)}\"\n";
    let out = assert_preserved(src);
    assert_contains(&out, "{(a + b) * 2}");
    assert_contains(&out, "{-(a - b)}");
}

#[test]
fn fstring_literal_text_and_escapes_are_kept_verbatim() {
    // The parser keeps f-string text as the raw source slice, so re-escaping
    // it turned `\n` into `\\n` and `\"` into `\\\"`.
    let src = "module T\n\nfn f(a: Int): Text = f\"a\\tb \\\"q\\\" {a}\\n\"\n";
    let out = assert_preserved(src);
    assert_contains(&out, "f\"a\\tb \\\"q\\\" {a}\\n\"");
}

#[test]
fn realistic_function_round_trips_unchanged() {
    // A bigger mixed program: whatever the formatter does must still parse to
    // the same program (match arms, guards, records, closures, calls).
    let src = "module Shop\n\nimport Stdlib.Core\n\n\
        type Item = { name: Text, price: Int, qty: Int }\n\n\
        fn total(items: List<Item>, discount: Int): Int = List.fold(items, 0, (acc, i) => acc + (i.price - discount) * i.qty)\n\n\
        fn label(i: Item): Text = if (i.price * i.qty) > 100 then f\"big: {i.name}\" else f\"small: {i.name}\"\n\n\
        fn ok(p: Bool, q: Bool, r: Bool): Bool = not (p and q) or (q and not r)\n";
    assert_preserved(src);
}

#[test]
fn typed_val_and_var_keep_the_annotation_after_the_name() {
    // `val n: Int = 1` was printed as `val: Int n = 1`, which doesn't parse.
    let src = "module T\n\nfn f(a: Int): Int = {\n    val n: Int = a + 1\n    var m: Int = n * 2\n    val (x, y): (Int, Int) = (n, m)\n    x + y\n}\n".replace("\n", "\n");
    let out = assert_preserved(&src);
    assert_contains(&out, "val n: Int = a + 1");
    assert_contains(&out, "var m: Int = n * 2");
    assert_not_contains(&out, "val: ");
}

#[test]
fn every_example_program_formats_to_the_same_program() {
    // Real programs rather than hand-picked snippets: format each file in
    // `examples/` and require the result to parse to exactly the same program.
    // Files that don't parse are skipped (nothing to compare). Files with a
    // `view` are skipped too: `fmt_view` is a stub that replaces the whole view
    // body with `// ...` (BACKLOG), a known, separate gap.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut checked = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("cto") { continue; }
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        let Ok(module) = certo_parser::parse(&src) else { continue };
        let once = format_source(&src).expect("a file that parsed must format");
        let (before, after) = (shape(&src), shape(&once));
        assert!(
            before == after,
            "formatting changed the program in {}\n{}", path.display(), first_difference(&before, &after)
        );
        checked += 1;
    }
    assert!(checked > 0 || !dir.exists(), "expected to check at least one example in {}", dir.display());
}

#[test]
fn loop_and_block_bodies_do_not_gain_a_layer_of_braces_on_every_format() {
    // The body of `for`/`while`/`unsafe`/`db.transaction`/`defer` is already a
    // block; printing braces around it again made each `certo fmt` run add
    // another `{ ... }` and a file grew every time it was formatted.
    let src = "module T\n\nfn f(xs: List<Int>): Int = {\n    var total = 0\n    for x in xs {\n        total = total + x\n    }\n    while total > 100 {\n        total = total - 1\n    }\n    defer {\n        println(\"done\")\n    }\n    total\n}\n";
    let out = assert_preserved(src);
    assert_eq!(out.matches('{').count(), src.matches('{').count(), "no extra braces expected:\n{}", out);
    assert_not_contains(&out, "{\n    {");
}

#[test]
fn a_view_keeps_its_live_values_keys_and_layout() {
    let src = "module T\n\nview Orders {\n    live val count = 1\n    pk = id\n    filter = customerId\n    layout = (stack)(a, b)\n}\n";
    let out = assert_preserved(src);
    for part in ["live val count", "pk = id", "filter = customerId", "layout = "] {
        assert!(out.contains(part), "missing `{}` in:\n{}", part, out);
    }
}
