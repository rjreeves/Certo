use certo_parser::parse;
use super::{resolve, resolve_seeded, ResolveErrorKind};

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

// ------------------------------------------------------------------ //
// Phase 3 — validator feature name resolution
// ------------------------------------------------------------------ //

const MINIMAL_VALIDATOR: &str = "module A
validator V for Order errors OE {
    rule r { require true else Err(OE.X) }
}";

#[test]
fn constraint_hoisted() {
    ok("module A\nconstraint WithinCredit = order.total <= credit");
}

#[test]
fn constraint_pub_hoisted() {
    ok("module A\npub constraint UserIsAdmin = user.role == admin");
}

#[test]
fn temporal_hoisted() {
    ok("module A\ntemporal GracePeriod = Duration.days(30)");
}

#[test]
fn temporal_pub_hoisted() {
    ok("module A\npub temporal VoidWindow = Duration.days(30)");
}

#[test]
fn validator_hoisted() {
    ok(MINIMAL_VALIDATOR);
}

#[test]
fn validator_pub_hoisted() {
    ok("module A
pub validator V for Order errors OE {
    rule r { require true else Err(OE.X) }
}");
}

#[test]
fn validator_after_existing_rule_ok() {
    ok("module A
validator V for Order errors OE {
    rule a { require true else Err(OE.X) }
    rule b { after a  require true else Err(OE.X) }
}");
}

#[test]
fn validator_after_multiple_ok() {
    ok("module A
validator V for Order errors OE {
    rule a { require true else Err(OE.X) }
    rule b { require true else Err(OE.X) }
    rule c { after a  after b  require true else Err(OE.X) }
}");
}

#[test]
fn validator_overrides_existing_rule_ok() {
    ok("module A
validator V for Order errors OE {
    rule a { require true else Err(OE.X) }
    rule b { overrides a  require true else Err(OE.X) }
}");
}

#[test]
fn validator_after_missing_rule_e0701() {
    let kind = first_error_kind("module A
validator V for Order errors OE {
    rule b { after nonexistent  require true else Err(OE.X) }
}");
    assert!(matches!(kind, ResolveErrorKind::AfterRuleNotFound { after_name, .. } if after_name == "nonexistent"));
}

#[test]
fn validator_overrides_missing_rule_e0702() {
    let kind = first_error_kind("module A
validator V for Order errors OE {
    rule b { overrides ghost  require true else Err(OE.X) }
}");
    assert!(matches!(kind, ResolveErrorKind::OverridesRuleNotFound { overrides_name, .. } if overrides_name == "ghost"));
}

#[test]
fn validator_rule_cycle_e0700() {
    let kind = first_error_kind("module A
validator V for Order errors OE {
    rule a { after b  require true else Err(OE.X) }
    rule b { after a  require true else Err(OE.X) }
}");
    assert!(matches!(kind, ResolveErrorKind::RuleCycle { validator, .. } if validator == "V"));
}

#[test]
fn validator_no_cycle_chain_ok() {
    // a → b → c is a valid chain with no cycle
    ok("module A
validator V for Order errors OE {
    rule a { require true else Err(OE.X) }
    rule b { after a  require true else Err(OE.X) }
    rule c { after b  require true else Err(OE.X) }
}");
}

// ------------------------------------------------------------------ //
// Sum-type variant resolution (BACKLOG item 110)
// ------------------------------------------------------------------ //

#[test]
fn sum_type_variant_match_ok() {
    ok("module A
type Shape =
    | Circle(radius: Float)
    | Square(side: Float)
fn area(s: Shape): Float = match s {
    Circle(r) => r
    Square(side) => side
}");
}

#[test]
fn sum_type_unit_variant_ok() {
    ok("module A
type Color = | Red | Green | Blue
fn f(c: Color): Color = Red");
}

#[test]
fn sum_type_variant_constructor_call_ok() {
    ok("module A
type Shape = | Circle(radius: Float)
fn f(): Shape = Circle(1.0)");
}

#[test]
fn sum_type_unknown_variant_still_e0100() {
    let kind = first_error_kind("module A
type Shape = | Circle(radius: Float)
fn f(s: Shape): Int = match s {
    Triangle(a, b, c) => 0
}");
    assert!(matches!(kind, ResolveErrorKind::UndefinedIdent(n) if n == "Triangle"));
}

// ------------------------------------------------------------------ //
// resolve_seeded — extra builtin names (used by certo-lsp for stdlib)
// ------------------------------------------------------------------ //

#[test]
fn resolve_seeded_allows_extra_builtins() {
    let m = parsed("module A\nfn f(): Int = {\n    print(\"hi\")\n    0\n}");
    resolve_seeded(&m, vec!["print".to_string()]).unwrap_or_else(|errs| {
        panic!("resolve error(s):\n{}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))
    });
}

#[test]
fn resolve_seeded_local_decl_still_wins_over_extra_builtin() {
    // A local fn named `print` should still work fine even when `print`
    // is also seeded as an extra builtin (seed_extra uses entry().or_insert).
    let m = parsed("module A\nfn print(x: Int): Int = x\nfn f(): Int = print(1)");
    resolve_seeded(&m, vec!["print".to_string()]).unwrap_or_else(|errs| {
        panic!("resolve error(s):\n{}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))
    });
}

#[test]
fn duplicate_validator_names_e0102() {
    let kind = first_error_kind("module A
validator V for Order errors OE {
    rule r { require true else Err(OE.X) }
}
validator V for Order errors OE {
    rule r { require true else Err(OE.X) }
}");
    assert!(matches!(kind, ResolveErrorKind::DuplicateDefinition { name, .. } if name == "V"));
}
