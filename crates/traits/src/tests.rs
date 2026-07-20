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
