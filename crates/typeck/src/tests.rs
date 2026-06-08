use certo_parser::parse;
use certo_resolve::resolve;
use crate::infer_decl::check_module;

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
