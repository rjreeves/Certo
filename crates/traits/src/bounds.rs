use std::collections::HashMap;
use crate::trait_db::TraitDb;
use crate::error::{TraitError, TraitErrorKind};
use certo_ast::decl::{Decl, FnDecl};
use certo_ast::expr::{Expr, Stmt};
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
    for sd in &module.decls {
        if let Decl::Fn(f) = &sd.node {
            if let Some(body) = &f.body {
                walk_expr(body, &fn_ret, db, &mut errors);
            }
        }
    }
    errors
}

fn type_expr_simple_name(te: &TypeExpr) -> Option<String> {
    if let TypeExpr::Named { path, .. } = te {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

fn walk_expr(
    expr:   &S<Expr>,
    fn_ret: &HashMap<String, String>,
    db:     &TraitDb,
    errors: &mut Vec<TraitError>,
) {
    match &expr.node {
        Expr::App { func, args, span: _ } => {
            let is_dbqt = matches!(&func.node,
                Expr::Path { path, .. }
                if path.segments.last().map(|s| s.node.as_str()) == Some("dbQueryTyped")
            );

            if is_dbqt && args.len() >= 4 {
                let mapper = &args[3].value;
                if let Expr::Path { path, .. } = &mapper.node {
                    let mapper_name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                    if let Some(ret_ty) = fn_ret.get(mapper_name) {
                        if !db.implements(ret_ty, "DbRow") {
                            errors.push(TraitError {
                                kind: TraitErrorKind::UnsatisfiedBound {
                                    ty:         ret_ty.clone(),
                                    trait_name: "DbRow".to_string(),
                                },
                                span: mapper.span,
                            });
                        }
                    }
                }
            }

            walk_expr(func, fn_ret, db, errors);
            for arg in args { walk_expr(&arg.value, fn_ret, db, errors); }
        }

        Expr::Block { stmts, .. } => {
            for stmt in stmts { walk_stmt(stmt, fn_ret, db, errors); }
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            walk_expr(cond, fn_ret, db, errors);
            walk_expr(then_expr, fn_ret, db, errors);
            walk_expr(else_expr, fn_ret, db, errors);
        }
        Expr::BinOp { left, right, .. } | Expr::Pipe { left, right, .. } => {
            walk_expr(left, fn_ret, db, errors);
            walk_expr(right, fn_ret, db, errors);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } => walk_expr(expr, fn_ret, db, errors),
        Expr::Lambda { body, .. } => walk_expr(body, fn_ret, db, errors),
        Expr::Match { scrutinee, arms, .. } => {
            walk_expr(scrutinee, fn_ret, db, errors);
            for arm in arms { walk_expr(&arm.body, fn_ret, db, errors); }
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { walk_expr(e, fn_ret, db, errors); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { walk_expr(b, fn_ret, db, errors); }
            for f in fields { walk_expr(&f.value, fn_ret, db, errors); }
        }
        Expr::For { iter, body, .. } => {
            walk_expr(iter, fn_ret, db, errors);
            walk_expr(body, fn_ret, db, errors);
        }
        Expr::Guard { cond, else_expr, .. } | Expr::While { cond, body: else_expr, .. } => {
            walk_expr(cond, fn_ret, db, errors);
            walk_expr(else_expr, fn_ret, db, errors);
        }
        Expr::Require { expr, error, .. } => {
            walk_expr(expr, fn_ret, db, errors);
            walk_expr(error, fn_ret, db, errors);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { walk_expr(t, fn_ret, db, errors); }
            if let Some(to) = timeout { walk_expr(to, fn_ret, db, errors); }
        }
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => {
            walk_expr(body, fn_ret, db, errors);
        }
        Expr::Age { expr, .. } => walk_expr(expr, fn_ret, db, errors),
        // Terminals — nothing to recurse into
        Expr::Lit { .. } | Expr::Path { .. } => {}
    }
}

fn walk_stmt(
    stmt:   &Stmt,
    fn_ret: &HashMap<String, String>,
    db:     &TraitDb,
    errors: &mut Vec<TraitError>,
) {
    match stmt {
        Stmt::Expr   { expr, .. }  => walk_expr(expr, fn_ret, db, errors),
        Stmt::Val    { value, .. } => walk_expr(value, fn_ret, db, errors),
        Stmt::Var    { value, .. } => walk_expr(value, fn_ret, db, errors),
        Stmt::Assign { value, .. } => walk_expr(value, fn_ret, db, errors),
        Stmt::Defer  { body, .. }  => walk_expr(body, fn_ret, db, errors),
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
