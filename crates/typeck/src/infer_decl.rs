use certo_ast::module::Module;
use certo_ast::decl::Decl;
use certo_ast::span::{S, Span};
use certo_ast::expr::{Expr, ExpectMatcher};
use certo_ast::types::TypeExpr;
use crate::ty::Ty;
use crate::env::TypeEnv;
use crate::unify::UnionFind;
use crate::error::{TypeError, TypeErrorKind, Warning, WarningKind};
use crate::infer_expr::{infer, infer_block, type_expr_to_ty, literal_matches_fixed_width, Ctx};

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

    // Pass 4 — smart constructors: a `priv` type's raw constructor may only
    // be called from within an `impl` block for that same type.
    check_priv_ctors(module, &mut errors);

    // Pass 5 — E0704: a named constraint referenced from a validator rule
    // must only touch fields in that validator's own scope.
    check_constraint_scope(module, &mut errors);

    // Pass 6 — E0700/E0701/E0702: a validator's `after`/`overrides`
    // references must name real rules in the same validator, and the
    // `after` dependency graph must be acyclic (BACKLOG item 252, ported
    // from `crates/resolve`, which never actually ran for the real CLI).
    check_rule_dependencies(module, &mut errors);

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
        Expr::ExpectAssertion { actual, matcher, .. } => {
            walk_ffi(actual, in_unsafe, extern_fns, errors);
            if let ExpectMatcher::ToBe(y) = matcher { walk_ffi(y, in_unsafe, extern_fns, errors); }
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
        Expr::WithTimeout { duration, body, .. } => {
            walk_ffi(duration, in_unsafe, extern_fns, errors);
            walk_ffi(body, in_unsafe, extern_fns, errors);
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { walk_ffi(b, in_unsafe, extern_fns, errors); }
            for fld in fields { walk_ffi(&fld.value, in_unsafe, extern_fns, errors); }
        }
    }
}

// ------------------------------------------------------------------ //
// E0704 — named-constraint field scope (BACKLOG item 225, split out of
// item 220's own investigation)
// ------------------------------------------------------------------ //

/// Generic recursive expression walker calling `visit` on every sub-node —
/// exhaustive over every `Expr` variant, mirroring `walk_ffi` above (a
/// missed variant here would just be a false-negative on this one new
/// diagnostic, not a regression of already-working behavior, but kept
/// exhaustive anyway so the compiler forces an update if `Expr` ever grows
/// a new shape, matching this file's own established convention).
fn walk_expr(expr: &S<Expr>, visit: &mut dyn FnMut(&Expr)) {
    use certo_ast::expr::{Lit, FStringPart, Stmt};
    visit(&expr.node);
    match &expr.node {
        Expr::Lit { value: Lit::FString(parts), .. } => {
            for p in parts {
                if let FStringPart::Interpolated(e) = p { walk_expr(e, visit); }
            }
        }
        Expr::Lit { .. } | Expr::Path { .. } => {}
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            walk_expr(left, visit);
            walk_expr(right, visit);
        }
        Expr::App { func, args, .. } => {
            walk_expr(func, visit);
            for a in args { walk_expr(&a.value, visit); }
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. } | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. } | Expr::Await { expr, .. } | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } | Expr::Age { expr, .. } => walk_expr(expr, visit),
        Expr::ExpectAssertion { actual, matcher, .. } => {
            walk_expr(actual, visit);
            if let ExpectMatcher::ToBe(y) = matcher { walk_expr(y, visit); }
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            walk_expr(cond, visit);
            walk_expr(then_expr, visit);
            walk_expr(else_expr, visit);
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_expr(scrutinee, visit);
            for arm in arms {
                if let Some(g) = &arm.guard { walk_expr(g, visit); }
                walk_expr(&arm.body, visit);
            }
        }
        Expr::Block { stmts, .. } => {
            for st in stmts {
                match st {
                    Stmt::Val { value, .. } | Stmt::Var { value, .. } | Stmt::Assign { value, .. } =>
                        walk_expr(value, visit),
                    Stmt::Defer { body, .. } | Stmt::Expr { expr: body, .. } => walk_expr(body, visit),
                }
            }
        }
        Expr::Lambda { body, .. } | Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } =>
            walk_expr(body, visit),
        Expr::For { iter, body, .. } => {
            walk_expr(iter, visit);
            walk_expr(body, visit);
        }
        Expr::While { cond, body, .. } => {
            walk_expr(cond, visit);
            walk_expr(body, visit);
        }
        Expr::Guard { cond, else_expr, .. } => {
            walk_expr(cond, visit);
            walk_expr(else_expr, visit);
        }
        Expr::Require { expr, error, .. } => {
            walk_expr(expr, visit);
            walk_expr(error, visit);
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { walk_expr(e, visit); }
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { walk_expr(t, visit); }
            if let Some(t) = timeout { walk_expr(t, visit); }
        }
        Expr::WithTimeout { duration, body, .. } => {
            walk_expr(duration, visit);
            walk_expr(body, visit);
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { walk_expr(b, visit); }
            for fld in fields { walk_expr(&fld.value, visit); }
        }
    }
}

/// E0704 — a named `constraint`'s body references fields (`user.role`,
/// `customer.status`) that don't exist at its own declaration site by
/// design (spec §16.8's "deferred resolution": a constraint's body is
/// never type-checked until it's actually used inside a validator, where
/// the context fields are finally known). This is that deferred check,
/// run directly against every `Decl::Validator` — unlike the *generated*
/// code's own inlined-constraint checking (item 220), this runs under
/// `certo check` too, since it doesn't depend on `expand_validators` ever
/// having run at all.
///
/// Scoped to *direct* constraint references (a rule's `require`/`else`
/// naming a constraint by its bare name) — a constraint referencing
/// *another* constraint isn't transitively expanded here; not attempted,
/// since no example in `docs/section-16-validators.md` demonstrates it and
/// guessing at the right resolution order wasn't worth it for an
/// unconfirmed need.
fn check_constraint_scope(module: &Module, errors: &mut Vec<TypeError>) {
    use std::collections::{HashMap, HashSet};

    let constraints: HashMap<&str, &S<Expr>> = module.decls.iter()
        .filter_map(|d| match &d.node {
            Decl::Constraint(c) => Some((c.name.node.as_str(), &c.body)),
            _ => None,
        })
        .collect();
    if constraints.is_empty() { return; }

    for sdecl in &module.decls {
        let Decl::Validator(v) = &sdecl.node else { continue };

        let mut scope: HashSet<String> = HashSet::new();
        if let Some(entity_name) = type_expr_simple_name(&v.entity.node) {
            scope.insert(lowercase_first(&entity_name));
        }
        for field in &v.context {
            scope.insert(field.name.node.clone());
        }

        for rule in &v.rules {
            for site in [&rule.require, &rule.else_] {
                // Every bare, single-segment name in this require/else that
                // happens to match a declared constraint — a real reference,
                // not a guess (a name that isn't a constraint is simply
                // ignored here; normal inference already checks it elsewhere).
                let mut referenced: Vec<String> = Vec::new();
                walk_expr(site, &mut |e| {
                    if let Expr::Path { path, .. } = e {
                        if path.segments.len() == 1 {
                            let name = path.segments[0].node.as_str();
                            if constraints.contains_key(name) && !referenced.iter().any(|r| r == name) {
                                referenced.push(name.to_string());
                            }
                        }
                    }
                });

                for cname in referenced {
                    let body = constraints[cname.as_str()];
                    let mut bases: Vec<String> = Vec::new();
                    walk_expr(body, &mut |e| {
                        if let Expr::Field { expr, .. } | Expr::SafeField { expr, .. } = e {
                            if let Expr::Path { path, .. } = &expr.node {
                                if path.segments.len() == 1 {
                                    let base = path.segments[0].node.clone();
                                    let starts_lower = base.chars().next()
                                        .map(|c| c.is_lowercase()).unwrap_or(false);
                                    if starts_lower && !bases.contains(&base) { bases.push(base); }
                                }
                            }
                        }
                    });
                    for base in bases {
                        if !scope.contains(&base) {
                            errors.push(TypeError {
                                kind: TypeErrorKind::ConstraintFieldNotInScope {
                                    constraint_name: cname.to_string(),
                                    field_name: base,
                                },
                                span: site.span,
                            });
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ //
// Rule dependency validation — E0700/E0701/E0702 (BACKLOG item 252).
// Ported directly from `crates/resolve/src/resolve_decl.rs`'s own
// `resolve_validator`/`detect_rule_cycles`/`dfs_cycle` (unchanged logic,
// just `ResolveError`/`ResolveErrorKind` → `TypeError`/`TypeErrorKind`) —
// that crate's own version is fully correct, it just never actually runs
// for the real CLI (`crates/resolve` is dead code there; its only real
// consumer is the LSP). Runs directly against the un-expanded
// `Decl::Validator` AST, matching `check_constraint_scope`'s own
// no-`certo_resolve`-dependency design just above.
// ------------------------------------------------------------------ //

fn check_rule_dependencies(module: &Module, errors: &mut Vec<TypeError>) {
    use std::collections::{HashMap, HashSet};
    for sdecl in &module.decls {
        let Decl::Validator(v) = &sdecl.node else { continue };

        let rule_map: HashMap<&str, ()> = v.rules.iter().map(|r| (r.name.node.as_str(), ())).collect();

        let mut has_ref_errors = false;
        for rule in &v.rules {
            for after_ref in &rule.after {
                if !rule_map.contains_key(after_ref.node.as_str()) {
                    errors.push(TypeError {
                        kind: TypeErrorKind::AfterRuleNotFound {
                            rule_name:  rule.name.node.clone(),
                            after_name: after_ref.node.clone(),
                        },
                        span: after_ref.span,
                    });
                    has_ref_errors = true;
                }
            }
            if let Some(ov) = &rule.overrides {
                if !rule_map.contains_key(ov.node.as_str()) {
                    errors.push(TypeError {
                        kind: TypeErrorKind::OverridesRuleNotFound {
                            rule_name:      rule.name.node.clone(),
                            overrides_name: ov.node.clone(),
                        },
                        span: ov.span,
                    });
                    has_ref_errors = true;
                }
            }
        }

        // Cycle detection — only meaningful when all references resolved.
        if !has_ref_errors {
            let adj: HashMap<&str, Vec<&str>> = v.rules.iter()
                .map(|r| (r.name.node.as_str(), r.after.iter().map(|a| a.node.as_str()).collect()))
                .collect();
            let mut visited:   HashSet<&str> = HashSet::new();
            let mut rec_stack: Vec<&str>     = Vec::new();
            for rule in &v.rules {
                let name = rule.name.node.as_str();
                if !visited.contains(name) {
                    if let Some(cycle) = dfs_rule_cycle(name, &adj, &mut visited, &mut rec_stack) {
                        errors.push(TypeError {
                            kind: TypeErrorKind::RuleCycle {
                                validator: v.name.node.clone(),
                                cycle,
                            },
                            span: v.span,
                        });
                        break; // report only the first cycle
                    }
                }
            }
        }
    }
}

fn dfs_rule_cycle<'a>(
    node:      &'a str,
    adj:       &std::collections::HashMap<&'a str, Vec<&'a str>>,
    visited:   &mut std::collections::HashSet<&'a str>,
    rec_stack: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    visited.insert(node);
    rec_stack.push(node);

    if let Some(neighbors) = adj.get(node) {
        for &next in neighbors {
            if !visited.contains(next) {
                if let Some(cycle) = dfs_rule_cycle(next, adj, visited, rec_stack) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(&next) {
                let start = rec_stack.iter().position(|&n| n == next).unwrap();
                let mut cycle: Vec<String> = rec_stack[start..].iter().map(|&n| n.to_string()).collect();
                cycle.push(next.to_string());
                return Some(cycle);
            }
        }
    }

    rec_stack.pop();
    None
}

// ------------------------------------------------------------------ //
// Validator warnings (BACKLOG item 230) — W0100/W0101/W0102. Unlike every
// pass above, these are non-fatal: collected separately from `TypeError`
// and never abort compilation on their own (see `crate::error::Warning`'s
// own doc comment). Run directly against the un-expanded `Decl::Validator`
// AST, the same way `check_constraint_scope` above does, so these work
// under `certo check` alone with no dependency on `expand_validators`.
// ------------------------------------------------------------------ //

/// Entry point: collect every validator warning in a module. Called
/// separately from `check_module_seeded` (which only ever returns hard
/// errors) — see that function's own doc comment for why a parallel
/// `Vec<Warning>` was chosen over widening its own return type.
pub fn check_module_warnings(module: &Module) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Validator(v) => {
                check_validator_overrides(v, &mut warnings);
                check_validator_context_loaded_by(v, &mut warnings);
            }
            Decl::Fn(f) => {
                if let Some(body) = &f.body {
                    check_parallel_shared_mutable_state(&body.node, &mut warnings);
                }
            }
            Decl::Impl(i) => {
                for m in &i.methods {
                    if let Some(body) = &m.body {
                        check_parallel_shared_mutable_state(&body.node, &mut warnings);
                    }
                }
            }
            _ => {}
        }
    }
    warnings
}

/// W0100/W0101 — a rule's `overrides` relationship with no explicit
/// `priority` on either rule (W0100), or whose own condition is a
/// compile-time-obvious tautology, permanently shadowing the rule it
/// overrides (W0101).
fn check_validator_overrides(v: &certo_ast::decl::ValidatorDecl, warnings: &mut Vec<Warning>) {
    for rule in &v.rules {
        let Some(overridden) = &rule.overrides else { continue };
        // A name that doesn't resolve to a real rule in this validator is
        // a separate, existing concern (E0702) — not this check's job;
        // skip it silently rather than warning about a rule that may not
        // even exist.
        if !v.rules.iter().any(|r| r.name.node == overridden.node) { continue; }

        if rule.priority.is_none() {
            warnings.push(Warning {
                kind: WarningKind::AmbiguousOverridePriority {
                    rule_name: rule.name.node.clone(),
                    overridden_name: overridden.node.clone(),
                },
                span: rule.span,
            });
        }

        if matches!(&rule.require.node, Expr::Lit { value: certo_ast::expr::Lit::Bool(true), .. }) {
            warnings.push(Warning {
                kind: WarningKind::UnreachableOverriddenRule {
                    overriding_name: rule.name.node.clone(),
                    overridden_name: overridden.node.clone(),
                },
                span: rule.require.span,
            });
        }
    }
}

/// W0102 — a context field with no `loaded by` clause. Mirrors
/// `crates/codegen/src/emit_validator.rs`'s own `all_have_loaded_by` gate
/// exactly (`validateWithDb` is only generated when *every* context field
/// has `loaded by`) — this is that same condition, surfaced as a warning
/// at typeck time instead of silently never generating the function.
fn check_validator_context_loaded_by(v: &certo_ast::decl::ValidatorDecl, warnings: &mut Vec<Warning>) {
    for field in &v.context {
        if field.loaded_by.is_none() {
            warnings.push(Warning {
                kind: WarningKind::ContextFieldMissingLoadedBy {
                    validator_name: v.name.node.clone(),
                    field_name: field.name.node.clone(),
                },
                span: field.span,
            });
        }
    }
}

// ------------------------------------------------------------------ //
// W0103 — shared mutable state across `parallel { }` tasks (BACKLOG item 208)
//
// `parallel { task1, task2, ... }` runs each task expression concurrently
// on a real OS thread (`crates/codegen`/`crates/stdlib`'s own
// `__certo_thread_spawn`, the same primitive `spawn`/`withTimeout` share) —
// spec §7.2's own comment promises "compiler verifies tasks do not share
// mutable state," but nothing ever implemented this.
//
// Scoped narrowly and precisely because of what this language actually
// allows: Certo's entire mutation surface is `name = expr` reassignment of
// a bare local (`Stmt::Assign { target: S<String>, .. }` — confirmed no
// mutable record-field assignment, no references exposed to user code, a
// record's own only "mutation" is the immutable `.with(...)` copy-update).
// So this is a purely syntactic, well-defined check, not general aliasing
// analysis: collect every name assigned to *anywhere* in the enclosing
// function, then for each `parallel { }` block found in that same
// function, flag any such name referenced by two or more of its own
// sibling task expressions.
//
// Deliberately does NOT attempt general `val`/parameter mutability
// enforcement — a real, separate, larger gap found while scoping this item
// (confirmed directly: reassigning a `val` or an ordinary function
// parameter is accepted silently everywhere in this compiler today, not
// just here) — filed separately as BACKLOG item 255. This check identifies
// "mutable" empirically, from `Stmt::Assign` targets actually present in
// the source, independent of that gap; it would keep working correctly
// even after item 255 lands.
//
// Deliberately approximate rather than fully lexically-scoped, mirroring
// `collect_referenced_idents`'s own documented tradeoff
// (`crates/parser/src/parse_decl.rs`) — "assigned anywhere in this
// function" is coarser than "assigned in a scope actually visible to this
// parallel block," so an unrelated same-named local in a distant nested
// scope could in principle produce a false positive. An accepted cost for
// a real, useful check where none existed before, not a design blocker.
// ------------------------------------------------------------------ //

fn check_parallel_shared_mutable_state(body: &Expr, warnings: &mut Vec<Warning>) {
    // First pass: every name assigned to anywhere in this whole function —
    // must be fully populated before any parallel block's own tasks are
    // checked, so this genuinely has to be a separate pass, not folded into
    // the second walk below (which finds the parallel blocks themselves),
    // matching this function's "assigned anywhere in this function" intent
    // regardless of whether a parallel block textually precedes or follows
    // the relevant assignment.
    let mut assigned = std::collections::HashSet::new();
    collect_assigned_names(body, &mut assigned);

    // Second pass: every `parallel { }` block anywhere in this function.
    let mut found: Vec<(Vec<S<Expr>>, Span)> = Vec::new();
    collect_parallels(body, &mut found);
    for (tasks, span) in &found {
        let mut refs_by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for task in tasks {
            let mut refs = std::collections::HashSet::new();
            collect_referenced_names(&task.node, &mut refs);
            for name in refs {
                if assigned.contains(&name) {
                    *refs_by_name.entry(name).or_insert(0) += 1;
                }
            }
        }
        for (name, count) in refs_by_name {
            if count >= 2 {
                warnings.push(Warning {
                    kind: WarningKind::ParallelSharedMutableState { name },
                    span: *span,
                });
            }
        }
    }
}

/// Collects every `Stmt::Assign` target reachable anywhere inside `expr`
/// into `out` — see `check_parallel_shared_mutable_state`'s own doc
/// comment for why "anywhere in the function," not precise lexical scope,
/// is the right granularity here.
fn collect_assigned_names(expr: &Expr, out: &mut std::collections::HashSet<String>) {
    match expr {
        Expr::Lit { value, .. } => {
            if let certo_ast::expr::Lit::FString(parts) = value {
                for p in parts {
                    if let certo_ast::expr::FStringPart::Interpolated(e) = p { collect_assigned_names(&e.node, out); }
                }
            }
        }
        Expr::Path { .. } => {}
        Expr::App { func, args, .. } => {
            collect_assigned_names(&func.node, out);
            for a in args { collect_assigned_names(&a.value.node, out); }
        }
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            collect_assigned_names(&left.node, out);
            collect_assigned_names(&right.node, out);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. }
        | Expr::Age { expr, .. } => collect_assigned_names(&expr.node, out),
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => collect_assigned_names(&body.node, out),
        Expr::If { cond, then_expr, else_expr, .. } => {
            collect_assigned_names(&cond.node, out);
            collect_assigned_names(&then_expr.node, out);
            collect_assigned_names(&else_expr.node, out);
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_assigned_names(&scrutinee.node, out);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_assigned_names(&g.node, out); }
                collect_assigned_names(&arm.body.node, out);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                match s {
                    certo_ast::expr::Stmt::Val { value, .. } | certo_ast::expr::Stmt::Var { value, .. } =>
                        collect_assigned_names(&value.node, out),
                    certo_ast::expr::Stmt::Assign { target, value, .. } => {
                        out.insert(target.node.clone());
                        collect_assigned_names(&value.node, out);
                    }
                    certo_ast::expr::Stmt::Defer { body, .. } => collect_assigned_names(&body.node, out),
                    certo_ast::expr::Stmt::Expr { expr, .. } => collect_assigned_names(&expr.node, out),
                }
            }
        }
        Expr::Lambda { body, .. } => collect_assigned_names(&body.node, out),
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { collect_assigned_names(&e.node, out); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { collect_assigned_names(&b.node, out); }
            for f in fields { collect_assigned_names(&f.value.node, out); }
        }
        Expr::Guard { cond, else_expr, .. } => {
            collect_assigned_names(&cond.node, out);
            collect_assigned_names(&else_expr.node, out);
        }
        Expr::Require { expr, error, .. } => {
            collect_assigned_names(&expr.node, out);
            collect_assigned_names(&error.node, out);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { collect_assigned_names(&t.node, out); }
            if let Some(t) = timeout { collect_assigned_names(&t.node, out); }
        }
        Expr::WithTimeout { duration, body, .. } => {
            collect_assigned_names(&duration.node, out);
            collect_assigned_names(&body.node, out);
        }
        Expr::For { iter, body, .. } => {
            collect_assigned_names(&iter.node, out);
            collect_assigned_names(&body.node, out);
        }
        Expr::While { cond, body, .. } => {
            collect_assigned_names(&cond.node, out);
            collect_assigned_names(&body.node, out);
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            collect_assigned_names(&actual.node, out);
            if let ExpectMatcher::ToBe(e) = matcher { collect_assigned_names(&e.node, out); }
        }
    }
}

/// Collects every `Expr::Parallel { tasks, .. }` occurrence anywhere inside
/// `expr`, cloning its task list and span, for a later, separate pass over
/// `collect_assigned_names`'s own fully-populated result — see
/// `check_parallel_shared_mutable_state`'s doc comment for why this needs
/// to be a genuinely separate second walk.
fn collect_parallels(expr: &Expr, out: &mut Vec<(Vec<S<Expr>>, Span)>) {
    match expr {
        Expr::Parallel { tasks, timeout, span } => {
            out.push((tasks.clone(), *span));
            for t in tasks { collect_parallels(&t.node, out); }
            if let Some(t) = timeout { collect_parallels(&t.node, out); }
        }
        Expr::Lit { value, .. } => {
            if let certo_ast::expr::Lit::FString(parts) = value {
                for p in parts {
                    if let certo_ast::expr::FStringPart::Interpolated(e) = p { collect_parallels(&e.node, out); }
                }
            }
        }
        Expr::Path { .. } => {}
        Expr::App { func, args, .. } => {
            collect_parallels(&func.node, out);
            for a in args { collect_parallels(&a.value.node, out); }
        }
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            collect_parallels(&left.node, out);
            collect_parallels(&right.node, out);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. }
        | Expr::Age { expr, .. } => collect_parallels(&expr.node, out),
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => collect_parallels(&body.node, out),
        Expr::If { cond, then_expr, else_expr, .. } => {
            collect_parallels(&cond.node, out);
            collect_parallels(&then_expr.node, out);
            collect_parallels(&else_expr.node, out);
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_parallels(&scrutinee.node, out);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_parallels(&g.node, out); }
                collect_parallels(&arm.body.node, out);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                match s {
                    certo_ast::expr::Stmt::Val { value, .. }
                    | certo_ast::expr::Stmt::Var { value, .. }
                    | certo_ast::expr::Stmt::Assign { value, .. } => collect_parallels(&value.node, out),
                    certo_ast::expr::Stmt::Defer { body, .. } => collect_parallels(&body.node, out),
                    certo_ast::expr::Stmt::Expr { expr, .. } => collect_parallels(&expr.node, out),
                }
            }
        }
        Expr::Lambda { body, .. } => collect_parallels(&body.node, out),
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { collect_parallels(&e.node, out); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { collect_parallels(&b.node, out); }
            for f in fields { collect_parallels(&f.value.node, out); }
        }
        Expr::Guard { cond, else_expr, .. } => {
            collect_parallels(&cond.node, out);
            collect_parallels(&else_expr.node, out);
        }
        Expr::Require { expr, error, .. } => {
            collect_parallels(&expr.node, out);
            collect_parallels(&error.node, out);
        }
        Expr::WithTimeout { duration, body, .. } => {
            collect_parallels(&duration.node, out);
            collect_parallels(&body.node, out);
        }
        Expr::For { iter, body, .. } => {
            collect_parallels(&iter.node, out);
            collect_parallels(&body.node, out);
        }
        Expr::While { cond, body, .. } => {
            collect_parallels(&cond.node, out);
            collect_parallels(&body.node, out);
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            collect_parallels(&actual.node, out);
            if let ExpectMatcher::ToBe(e) = matcher { collect_parallels(&e.node, out); }
        }
    }
}

/// Every free identifier referenced inside `expr` — a typeck-local mirror
/// of `crates/parser/src/parse_decl.rs`'s own `collect_referenced_idents`
/// (typeck has no dependency on `crates/parser` outside its own tests, so
/// that function isn't reachable from here; duplicated rather than
/// refactored into a shared crate, matching this codebase's own
/// established convention of small, independent per-crate AST walkers
/// rather than one shared traversal library).
fn collect_referenced_names(expr: &Expr, out: &mut std::collections::HashSet<String>) {
    match expr {
        Expr::Lit { value, .. } => {
            if let certo_ast::expr::Lit::FString(parts) = value {
                for p in parts {
                    if let certo_ast::expr::FStringPart::Interpolated(e) = p { collect_referenced_names(&e.node, out); }
                }
            }
        }
        Expr::Path { path, .. } => {
            if path.segments.len() == 1 { out.insert(path.segments[0].node.clone()); }
        }
        Expr::App { func, args, .. } => {
            collect_referenced_names(&func.node, out);
            for a in args { collect_referenced_names(&a.value.node, out); }
        }
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            collect_referenced_names(&left.node, out);
            collect_referenced_names(&right.node, out);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. }
        | Expr::Age { expr, .. } => collect_referenced_names(&expr.node, out),
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => collect_referenced_names(&body.node, out),
        Expr::If { cond, then_expr, else_expr, .. } => {
            collect_referenced_names(&cond.node, out);
            collect_referenced_names(&then_expr.node, out);
            collect_referenced_names(&else_expr.node, out);
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_referenced_names(&scrutinee.node, out);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_referenced_names(&g.node, out); }
                collect_referenced_names(&arm.body.node, out);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                match s {
                    certo_ast::expr::Stmt::Val { value, .. }
                    | certo_ast::expr::Stmt::Var { value, .. }
                    | certo_ast::expr::Stmt::Assign { value, .. } => collect_referenced_names(&value.node, out),
                    certo_ast::expr::Stmt::Defer { body, .. } => collect_referenced_names(&body.node, out),
                    certo_ast::expr::Stmt::Expr { expr, .. } => collect_referenced_names(&expr.node, out),
                }
            }
        }
        Expr::Lambda { body, .. } => collect_referenced_names(&body.node, out),
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { collect_referenced_names(&e.node, out); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { collect_referenced_names(&b.node, out); }
            for f in fields { collect_referenced_names(&f.value.node, out); }
        }
        Expr::Guard { cond, else_expr, .. } => {
            collect_referenced_names(&cond.node, out);
            collect_referenced_names(&else_expr.node, out);
        }
        Expr::Require { expr, error, .. } => {
            collect_referenced_names(&expr.node, out);
            collect_referenced_names(&error.node, out);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { collect_referenced_names(&t.node, out); }
            if let Some(t) = timeout { collect_referenced_names(&t.node, out); }
        }
        Expr::WithTimeout { duration, body, .. } => {
            collect_referenced_names(&duration.node, out);
            collect_referenced_names(&body.node, out);
        }
        Expr::For { iter, body, .. } => {
            collect_referenced_names(&iter.node, out);
            collect_referenced_names(&body.node, out);
        }
        Expr::While { cond, body, .. } => {
            collect_referenced_names(&cond.node, out);
            collect_referenced_names(&body.node, out);
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            collect_referenced_names(&actual.node, out);
            if let ExpectMatcher::ToBe(e) = matcher { collect_referenced_names(&e.node, out); }
        }
    }
}

// ------------------------------------------------------------------ //
// Smart constructors — `type X = priv X(...)`'s raw constructor may only
// be called from within an `impl X { ... }` block for the same type.
// ------------------------------------------------------------------ //

/// Collect the names of `priv`-constructor types, then walk every top-level
/// function/val/var body and every `impl` block's methods, flagging any bare
/// call to one of those constructors that isn't lexically inside an `impl`
/// block for that same type.
fn check_priv_ctors(module: &Module, errors: &mut Vec<TypeError>) {
    use std::collections::HashSet;
    let priv_ctors: HashSet<&str> = module.decls.iter()
        .filter_map(|d| match &d.node {
            Decl::Type(t) if t.is_priv_ctor => Some(t.name.node.as_str()),
            _ => None,
        })
        .collect();
    if priv_ctors.is_empty() { return; }

    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                if let Some(body) = &f.body {
                    walk_priv_ctors(body, None, &priv_ctors, errors);
                }
            }
            Decl::Impl(i) => {
                let self_type = i.type_path.segments.last().map(|s| s.node.as_str());
                for m in &i.methods {
                    if let Some(body) = &m.body {
                        walk_priv_ctors(body, self_type, &priv_ctors, errors);
                    }
                }
            }
            Decl::Val(v) => walk_priv_ctors(&v.value, None, &priv_ctors, errors),
            Decl::Var(v) => walk_priv_ctors(&v.value, None, &priv_ctors, errors),
            _ => {}
        }
    }
}

fn walk_priv_ctors(
    expr:       &S<Expr>,
    self_type:  Option<&str>,
    priv_ctors: &std::collections::HashSet<&str>,
    errors:     &mut Vec<TypeError>,
) {
    use certo_ast::expr::Stmt;
    match &expr.node {
        Expr::App { func, args, span } => {
            if let Expr::Path { path, .. } = &func.node {
                // A bare, single-segment call to the type's own name is the
                // raw constructor (`Email(raw)`); `Email.new(raw)` parses as
                // Expr::Field, not Path, so it never matches here.
                if let [seg] = path.segments.as_slice() {
                    let name = seg.node.as_str();
                    if priv_ctors.contains(name) && self_type != Some(name) {
                        errors.push(TypeError {
                            kind: TypeErrorKind::PrivConstructorCall { type_name: name.to_string() },
                            span: *span,
                        });
                    }
                }
            }
            walk_priv_ctors(func, self_type, priv_ctors, errors);
            for a in args { walk_priv_ctors(&a.value, self_type, priv_ctors, errors); }
        }

        Expr::Lit { .. } | Expr::Path { .. } => {}
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            walk_priv_ctors(left, self_type, priv_ctors, errors);
            walk_priv_ctors(right, self_type, priv_ctors, errors);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. } | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. } | Expr::Await { expr, .. } | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. } | Expr::Age { expr, .. } | Expr::Unsafe { body: expr, .. } => {
            walk_priv_ctors(expr, self_type, priv_ctors, errors);
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            walk_priv_ctors(actual, self_type, priv_ctors, errors);
            if let ExpectMatcher::ToBe(y) = matcher { walk_priv_ctors(y, self_type, priv_ctors, errors); }
        }
        Expr::If { cond, then_expr, else_expr, .. } => {
            walk_priv_ctors(cond, self_type, priv_ctors, errors);
            walk_priv_ctors(then_expr, self_type, priv_ctors, errors);
            walk_priv_ctors(else_expr, self_type, priv_ctors, errors);
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_priv_ctors(scrutinee, self_type, priv_ctors, errors);
            for arm in arms {
                if let Some(g) = &arm.guard { walk_priv_ctors(g, self_type, priv_ctors, errors); }
                walk_priv_ctors(&arm.body, self_type, priv_ctors, errors);
            }
        }
        Expr::Block { stmts, .. } => {
            for st in stmts {
                match st {
                    Stmt::Val { value, .. } | Stmt::Var { value, .. } | Stmt::Assign { value, .. } => {
                        walk_priv_ctors(value, self_type, priv_ctors, errors);
                    }
                    Stmt::Defer { body, .. } | Stmt::Expr { expr: body, .. } => {
                        walk_priv_ctors(body, self_type, priv_ctors, errors);
                    }
                }
            }
        }
        Expr::Lambda { body, .. } | Expr::Transaction { body, .. } => {
            walk_priv_ctors(body, self_type, priv_ctors, errors);
        }
        Expr::For { iter, body, .. } => {
            walk_priv_ctors(iter, self_type, priv_ctors, errors);
            walk_priv_ctors(body, self_type, priv_ctors, errors);
        }
        Expr::While { cond, body, .. } => {
            walk_priv_ctors(cond, self_type, priv_ctors, errors);
            walk_priv_ctors(body, self_type, priv_ctors, errors);
        }
        Expr::Guard { cond, else_expr, .. } => {
            walk_priv_ctors(cond, self_type, priv_ctors, errors);
            walk_priv_ctors(else_expr, self_type, priv_ctors, errors);
        }
        Expr::Require { expr, error, .. } => {
            walk_priv_ctors(expr, self_type, priv_ctors, errors);
            walk_priv_ctors(error, self_type, priv_ctors, errors);
        }
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { walk_priv_ctors(e, self_type, priv_ctors, errors); }
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { walk_priv_ctors(t, self_type, priv_ctors, errors); }
            if let Some(t) = timeout { walk_priv_ctors(t, self_type, priv_ctors, errors); }
        }
        Expr::WithTimeout { duration, body, .. } => {
            walk_priv_ctors(duration, self_type, priv_ctors, errors);
            walk_priv_ctors(body, self_type, priv_ctors, errors);
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { walk_priv_ctors(b, self_type, priv_ctors, errors); }
            for fld in fields { walk_priv_ctors(&fld.value, self_type, priv_ctors, errors); }
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
                // `F<_>` — a 1-ary type-constructor parameter (BACKLOG item 76).
                if tp.is_constructor { ctx.env.constructor_vars.insert(v); }
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

            // Bring the type's own type params into scope (as fresh vars) so
            // record/variant field types that reference them (e.g.
            // `type Secret<T> = priv Secret(T)`) resolve correctly instead of
            // becoming a bogus rigid `Ty::Named("T")` — see BACKLOG item 78.
            let mut ctx = Ctx { env, uf, errors, counter };
            ctx.env.push();
            let type_param_vars: Vec<u32> = t.type_params.iter().map(|tp| {
                *ctx.counter += 1;
                let v = *ctx.counter;
                ctx.env.define(tp.name.node.clone(), Ty::Var(v));
                v
            }).collect();
            // Recorded so a `TypeName { field: val }` record literal (see
            // `Expr::Record` in `infer_expr.rs`) can instantiate these fresh
            // per occurrence and report its own real instantiated type
            // instead of a bare, argument-less `Named` — BACKLOG item 76's
            // own HKT test case is what surfaced this real, independent,
            // pre-existing gap (a generic record type had never been used
            // together with an explicit type annotation anywhere before).
            ctx.env.type_param_vars.insert(t.name.node.clone(), type_param_vars.clone());
            let parent_ty = Ty::Named {
                name: t.name.node.clone(),
                args: type_param_vars.iter().map(|&v| Ty::Var(v)).collect(),
            };

            // Register record fields so field-access typeck can resolve `a.field` on named types.
            if let certo_ast::decl::TypeBody::Record(rec) = &t.body {
                let fields: Vec<(String, Ty)> = rec.fields.iter()
                    .map(|f| (f.name.node.clone(), type_expr_to_ty(&f.ty.node, &mut ctx)))
                    .collect();
                ctx.env.record_fields.insert(t.name.node.clone(), fields);

                // `computed` properties (BACKLOG item 143) — registered
                // separately from `record_fields` on purpose (see
                // `TypeEnv.computed_fields`'s own doc comment): a computed
                // name has a real declared type for read access, but must
                // stay invisible to record-literal/`.with(...)`
                // construction, which only ever consults `record_fields`.
                if !rec.computed.is_empty() {
                    let computed: Vec<(String, Ty)> = rec.computed.iter()
                        .map(|c| (c.name.node.clone(), type_expr_to_ty(&c.ty.node, &mut ctx)))
                        .collect();
                    ctx.env.computed_fields.insert(t.name.node.clone(), computed);
                }
            }

            // Register sum variants so constructors are bound in the environment.
            // Computed *while* the type-param scope above is active (so field
            // types referencing them resolve correctly), but the resulting
            // constructor bindings are `define`d only after popping that scope
            // — `define` always targets the innermost frame, so defining them
            // before the `pop()` would silently discard them with it.
            let mut variant_defs: Vec<(String, Ty)> = Vec::new();
            if let certo_ast::decl::TypeBody::Sum(variants) = &t.body {
                ctx.env.sum_variants.insert(
                    t.name.node.clone(),
                    variants.iter().map(|v| v.name.node.clone()).collect(),
                );
                for v in variants {
                    if v.fields.is_empty() {
                        // Unit variant: `Red` — just a value of the parent type.
                        let ty = if type_param_vars.is_empty() {
                            parent_ty.clone()
                        } else {
                            Ty::Forall { vars: type_param_vars.clone(), body: Box::new(parent_ty.clone()) }
                        };
                        variant_defs.push((v.name.node.clone(), ty));
                    } else {
                        // Variant with fields: constructor function.
                        let param_tys: Vec<Ty> = v.fields.iter()
                            .map(|f| type_expr_to_ty(&f.ty.node, &mut ctx))
                            .collect();
                        let fn_ty = Ty::Fn { params: param_tys, ret: Box::new(parent_ty.clone()) };
                        let fn_ty = if type_param_vars.is_empty() {
                            fn_ty
                        } else {
                            Ty::Forall { vars: type_param_vars.clone(), body: Box::new(fn_ty) }
                        };
                        variant_defs.push((v.name.node.clone(), fn_ty));
                    }
                }
            }
            ctx.env.pop();
            for (name, ty) in variant_defs {
                ctx.env.define(name, ty);
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
            // BACKLOG item 224 — a validator with a `context` block's real
            // generated function (`crates/codegen/src/emit_validator.rs`'s
            // `build_fn_sig`) takes `(entity, ctx)`, but this hoist always
            // registered a 1-parameter signature regardless, rejecting the
            // correct, spec-shaped 2-argument call site
            // (`V.validate(entity, ctx)`) with a hard type error — confirmed
            // directly. Resolved *nominally* (`Ty::Named("{Name}Context")`),
            // matching the real synthesized context type's own name
            // (`emit_validator.rs`'s `ctx_type = format!("{}Context", vname)`)
            // exactly, rather than an anonymous structural `Ty::Record` —
            // under `certo build`/`run` (where `expand_validators` splices
            // that type's own declaration in before this hoist runs) this is
            // fully precise; under `certo check` alone (which never expands
            // validators, so the name is never actually declared) it falls
            // back to this codebase's own established "an undeclared
            // capitalized name is accepted as a valid opaque nominal type"
            // leniency — a real, already-flagged, pre-existing check-vs-build
            // divergence (see item 220's own note), not made any worse here:
            // a context-free validator's call sites are already fully
            // correct either way, and a context-bearing one at least now
            // accepts the right *arity*, where before it rejected every
            // context-using call site categorically.
            let params = if v.context.is_empty() {
                vec![entity_ty.clone()]
            } else {
                let ctx_ty = Ty::Named { name: format!("{}Context", v.name.node), args: vec![] };
                vec![entity_ty.clone(), ctx_ty]
            };
            // validate(entity[, ctx]) -> Result<Unit, ErrorsType>
            env.define(
                format!("{}.validate", v.name.node),
                Ty::Fn { params: params.clone(), ret: Box::new(result_ty) },
            );
            // validateAll(entity[, ctx]) -> List<ErrorsType>
            env.define(
                format!("{}.validateAll", v.name.node),
                Ty::Fn { params, ret: Box::new(result_list) },
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
                // Both the impl block's own type params (`impl<T> Secret { ... }`)
                // and the method's own (`fn m<U>(...)`) are in scope for the
                // method's signature — see BACKLOG item 78.
                let type_param_vars: Vec<u32> = i.type_params.iter().chain(m.type_params.iter()).map(|tp| {
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
                    if !literal_matches_fixed_width(&body.node, &declared) {
                        ctx.unify(body_ty, declared, f.span);
                    }
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
            let inferred = if let Some(ann) = &v.ty {
                let declared = type_expr_to_ty(&ann.node, ctx);
                if literal_matches_fixed_width(&v.value.node, &declared) {
                    declared
                } else {
                    ctx.unify(inferred.clone(), declared, v.span);
                    inferred
                }
            } else {
                inferred
            };
            let generalised = ctx.env.generalise(inferred, ctx.uf);
            use certo_ast::pattern::Pattern;
            if let Pattern::Ident { name, .. } = &v.pattern.node {
                ctx.env.define(name.node.clone(), generalised);
            }
        }

        Decl::Var(v) => {
            let inferred = infer(&v.value, ctx);
            let inferred = if let Some(ann) = &v.ty {
                let declared = type_expr_to_ty(&ann.node, ctx);
                if literal_matches_fixed_width(&v.value.node, &declared) {
                    declared
                } else {
                    ctx.unify(inferred.clone(), declared, v.span);
                    inferred
                }
            } else {
                inferred
            };
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
            let entity_name = type_expr_simple_name(&v.entity.node);
            if let Some(entity_name) = &entity_name {
                let entity_ty = type_expr_to_ty(&v.entity.node, ctx);
                ctx.env.define(lowercase_first(entity_name), entity_ty);
            }

            // Bind context fields
            for field in &v.context {
                let field_ty = type_expr_to_ty(&field.type_ref.node, ctx);
                ctx.env.define(field.name.node.clone(), field_ty.clone());

                // BACKLOG item 220 (E0705) — a `loaded by` expression must
                // itself produce the field's own declared type; nothing
                // checked this before. Compares resolved types directly
                // (mirrors E0703's own pattern above) rather than
                // `ctx.unify`, which would only ever report the generic
                // E0200. NOTE: found while implementing this that the
                // `db.<table>.find(...)` accessor syntax every spec example
                // of `loaded by` actually uses doesn't type-check at all
                // anywhere in this codebase (`db` is never a bound name) —
                // a separate, much larger, pre-existing gap (filed as its
                // own item, not attempted here). Until that exists, this
                // check can only ever fire for a `loaded by` expression that
                // avoids that syntax; it's still real, correct, forward-
                // compatible code, just not yet exercisable end-to-end by
                // the spec's own examples.
                if let Some(lb) = &field.loaded_by {
                    let lb_ty = infer(lb, ctx);
                    let lb_ty_resolved = ctx.uf.apply(&lb_ty);
                    let field_ty_resolved = ctx.uf.apply(&field_ty);
                    match &lb_ty_resolved {
                        Ty::Var(_) | Ty::Error => {}
                        other if other != &field_ty_resolved => {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::LoadedByTypeMismatch {
                                    field_name: field.name.node.clone(),
                                    expected:   field_ty_resolved.clone(),
                                    found:      lb_ty_resolved.clone(),
                                },
                                span: lb.span,
                            });
                        }
                        _ => {}
                    }
                }
            }

            // BACKLOG item 220 (E0706/E0707) — a `trigger on ... when
            // <field> == <value>` condition previously had its field/value
            // never checked against the entity type at all: an undeclared
            // field or a value that isn't a real variant of the field's own
            // type both passed `certo check` silently.
            if let (Some(trigger), Some(entity_name)) = (&v.trigger, &entity_name) {
                if let Some(cond) = &trigger.condition {
                    // Only checked when the entity resolves to a known local
                    // record — an entity type we can't see the fields of
                    // (imported, or the "declared" but not `type X = {...}`-
                    // backed convention some tests/forward-refs rely on)
                    // can't be verified either way, so it's silently skipped
                    // rather than risking a false positive.
                    if let Some(fields) = ctx.env.record_fields.get(entity_name.as_str()).cloned() {
                    match fields.iter().find(|(n, _)| n == &cond.field.node) {
                        None => {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::TriggerFieldNotFound {
                                    field:  cond.field.node.clone(),
                                    entity: entity_name.clone(),
                                },
                                span: cond.span,
                            });
                        }
                        Some((_, field_ty)) => {
                            // Only a bare identifier value (`status == Submitted`)
                            // names a literal variant to check; other shapes
                            // (`status != OLD.status`) reference a runtime value,
                            // not a variant literal, and aren't checked here.
                            if let Expr::Path { path, .. } = &cond.value.node {
                                if path.segments.len() == 1 {
                                    let value_name = &path.segments[0].node;
                                    if let Ty::Named { name: ty_name, .. } = field_ty {
                                        if let Some(variants) = ctx.env.sum_variants.get(ty_name) {
                                            if !variants.iter().any(|v| v == value_name) {
                                                ctx.errors.push(TypeError {
                                                    kind: TypeErrorKind::TriggerValueNotVariant {
                                                        value:    value_name.clone(),
                                                        field:    cond.field.node.clone(),
                                                        field_ty: field_ty.clone(),
                                                    },
                                                    span: cond.value.span,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    }
                }
            }

            // Type-check each rule
            let errors_ty = type_expr_to_ty(&v.errors.node, ctx);
            for rule in &v.rules {
                let req_ty = infer(&rule.require, ctx);
                ctx.unify(req_ty, Ty::Bool, rule.require.span);

                // BACKLOG item 219 (E0703) — the `else` clause must produce
                // a value of the validator's own declared `errors` type.
                // Compares already-resolved types directly (mirroring
                // `Decl::Temporal`'s identical pattern above) rather than
                // going through `ctx.unify`, which would only ever report
                // the generic E0200 `Mismatch` — this needs its own
                // dedicated diagnostic naming the offending rule.
                let else_ty = infer(&rule.else_, ctx);
                let else_ty_resolved = ctx.uf.apply(&else_ty);
                let errors_ty_resolved = ctx.uf.apply(&errors_ty);
                match &else_ty_resolved {
                    Ty::Var(_) | Ty::Error => {} // unresolved or already errored
                    other if other != &errors_ty_resolved => {
                        ctx.errors.push(TypeError {
                            kind: TypeErrorKind::ElseTypeMismatch {
                                rule_name: rule.name.node.clone(),
                                expected:  errors_ty_resolved.clone(),
                                found:     else_ty_resolved.clone(),
                            },
                            span: rule.else_.span,
                        });
                    }
                    _ => {}
                }
            }

            ctx.env.pop();
        }

        // Impl method bodies — previously never checked at all (only their
        // hoisted signatures existed, for callers to unify against); mirrors
        // Decl::Fn's own body-checking, with both the impl block's own type
        // params and the method's own in scope. See BACKLOG item 78.
        Decl::Impl(i) => {
            for m in &i.methods {
                if let Some(body) = &m.body {
                    ctx.env.push();
                    for tp in i.type_params.iter().chain(m.type_params.iter()) {
                        let fresh = ctx.fresh();
                        ctx.env.define(tp.name.node.clone(), fresh);
                    }
                    for p in &m.params {
                        let ty = type_expr_to_ty(&p.ty.node, ctx);
                        ctx.env.define(p.name.node.clone(), ty);
                    }
                    let body_ty = infer_fn_body(body, ctx);
                    if let Some(ann) = &m.ret_ty {
                        let declared = type_expr_to_ty(&ann.node, ctx);
                        if !literal_matches_fixed_width(&body.node, &declared) {
                            ctx.unify(body_ty, declared, m.span);
                        }
                    }
                    ctx.env.pop();
                }
            }
        }

        // Other decls: skip for now (traits, state machines, etc.)
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
