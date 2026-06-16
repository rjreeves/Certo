use certo_ast::module::Module;
use certo_ast::decl::Decl;
use certo_ast::span::S;
use certo_ast::expr::Expr;
use certo_ast::types::TypeExpr;
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

            // Push a temporary scope so type param bindings don't leak.
            ctx.env.push();
            let type_param_vars: Vec<u32> = f.type_params.iter().map(|tp| {
                *ctx.counter += 1;
                let v = *ctx.counter;
                ctx.env.define(tp.name.node.clone(), Ty::Var(v));
                v
            }).collect();

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
            ctx.env.pop();

            let fn_ty = Ty::Fn { params: param_tys, ret: Box::new(ret_ty) };
            let fn_ty = if type_param_vars.is_empty() {
                fn_ty
            } else {
                Ty::Forall { vars: type_param_vars, body: Box::new(fn_ty) }
            };

            ctx.env.define(f.name.node.clone(), fn_ty);
            let meta: Vec<(String, bool)> = f.params.iter()
                .map(|p| (p.name.node.clone(), p.default.is_some()))
                .collect();
            ctx.env.define_param_meta(f.name.node.clone(), meta);
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

            // Register record fields so field-access typeck can resolve `a.field` on named types.
            if let certo_ast::decl::TypeBody::Record(rec) = &t.body {
                let mut ctx = Ctx { env, uf, errors, counter };
                let fields: Vec<(String, Ty)> = rec.fields.iter()
                    .map(|f| (f.name.node.clone(), type_expr_to_ty(&f.ty.node, &mut ctx)))
                    .collect();
                env.record_fields.insert(t.name.node.clone(), fields);
            }

            // Register sum variants so constructors are bound in the environment.
            if let certo_ast::decl::TypeBody::Sum(variants) = &t.body {
                let parent_ty = Ty::Named { name: t.name.node.clone(), args: vec![] };
                for v in variants {
                    if v.fields.is_empty() {
                        // Unit variant: `Red` — just a value of the parent type.
                        env.define(v.name.node.clone(), parent_ty.clone());
                    } else {
                        // Variant with fields: constructor function.
                        let mut ctx = Ctx { env, uf, errors, counter };
                        let param_tys: Vec<Ty> = v.fields.iter()
                            .map(|f| type_expr_to_ty(&f.ty.node, &mut ctx))
                            .collect();
                        env.define(
                            v.name.node.clone(),
                            Ty::Fn { params: param_tys, ret: Box::new(parent_ty.clone()) },
                        );
                    }
                }
            }
        }
        // State machine declarations: register generated types and functions.
        Decl::StateMachine(sm) => {
            let mname = &sm.name.node;
            let machine_ty = Ty::Named { name: mname.clone(), args: vec![] };
            let state_ty   = Ty::Named { name: format!("{}State", mname), args: vec![] };

            // The machine type and its state type.
            env.define(mname.clone(), machine_ty.clone());
            env.define(format!("{}State", mname), state_ty.clone());

            // Constructor: MachineName_new() -> MachineName
            env.define(
                format!("{}_new", mname),
                Ty::Fn { params: vec![], ret: Box::new(machine_ty.clone()) },
            );

            // Transition functions: MachineName_event(MachineName, params...) -> MachineName
            for t in &sm.transitions {
                let mut ctx = Ctx { env, uf, errors, counter };
                let mut param_tys = vec![machine_ty.clone()];
                for p in &t.params {
                    param_tys.push(type_expr_to_ty(&p.ty.node, &mut ctx));
                }
                env.define(
                    format!("{}_{}", mname, t.event.node),
                    Ty::Fn { params: param_tys, ret: Box::new(machine_ty.clone()) },
                );
            }

            // State predicate functions: MachineName_isState(MachineName) -> Bool
            for state in &sm.states {
                env.define(
                    format!("{}_is{}", mname, state.node),
                    Ty::Fn { params: vec![machine_ty.clone()], ret: Box::new(Ty::Bool) },
                );
            }

            // Current state accessor: MachineName_state(MachineName) -> MachineName_state
            env.define(
                format!("{}_state", mname),
                Ty::Fn { params: vec![machine_ty.clone()], ret: Box::new(state_ty) },
            );
        }
        // Constraint names resolve to Bool at use sites.
        Decl::Constraint(c) => {
            env.define(c.name.node.clone(), Ty::Bool);
        }
        // Temporal names resolve to Duration at use sites.
        Decl::Temporal(t) => {
            env.define(t.name.node.clone(), Ty::Named { name: "Duration".to_string(), args: vec![] });
        }
        // Validator: register the generated validate/validateAll functions.
        Decl::Validator(v) => {
            let entity_ty  = type_expr_to_ty(&v.entity.node,  &mut Ctx { env, uf, errors, counter });
            let errors_ty  = type_expr_to_ty(&v.errors.node,  &mut Ctx { env, uf, errors, counter });
            let result_ty  = Ty::Result(Box::new(Ty::Unit), Box::new(errors_ty.clone()));
            let result_list = Ty::List(Box::new(errors_ty));
            // validate(entity) -> Result<Unit, ErrorsType>
            env.define(
                format!("{}.validate", v.name.node),
                Ty::Fn { params: vec![entity_ty.clone()], ret: Box::new(result_ty) },
            );
            // validateAll(entity) -> List<ErrorsType>
            env.define(
                format!("{}.validateAll", v.name.node),
                Ty::Fn { params: vec![entity_ty], ret: Box::new(result_list) },
            );
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
                // Pre-define type params as fresh vars so every occurrence of T
                // in the params and return type resolves to the same inference var.
                for tp in &f.type_params {
                    let fresh = ctx.fresh();
                    ctx.env.define(tp.name.node.clone(), fresh);
                }
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

        Decl::Temporal(t) => {
            let body_ty = infer(&t.body, ctx);
            let body_ty = ctx.uf.apply(&body_ty);
            let dur_ty  = Ty::Named { name: "Duration".to_string(), args: vec![] };
            match &body_ty {
                Ty::Var(_) | Ty::Error => {} // unresolved or already errored
                other if other != &dur_ty => {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::TemporalNotDuration { found: body_ty.clone() },
                        span: t.span,
                    });
                }
                _ => {}
            }
        }

        Decl::Validator(v) => {
            ctx.env.push();

            // Bind entity variable: `Order` → `order: Order`
            if let Some(entity_name) = type_expr_simple_name(&v.entity.node) {
                let entity_ty = type_expr_to_ty(&v.entity.node, ctx);
                ctx.env.define(lowercase_first(&entity_name), entity_ty);
            }

            // Bind context fields
            for field in &v.context {
                let field_ty = type_expr_to_ty(&field.type_ref.node, ctx);
                ctx.env.define(field.name.node.clone(), field_ty);
            }

            // Type-check each rule
            for rule in &v.rules {
                let req_ty = infer(&rule.require, ctx);
                ctx.unify(req_ty, Ty::Bool, rule.require.span);
                infer(&rule.else_, ctx); // deferred strict check (E0703)
            }

            ctx.env.pop();
        }

        // Other decls: skip for now (traits, impls, state machines, etc.)
        _ => {}
    }
}

fn infer_fn_body(body: &S<Expr>, ctx: &mut Ctx<'_>) -> Ty {
    match &body.node {
        Expr::Block { stmts, .. } => {
            ctx.env.push();
            let ty = infer_block(stmts, ctx);
            ctx.env.pop();
            ty
        }
        _ => infer(body, ctx),
    }
}

fn type_expr_simple_name(te: &TypeExpr) -> Option<String> {
    if let TypeExpr::Named { path, .. } = te {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None    => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
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
