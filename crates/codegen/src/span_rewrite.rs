//! Rewrites every span in a freshly-parsed `Decl` subtree by adding a fixed
//! byte offset — used by `expand_validators`/`expand_state_machines`
//! (`crates/codegen/src/expand.rs`, BACKLOG item 223) so a declaration
//! generated from Certo source text and re-parsed on its own (starting at
//! byte 0) can be spliced into the real module and still carry spans that
//! correctly index into a *combined* source string (the real file's own
//! text, followed by each generated chunk in turn), rather than numerically
//! colliding with the real file's own span range.
//!
//! Only `Decl::Fn` and `Decl::Type` are covered at the top level: those are
//! the only two declaration kinds `emit_validator.rs`/`emit_statemachine.rs`
//! ever generate (confirmed by inspecting their own `to_source()`/codegen
//! output). Everything reachable *underneath* a `Fn`/`Type` — expressions,
//! patterns, type expressions, statements — is covered exhaustively (no
//! catch-all arms), so the compiler forces this file to be updated if any of
//! those enums ever grow a new variant, rather than silently leaving a new
//! kind of node un-rewritten and reopening this exact bug.
//!
//! BACKLOG item 222 — moved here from `crates/cli/src/span_rewrite.rs` so
//! `crates/testrunner` can reuse the same `expand_validators`/
//! `expand_state_machines` `certo build`/`check`/`run` already use, instead
//! of `certo test` never expanding either at all.

use certo_ast::decl::*;
use certo_ast::expr::*;
use certo_ast::pattern::*;
use certo_ast::span::{S, Span};
use certo_ast::types::*;

fn off(span: &mut Span, offset: u32) {
    if *span == Span::DUMMY { return; }
    span.start += offset;
    span.end += offset;
}

fn off_s<T>(s: &mut S<T>, offset: u32) {
    off(&mut s.span, offset);
}

/// Entry point: offset every span in one top-level generated declaration
/// (including the outer `S<Decl>` wrapper's own span).
pub fn offset_decl_spans(decl: &mut S<Decl>, offset: u32) {
    match &mut decl.node {
        Decl::Fn(f) => offset_fn(f, offset),
        Decl::Type(t) => offset_type(t, offset),
        // `emit_validator.rs`/`emit_statemachine.rs` never generate any other
        // declaration kind — see this module's own doc comment.
        _ => {}
    }
    off(&mut decl.span, offset);
}

fn offset_fn(f: &mut FnDecl, offset: u32) {
    off_s(&mut f.name, offset);
    for tp in &mut f.type_params { offset_type_param(tp, offset); }
    for p in &mut f.params { offset_fn_param(p, offset); }
    if let Some(ret) = &mut f.ret_ty { offset_type_expr_s(ret, offset); }
    if let Some(effects) = &mut f.effects { offset_effect_set(effects, offset); }
    if let Some(body) = &mut f.body { offset_expr_s(body, offset); }
    off(&mut f.span, offset);
}

fn offset_fn_param(p: &mut FnParam, offset: u32) {
    off_s(&mut p.name, offset);
    offset_type_expr_s(&mut p.ty, offset);
    if let Some(d) = &mut p.default { offset_expr_s(d, offset); }
    off(&mut p.span, offset);
}

fn offset_type(t: &mut TypeDecl, offset: u32) {
    off_s(&mut t.name, offset);
    for tp in &mut t.type_params { offset_type_param(tp, offset); }
    match &mut t.body {
        TypeBody::Record(r) => {
            for field in &mut r.fields { offset_record_field_def(field, offset); }
            for c in &mut r.computed {
                off_s(&mut c.name, offset);
                offset_type_expr_s(&mut c.ty, offset);
                offset_expr_s(&mut c.body, offset);
                off(&mut c.span, offset);
            }
            for m in &mut r.methods { offset_fn(m, offset); }
            off(&mut r.span, offset);
        }
        TypeBody::Sum(variants) => {
            for v in variants {
                off_s(&mut v.name, offset);
                for field in &mut v.fields {
                    if let Some(n) = &mut field.name { off_s(n, offset); }
                    offset_type_expr_s(&mut field.ty, offset);
                    off(&mut field.span, offset);
                }
                off(&mut v.span, offset);
            }
        }
        TypeBody::Alias(ty) => offset_type_expr_s(ty, offset),
    }
    off(&mut t.span, offset);
}

fn offset_record_field_def(field: &mut RecordFieldDef, offset: u32) {
    off_s(&mut field.name, offset);
    offset_type_expr_s(&mut field.ty, offset);
    off(&mut field.span, offset);
}

/// `RecordTypeField` (`certo_ast::types`) — the row-bound/anonymous-record-type
/// shape, distinct from `RecordFieldDef` (`certo_ast::decl`) above despite the
/// identical field layout; the two aren't the same Rust type.
fn offset_record_type_field(field: &mut RecordTypeField, offset: u32) {
    off_s(&mut field.name, offset);
    offset_type_expr_s(&mut field.ty, offset);
    off(&mut field.span, offset);
}

fn offset_type_param(tp: &mut TypeParam, offset: u32) {
    off_s(&mut tp.name, offset);
    for b in &mut tp.bounds {
        match b {
            Bound::Trait(tb) => {
                offset_module_path(&mut tb.name, offset);
                off(&mut tb.span, offset);
            }
            Bound::Row(rb) => {
                for f in &mut rb.fields { offset_record_type_field(f, offset); }
                off(&mut rb.span, offset);
            }
        }
    }
    off(&mut tp.span, offset);
}

fn offset_effect_set(e: &mut EffectSet, offset: u32) {
    for eff in &mut e.effects { off_s(eff, offset); }
    off(&mut e.span, offset);
}

fn offset_module_path(p: &mut ModulePath, offset: u32) {
    for seg in &mut p.segments { off_s(seg, offset); }
    off(&mut p.span, offset);
}

fn offset_type_expr_s(t: &mut S<TypeExpr>, offset: u32) {
    offset_type_expr(&mut t.node, offset);
    off(&mut t.span, offset);
}

fn offset_type_expr(t: &mut TypeExpr, offset: u32) {
    match t {
        TypeExpr::Named { path, args, span } => {
            offset_module_path(path, offset);
            for a in args { offset_type_expr_s(a, offset); }
            off(span, offset);
        }
        TypeExpr::Option { inner, span } => {
            offset_type_expr_s(inner, offset);
            off(span, offset);
        }
        TypeExpr::Tuple { elements, span } => {
            for e in elements { offset_type_expr_s(e, offset); }
            off(span, offset);
        }
        TypeExpr::Fn { params, ret, span } => {
            for p in params { offset_type_expr_s(p, offset); }
            offset_type_expr_s(ret, offset);
            off(span, offset);
        }
        TypeExpr::Record { fields, span } => {
            for f in fields {
                off_s(&mut f.name, offset);
                offset_type_expr_s(&mut f.ty, offset);
                off(&mut f.span, offset);
            }
            off(span, offset);
        }
        TypeExpr::Ptr { inner, span } => {
            offset_type_expr_s(inner, offset);
            off(span, offset);
        }
        TypeExpr::Param { name, span } => {
            off_s(name, offset);
            off(span, offset);
        }
        TypeExpr::DecimalParam { span, .. } => off(span, offset),
        TypeExpr::BoundedTextParam { span, .. } => off(span, offset),
    }
}

fn offset_expr_s(e: &mut S<Expr>, offset: u32) {
    offset_expr(&mut e.node, offset);
    off(&mut e.span, offset);
}

fn offset_expr(e: &mut Expr, offset: u32) {
    match e {
        Expr::Lit { value, span } => {
            offset_lit(value, offset);
            off(span, offset);
        }
        Expr::Path { path, span } => {
            offset_module_path(path, offset);
            off(span, offset);
        }
        Expr::App { func, args, span } => {
            offset_expr_s(func, offset);
            for a in args {
                if let Some(l) = &mut a.label { off_s(l, offset); }
                offset_expr_s(&mut a.value, offset);
                off(&mut a.span, offset);
            }
            off(span, offset);
        }
        Expr::Pipe { left, right, span }
        | Expr::BinOp { left, right, span, .. } => {
            offset_expr_s(left, offset);
            offset_expr_s(right, offset);
            off(span, offset);
        }
        Expr::UnOp { expr, span, .. }
        | Expr::Try { expr, span }
        | Expr::Await { expr, span }
        | Expr::Spawn { expr, span }
        | Expr::Transaction { body: expr, span }
        | Expr::Unsafe { body: expr, span }
        | Expr::Age { expr, span } => {
            offset_expr_s(expr, offset);
            off(span, offset);
        }
        Expr::Field { expr, field, span } | Expr::SafeField { expr, field, span } => {
            offset_expr_s(expr, offset);
            off_s(field, offset);
            off(span, offset);
        }
        Expr::If { cond, then_expr, else_expr, span } => {
            offset_expr_s(cond, offset);
            offset_expr_s(then_expr, offset);
            offset_expr_s(else_expr, offset);
            off(span, offset);
        }
        Expr::Match { scrutinee, arms, span } => {
            offset_expr_s(scrutinee, offset);
            for arm in arms {
                offset_pattern_s(&mut arm.pattern, offset);
                if let Some(g) = &mut arm.guard { offset_expr_s(g, offset); }
                offset_expr_s(&mut arm.body, offset);
                off(&mut arm.span, offset);
            }
            off(span, offset);
        }
        Expr::Block { stmts, span } => {
            for s in stmts { offset_stmt(s, offset); }
            off(span, offset);
        }
        Expr::Lambda { params, body, span } => {
            for p in params {
                off_s(&mut p.name, offset);
                if let Some(ty) = &mut p.ty { offset_type_expr_s(ty, offset); }
                off(&mut p.span, offset);
            }
            offset_expr_s(body, offset);
            off(span, offset);
        }
        Expr::List { elements, span } | Expr::Tuple { elements, span } => {
            for el in elements { offset_expr_s(el, offset); }
            off(span, offset);
        }
        Expr::Record { base, fields, span, .. } => {
            if let Some(b) = base { offset_expr_s(b, offset); }
            for f in fields {
                off_s(&mut f.name, offset);
                offset_expr_s(&mut f.value, offset);
                off(&mut f.span, offset);
            }
            off(span, offset);
        }
        Expr::Guard { cond, else_expr, span } => {
            offset_expr_s(cond, offset);
            offset_expr_s(else_expr, offset);
            off(span, offset);
        }
        Expr::Require { expr, error, span } => {
            offset_expr_s(expr, offset);
            offset_expr_s(error, offset);
            off(span, offset);
        }
        Expr::Parallel { tasks, timeout, span } => {
            for t in tasks { offset_expr_s(t, offset); }
            if let Some(t) = timeout { offset_expr_s(t, offset); }
            off(span, offset);
        }
        Expr::WithTimeout { duration, body, span } => {
            offset_expr_s(duration, offset);
            offset_expr_s(body, offset);
            off(span, offset);
        }
        Expr::Ascribe { expr, ty, span } => {
            offset_expr_s(expr, offset);
            offset_type_expr_s(ty, offset);
            off(span, offset);
        }
        Expr::For { binding, iter, body, span } => {
            off_s(binding, offset);
            offset_expr_s(iter, offset);
            offset_expr_s(body, offset);
            off(span, offset);
        }
        Expr::While { cond, body, span } => {
            offset_expr_s(cond, offset);
            offset_expr_s(body, offset);
            off(span, offset);
        }
        Expr::ExpectAssertion { actual, matcher, span } => {
            offset_expr_s(actual, offset);
            match matcher {
                ExpectMatcher::ToBe(e) => offset_expr_s(e, offset),
                ExpectMatcher::ToBeTrue
                | ExpectMatcher::ToBeFalse
                | ExpectMatcher::ToBeSome
                | ExpectMatcher::ToBeNone
                | ExpectMatcher::ToBeOk
                | ExpectMatcher::ToBeErr => {}
            }
            off(span, offset);
        }
    }
}

fn offset_lit(lit: &mut Lit, offset: u32) {
    if let Lit::FString(parts) = lit {
        for part in parts {
            if let FStringPart::Interpolated(e) = part {
                offset_expr_s(e, offset);
            }
        }
    }
}

fn offset_stmt(s: &mut Stmt, offset: u32) {
    match s {
        Stmt::Val { pattern, ty, value, span } => {
            offset_pattern_s(pattern, offset);
            if let Some(t) = ty { offset_type_expr_s(t, offset); }
            offset_expr_s(value, offset);
            off(span, offset);
        }
        Stmt::Var { name, ty, value, span } => {
            off_s(name, offset);
            if let Some(t) = ty { offset_type_expr_s(t, offset); }
            offset_expr_s(value, offset);
            off(span, offset);
        }
        Stmt::Assign { target, value, span } => {
            off_s(target, offset);
            offset_expr_s(value, offset);
            off(span, offset);
        }
        Stmt::Defer { body, span } => {
            offset_expr_s(body, offset);
            off(span, offset);
        }
        Stmt::Expr { expr, span } => {
            offset_expr_s(expr, offset);
            off(span, offset);
        }
    }
}

fn offset_pattern_s(p: &mut S<Pattern>, offset: u32) {
    offset_pattern(&mut p.node, offset);
    off(&mut p.span, offset);
}

fn offset_pattern(p: &mut Pattern, offset: u32) {
    match p {
        Pattern::Wildcard { span } => off(span, offset),
        Pattern::Ident { name, span } => {
            off_s(name, offset);
            off(span, offset);
        }
        Pattern::Constructor { path, fields, span } => {
            offset_module_path(path, offset);
            for f in fields { offset_pattern_s(f, offset); }
            off(span, offset);
        }
        Pattern::Record { path, fields, span, .. } => {
            if let Some(p) = path { offset_module_path(p, offset); }
            for f in fields {
                off_s(&mut f.name, offset);
                if let Some(pat) = &mut f.pattern { offset_pattern_s(pat, offset); }
                off(&mut f.span, offset);
            }
            off(span, offset);
        }
        Pattern::Tuple { elements, span } => {
            for e in elements { offset_pattern_s(e, offset); }
            off(span, offset);
        }
        Pattern::List { head, tail, span } => {
            for h in head { offset_pattern_s(h, offset); }
            if let Some(t) = tail { offset_pattern_s(t, offset); }
            off(span, offset);
        }
        Pattern::Literal { span, .. } => off(span, offset),
        Pattern::Guard { pattern, guard, span } => {
            offset_pattern_s(pattern, offset);
            offset_expr_s(guard, offset);
            off(span, offset);
        }
        Pattern::As { pattern, name, span } => {
            offset_pattern_s(pattern, offset);
            off_s(name, offset);
            off(span, offset);
        }
        Pattern::Or { left, right, span } => {
            offset_pattern_s(left, offset);
            offset_pattern_s(right, offset);
            off(span, offset);
        }
    }
}
