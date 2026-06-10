use certo_ast::module::Module;
use certo_ast::decl::Decl;
use certo_ast::span::S;
use certo_ast::expr::Expr;
use crate::ty::Ty;
use crate::env::TypeEnv;
use crate::unify::UnionFind;
use crate::error::{TypeError, TypeErrorKind};
use crate::infer_expr::{infer, infer_block, type_expr_to_ty, Ctx};

/// Entry point: check all declarations in a module.
pub fn check_module(module: &Module) -> Result<(), Vec<TypeError>> {
    let mut env     = TypeEnv::new();
    let mut counter = 0u32;
    env.seed_builtins(&mut counter);
    check_module_seeded(module, env, counter)
}

/// Like `check_module` but accepts a pre-seeded environment and counter.
/// Use this when you want to inject stdlib types before checking.
pub fn check_module_seeded(
    module:  &Module,
    env:     TypeEnv,
    counter: u32,
) -> Result<(), Vec<TypeError>> {
    let mut env     = env;
    let mut uf      = UnionFind::default();
    let mut errors  = Vec::new();
    let mut counter = counter;

    // Pass 1 — hoist function signatures so mutually-recursive calls work.
    for sdecl in &module.decls {
        hoist_decl(&sdecl.node, &mut env, &mut uf, &mut errors, &mut counter);
    }

    // Pass 2 — check bodies.
    for sdecl in &module.decls {
        let mut ctx = Ctx { env: &mut env, uf: &mut uf, errors: &mut errors, counter: &mut counter };
        check_decl(&sdecl.node, &mut ctx);
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

// ------------------------------------------------------------------ //
// Hoist
// ------------------------------------------------------------------ //

fn hoist_decl(
    decl:    &Decl,
    env:     &mut TypeEnv,
    uf:      &mut UnionFind,
    errors:  &mut Vec<TypeError>,
    counter: &mut u32,
) {
    match decl {
        Decl::Fn(f) => {
            let mut ctx = Ctx { env, uf, errors, counter };
            let param_tys: Vec<Ty> = f.params.iter()
                .map(|p| type_expr_to_ty(&p.ty.node, &mut ctx))
                .collect();
            let ret_ty = match &f.ret_ty {
                Some(ann) => type_expr_to_ty(&ann.node, &mut ctx),
                None      => {
                    *ctx.counter += 1;
                    Ty::Var(*ctx.counter)
                }
            };
            let fn_ty = Ty::Fn { params: param_tys, ret: Box::new(ret_ty) };
            ctx.env.define(f.name.node.clone(), fn_ty);
        }
        Decl::Val(v) => {
            let mut ctx = Ctx { env, uf, errors, counter };
            let ty = match &v.ty {
                Some(ann) => type_expr_to_ty(&ann.node, &mut ctx),
                None      => ctx.fresh(),
            };
            use certo_ast::pattern::Pattern;
            if let Pattern::Ident { name, .. } = &v.pattern.node {
                ctx.env.define(name.node.clone(), ty);
            }
        }
        Decl::Var(v) => {
            let mut ctx = Ctx { env, uf, errors, counter };
            let ty = match &v.ty {
                Some(ann) => type_expr_to_ty(&ann.node, &mut ctx),
                None      => ctx.fresh(),
            };
            ctx.env.define(v.name.node.clone(), ty);
        }
        // Type declarations just introduce a named type constructor.
        Decl::Type(t) => {
            let ty = Ty::Named { name: t.name.node.clone(), args: vec![] };
            env.define(t.name.node.clone(), ty);
        }
        _ => {} // Other decls handled later or not yet
    }
}

// ------------------------------------------------------------------ //
// Check bodies
// ------------------------------------------------------------------ //

fn check_decl(decl: &Decl, ctx: &mut Ctx<'_>) {
    match decl {
        Decl::Fn(f) => {
            if let Some(body) = &f.body {
                ctx.env.push();
                // Bind parameters
                for p in &f.params {
                    let ty = type_expr_to_ty(&p.ty.node, ctx);
                    ctx.env.define(p.name.node.clone(), ty);
                }
                // Infer body
                let body_ty = infer_fn_body(body, ctx);

                // Unify body type with declared return type (if present)
                if let Some(ann) = &f.ret_ty {
                    let declared = type_expr_to_ty(&ann.node, ctx);
                    ctx.unify(body_ty, declared, f.span);
                } else if is_recursive_call(body, &f.name.node) {
                    // Recursive fn without annotation — emit E0203
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::MissingAnnotation { name: f.name.node.clone() },
                        span: f.span,
                    });
                }

                ctx.env.pop();
            }
        }

        Decl::Val(v) => {
            let inferred = infer(&v.value, ctx);
            if let Some(ann) = &v.ty {
                let declared = type_expr_to_ty(&ann.node, ctx);
                ctx.unify(inferred.clone(), declared, v.span);
            }
            let generalised = ctx.env.generalise(inferred, ctx.uf);
            use certo_ast::pattern::Pattern;
            if let Pattern::Ident { name, .. } = &v.pattern.node {
                ctx.env.define(name.node.clone(), generalised);
            }
        }

        Decl::Var(v) => {
            let inferred = infer(&v.value, ctx);
            if let Some(ann) = &v.ty {
                let declared = type_expr_to_ty(&ann.node, ctx);
                ctx.unify(inferred.clone(), declared, v.span);
            }
            ctx.env.define(v.name.node.clone(), inferred);
        }

        // Other decls: skip for now (traits, impls, state machines, etc.)
        _ => {}
    }
}

fn infer_fn_body(body: &S<Expr>, ctx: &mut Ctx<'_>) -> Ty {
    match &body.node {
        Expr::Block { stmts, span } => {
            ctx.env.push();
            let ty = infer_block(stmts, *span, ctx);
            ctx.env.pop();
            ty
        }
        _ => infer(body, ctx),
    }
}

/// Very lightweight check: does this expression contain a call to `name`?
/// Used to detect recursion without a full control-flow analysis.
fn is_recursive_call(expr: &S<Expr>, name: &str) -> bool {
    match &expr.node {
        Expr::App { func, args, .. } => {
            is_recursive_call(func, name)
                || args.iter().any(|a| is_recursive_call(&a.value, name))
        }
        Expr::Path { path, .. } => {
            path.segments.last().map(|s| s.node.as_str()) == Some(name)
        }
        Expr::Block { stmts, .. } => {
            stmts.iter().any(|s| match s {
                certo_ast::expr::Stmt::Expr { expr, .. } => is_recursive_call(expr, name),
                certo_ast::expr::Stmt::Val { value, .. } => is_recursive_call(value, name),
                certo_ast::expr::Stmt::Var { value, .. } => is_recursive_call(value, name),
                _ => false,
            })
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            is_recursive_call(cond, name)
                || is_recursive_call(then_expr, name)
                || is_recursive_call(else_expr, name)
        }
        Expr::BinOp { left, right, .. } | Expr::Pipe { left, right, .. } => {
            is_recursive_call(left, name) || is_recursive_call(right, name)
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Ascribe { expr, .. } => is_recursive_call(expr, name),
        _ => false,
    }
}
