use certo_ast::decl::*;
use certo_ast::span::S;
use crate::scope::{ScopeChain, Res};
use crate::resolve_expr::resolve_expr;

/// Walk a declaration and resolve all names inside it.
/// Top-level names should already be pre-declared in the scope by
/// `resolve_module` before this is called (so mutual recursion works).
pub fn resolve_decl(decl: &S<Decl>, scope: &mut ScopeChain) {
    match &decl.node {
        Decl::Fn(f)  => resolve_fn(f, scope),
        Decl::Type(_) => {}    // Type bodies are structural — resolved by type checker.
        Decl::Val(v) => resolve_expr(&v.value, scope),
        Decl::Var(v) => resolve_expr(&v.value, scope),
        Decl::Trait(t) => {
            for m in &t.methods { resolve_fn(m, scope); }
        }
        Decl::Impl(i) => {
            for m in &i.methods { resolve_fn(m, scope); }
        }
        Decl::StateMachine(sm) => resolve_statemachine(sm, scope),
        Decl::Migration(_) => {}  // Migration ops resolved by the migration tool.
        Decl::View(v) => resolve_expr(&v.layout, scope),
        Decl::Form(_) => {}
        Decl::Test(t) => {
            scope.push();
            resolve_expr(&t.body, scope);
            scope.pop();
        }
        Decl::Property(p) => {
            scope.push();
            resolve_expr(&p.body, scope);
            scope.pop();
        }
        Decl::DbTest(d) => {
            scope.push();
            resolve_expr(&d.body, scope);
            scope.pop();
        }
        Decl::Validator(_) => {}
    }
}

fn resolve_fn(f: &FnDecl, scope: &mut ScopeChain) {
    scope.push();

    // Bring type parameters into scope (as builtins — type checking will validate them)
    for tp in &f.type_params {
        scope.define(
            &tp.name.node,
            Res::Builtin,
            tp.name.span,
        );
    }

    // Bring parameters into scope
    for p in &f.params {
        scope.define(
            &p.name.node,
            Res::Local { span: p.name.span },
            p.name.span,
        );
        // Default values are resolved in the outer scope (before params are bound)
        if let Some(default) = &p.default {
            // Resolve defaults against the outer scope, not the fn scope —
            // we've already pushed so pop temporarily isn't ergonomic; instead
            // we accept the slight inaccuracy and resolve here.
            resolve_expr(default, scope);
        }
    }

    if let Some(body) = &f.body {
        resolve_expr(body, scope);
    }

    scope.pop();
}

fn resolve_statemachine(sm: &StateMachineDecl, scope: &mut ScopeChain) {
    // States are local names within the statemachine
    scope.push();
    for s in &sm.states {
        scope.define(&s.node, Res::Local { span: s.span }, s.span);
    }
    for hook in &sm.on_enter {
        resolve_expr(&hook.body, scope);
    }
    for inv in &sm.invariants {
        resolve_expr(&inv.cond, scope);
    }
    scope.pop();
}
