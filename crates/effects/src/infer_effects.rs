use std::collections::HashSet;
use certo_ast::expr::{Expr, Stmt};
use certo_ast::span::{S, Span};
use certo_ast::types::Effect;
use crate::effect_env::EffectEnv;

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
pub fn infer_expr(expr: &S<Expr>, env: &EffectEnv, out: &mut InferredEffects) {
    match &expr.node {
        Expr::Lit { .. } | Expr::Path { .. } => {}

        Expr::App { func, args, span } => {
            infer_expr(func, env, out);
            for a in args { infer_expr(&a.value, env, out); }

            // If calling a named function, inherit its declared effects.
            if let Expr::Path { path, .. } = &func.node {
                if let Some(name) = path.segments.last().map(|s| s.node.as_str()) {
                    if let Some(decl) = env.get(name) {
                        for e in &decl.effects {
                            out.add(e.clone(), *span, Some(name.to_string()));
                        }
                    }
                }
            }
        }

        Expr::Await { expr, span } => {
            infer_expr(expr, env, out);
            out.add(Effect::Async, *span, None);
        }

        Expr::Spawn { expr, span } => {
            infer_expr(expr, env, out);
            out.add(Effect::Async, *span, None);
        }

        Expr::Transaction { body, span } => {
            infer_expr(body, env, out);
            out.add(Effect::DbWrite, *span, None);
        }

        Expr::Unsafe { body, span } => {
            infer_expr(body, env, out);
            out.add(Effect::Unsafe, *span, None);
        }

        Expr::Try { expr, span } => {
            infer_expr(expr, env, out);
            out.add(Effect::Fallible, *span, None);
        }

        // Recursive walks
        Expr::Pipe { left, right, .. } => { infer_expr(left, env, out); infer_expr(right, env, out); }
        Expr::BinOp { left, right, .. } => { infer_expr(left, env, out); infer_expr(right, env, out); }
        Expr::UnOp { expr, .. } => infer_expr(expr, env, out),
        Expr::Field { expr, .. } | Expr::SafeField { expr, .. } => infer_expr(expr, env, out),
        Expr::Ascribe { expr, .. } => infer_expr(expr, env, out),

        Expr::If { cond, then_expr, else_expr, .. } => {
            infer_expr(cond, env, out);
            infer_expr(then_expr, env, out);
            infer_expr(else_expr, env, out);
        }

        Expr::Match { scrutinee, arms, .. } => {
            infer_expr(scrutinee, env, out);
            for arm in arms {
                if let Some(g) = &arm.guard { infer_expr(g, env, out); }
                infer_expr(&arm.body, env, out);
            }
        }

        Expr::Block { stmts, .. } => {
            for s in stmts { infer_stmt(s, env, out); }
        }

        Expr::Lambda { body, .. } => infer_expr(body, env, out),
        Expr::List { elements, .. } => { for e in elements { infer_expr(e, env, out); } }
        Expr::Tuple { elements, .. } => { for e in elements { infer_expr(e, env, out); } }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { infer_expr(b, env, out); }
            for f in fields { infer_expr(&f.value, env, out); }
        }

        Expr::Guard { cond, else_expr, .. } => {
            infer_expr(cond, env, out);
            infer_expr(else_expr, env, out);
        }
        Expr::Require { expr, error, .. } => {
            infer_expr(expr, env, out);
            infer_expr(error, env, out);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { infer_expr(t, env, out); }
            if let Some(t) = timeout { infer_expr(t, env, out); }
            // parallel implies async
            out.add(Effect::Async, expr.span, None);
        }

        Expr::For { iter, body, .. } => {
            infer_expr(iter, env, out);
            infer_expr(body, env, out);
        }

        Expr::While { cond, body, .. } => {
            infer_expr(cond, env, out);
            infer_expr(body, env, out);
        }
        Expr::Age { expr, .. } => infer_expr(expr, env, out),
    }
}

fn infer_stmt(stmt: &Stmt, env: &EffectEnv, out: &mut InferredEffects) {
    match stmt {
        Stmt::Val { value, .. } | Stmt::Var { value, .. } => infer_expr(value, env, out),
        Stmt::Assign { value, .. } => infer_expr(value, env, out),
        Stmt::Defer { body, .. } | Stmt::Expr { expr: body, .. } => infer_expr(body, env, out),
    }
}
