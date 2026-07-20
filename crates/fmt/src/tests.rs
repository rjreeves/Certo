use crate::format_source;

fn fmt(src: &str) -> String {
    format_source(src).expect("parse error")
}

fn assert_contains(out: &str, needle: &str) {
    assert!(out.contains(needle), "expected {:?} in:\n{}", needle, out);
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
fn sum_type() {
    let out = fmt("module A\ntype Color = | Red | Green | Blue");
    assert_contains(&out, "type Color =");
    assert_contains(&out, "| Red");
    assert_contains(&out, "| Green");
    assert_contains(&out, "| Blue");
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
