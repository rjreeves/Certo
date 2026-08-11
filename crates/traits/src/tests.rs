use certo_parser::parse;
use crate::{check_module, TraitErrorKind};

fn ok(src: &str) {
    let module = parse(src).expect("parse error");
    check_module(&module).expect("unexpected trait error");
}

fn err(src: &str) -> Vec<crate::TraitError> {
    let module = parse(src).expect("parse error");
    check_module(&module).unwrap_err()
}

fn has_kind(errs: &[crate::TraitError], f: impl Fn(&TraitErrorKind) -> bool) -> bool {
    errs.iter().any(|e| f(&e.kind))
}

// ------------------------------------------------------------------ //
// Happy-path
// ------------------------------------------------------------------ //

#[test]
fn empty_module() {
    ok("module A");
}

#[test]
fn trait_no_impl() {
    ok("module A
trait Greet {
    fn greet(self: Text): Text
}");
}

#[test]
fn valid_impl() {
    ok("module A
trait Greet {
    fn greet(name: Text): Text
}

type Person = { name: Text }

impl Greet for Person {
    fn greet(name: Text): Text = name
}");
}

#[test]
fn impl_with_default_method_not_overridden() {
    // default method — impl need not provide it
    ok("module A
trait Printable {
    fn print(self: Text): Unit = {}
}

type Doc = { content: Text }

impl Printable for Doc {}");
}

#[test]
fn inherent_impl() {
    // Inherent impls need no trait — no errors expected
    ok("module A
type Counter = { n: Int }

impl Counter {
    fn increment(self: Counter): Counter = self
}");
}

// ------------------------------------------------------------------ //
// Error cases
// ------------------------------------------------------------------ //

#[test]
fn missing_required_method() {
    let errs = err("module A
trait Shape {
    fn area(self: Int): Float
}

type Circle = { r: Float }

impl Shape for Circle {}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::MissingMethod { method, .. } if method == "area")),
        "expected E0301 for missing `area`, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unknown_method_in_impl() {
    let errs = err("module A
trait Greet {
    fn greet(name: Text): Text
}

type Bot = { id: Int }

impl Greet for Bot {
    fn greet(name: Text): Text = name
    fn extra(x: Int): Int = x
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::UnknownMethod { method, .. } if method == "extra")),
        "expected E0300 for `extra`, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn param_count_mismatch() {
    let errs = err("module A
trait Adder {
    fn add(a: Int, b: Int): Int
}

type MyAdder = { id: Int }

impl Adder for MyAdder {
    fn add(a: Int): Int = a
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::ParamCountMismatch { method, .. } if method == "add")),
        "expected E0302, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn return_type_mismatch() {
    let errs = err("module A
trait Converter {
    fn convert(x: Int): Text
}

type MyConverter = { id: Int }

impl Converter for MyConverter {
    fn convert(x: Int): Int = x
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::ReturnTypeMismatch { method, .. } if method == "convert")),
        "expected E0303, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn duplicate_impl() {
    let errs = err("module A
trait Greet {
    fn greet(name: Text): Text
}

type Person = { name: Text }

impl Greet for Person {
    fn greet(name: Text): Text = name
}

impl Greet for Person {
    fn greet(name: Text): Text = name
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::DuplicateImpl { .. })),
        "expected E0306, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn param_type_mismatch() {
    // Int vs Text in first param should produce E0304
    let errs = err("module A
trait Namer {
    fn name(x: Int): Text
}

type Widget = { id: Int }

impl Namer for Widget {
    fn name(x: Text): Text = x
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::ParamTypeMismatch { method, .. } if method == "name")),
        "expected E0304, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Generic call-site bound checking (check_generic_call_bounds)
// ------------------------------------------------------------------ //

#[test]
fn generic_call_with_satisfying_record_literal_ok() {
    ok("module A
trait Greet {
    fn greet(name: Text): Text
}

type Person = { name: Text }

impl Greet for Person {
    fn greet(name: Text): Text = name
}

fn callGreet<T: Greet>(x: T): Unit = {}

fn main(): Unit = callGreet(Person { name: \"Alice\" })");
}

#[test]
fn generic_call_with_unsatisfying_record_literal_errors() {
    let errs = err("module A
trait Greet {
    fn greet(name: Text): Text
}

type Robot = { id: Int }

fn callGreet<T: Greet>(x: T): Unit = {}

fn main(): Unit = callGreet(Robot { id: 1 })");
    assert!(has_kind(&errs, |k|
        matches!(k, TraitErrorKind::UnsatisfiedBound { ty, trait_name } if ty == "Robot" && trait_name == "Greet")),
        "expected E0305 for Robot not implementing Greet, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn generic_call_via_dot_syntax_is_checked() {
    // `Type.method(...)` parses as Expr::Field wrapping the App, not a bare
    // Expr::Path — `callee_lookup_key` recognizes this dot-call shape too
    // (was a known limitation; fixed alongside the analogous gap in
    // row-polymorphism bound checking).
    let errs = err("module A
trait Greet {
    fn greet(name: Text): Text
}

type Robot = { id: Int }

type Caller = { id: Int }

impl Caller {
    fn callGreet<T: Greet>(x: T): Unit = {}
}

fn main(): Unit = Caller.callGreet(Robot { id: 1 })");
    assert!(has_kind(&errs, |k|
        matches!(k, TraitErrorKind::UnsatisfiedBound { ty, trait_name } if ty == "Robot" && trait_name == "Greet")),
        "expected E0305 for Robot not implementing Greet via a dot-call, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn generic_call_via_lowercase_receiver_not_mistaken_for_qualified_call() {
    // `callee_lookup_key` must only treat `expr.field(...)` as a qualified
    // Type.method call when `expr` is an uppercase-first-segment Path — a
    // lowercase local (`caller`, not `Caller`) must not false-positive into
    // a bogus "generics" lookup, even though the AST shape is identical.
    ok("module A
trait Greet {
    fn greet(name: Text): Text
}

type Robot = { id: Int }

fn callGreet<T: Greet>(x: T): Unit = {}

fn main(): Unit = {
    val caller = Robot { id: 1 }
    caller.callGreet(Robot { id: 2 })
}");
}

#[test]
fn same_named_methods_on_different_impls_do_not_collide() {
    // collect_generic_fns keys impl methods by "Type.method", not bare
    // method name — two impls with a same-named generic method must be
    // checked independently, not have one silently overwrite the other's
    // entry in the lookup map.
    let errs = err("module A
trait Greet {
    fn greet(name: Text): Text
}

type Person = { name: Text }

impl Greet for Person {
    fn greet(name: Text): Text = name
}

type Robot = { id: Int }
type Alpha = { id: Int }
type Beta = { id: Int }

impl Alpha {
    fn callGreet<T: Greet>(x: T): Unit = {}
}

impl Beta {
    fn callGreet<T: Greet>(x: T): Unit = {}
}

fn main(): Unit = {
    Alpha.callGreet(Person { name: \"Alice\" })
    Beta.callGreet(Robot { id: 1 })
}");
    assert!(has_kind(&errs, |k|
        matches!(k, TraitErrorKind::UnsatisfiedBound { ty, trait_name } if ty == "Robot" && trait_name == "Greet")),
        "expected Beta.callGreet's Robot argument to be flagged independently of Alpha.callGreet, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
    assert!(!has_kind(&errs, |k|
        matches!(k, TraitErrorKind::UnsatisfiedBound { ty, .. } if ty == "Person")),
        "Alpha.callGreet's satisfying Person argument must not be flagged, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Trait-level and method-level generic type parameters
// ------------------------------------------------------------------ //

#[test]
fn generic_trait_correctly_implemented_for_concrete_type_param() {
    // `T` is the trait's own declared type parameter — the impl provides a
    // concrete `Int` for it, which must be accepted for both the param and
    // the return type.
    ok("module A
trait Container<T> {
    fn get(x: T): T
}

type Box = { n: Int }

impl Container for Box {
    fn get(x: Int): Int = x
}");
}

#[test]
fn concrete_return_type_mismatch_via_capitalized_name_is_rejected() {
    // Regression for the naive "any capitalized identifier is a wildcard"
    // heuristic: `Order` and `Customer` are ordinary concrete type names,
    // not declared type parameters on the trait or the impl, so an impl
    // returning `Customer` where the trait requires `Order` must be
    // rejected, not silently accepted just because both names are
    // capitalized.
    let errs = err("module A
type Order = { id: Int }
type Customer = { id: Int }

trait Greet {
    fn process(self: Order): Order
}

impl Greet for Order {
    fn process(self: Order): Customer = self
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::ReturnTypeMismatch { method, .. } if method == "process")),
        "expected E0303 for Customer vs Order, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn concrete_param_type_mismatch_via_capitalized_name_is_rejected() {
    // Same as above but for a parameter position instead of the return type.
    let errs = err("module A
type Order = { id: Int }
type Customer = { id: Int }

trait Handler {
    fn handle(x: Order): Unit
}

impl Handler for Order {
    fn handle(x: Customer): Unit = {}
}");
    assert!(has_kind(&errs, |k| matches!(k, TraitErrorKind::ParamTypeMismatch { method, .. } if method == "handle")),
        "expected E0304 for Customer vs Order param, got: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn method_level_generic_type_param_still_wildcards() {
    // `B` is declared on the method itself (not the trait), and the impl's
    // own method redeclares the same method-level generic — this must still
    // wildcard-match, not require literal equality.
    ok("module A
trait Mapper {
    fn map<B>(x: Int): B
}

type Widget = { id: Int }

impl Mapper for Widget {
    fn map<B>(x: Int): B = panic(\"unimplemented\")
}");
}

#[test]
fn impl_own_type_param_wildcards_against_trait_type_param() {
    // The impl declares its own generic `T` (`impl<T> Container for Box`),
    // which must still wildcard-match the trait's own declared `T`.
    ok("module A
trait Container<T> {
    fn get(x: T): T
}

type Box = { n: Int }

impl<T> Container for Box {
    fn get(x: T): T = x
}");
}

#[test]
fn generic_call_with_non_literal_argument_not_checked() {
    // Best-effort/syntactic: a variable argument's type isn't visible in the
    // AST, so this must NOT raise a false positive.
    ok("module A
trait Greet {
    fn greet(name: Text): Text
}

type Robot = { id: Int }

fn callGreet<T: Greet>(x: T): Unit = {}

fn main(): Unit = {
    val r = Robot { id: 1 }
    callGreet(r)
}");
}
