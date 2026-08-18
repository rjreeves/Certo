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
