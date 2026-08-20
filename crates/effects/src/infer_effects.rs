use std::collections::{HashMap, HashSet};
use certo_ast::expr::{Expr, ExpectMatcher, Stmt};
use certo_ast::pattern::Pattern;
use certo_ast::span::{S, Span};
use certo_ast::types::{Effect, TypeExpr};
use crate::effect_env::EffectEnv;

/// Known types of local names *visible from their own syntax alone* —
/// function/method parameters (always explicitly typed in Certo) and
/// `val`/`var` locals with an explicit annotation — BACKLOG item 178.
/// Deliberately not real type inference: an unannotated `val` or a
/// chained call's own return value used directly as a receiver
/// (`fetchOrder().persist()`) is never in this map, and `callee_lookup_key`
/// below correctly falls back to `None` (unresolved, same as before this
/// item) for those, exactly as it always has.
pub type LocalTypes = HashMap<String, String>;

/// The bare type name a type annotation names, if it's a simple `Named`
/// type (`Order`, not `List<Order>`/`(A, B)`/a function type) — those
/// compound shapes are never a valid UFCS receiver type to key a
/// `"Type.method"` lookup on anyway.
pub(crate) fn type_expr_simple_name(te: &TypeExpr) -> Option<String> {
    if let TypeExpr::Named { path, .. } = te {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

/// Inferred effects from walking a function body.
#[derive(Debug, Default)]
pub struct InferredEffects {
    pub effects: HashSet<Effect>,
    /// (effect, span, callee) for each effect kind's first observed origin —
    /// `callee` is the named function that introduced it (for a call whose
    /// declared effects were inherited), or `None` for a direct syntactic
    /// form (`await`, `unsafe { }`, `db.transaction { }`, `?`).
    pub origins: Vec<(Effect, Span, Option<String>)>,
}

impl InferredEffects {
    fn add(&mut self, e: Effect, span: Span, callee: Option<String>) {
        if self.effects.insert(e.clone()) {
            self.origins.push((e, span, callee));
        }
    }
}

/// Walk an expression, collecting every effect it uses.
pub fn infer_expr(expr: &S<Expr>, env: &EffectEnv, locals: &LocalTypes, out: &mut InferredEffects) {
    match &expr.node {
        Expr::Lit { .. } | Expr::Path { .. } => {}

        Expr::App { func, args, span } => {
            infer_expr(func, env, locals, out);
            for a in args { infer_expr(&a.value, env, locals, out); }

            // If calling a named function — bare `f(x)`, qualified
            // `Type.method(x)`, or (BACKLOG item 178) a UFCS dot-call
            // `record.method(x)` where `record`'s type is known from its
            // own declaration — inherit its declared effects.
            if let Some(name) = callee_lookup_key(&func.node, locals) {
                if let Some(decl) = env.get(&name) {
                    for e in &decl.effects {
                        out.add(e.clone(), *span, Some(name.clone()));
                    }
                }
            }
        }

        Expr::Await { expr, span } => {
            infer_expr(expr, env, locals, out);
            out.add(Effect::Async, *span, None);
        }

        Expr::Spawn { expr, span } => {
            infer_expr(expr, env, locals, out);
            out.add(Effect::Async, *span, None);
        }

        Expr::Transaction { body, span } => {
            infer_expr(body, env, locals, out);
            out.add(Effect::DbWrite, *span, None);
        }

        Expr::Unsafe { body, span } => {
            infer_expr(body, env, locals, out);
            out.add(Effect::Unsafe, *span, None);
        }

        Expr::Try { expr, span } => {
            infer_expr(expr, env, locals, out);
            out.add(Effect::Fallible, *span, None);
        }

        // Recursive walks
        Expr::Pipe { left, right, .. } => { infer_expr(left, env, locals, out); infer_expr(right, env, locals, out); }
        Expr::BinOp { left, right, .. } => { infer_expr(left, env, locals, out); infer_expr(right, env, locals, out); }
        Expr::UnOp { expr, .. } => infer_expr(expr, env, locals, out),
        Expr::Field { expr, .. } | Expr::SafeField { expr, .. } => infer_expr(expr, env, locals, out),
        Expr::Ascribe { expr, .. } => infer_expr(expr, env, locals, out),

        Expr::If { cond, then_expr, else_expr, .. } => {
            infer_expr(cond, env, locals, out);
            infer_expr(then_expr, env, locals, out);
            infer_expr(else_expr, env, locals, out);
        }

        Expr::Match { scrutinee, arms, .. } => {
            infer_expr(scrutinee, env, locals, out);
            for arm in arms {
                if let Some(g) = &arm.guard { infer_expr(g, env, locals, out); }
                infer_expr(&arm.body, env, locals, out);
            }
        }

        Expr::Block { stmts, .. } => {
            // BACKLOG item 178 — a fresh, cloned scope so a `val`/`var`
            // declared inside this block (with an explicit annotation)
            // becomes resolvable for the *rest* of this block without
            // leaking into whatever code runs after the block ends. A
            // later statement re-declaring the same name correctly
            // overwrites the earlier one, matching real shadowing —
            // `scope` is updated incrementally, in statement order, not
            // pre-scanned all at once.
            let mut scope = locals.clone();
            for s in stmts {
                if let Some((name, ty)) = infer_stmt(s, env, &scope, out) {
                    scope.insert(name, ty);
                }
            }
        }

        Expr::Lambda { body, .. } => infer_expr(body, env, locals, out),
        Expr::List { elements, .. } => { for e in elements { infer_expr(e, env, locals, out); } }
        Expr::Tuple { elements, .. } => { for e in elements { infer_expr(e, env, locals, out); } }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { infer_expr(b, env, locals, out); }
            for f in fields { infer_expr(&f.value, env, locals, out); }
        }

        Expr::Guard { cond, else_expr, .. } => {
            infer_expr(cond, env, locals, out);
            infer_expr(else_expr, env, locals, out);
        }
        Expr::Require { expr, error, .. } => {
            infer_expr(expr, env, locals, out);
            infer_expr(error, env, locals, out);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { infer_expr(t, env, locals, out); }
            if let Some(t) = timeout { infer_expr(t, env, locals, out); }
            // parallel implies async
            out.add(Effect::Async, expr.span, None);
        }

        Expr::WithTimeout { duration, body, .. } => {
            infer_expr(duration, env, locals, out);
            infer_expr(body, env, locals, out);
            // withTimeout spawns a background task, same as parallel/spawn.
            out.add(Effect::Async, expr.span, None);
        }

        Expr::For { iter, body, .. } => {
            infer_expr(iter, env, locals, out);
            infer_expr(body, env, locals, out);
        }

        Expr::While { cond, body, .. } => {
            infer_expr(cond, env, locals, out);
            infer_expr(body, env, locals, out);
        }
        Expr::Age { expr, .. } => infer_expr(expr, env, locals, out),
        Expr::ExpectAssertion { actual, matcher, .. } => {
            infer_expr(actual, env, locals, out);
            if let ExpectMatcher::ToBe(y) = matcher { infer_expr(y, env, locals, out); }
        }
    }
}

/// Returns `Some((name, type))` when this statement is a `val`/`var` with
/// an explicit type annotation binding a plain name — BACKLOG item 178 —
/// so the caller (`Expr::Block`'s own arm) can extend its scope for
/// subsequent statements. `None` for everything else, including an
/// unannotated `val` or a destructuring pattern (`val (a, b) = ...`):
/// deliberately not resolved, same conservative behavior as before this
/// item, not a regression.
fn infer_stmt(stmt: &Stmt, env: &EffectEnv, locals: &LocalTypes, out: &mut InferredEffects) -> Option<(String, String)> {
    match stmt {
        Stmt::Val { pattern, ty, value, .. } => {
            infer_expr(value, env, locals, out);
            let Pattern::Ident { name, .. } = &pattern.node else { return None };
            let ty = type_expr_simple_name(&ty.as_ref()?.node)?;
            Some((name.node.clone(), ty))
        }
        Stmt::Var { name, ty, value, .. } => {
            infer_expr(value, env, locals, out);
            let ty = type_expr_simple_name(&ty.as_ref()?.node)?;
            Some((name.node.clone(), ty))
        }
        Stmt::Assign { value, .. } => { infer_expr(value, env, locals, out); None }
        Stmt::Defer { body, .. } | Stmt::Expr { expr: body, .. } => { infer_expr(body, env, locals, out); None }
    }
}

/// Resolve a call's callee to an `EffectEnv` lookup key: a bare `Path` by its
/// name, a `Type.method(...)` dot-call (`Expr::Field` on an
/// uppercase-first-segment `Path`) by its qualified `"Type.method"` name —
/// matching how impl methods are keyed in `effect_env::collect_fn_effects`
/// — or (BACKLOG item 178) `record.method(...)` UFCS on a lowercase local
/// whose type is in `locals`. This crate still has no *real* type
/// inference — `locals` only ever holds what's syntactically obvious
/// (parameters, explicitly-annotated `val`/`var`) — so a receiver not in
/// `locals` (an unannotated `val`, a chained call) still resolves to
/// `None`, same as always.
fn callee_lookup_key(func: &Expr, locals: &LocalTypes) -> Option<String> {
    match func {
        Expr::Path { path, .. } => path.segments.last().map(|s| s.node.clone()),
        Expr::Field { expr, field, .. } => {
            let Expr::Path { path, .. } = &expr.node else { return None };
            let first = path.segments.first()?;
            if first.node.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                Some(format!("{}.{}", first.node, field.node))
            } else {
                // BACKLOG item 178 — a lowercase-first-segment receiver
                // (`record.method(...)`, UFCS on a value): resolve via
                // known local types (a parameter, or an explicitly
                // annotated val/var) if we happen to know it. Still `None`
                // for anything not in `locals` (an unannotated val, a
                // chained call's own return value) — unresolved, exactly
                // as before this item, not a new false-positive risk.
                locals.get(&first.node).map(|ty| format!("{}.{}", ty, field.node))
            }
        }
        _ => None,
    }
}
