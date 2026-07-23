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

/// Extract row-polymorphism bounds (`R: { name: Text }`) from a function/method's
/// type parameters, evaluating each required field's type. `type_param_vars` must
/// be the fresh `TyVar`s already created for `type_params`, in the same order —
/// see the `type_param_vars` construction just above each call site. Must be
/// called before the type-param scope is popped, so a field type that refers to
/// a sibling type parameter (e.g. `R: { value: T }`) resolves to that param's
/// actual var rather than an unrelated fresh one.
fn collect_row_bounds(
    type_params:     &[certo_ast::types::TypeParam],
    type_param_vars: &[u32],
    ctx:             &mut Ctx<'_>,
) -> Vec<(u32, Vec<(String, Ty)>)> {
    use certo_ast::types::Bound;
    type_params.iter().zip(type_param_vars.iter()).filter_map(|(tp, &var)| {
        let fields: Vec<(String, Ty)> = tp.bounds.iter()
            .filter_map(|b| match b {
                Bound::Row(row) => Some(row.fields.iter()
                    .map(|f| (f.name.node.clone(), type_expr_to_ty(&f.ty.node, ctx)))
                    .collect::<Vec<_>>()),
                Bound::Trait(_) => None,
            })
            .flatten()
            .collect();
        if fields.is_empty() { None } else { Some((var, fields)) }
    }).collect()
}

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

    // Pass 3 — FFI safety: calls to `extern "C"` functions must be inside `unsafe { }`.
    check_ffi_unsafe(module, &mut errors);

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

// ------------------------------------------------------------------ //
// FFI safety — extern calls must be wrapped in `unsafe { }`
// ------------------------------------------------------------------ //

/// Collect the names of `extern "C"` functions, then walk every function body
/// flagging any call to one that is not lexically inside an `unsafe { }` block.
fn check_ffi_unsafe(module: &Module, errors: &mut Vec<TypeError>) {
    use std::collections::HashSet;
    let extern_fns: HashSet<&str> = module.decls.iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) if f.is_extern => Some(f.name.node.as_str()),
            _ => None,
        })
        .collect();
    if extern_fns.is_empty() { return; }

    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            if let Some(body) = &f.body {
                walk_ffi(body, false, &extern_fns, errors);
            }
        }
    }
}

fn walk_ffi(
    expr:       &S<Expr>,
    in_unsafe:  bool,
    extern_fns: &std::collections::HashSet<&str>,
    errors:     &mut Vec<TypeError>,
) {
    use certo_ast::expr::Stmt;
    match &expr.node {
        // Entering an `unsafe { }` block discharges the requirement for its body.
        Expr::Unsafe { body, .. } => walk_ffi(body, true, extern_fns, errors),

        Expr::App { func, args, span } => {
            if !in_unsafe {
                if let Expr::Path { path, .. } = &func.node {
                    if let Some(name) = path.segments.last().map(|s| s.node.as_str()) {
                        if extern_fns.contains(name) {
                            errors.push(TypeError {
                                kind: TypeErrorKind::FfiCallOutsideUnsafe { name: name.to_string() },
                                span: *span,
                            });
                        }
                    }
                }
            }
            walk_ffi(func, in_unsafe, extern_fns, errors);
            for a in args { walk_ffi(&a.value, in_unsafe, extern_fns, errors); }
        }

        // Recurse into every expression-bearing child, preserving `in_unsafe`.
        Expr::Lit { .. } | Expr::Path { .. } => {}
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            walk_ffi(left, in_unsafe, extern_fns, errors);
            walk_ffi(right, in_unsafe, extern_fns, errors);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. } | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. } | Expr::Await { expr, .. } | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } | Expr::Age { expr, .. } => {
            walk_ffi(expr, in_unsafe, extern_fns, errors);
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            walk_ffi(cond, in_unsafe, extern_fns, errors);
            walk_ffi(then_expr, in_unsafe, extern_fns, errors);
            walk_ffi(else_expr, in_unsafe, extern_fns, errors);
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_ffi(scrutinee, in_unsafe, extern_fns, errors);
            for arm in arms {
                if let Some(g) = &arm.guard { walk_ffi(g, in_unsafe, extern_fns, errors); }
                walk_ffi(&arm.body, in_unsafe, extern_fns, errors);
            }
        }
        Expr::Block { stmts, .. } => {
            for st in stmts {
                match st {
                    Stmt::Val { value, .. } | Stmt::Var { value, .. } | Stmt::Assign { value, .. } => {
                        walk_ffi(value, in_unsafe, extern_fns, errors);
                    }
                    Stmt::Defer { body, .. } | Stmt::Expr { expr: body, .. } => {
                        walk_ffi(body, in_unsafe, extern_fns, errors);
                    }
                }
            }
        }
        Expr::Lambda { body, .. } | Expr::Transaction { body, .. } => {
            walk_ffi(body, in_unsafe, extern_fns, errors);
        }
        Expr::For { iter, body, .. } => {
            walk_ffi(iter, in_unsafe, extern_fns, errors);
            walk_ffi(body, in_unsafe, extern_fns, errors);
        }
        Expr::While { cond, body, .. } => {
            walk_ffi(cond, in_unsafe, extern_fns, errors);
            walk_ffi(body, in_unsafe, extern_fns, errors);
        }
        Expr::Guard { cond, else_expr, .. } => {
            walk_ffi(cond, in_unsafe, extern_fns, errors);
            walk_ffi(else_expr, in_unsafe, extern_fns, errors);
        }
        Expr::Require { expr, error, .. } => {
            walk_ffi(expr, in_unsafe, extern_fns, errors);
            walk_ffi(error, in_unsafe, extern_fns, errors);
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { walk_ffi(e, in_unsafe, extern_fns, errors); }
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { walk_ffi(t, in_unsafe, extern_fns, errors); }
            if let Some(t) = timeout { walk_ffi(t, in_unsafe, extern_fns, errors); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { walk_ffi(b, in_unsafe, extern_fns, errors); }
            for fld in fields { walk_ffi(&fld.value, in_unsafe, extern_fns, errors); }
        }
    }
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
            let row_bounds = collect_row_bounds(&f.type_params, &type_param_vars, &mut ctx);
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
            ctx.env.define_row_bounds(f.name.node.clone(), row_bounds);
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
        // Impl blocks register each method as a qualified function `Type.method`,
        // callable like a stdlib function (e.g. `Person.greet(p)`).
        Decl::Impl(i) => {
            let type_name = i.type_path.segments.last()
                .map(|s| s.node.clone()).unwrap_or_default();
            for m in &i.methods {
                let mut ctx = Ctx { env, uf, errors, counter };
                ctx.env.push();
                let type_param_vars: Vec<u32> = m.type_params.iter().map(|tp| {
                    *ctx.counter += 1;
                    let v = *ctx.counter;
                    ctx.env.define(tp.name.node.clone(), Ty::Var(v));
                    v
                }).collect();
                let param_tys: Vec<Ty> = m.params.iter()
                    .map(|p| type_expr_to_ty(&p.ty.node, &mut ctx))
                    .collect();
                let ret_ty = match &m.ret_ty {
                    Some(ann) => type_expr_to_ty(&ann.node, &mut ctx),
                    None      => { *ctx.counter += 1; Ty::Var(*ctx.counter) }
                };
                let row_bounds = collect_row_bounds(&m.type_params, &type_param_vars, &mut ctx);
                ctx.env.pop();
                let fn_ty = Ty::Fn { params: param_tys, ret: Box::new(ret_ty) };
                let fn_ty = if type_param_vars.is_empty() {
                    fn_ty
                } else {
                    Ty::Forall { vars: type_param_vars, body: Box::new(fn_ty) }
                };
                let qname = format!("{}.{}", type_name, m.name.node);
                ctx.env.define(qname.clone(), fn_ty);
                let meta: Vec<(String, bool)> = m.params.iter()
                    .map(|p| (p.name.node.clone(), p.default.is_some()))
                    .collect();
                ctx.env.define_param_meta(qname.clone(), meta);
                ctx.env.define_row_bounds(qname, row_bounds);
            }
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
