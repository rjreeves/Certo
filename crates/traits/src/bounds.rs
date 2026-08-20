use std::collections::HashMap;
use crate::trait_db::TraitDb;
use crate::error::{TraitError, TraitErrorKind};
use certo_ast::decl::{Decl, FnDecl, FnParam};
use certo_ast::expr::{Arg, Expr, ExpectMatcher, Stmt};
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
    for_each_body(module, &mut |params, body| {
        let locals = params_to_locals(params);
        for_each_call(body, &locals, &mut |func, args, _locals| {
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
/// computed expression is not tracked. Covers both bare-name calls (`f(x)`)
/// and `Type.method(x)` dot-call syntax (via `callee_lookup_key`).
pub fn check_generic_call_bounds(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    let generics = collect_generic_fns(module);

    let mut errors = Vec::new();
    for_each_body(module, &mut |params, body| {
        let locals = params_to_locals(params);
        for_each_call(body, &locals, &mut |func, args, locals| {
            let Some(name) = callee_lookup_key(&func.node, locals) else { return };
            let Some(sig) = generics.get(&name) else { return };

            // BACKLOG item 178 — a genuine UFCS instance call on a known
            // local (`c.callGreet(x)`) is spliced by `certo-typeck` into
            // `Caller.callGreet(c, x)` before typeck ever sees it — so a
            // method meant to be callable this way must declare the
            // receiver as an explicit leading param, and `sig.params`
            // includes it. Mirror that same splice here before zipping
            // against `args` (which, unlike typeck's rewritten AST, is
            // still the call's *original* argument list with no receiver),
            // or a receiver-inclusive signature's params and args go out
            // of alignment by one — silently skipping the bound check on
            // whichever param the missing slot shifts onto.
            let spliced_args;
            let effective_args: &[Arg] = match &func.node {
                Expr::Field { expr, .. } if matches!(&expr.node, Expr::Path { path, .. }
                    if path.segments.first().map(|s| {
                        !s.node.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                            && locals.contains_key(&s.node)
                    }).unwrap_or(false)) =>
                {
                    spliced_args = std::iter::once(Arg { label: None, value: (**expr).clone(), span: expr.span })
                        .chain(args.iter().cloned())
                        .collect::<Vec<_>>();
                    &spliced_args
                }
                _ => args,
            };

            for (param, arg) in sig.params.iter().zip(effective_args.iter()) {
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

/// Top-level functions are keyed by bare name; impl methods by `"Type.method"`
/// — matching how `resolve_callee` in `certo_typeck` and `qualified_call_name`
/// below resolve a `Type.method(...)` call site. Keying impl methods by bare
/// name alone would collide whenever two impls declare a same-named method.
fn collect_generic_fns(module: &Module) -> HashMap<String, GenericFnSig> {
    let mut out = HashMap::new();
    let mut add = |key: String, f: &FnDecl| {
        if f.type_params.is_empty() { return; }
        out.insert(key, GenericFnSig {
            type_params: f.type_params.clone(),
            params:      f.params.clone(),
        });
    };
    for sd in &module.decls {
        match &sd.node {
            Decl::Fn(f) => add(f.name.node.clone(), f),
            Decl::Impl(i) => {
                let type_name = i.type_path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                for m in &i.methods { add(format!("{}.{}", type_name, m.name.node), m); }
            }
            _ => {}
        }
    }
    out
}

fn is_bare_type_param(ty: &TypeExpr, param_name: &str) -> bool {
    matches!(ty, TypeExpr::Named { path, args, .. }
        if args.is_empty() && path.segments.len() == 1 && path.segments[0].node == param_name)
}

/// Resolve a call's callee to a lookup key: a bare `Path` by its name, a
/// `Type.method(...)` dot-call (`Expr::Field` on an uppercase-first-segment
/// `Path`) by its qualified `"Type.method"` name, or (BACKLOG item 178)
/// `record.method(...)` UFCS on a lowercase local whose type is in `locals`
/// (a parameter, or an explicitly-annotated `val`/`var`). Still `None` for
/// a receiver not in `locals` (an unannotated `val`, a chained call) —
/// this crate still has no *real* type inference, only what's
/// syntactically obvious.
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

/// Call `visit` once for every function/impl-method body in the module,
/// along with that function/method's own `params` — BACKLOG item 178:
/// callers use this to build a `LocalTypes` map before walking the body,
/// so a UFCS receiver matching one of these parameter names can resolve.
fn for_each_body<'a>(module: &'a Module, visit: &mut dyn FnMut(&'a [FnParam], &'a S<Expr>)) {
    for sd in &module.decls {
        match &sd.node {
            Decl::Fn(f) => { if let Some(body) = &f.body { visit(&f.params, body); } }
            Decl::Impl(i) => {
                for m in &i.methods { if let Some(body) = &m.body { visit(&m.params, body); } }
            }
            _ => {}
        }
    }
}

/// Known types of local names *visible from their own syntax alone* —
/// function/method parameters (always explicitly typed in Certo) and
/// `val`/`var` locals with an explicit annotation — BACKLOG item 178,
/// same design as `certo_effects::infer_effects::LocalTypes` (duplicated
/// rather than shared across these two independent crates, matching how
/// small helpers are already duplicated elsewhere in this codebase, e.g.
/// `crates/cli/src/main.rs`'s own `snake_to_pascal` alongside
/// `certo_dbschema`'s). Deliberately not real type inference: an
/// unannotated `val` or a chained call's own return value used directly
/// as a receiver is never in this map, and `callee_lookup_key` correctly
/// falls back to `None` (unresolved, same as before this item) for those.
pub type LocalTypes = HashMap<String, String>;

fn params_to_locals(params: &[FnParam]) -> LocalTypes {
    params.iter()
        .filter_map(|p| type_expr_simple_name(&p.ty.node).map(|ty| (p.name.node.clone(), ty)))
        .collect()
}

/// Recursively walk `expr`, calling `visit(func, args, locals)` for every
/// call expression (`Expr::App`) found anywhere inside it. `locals` is
/// extended (cloned, not mutated in place) on entry to each `Expr::Block`
/// so a `val`/`var` declared inside it — if explicitly annotated — is
/// resolvable for the rest of that block without leaking into whatever
/// runs after the block ends, same scoping approach as
/// `certo_effects::infer_effects::infer_expr`'s own `Expr::Block` arm.
fn for_each_call<'a>(expr: &'a S<Expr>, locals: &LocalTypes, visit: &mut dyn FnMut(&'a S<Expr>, &'a [Arg], &LocalTypes)) {
    match &expr.node {
        Expr::App { func, args, span: _ } => {
            visit(func, args, locals);
            for_each_call(func, locals, visit);
            for arg in args { for_each_call(&arg.value, locals, visit); }
        }
        Expr::Block { stmts, .. } => {
            let mut scope = locals.clone();
            for stmt in stmts {
                if let Some((name, ty)) = for_each_call_stmt(stmt, &scope, visit) {
                    scope.insert(name, ty);
                }
            }
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            for_each_call(cond, locals, visit);
            for_each_call(then_expr, locals, visit);
            for_each_call(else_expr, locals, visit);
        }
        Expr::BinOp { left, right, .. } | Expr::Pipe { left, right, .. } => {
            for_each_call(left, locals, visit);
            for_each_call(right, locals, visit);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } => for_each_call(expr, locals, visit),
        Expr::Lambda { body, .. } => for_each_call(body, locals, visit),
        Expr::Match { scrutinee, arms, .. } => {
            for_each_call(scrutinee, locals, visit);
            for arm in arms { for_each_call(&arm.body, locals, visit); }
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { for_each_call(e, locals, visit); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { for_each_call(b, locals, visit); }
            for f in fields { for_each_call(&f.value, locals, visit); }
        }
        Expr::For { iter, body, .. } => {
            for_each_call(iter, locals, visit);
            for_each_call(body, locals, visit);
        }
        Expr::Guard { cond, else_expr, .. } | Expr::While { cond, body: else_expr, .. } => {
            for_each_call(cond, locals, visit);
            for_each_call(else_expr, locals, visit);
        }
        Expr::Require { expr, error, .. } => {
            for_each_call(expr, locals, visit);
            for_each_call(error, locals, visit);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { for_each_call(t, locals, visit); }
            if let Some(to) = timeout { for_each_call(to, locals, visit); }
        }
        Expr::WithTimeout { duration, body, .. } => {
            for_each_call(duration, locals, visit);
            for_each_call(body, locals, visit);
        }
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => {
            for_each_call(body, locals, visit);
        }
        Expr::Age { expr, .. } => for_each_call(expr, locals, visit),
        Expr::ExpectAssertion { actual, matcher, .. } => {
            for_each_call(actual, locals, visit);
            if let ExpectMatcher::ToBe(y) = matcher { for_each_call(y, locals, visit); }
        }
        // Terminals — nothing to recurse into
        Expr::Lit { .. } | Expr::Path { .. } => {}
    }
}

/// Returns `Some((name, type))` for a `val`/`var` with an explicit type
/// annotation binding a plain name, so `Expr::Block`'s own arm above can
/// extend its scope — same convention as
/// `certo_effects::infer_effects::infer_stmt`.
fn for_each_call_stmt<'a>(stmt: &'a Stmt, locals: &LocalTypes, visit: &mut dyn FnMut(&'a S<Expr>, &'a [Arg], &LocalTypes)) -> Option<(String, String)> {
    match stmt {
        Stmt::Expr   { expr, .. }  => { for_each_call(expr, locals, visit); None }
        Stmt::Val    { pattern, ty, value, .. } => {
            for_each_call(value, locals, visit);
            let certo_ast::pattern::Pattern::Ident { name, .. } = &pattern.node else { return None };
            let ty = type_expr_simple_name(&ty.as_ref()?.node)?;
            Some((name.node.clone(), ty))
        }
        Stmt::Var    { name, ty, value, .. } => {
            for_each_call(value, locals, visit);
            let ty = type_expr_simple_name(&ty.as_ref()?.node)?;
            Some((name.node.clone(), ty))
        }
        Stmt::Assign { value, .. } => { for_each_call(value, locals, visit); None }
        Stmt::Defer  { body, .. }  => { for_each_call(body, locals, visit); None }
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
