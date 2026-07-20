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
    assert!(!errs.is_empty(), "expected error");
    assert!(errs.iter().any(|e| matches!(&e.kind,
        EffectErrorKind::ImpureCallInPure { .. } | EffectErrorKind::UndeclaredEffect { .. }
    )), "unexpected error kinds: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
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
