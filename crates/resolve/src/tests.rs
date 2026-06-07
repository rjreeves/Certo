use certo_parser::parse;
use super::{resolve, ResolveErrorKind};

fn parsed(src: &str) -> certo_ast::module::Module {
    parse(src).unwrap_or_else(|errs| {
        panic!("parse error: {}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "))
    })
}

fn ok(src: &str) {
    let m = parsed(src);
    resolve(&m).unwrap_or_else(|errs| {
        panic!("resolve error(s):\n{}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))
    });
}

fn first_error_kind(src: &str) -> ResolveErrorKind {
    let m = parsed(src);
    let errs = resolve(&m).expect_err("expected resolve errors but got Ok");
    errs.into_iter().next().unwrap().kind
}

// ------------------------------------------------------------------ //
// Happy-path tests
// ------------------------------------------------------------------ //

#[test]
fn empty_module_ok() {
    ok("module A");
}

#[test]
fn simple_fn_ok() {
    ok("module A\nfn add(a: Int, b: Int): Int = a + b");
}

#[test]
fn fn_uses_param() {
    ok("module A\nfn double(x: Int): Int = x + x");
}

#[test]
fn mutual_recursion_ok() {
    ok("module A
fn isEven(n: Int): Bool = if n == 0 then true else isOdd(n)
fn isOdd(n: Int): Bool  = if n == 0 then false else isEven(n)");
}

#[test]
fn builtin_types_ok() {
    ok("module A\nfn f(x: Int): Bool = true");
}

#[test]
fn val_binding_in_block() {
    ok("module A\nfn f(): Int = {\n    val x = 1\n    x\n}");
}

#[test]
fn match_binds_pattern_vars() {
    ok("module A
fn describe(r: Result<Int, Text>): Int =
    match r {
        Ok(v)  => v
        Err(e) => 0
    }");
}

#[test]
fn import_named_ok() {
    ok("module A\nimport Stdlib.Collections.{ List, Map }\nfn f(xs: List<Int>): Int = 0");
}

#[test]
fn import_aliased_ok() {
    ok("module A\nimport Stdlib.DateTime as DT\nfn f(): DT = todo()");
}

#[test]
fn type_decl_hoisted() {
    // MyType is declared after the fn that references it — hoisting must work
    ok("module A
fn f(x: MyType): Int = 0
type MyType = { value: Int }");
}

// ------------------------------------------------------------------ //
// Error cases
// ------------------------------------------------------------------ //

#[test]
fn undefined_ident_e0100() {
    let kind = first_error_kind("module A\nfn f(): Int = undefinedName");
    assert!(matches!(kind, ResolveErrorKind::UndefinedIdent(n) if n == "undefinedName"));
}

#[test]
fn duplicate_definition_e0102() {
    let kind = first_error_kind("module A
fn foo(): Int = 1
fn foo(): Int = 2");
    assert!(matches!(kind, ResolveErrorKind::DuplicateDefinition { name, .. } if name == "foo"));
}

#[test]
fn ambiguous_import_e0101() {
    let kind = first_error_kind("module A
import Pkg.A.{ Foo }
import Pkg.B.{ Foo }
fn f(x: Foo): Int = 0");
    assert!(matches!(kind, ResolveErrorKind::AmbiguousImport { name, .. } if name == "Foo"));
}

#[test]
fn use_before_val_binding_is_error() {
    // `y` is used before the `val y = ...` binding in the same block.
    // Note: Certo requires sequential binding — y is not yet in scope.
    let kind = first_error_kind("module A
fn f(): Int = {
    val z = y
    val y = 1
    z
}");
    assert!(matches!(kind, ResolveErrorKind::UndefinedIdent(n) if n == "y"));
}
