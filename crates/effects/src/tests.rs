use certo_ast::decl::{Decl, FnDecl};
use certo_ast::expr::{Expr, Lit};
use certo_ast::module::Module;
use certo_ast::span::{S, Span};
use certo_ast::types::{Effect, EffectSet, ModulePath};
use certo_parser::parse;
use crate::{build_env, check_module, EffectErrorKind};

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

const DUMMY: Span = Span::DUMMY;

fn ident(s: &str) -> S<String> { S::new(s.to_string(), DUMMY) }

fn simple_module(decls: Vec<Decl>) -> Module {
    Module {
        path:    ModulePath { segments: vec![ident("A")], span: DUMMY },
        imports: vec![],
        decls:   decls.into_iter().map(|d| S::new(d, DUMMY)).collect(),
        span:    DUMMY,
    }
}

fn lit_expr(v: i64) -> S<Expr> {
    S::new(Expr::Lit { value: Lit::Int(v), span: DUMMY }, DUMMY)
}

fn effect_set(effects: &[Effect]) -> Option<EffectSet> {
    Some(EffectSet {
        effects: effects.iter().map(|e| S::new(e.clone(), DUMMY)).collect(),
        span:    DUMMY,
    })
}

fn pure_set() -> Option<EffectSet> {
    Some(EffectSet { effects: vec![S::new(Effect::Pure, DUMMY)], span: DUMMY })
}

fn fn_decl(name: &str, effects: Option<EffectSet>, body: S<Expr>) -> FnDecl {
    FnDecl {
        is_async:    false,
        is_pub:      false,
        name:        ident(name),
        type_params: vec![],
        params:      vec![],
        ret_ty:      None,
        effects,
        body:        Some(body),
        is_extern:   false,
        span:        DUMMY,
    }
}

fn parse_ok(src: &str) {
    let module = parse(src).expect("parse error");
    if let Err(errs) = check_module(&module) {
        panic!("unexpected: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
    }
}

// ------------------------------------------------------------------ //
// Unit tests via direct AST construction
// ------------------------------------------------------------------ //

#[test]
fn pure_fn_with_literal_body_is_ok() {
    let decl = fn_decl("add", pure_set(), lit_expr(42));
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(errs.is_empty(), "unexpected: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn pure_fn_with_unsafe_body_is_error() {
    let unsafe_body = S::new(Expr::Unsafe { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("bad", pure_set(), unsafe_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    // Exactly one, specific error — not the generic ImpureCallInPure/UndeclaredEffect
    // duplicate that used to also fire alongside it (see BACKLOG item 106).
    assert_eq!(errs.len(), 1, "expected exactly one error, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
    assert!(matches!(&errs[0].kind, EffectErrorKind::UnsafeOutsideUnsafe { .. }),
        "expected E0404 UnsafeOutsideUnsafe, got: {}", errs[0].message());
}

#[test]
fn db_write_fn_with_transaction_is_ok() {
    let tx_body = S::new(Expr::Transaction { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("save", effect_set(&[Effect::DbWrite]), tx_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(errs.is_empty(), "unexpected: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn db_read_fn_with_transaction_is_error() {
    let tx_body = S::new(Expr::Transaction { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("query", effect_set(&[Effect::DbRead]), tx_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(!errs.is_empty(), "expected error");
    assert!(errs.iter().any(|e| matches!(&e.kind, EffectErrorKind::TransactionOutsideDbWrite { .. })),
        "expected E0403: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unsafe_annotation_allows_unsafe_block() {
    let unsafe_body = S::new(Expr::Unsafe { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("low_level", effect_set(&[Effect::Unsafe]), unsafe_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(errs.is_empty(), "unexpected: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unsafe_without_annotation_is_error() {
    let unsafe_body = S::new(Expr::Unsafe { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    // no effects annotation — open set, but unsafe specifically always requires [unsafe]
    let decl = fn_decl("oops", None, unsafe_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(!errs.is_empty(), "expected E0404");
    assert!(errs.iter().any(|e| matches!(&e.kind, EffectErrorKind::UnsafeOutsideUnsafe { .. })),
        "expected E0404: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn await_without_async_annotation_is_error() {
    let await_body = S::new(Expr::Await { expr: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("fetch", effect_set(&[Effect::DbRead]), await_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(!errs.is_empty(), "expected E0402");
    assert!(errs.iter().any(|e| matches!(&e.kind, EffectErrorKind::MissingAsyncAnnotation { .. })),
        "expected E0402: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn pure_fn_calling_impure_named_fn_names_the_callee() {
    // BACKLOG item 106: ImpureCallInPure's `callee` field used to always be
    // constructed as an empty string. It must now name the actual callee.
    let call_body = S::new(Expr::App {
        func: Box::new(S::new(Expr::Path {
            path: certo_ast::types::ModulePath { segments: vec![ident("doIo")], span: DUMMY },
            span: DUMMY,
        }, DUMMY)),
        args: vec![],
        span: DUMMY,
    }, DUMMY);
    let callee = fn_decl("doIo", effect_set(&[Effect::Io]), lit_expr(1));
    let caller = fn_decl("compute", pure_set(), call_body);
    let module = simple_module(vec![Decl::Fn(callee), Decl::Fn(caller)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);

    assert_eq!(errs.len(), 1, "expected exactly one error (no duplicate), got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
    assert!(matches!(&errs[0].kind,
        EffectErrorKind::ImpureCallInPure { caller, callee, .. } if caller == "compute" && callee == "doIo"),
        "expected ImpureCallInPure naming callee `doIo`, got: {}", errs[0].message());
}

#[test]
fn pure_fn_calling_impure_method_via_dot_call_is_checked() {
    // `Type.method(...)` parses as Expr::Field wrapping the App, not a bare
    // Expr::Path — callee_lookup_key must recognize this dot-call shape too.
    let module = parse("module A
type Widget = { id: Int }
impl Widget {
    fn doIo(): Unit [io] = {}
}
fn compute(): Int [pure] = {
    Widget.doIo()
    1
}").expect("parse error");
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);

    assert_eq!(errs.len(), 1, "expected exactly one error, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
    assert!(matches!(&errs[0].kind,
        EffectErrorKind::ImpureCallInPure { caller, callee, .. } if caller == "compute" && callee == "Widget.doIo"),
        "expected ImpureCallInPure naming callee `Widget.doIo`, got: {}", errs[0].message());
}

#[test]
fn same_named_methods_on_different_impls_do_not_collide() {
    // collect_fn_effects keys impl methods by "Type.method", not bare method
    // name — two impls with a same-named method must have independent
    // declared effects, not have one silently overwrite the other's entry.
    let module = parse("module A
type Quiet = { id: Int }
type Loud = { id: Int }
impl Quiet {
    fn run(): Unit [pure] = {}
}
impl Loud {
    fn run(): Unit [io] = { println(\"x\") }
}
fn compute(): Int [pure] = {
    Quiet.run()
    1
}").expect("parse error");
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(errs.is_empty(),
        "Quiet.run is [pure]; calling it from a pure fn must not be flagged, got: {:?}",
        errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unannotated_fn_allows_all_effects_except_unsafe() {
    // Open effect set: transaction is OK, but unsafe is not
    let tx_body = S::new(Expr::Transaction { body: Box::new(lit_expr(1)), span: DUMMY }, DUMMY);
    let decl = fn_decl("save", None, tx_body);
    let module = simple_module(vec![Decl::Fn(decl)]);
    let env = build_env(&module);
    let errs = crate::check_effects::check_module(&module, &env);
    assert!(errs.is_empty(), "unexpected: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Parser-based integration tests (parseable effect annotations only)
// ------------------------------------------------------------------ //

#[test]
fn parsed_pure_fn_with_pure_body() {
    parse_ok("module A\nfn add(a: Int, b: Int): Int [pure] = a + b");
}

#[test]
fn parsed_unannotated_fn_is_ok() {
    parse_ok("module A\nfn f(x: Int): Int = x + 1");
}

#[test]
fn parsed_fn_with_db_read_annotation() {
    parse_ok("module A\nfn query(id: Int): Int [db.read] = id");
}
