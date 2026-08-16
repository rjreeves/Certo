use certo_ast::expr::{Expr, ExpectMatcher, Stmt};
use certo_ast::span::S;
use crate::scope::{ScopeChain, Res};
use crate::resolve_pattern::define_pattern_bindings;

/// Resolve all name references inside an expression.
pub fn resolve_expr(expr: &S<Expr>, scope: &mut ScopeChain) {
    use certo_ast::expr::{Lit, FStringPart};
    match &expr.node {
        // f-string interpolations contain real expressions — resolve their names.
        Expr::Lit { value: Lit::FString(parts), .. } => {
            for part in parts {
                if let FStringPart::Interpolated(e) = part {
                    resolve_expr(e, scope);
                }
            }
        }
        Expr::Lit { .. } => {}

        Expr::Path { path, span } => {
            // Only look up the leaf name — qualified paths are resolved
            // after module loading (out of scope for this pass).
            if path.segments.len() == 1 {
                let name = &path.segments[0].node;
                scope.lookup(name, *span);
            }
            // Multi-segment paths: trust the module loader for now.
        }

        Expr::App { func, args, .. } => {
            resolve_expr(func, scope);
            for arg in args { resolve_expr(&arg.value, scope); }
        }

        Expr::Pipe { left, right, .. } => {
            resolve_expr(left, scope);
            resolve_expr(right, scope);
        }

        Expr::BinOp { left, right, .. } => {
            resolve_expr(left, scope);
            resolve_expr(right, scope);
        }

        Expr::UnOp { expr, .. } => resolve_expr(expr, scope),

        Expr::Field { expr, .. } | Expr::SafeField { expr, .. } => resolve_expr(expr, scope),

        Expr::If { cond, then_expr, else_expr, .. } => {
            resolve_expr(cond, scope);
            resolve_expr(then_expr, scope);
            resolve_expr(else_expr, scope);
        }

        Expr::Match { scrutinee, arms, .. } => {
            resolve_expr(scrutinee, scope);
            for arm in arms {
                scope.push();
                define_pattern_bindings(&arm.pattern, scope);
                if let Some(g) = &arm.guard { resolve_expr(g, scope); }
                resolve_expr(&arm.body, scope);
                scope.pop();
            }
        }

        Expr::Block { stmts, .. } => {
            scope.push();
            resolve_stmts(stmts, scope);
            scope.pop();
        }

        Expr::Lambda { params, body, .. } => {
            scope.push();
            for p in params {
                scope.define(
                    &p.name.node,
                    Res::Local { span: p.name.span },
                    p.name.span,
                );
            }
            resolve_expr(body, scope);
            scope.pop();
        }

        Expr::List { elements, .. } => {
            for e in elements { resolve_expr(e, scope); }
        }

        Expr::Tuple { elements, .. } => {
            for e in elements { resolve_expr(e, scope); }
        }

        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { resolve_expr(b, scope); }
            for f in fields { resolve_expr(&f.value, scope); }
        }

        Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. } => resolve_expr(expr, scope),

        Expr::Guard { cond, else_expr, .. } => {
            resolve_expr(cond, scope);
            resolve_expr(else_expr, scope);
        }

        Expr::Require { expr, error, .. } => {
            resolve_expr(expr, scope);
            resolve_expr(error, scope);
        }

        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { resolve_expr(t, scope); }
            if let Some(t) = timeout { resolve_expr(t, scope); }
        }

        Expr::Transaction { body, .. }
        | Expr::Unsafe { body, .. } => resolve_expr(body, scope),

        Expr::Ascribe { expr, .. } => resolve_expr(expr, scope),

        Expr::For { binding, iter, body, .. } => {
            resolve_expr(iter, scope);
            scope.push();
            scope.define(&binding.node, Res::Local { span: binding.span }, binding.span);
            resolve_expr(body, scope);
            scope.pop();
        }

        Expr::While { cond, body, .. } => {
            resolve_expr(cond, scope);
            resolve_expr(body, scope);
        }
        Expr::Age { expr, .. } => resolve_expr(expr, scope),
        Expr::ExpectAssertion { actual, matcher, .. } => {
            resolve_expr(actual, scope);
            if let ExpectMatcher::ToBe(y) = matcher { resolve_expr(y, scope); }
        }
    }
}

/// Resolve a sequence of statements inside a block, handling `val`/`var`
/// bindings so later statements can see earlier ones.
pub fn resolve_stmts(stmts: &[Stmt], scope: &mut ScopeChain) {
    for stmt in stmts {
        match stmt {
            Stmt::Val { pattern, value, .. } => {
                // Resolve the RHS before introducing the binding (no self-reference).
                resolve_expr(value, scope);
                define_pattern_bindings(pattern, scope);
            }
            Stmt::Var { name, value, .. } => {
                resolve_expr(value, scope);
                scope.define(&name.node, Res::Local { span: name.span }, name.span);
            }
            Stmt::Assign { target, value, .. } => {
                resolve_expr(value, scope);
                // Verify the target is already in scope.
                scope.lookup(&target.node, target.span);
            }
            Stmt::Defer { body, .. } => resolve_expr(body, scope),
            Stmt::Expr { expr, .. } => resolve_expr(expr, scope),
        }
    }
}
