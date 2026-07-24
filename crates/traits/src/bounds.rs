use std::collections::HashMap;
use crate::trait_db::TraitDb;
use crate::error::{TraitError, TraitErrorKind};
use certo_ast::decl::{Decl, FnDecl, FnParam};
use certo_ast::expr::{Arg, Expr, Stmt};
use certo_ast::module::Module;
use certo_ast::span::S;
use certo_ast::types::{TypeExpr, TypeParam};

/// Walk every function declaration in the module and verify that
/// concrete types supplied for bounded type parameters actually
/// implement the required traits.
///
/// This is a *static* check over the surface syntax — it catches cases
/// like `fn f<T: Serialize>(x: T)` being called with a type that has no
/// `impl Serialize for …` in the same module.  Cross-module satisfaction
/// is deferred to the linker/module-loader phase.
pub fn check_bounds(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            check_fn_bounds(f, db, &mut errors);
        }
    }
    errors
}

fn check_fn_bounds(f: &FnDecl, db: &TraitDb, errors: &mut Vec<TraitError>) {
    for tp in &f.type_params {
        check_type_param(tp, db, errors);
    }
}

fn check_type_param(tp: &TypeParam, db: &TraitDb, errors: &mut Vec<TraitError>) {
    // For each bound `T: SomeTrait`, verify there is at least one registered
    // impl of SomeTrait.  We cannot know the concrete T at this stage, so we
    // only flag bounds that refer to traits that don't exist at all.
    // Row bounds (`R: { name: Text }`) aren't traits — certo_typeck checks
    // those against the concrete type at each call site instead.
    for bound in &tp.bounds {
        let certo_ast::types::Bound::Trait(bound) = bound else { continue };
        let trait_name = bound.name.segments
            .iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
        if !db.traits.contains_key(&trait_name) {
            // Unknown trait in a bound — this should be caught by name
            // resolution, so we emit a best-effort error here for defence.
            errors.push(TraitError {
                kind: TraitErrorKind::UnsatisfiedBound {
                    ty:         tp.name.node.clone(),
                    trait_name: trait_name.clone(),
                },
                span: bound.span,
            });
        }
    }
}

/// Walk every `dbQueryTyped(conn, sql, params, mapper)` call in the module
/// and verify the mapper's return type implements `DbRow`.
///
/// Only fires when the mapper argument is a direct function reference (a `Path`
/// naming a function declared in this module). Lambda mappers are skipped —
/// we can't resolve their return type without full inference.
pub fn check_dbquery_typed_bounds(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    // Build name → return type name for every FnDecl in the module.
    let fn_ret: HashMap<String, String> = module.decls.iter()
        .filter_map(|sd| if let Decl::Fn(f) = &sd.node { Some(f) } else { None })
        .filter_map(|f| {
            let ret_name = f.ret_ty.as_ref().and_then(|r| type_expr_simple_name(&r.node))?;
            Some((f.name.node.clone(), ret_name))
        })
        .collect();

    let mut errors = Vec::new();
    for_each_body(module, &mut |body| {
        for_each_call(body, &mut |func, args| {
            let is_dbqt = matches!(&func.node,
                Expr::Path { path, .. }
                if path.segments.last().map(|s| s.node.as_str()) == Some("dbQueryTyped")
            );
            if !is_dbqt || args.len() < 4 { return; }

            let mapper = &args[3].value;
            let Expr::Path { path, .. } = &mapper.node else { return };
            let mapper_name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            let Some(ret_ty) = fn_ret.get(mapper_name) else { return };
            if !db.implements(ret_ty, "DbRow") {
                errors.push(TraitError {
                    kind: TraitErrorKind::UnsatisfiedBound {
                        ty:         ret_ty.clone(),
                        trait_name: "DbRow".to_string(),
                    },
                    span: mapper.span,
                });
            }
        });
    });
    errors
}

fn type_expr_simple_name(te: &TypeExpr) -> Option<String> {
    if let TypeExpr::Named { path, .. } = te {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

/// Best-effort static check: for a call `f(args...)` where `f` is a
/// module-local function or impl method with a `Trait`-bounded type param `T`,
/// and an argument at a parameter position declared as bare `T` is a record
/// literal `TypeName { .. }`, verify `TypeName` satisfies the bound.
///
/// Like `check_dbquery_typed_bounds`, this only fires when the concrete type
/// is directly visible in the call's own AST (a record-literal argument) —
/// this crate has no type inference, so a value passed via a variable or a
/// computed expression is not tracked. Only bare-name calls (`f(x)`) are
/// covered, not `Type.method(...)` dot-call syntax (mirrors the same v1
/// scope limitation as row-polymorphism bound checking in `certo-typeck`).
pub fn check_generic_call_bounds(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    let generics = collect_generic_fns(module);

    let mut errors = Vec::new();
    for_each_body(module, &mut |body| {
        for_each_call(body, &mut |func, args| {
            let Expr::Path { path, .. } = &func.node else { return };
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            let Some(sig) = generics.get(name) else { return };

            for (param, arg) in sig.params.iter().zip(args.iter()) {
                let Some(tp) = sig.type_params.iter().find(|tp| is_bare_type_param(&param.ty.node, &tp.name.node))
                    else { continue };
                if tp.bounds.iter().all(|b| matches!(b, certo_ast::types::Bound::Row(_))) { continue; }
                let Expr::Record { ty_name: Some(concrete), .. } = &arg.value.node else { continue };
                check_call_bounds(concrete, std::slice::from_ref(tp), db, &mut errors, arg.value.span);
            }
        });
    });
    errors
}

struct GenericFnSig {
    type_params: Vec<TypeParam>,
    params:      Vec<FnParam>,
}

fn collect_generic_fns(module: &Module) -> HashMap<String, GenericFnSig> {
    let mut out = HashMap::new();
    let mut add = |f: &FnDecl| {
        if f.type_params.is_empty() { return; }
        out.insert(f.name.node.clone(), GenericFnSig {
            type_params: f.type_params.clone(),
            params:      f.params.clone(),
        });
    };
    for sd in &module.decls {
        match &sd.node {
            Decl::Fn(f) => add(f),
            Decl::Impl(i) => { for m in &i.methods { add(m); } }
            _ => {}
        }
    }
    out
}

fn is_bare_type_param(ty: &TypeExpr, param_name: &str) -> bool {
    matches!(ty, TypeExpr::Named { path, args, .. }
        if args.is_empty() && path.segments.len() == 1 && path.segments[0].node == param_name)
}

/// Call `visit` once for every function/impl-method body in the module.
fn for_each_body<'a>(module: &'a Module, visit: &mut dyn FnMut(&'a S<Expr>)) {
    for sd in &module.decls {
        match &sd.node {
            Decl::Fn(f) => { if let Some(body) = &f.body { visit(body); } }
            Decl::Impl(i) => {
                for m in &i.methods { if let Some(body) = &m.body { visit(body); } }
            }
            _ => {}
        }
    }
}

/// Recursively walk `expr`, calling `visit(func, args)` for every call
/// expression (`Expr::App`) found anywhere inside it.
fn for_each_call<'a>(expr: &'a S<Expr>, visit: &mut dyn FnMut(&'a S<Expr>, &'a [Arg])) {
    match &expr.node {
        Expr::App { func, args, span: _ } => {
            visit(func, args);
            for_each_call(func, visit);
            for arg in args { for_each_call(&arg.value, visit); }
        }
        Expr::Block { stmts, .. } => {
            for stmt in stmts { for_each_call_stmt(stmt, visit); }
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            for_each_call(cond, visit);
            for_each_call(then_expr, visit);
            for_each_call(else_expr, visit);
        }
        Expr::BinOp { left, right, .. } | Expr::Pipe { left, right, .. } => {
            for_each_call(left, visit);
            for_each_call(right, visit);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } => for_each_call(expr, visit),
        Expr::Lambda { body, .. } => for_each_call(body, visit),
        Expr::Match { scrutinee, arms, .. } => {
            for_each_call(scrutinee, visit);
            for arm in arms { for_each_call(&arm.body, visit); }
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { for_each_call(e, visit); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { for_each_call(b, visit); }
            for f in fields { for_each_call(&f.value, visit); }
        }
        Expr::For { iter, body, .. } => {
            for_each_call(iter, visit);
            for_each_call(body, visit);
        }
        Expr::Guard { cond, else_expr, .. } | Expr::While { cond, body: else_expr, .. } => {
            for_each_call(cond, visit);
            for_each_call(else_expr, visit);
        }
        Expr::Require { expr, error, .. } => {
            for_each_call(expr, visit);
            for_each_call(error, visit);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { for_each_call(t, visit); }
            if let Some(to) = timeout { for_each_call(to, visit); }
        }
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => {
            for_each_call(body, visit);
        }
        Expr::Age { expr, .. } => for_each_call(expr, visit),
        // Terminals — nothing to recurse into
        Expr::Lit { .. } | Expr::Path { .. } => {}
    }
}

fn for_each_call_stmt<'a>(stmt: &'a Stmt, visit: &mut dyn FnMut(&'a S<Expr>, &'a [Arg])) {
    match stmt {
        Stmt::Expr   { expr, .. }  => for_each_call(expr, visit),
        Stmt::Val    { value, .. } => for_each_call(value, visit),
        Stmt::Var    { value, .. } => for_each_call(value, visit),
        Stmt::Assign { value, .. } => for_each_call(value, visit),
        Stmt::Defer  { body, .. }  => for_each_call(body, visit),
    }
}

/// At a call site `f::<ConcreteType>(…)`, verify `ConcreteType` satisfies
/// the bounds declared on `f`'s type parameter.
///
/// `concrete_type` is the string name of the type being substituted.
/// `type_params` are the function's declared type parameters.
pub fn check_call_bounds(
    concrete_type: &str,
    type_params:   &[TypeParam],
    db:            &TraitDb,
    errors:        &mut Vec<TraitError>,
    span:          certo_ast::span::Span,
) {
    for tp in type_params {
        for bound in &tp.bounds {
            let certo_ast::types::Bound::Trait(bound) = bound else { continue };
            let trait_name = bound.name.segments
                .iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            if !db.implements(concrete_type, &trait_name) {
                errors.push(TraitError {
                    kind: TraitErrorKind::UnsatisfiedBound {
                        ty:         concrete_type.to_string(),
                        trait_name: trait_name.clone(),
                    },
                    span,
                });
            }
        }
    }
}
