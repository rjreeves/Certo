use std::collections::{HashMap, HashSet};
use certo_ast::expr::{Expr, ExpectMatcher, Stmt, Lit, BinOp, UnOp, Arg};
use certo_ast::span::{S, Span};
use certo_ast::types::TypeExpr;
use crate::ty::{Ty, TyVar};
use crate::env::TypeEnv;
use crate::unify::UnionFind;
use crate::error::{TypeError, TypeErrorKind};

/// Inference context threaded through the whole pass.
pub struct Ctx<'e> {
    pub env:     &'e mut TypeEnv,
    pub uf:      &'e mut UnionFind,
    pub errors:  &'e mut Vec<TypeError>,
    pub counter: &'e mut u32,
}

impl<'e> Ctx<'e> {
    /// Allocate a fresh type variable.
    pub fn fresh(&mut self) -> Ty {
        *self.counter += 1;
        Ty::Var(*self.counter)
    }

    /// Unify, recording errors instead of returning them. `found` is the
    /// value's actual/inferred type, `expected` the declared/required one —
    /// see `UnionFind::unify`'s own doc comment (BACKLOG item 196).
    pub fn unify(&mut self, found: Ty, expected: Ty, span: Span) {
        if let Err(e) = self.uf.unify(found, expected, span) {
            self.errors.push(e);
        }
    }

    /// Instantiate a polytype (Forall) into a fresh mono type.
    pub fn instantiate(&mut self, ty: Ty) -> Ty {
        ty.instantiate(self.counter)
    }
}

// ------------------------------------------------------------------ //
// Convert a surface TypeExpr to our internal Ty.
// ------------------------------------------------------------------ //

pub fn type_expr_to_ty(te: &TypeExpr, ctx: &mut Ctx<'_>) -> Ty {
    use certo_ast::types::TypeExpr as TE;
    match te {
        TE::Named { path, args, .. } => {
            let name = path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            let targs: Vec<Ty> = args.iter().map(|a| type_expr_to_ty(&a.node, ctx)).collect();
            match name.as_str() {
                "Int"     => Ty::Int,
                "Int8"    => Ty::Int8,
                "Int16"   => Ty::Int16,
                "Int32"   => Ty::Int32,
                "UInt"    => Ty::UInt,
                "Float"   => Ty::Float,
                "Float32" => Ty::Float32,
                "Decimal" => Ty::Decimal(None),
                "Bool"    => Ty::Bool,
                "Char"    => Ty::Char,
                "Text"    => Ty::Text,
                "BoundedText" => Ty::BoundedText(None),
                "Unit"    => Ty::Unit,
                "UUID"    => Ty::Uuid,
                "Option"  => Ty::Option(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Result"  => {
                    let mut it = targs.into_iter();
                    let t = it.next().unwrap_or(Ty::Error);
                    let e = it.next().unwrap_or(Ty::Error);
                    Ty::Result(Box::new(t), Box::new(e))
                }
                "List"    => Ty::List(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Map"     => {
                    let mut it = targs.into_iter();
                    let k = it.next().unwrap_or(Ty::Error);
                    let v = it.next().unwrap_or(Ty::Error);
                    Ty::Map(Box::new(k), Box::new(v))
                }
                _ => {
                    // If this name is a type parameter in scope, use its var.
                    // Type parameters are always registered as a bare `Ty::Var`
                    // (see hoist_decl); anything else found in `env` under this
                    // name is a *value* binding (a function, a sum-type
                    // constructor, ...) that just happens to share the name —
                    // e.g. a `priv` newtype's constructor is named identically
                    // to its own type — and must not be mistaken for it here.
                    if targs.is_empty() {
                        if let Some(Ty::Var(v)) = ctx.env.lookup(&name) {
                            return Ty::Var(*v);
                        }
                    } else if targs.len() == 1 {
                        // `F<A>` where `F` is a declared 1-ary type-constructor
                        // parameter (`F<_>`, BACKLOG item 76) — higher-kinded
                        // application, not an ordinary named type with args.
                        // Anything else applying a *non*-constructor type
                        // param to an argument (a genuine kind error, e.g. a
                        // plain `T` written as `T<Int>`) deliberately falls
                        // through to the `Named` catch-all below rather than
                        // being special-cased here — unification will reject
                        // it as an ordinary type mismatch.
                        if let Some(Ty::Var(v)) = ctx.env.lookup(&name) {
                            if ctx.env.constructor_vars.contains(v) {
                                return Ty::App(Box::new(Ty::Var(*v)), Box::new(targs.into_iter().next().unwrap()));
                            }
                        }
                    }
                    Ty::Named { name, args: targs }
                }
            }
        }
        TE::Option { inner, .. } => Ty::Option(Box::new(type_expr_to_ty(&inner.node, ctx))),
        TE::Tuple { elements, .. } => Ty::Tuple(elements.iter().map(|e| type_expr_to_ty(&e.node, ctx)).collect()),
        TE::Fn { params, ret, .. } => Ty::Fn {
            params: params.iter().map(|p| type_expr_to_ty(&p.node, ctx)).collect(),
            ret:    Box::new(type_expr_to_ty(&ret.node, ctx)),
        },
        TE::Record { fields, .. } => Ty::Record(
            fields.iter().map(|f| (f.name.node.clone(), type_expr_to_ty(&f.ty.node, ctx))).collect()
        ),
        TE::Param { name, .. } => {
            // If the enclosing fn pre-defined this type param (via hoist/check),
            // reuse the same var so all occurrences of T unify correctly.
            if let Some(ty) = ctx.env.lookup(&name.node) {
                return ty.clone();
            }
            // Fallback: create a fresh var (unannotated generic context).
            let fresh = ctx.fresh();
            ctx.env.define(name.node.clone(), fresh.clone());
            fresh
        }
        TE::Ptr { inner, .. } => type_expr_to_ty(&inner.node, ctx),
        TE::DecimalParam { precision, scale, .. } => Ty::Decimal(Some((*precision, *scale))),
        TE::BoundedTextParam { max_len, .. } => Ty::BoundedText(Some(*max_len)),
    }
}

/// Resolve a field access against an already-inferred object type. Shared by
/// `Expr::Field` and `Expr::SafeField` (the latter runs this against the
/// unwrapped Some-payload of an Option, not the Option itself).
fn resolve_field_ty(obj_ty: Ty, field: &S<String>, span: Span, ctx: &mut Ctx<'_>) -> Ty {
    match &obj_ty {
        Ty::Record(fields) => {
            match fields.iter().find(|(n, _)| n == &field.node) {
                Some((_, ty)) => ty.clone(),
                None => {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::UnknownField { field: field.node.clone(), on: obj_ty },
                        span,
                    });
                    Ty::Error
                }
            }
        }
        Ty::Named { name, .. } => {
            // Look up the record definition for this named type.
            if let Some(fields) = ctx.env.record_fields.get(name.as_str()).cloned() {
                match fields.iter().find(|(n, _)| n == &field.node) {
                    Some((_, ty)) => ty.clone(),
                    // `computed` properties (BACKLOG item 143) — a real
                    // stored field never wins over a computed one of the
                    // same name (the parser rejects that shape earlier),
                    // so checking this only after `fields` misses is safe
                    // and keeps this the sole extra lookup on the hot path.
                    None => match ctx.env.computed_fields.get(name.as_str())
                        .and_then(|cs| cs.iter().find(|(n, _)| n == &field.node)) {
                        Some((_, ty)) => ty.clone(),
                        None => {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::UnknownField { field: field.node.clone(), on: obj_ty },
                                span,
                            });
                            Ty::Error
                        }
                    }
                }
            } else {
                // Named type not registered as a record — may be a statemachine type etc.
                ctx.fresh()
            }
        }
        Ty::Var(_) => {
            // Not yet resolved — return a fresh var; may be resolved later
            ctx.fresh()
        }
        other => {
            ctx.errors.push(TypeError {
                kind: TypeErrorKind::UnknownField { field: field.node.clone(), on: other.clone() },
                span,
            });
            Ty::Error
        }
    }
}

// ------------------------------------------------------------------ //
// Infer the type of an expression.
// ------------------------------------------------------------------ //

pub fn infer(expr: &S<Expr>, ctx: &mut Ctx<'_>) -> Ty {
    use certo_ast::expr::FStringPart;
    match &expr.node {
        // f-string: type-check each interpolated expression and require that its
        // type can actually be rendered to text (matches what codegen can convert).
        Expr::Lit { value: Lit::FString(parts), .. } => {
            for part in parts {
                if let FStringPart::Interpolated(e) = part {
                    let ty = infer(e, ctx);
                    let ty = ctx.uf.apply(&ty);
                    if !is_displayable(&ty) && !matches!(ty, Ty::Error | Ty::Var(_)) {
                        ctx.errors.push(TypeError {
                            kind: TypeErrorKind::NonDisplayableInterpolation { ty },
                            span: e.span,
                        });
                    }
                }
            }
            Ty::Text
        }

        Expr::Lit { value, .. } => infer_lit(value),

        Expr::Path { path, span } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            match ctx.env.lookup(name) {
                Some(ty) => {
                    let ty = ty.clone();
                    ctx.instantiate(ty)
                }
                None => {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::UnboundName(name.to_string()),
                        span: *span,
                    });
                    Ty::Error
                }
            }
        }

        Expr::App { func, args, span } => {
            // Dot-call UFCS (BACKLOG item 162): `xs.map(f)` parses to the
            // exact same `App{ func: Field{expr, field}, args }` shape as
            // the already-working module-qualified form `List.map(xs, f)`
            // (confirmed directly — the parser has no case-analysis on the
            // base at all). The only difference is whether `expr` is an
            // uppercase-`Path` (handled below by `qualified_call_name`) or
            // an arbitrary/lowercase value. When it's the latter, and a
            // real function named `"<TypeName>.<field>"` exists — where
            // `TypeName` comes from `expr`'s own resolved type via
            // `Ty::qualifying_name` — rewrite this call *locally* (for
            // inference purposes only, the real AST is untouched) into
            // that already-working shape, with `expr` spliced in as the
            // qualified function's first argument, before any of the rest
            // of this arm's logic runs. That logic (labeled-arg
            // reordering, default-param insertion, the SQL/secret-sink
            // checks, the `List.sortBy`-family key-projection check) is
            // then unchanged and unaware anything special happened — it
            // just sees an ordinary qualified call with one extra leading
            // argument, exactly as if the user had written the qualified
            // form directly.
            let ufcs_rewrite: Option<(S<Expr>, Vec<Arg>)> = if let Expr::Field { expr, field, .. } = &func.node {
                if qualified_call_name(expr, field).is_none() {
                    let base_ty = infer(expr, ctx);
                    let resolved = ctx.uf.apply(&base_ty);
                    resolved.qualifying_name().and_then(|type_name| {
                        let qualified = format!("{type_name}.{}", field.node);
                        if ctx.env.lookup(&qualified).is_some() {
                            // Mirror the exact shape a real qualified call
                            // (`List.map(xs, f)`) parses to —
                            // `Expr::Field{ expr: Path("List"), field: "map" }`
                            // — NOT a 2-segment `Expr::Path`. `resolve_callee`'s
                            // `Expr::Path` arm only reads `segments.last()`
                            // (it's built for single-segment local names), so a
                            // synthetic multi-segment `Path` here silently
                            // resolved to the bare field name and produced a
                            // spurious `UnboundName` — confirmed by the first
                            // run of `dot_call_ufcs_smoke_test`.
                            let type_path = certo_ast::types::ModulePath {
                                segments: vec![S::new(type_name, field.span)],
                                span: field.span,
                            };
                            let type_expr = Box::new(S::new(Expr::Path { path: type_path, span: field.span }, field.span));
                            let synthetic_func = S::new(
                                Expr::Field { expr: type_expr, field: field.clone(), span: field.span },
                                field.span,
                            );
                            let receiver_arg = Arg { label: None, value: (**expr).clone(), span: expr.span };
                            let mut new_args = vec![receiver_arg];
                            new_args.extend(args.iter().cloned());
                            Some((synthetic_func, new_args))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let args_owned;
            let func_owned;
            let (func, args): (&S<Expr>, &[Arg]) = match ufcs_rewrite {
                Some((f, a)) => { func_owned = f; args_owned = a; (&func_owned, &args_owned) }
                None => (func, args.as_slice()),
            };

            let has_labels = args.iter().any(|a| a.label.is_some());
            // Build both a full-qualified path (for stdlib) and a short name (for user-defined).
            //
            // A module-qualified call (`List.sortBy(...)`) parses as
            // `Expr::Field { expr: Path("List"), field: "sortBy" }`, not
            // `Expr::Path` — confirmed directly (BACKLOG item 171): before
            // this `Expr::Field` arm existed, `fn_name` was silently `None`
            // for every such call, so labeled-arg reordering/default-param
            // insertion below never ran for *any* qualified stdlib call
            // (`List.groupBy(key: ..., list: ...)` type-checked the
            // arguments positionally instead of by label, a real, reproduced
            // break). `resolve_callee` below already works around this gap
            // for its own, narrower purpose via the same `qualified_call_name`
            // helper reused here — this closes the gap at the source instead.
            let (fn_full_path, fn_short_name) = match &func.node {
                Expr::Path { path, .. } => {
                    let full = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
                    let short = path.segments.last().map(|s| s.node.clone());
                    (Some(full), short)
                }
                Expr::Field { expr, field, .. } => (qualified_call_name(expr, field), None),
                _ => (None, None),
            };
            // Prefer full-path lookup (stdlib), fall back to short name (user-defined).
            let fn_name: Option<String> = fn_full_path
                .as_ref()
                .filter(|fp| ctx.env.get_param_meta(fp).is_some())
                .cloned()
                .or(fn_short_name.clone());

            let (func_ty, row_check) = resolve_callee(func, ctx);

            // Collect (inferred_ty, span) per argument slot, respecting labels/defaults.
            let arg_info: Vec<(Ty, Span)> = if has_labels {
                if let Some(ref fname) = fn_name {
                    if let Some(params) = ctx.env.get_param_meta(fname).cloned() {
                        let mut slots: Vec<Option<(Ty, Span)>> = vec![None; params.len()];
                        let mut pos_cursor = 0usize;
                        for arg in args {
                            let ty = infer(&arg.value, ctx);
                            let sp = arg.value.span;
                            if let Some(label) = &arg.label {
                                if let Some(idx) = params.iter().position(|(n, _)| n == &label.node) {
                                    slots[idx] = Some((ty, sp));
                                } else {
                                    while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                                    if pos_cursor < slots.len() { slots[pos_cursor] = Some((ty, sp)); pos_cursor += 1; }
                                }
                            } else {
                                while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                                if pos_cursor < slots.len() { slots[pos_cursor] = Some((ty, sp)); pos_cursor += 1; }
                            }
                        }
                        slots.into_iter().map(|s| s.unwrap_or_else(|| (ctx.fresh(), *span))).collect()
                    } else {
                        args.iter().map(|a| (infer(&a.value, ctx), a.value.span)).collect()
                    }
                } else {
                    args.iter().map(|a| (infer(&a.value, ctx), a.value.span)).collect()
                }
            } else if let Some(ref fname) = fn_name {
                if let Some(params) = ctx.env.get_param_meta(fname).cloned() {
                    if args.len() < params.len() {
                        let mut info: Vec<(Ty, Span)> = args.iter()
                            .map(|a| (infer(&a.value, ctx), a.value.span)).collect();
                        for (_, has_default) in params.iter().skip(args.len()) {
                            if *has_default { info.push((ctx.fresh(), *span)); }
                        }
                        info
                    } else {
                        args.iter().map(|a| (infer(&a.value, ctx), a.value.span)).collect()
                    }
                } else {
                    args.iter().map(|a| (infer(&a.value, ctx), a.value.span)).collect()
                }
            } else {
                args.iter().map(|a| (infer(&a.value, ctx), a.value.span)).collect()
            };

            let ret_ty = ctx.fresh();
            // Build expected fn type with fresh param vars, then unify each arg individually.
            let param_vars: Vec<Ty> = arg_info.iter().map(|_| ctx.fresh()).collect();
            let expected_fn = Ty::Fn { params: param_vars.clone(), ret: Box::new(ret_ty.clone()) };
            // Unify the function itself (catches arity and non-function errors) at the call span.
            ctx.unify(func_ty, expected_fn, *span);
            // Unify each argument at its own span for precise arrows.
            for ((arg_ty, arg_span), param_ty) in arg_info.iter().zip(param_vars.iter()) {
                ctx.unify(arg_ty.clone(), param_ty.clone(), *arg_span);
            }

            // Now that unification has resolved as much as it's going to, check any
            // row-polymorphism bounds against whatever the bound type param resolved to.
            apply_row_check(&row_check, ctx, *span);

            // Reject a value whose type structurally contains `Secret<_>`
            // (however deeply wrapped) being passed to a logging/
            // serialization sink — BACKLOG item 78. Checked last, after
            // unification, so a variable's type is as resolved as it's
            // going to get. `Log.info`/`Json.encode` (the spec's own
            // names, §13.2) don't exist in this compiler — these are the
            // real sinks that do.
            const SENSITIVE_SINKS: &[&str] = &["println", "print", "eprint", "Json.stringify"];
            if let Some(name) = &fn_name {
                if SENSITIVE_SINKS.contains(&name.as_str()) {
                    for (arg_ty, arg_span) in &arg_info {
                        let resolved = ctx.uf.apply(arg_ty);
                        if resolved.contains_secret() {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::SecretInSensitiveContext { fn_name: name.clone(), ty: resolved },
                                span: *arg_span,
                            });
                        }
                    }
                }
            }

            // Reject an f-string with live interpolation passed directly as the
            // `sql` argument to a raw-SQL sink — BACKLOG item 159. Checked
            // syntactically against the argument expression actually written
            // at the call site: the resolved `Text` type can't distinguish a
            // literal from an interpolated string, so this can't be caught
            // via unification the way SecretInSensitiveContext above is.
            const RAW_SQL_SINKS: &[&str] = &[
                "dbExec", "dbQuery", "dbQueryTyped", "dbQueryRow", "dbQueryOne",
                "dbColumns", "dbStream", "dbRunScript", "dbRunScriptResult",
            ];
            if let Some(name) = &fn_name {
                if RAW_SQL_SINKS.contains(&name.as_str()) {
                    let param_names = ctx.env.get_param_meta(name).cloned();
                    let sql_idx = param_names.as_ref()
                        .and_then(|ps| ps.iter().position(|(n, _)| n == "sql"))
                        .unwrap_or(1);
                    let sql_arg = args.iter()
                        .find(|a| a.label.as_ref().map(|l| l.node.as_str()) == Some("sql"))
                        .or_else(|| args.get(sql_idx));
                    if let Some(arg) = sql_arg {
                        if let Expr::Lit { value: Lit::FString(parts), .. } = &arg.value.node {
                            if parts.iter().any(|p| matches!(p, certo_ast::expr::FStringPart::Interpolated(_))) {
                                ctx.errors.push(TypeError {
                                    kind: TypeErrorKind::SqlInjectionRisk { fn_name: name.clone() },
                                    span: arg.value.span,
                                });
                            }
                        }
                    }
                }
            }

            // `List.sortBy`/`minBy`/`maxBy`/`sumBy`'s key/numeric projection
            // (BACKLOG item 162b) — checked after unification so the
            // lambda's own inferred return type is as resolved as it's
            // going to get. Registered with an unrestricted `K`/`N` type
            // var (matching the spec's own `K: Ord`/`N: Numeric` bounds),
            // but only a real, closed set of types is *accepted*: this
            // codebase's `<`/`>`/`+` C operators are only correct for
            // plain numeric C types — `Text`'s `<` is pointer comparison,
            // not lexicographic, and `Decimal`'s `+`/`<` don't compile at
            // all (struct operands) — see `crates/mir/src/lower.rs`'s
            // per-call-site comparator/adder synthesis, which relies on
            // this check having already ruled those out.
            const KEY_PROJECTING_CALLEES: &[&str] =
                &["List.sortBy", "List.minBy", "List.maxBy", "List.sumBy"];
            // `fn_name` now correctly resolves a module-qualified callee
            // like `List.sortBy(...)` (BACKLOG item 171 fixed the shared
            // `Expr::Field` gap at its source) — this used to re-derive the
            // name itself via `qualified_call_name` directly, back when
            // that gap was worked around locally rather than fixed.
            if let Some(name) = &fn_name {
                if KEY_PROJECTING_CALLEES.contains(&name.as_str()) {
                    if let Some((key_fn_ty, key_span)) = arg_info.get(1) {
                        if let Ty::Fn { ret, .. } = ctx.uf.apply(key_fn_ty) {
                            let resolved_key = ctx.uf.apply(&ret);
                            // A key projection that does field access on a
                            // struct element (`(p) => p.price`) can't be
                            // resolved here at all: typeck's own
                            // `Expr::Field` inference (`resolve_field_ty`)
                            // returns a brand-new, permanently disconnected
                            // fresh var for field access on a still-
                            // unbound base type — confirmed directly, this
                            // stays `Ty::Var` forever regardless of later
                            // unification, since no constraint is ever
                            // recorded linking it back to the field name.
                            // Flagging a bare `Ty::Var` here would reject
                            // every legitimate struct-element key (Float
                            // included), a real regression confirmed by
                            // testing `List.sortBy(products, (p) =>
                            // p.price)`. HIR's *own* field resolution
                            // (`crates/hir/src/lower.rs`'s `resolve_field_ty`)
                            // correctly resolves this after the per-call-
                            // site lambda param hint is applied, so the
                            // genuinely-unsupported cases (Text/Decimal
                            // struct fields) are instead caught there, as
                            // E0601 — this check only covers what typeck
                            // can actually see (bare/identity projections).
                            if !matches!(resolved_key, Ty::Var(_)) && !is_supported_key_type(&resolved_key) {
                                ctx.errors.push(TypeError {
                                    kind: TypeErrorKind::UnsupportedKeyType {
                                        fn_name: name.clone(),
                                        found: resolved_key,
                                    },
                                    span: *key_span,
                                });
                            }
                        }
                    }
                }
            }

            ret_ty
        }

        Expr::Pipe { left, right, span } => {
            let left_ty = infer(left, ctx);
            match &right.node {
                // `a |> f(b, c)` — type-check as `f(a, b, c)`
                Expr::App { func, args, .. } => {
                    let (func_ty, row_check) = resolve_callee(func, ctx);
                    let mut param_tys = vec![left_ty];
                    param_tys.extend(args.iter().map(|a| infer(&a.value, ctx)));
                    let ret_ty = ctx.fresh();
                    let expected = Ty::Fn { params: param_tys, ret: Box::new(ret_ty.clone()) };
                    ctx.unify(func_ty, expected, *span);
                    apply_row_check(&row_check, ctx, *span);
                    ret_ty
                }
                // `a |> f` — type-check as `f(a)`
                _ => {
                    let (right_ty, row_check) = resolve_callee(right, ctx);
                    let ret_ty = ctx.fresh();
                    let expected_fn = Ty::Fn { params: vec![left_ty], ret: Box::new(ret_ty.clone()) };
                    ctx.unify(right_ty, expected_fn, *span);
                    apply_row_check(&row_check, ctx, *span);
                    ret_ty
                }
            }
        }

        Expr::BinOp { op, left, right, span } => {
            infer_binop(op, left, right, *span, ctx)
        }

        Expr::UnOp { op, expr, span } => {
            let ty = infer(expr, ctx);
            match op {
                UnOp::Neg => {
                    ctx.unify(ty.clone(), Ty::Int, *span);
                    ty
                }
                UnOp::Not => {
                    ctx.unify(ty, Ty::Bool, *span);
                    Ty::Bool
                }
            }
        }

        Expr::Field { expr, field, span } => {
            // Check if this is a module-qualified call: Text.join, List.map, etc.
            // (upper-case single-segment path not bound as a local)
            if let Some(qualified) = qualified_call_name(expr, field) {
                if let Some(ty) = ctx.env.lookup(&qualified) {
                    let ty = ty.clone();
                    return ctx.instantiate(ty);
                }
            }

            let obj_ty = infer(expr, ctx);
            let obj_ty = ctx.uf.apply(&obj_ty);
            resolve_field_ty(obj_ty, field, *span, ctx)
        }

        Expr::SafeField { expr, field, span } => {
            // expr?.field : Option<T> where expr has field : T. Unify the
            // base's own type against Option<fresh> first so the Some
            // payload can be resolved through ordinary field access,
            // regardless of whether expr's type is already known to be an
            // Option or still an unresolved type variable — previously this
            // ran field resolution directly against the still-Option-
            // wrapped base type, so it could never succeed on a real
            // Optional (BACKLOG item 146).
            let base_ty = infer(expr, ctx);
            let inner_var = ctx.fresh();
            ctx.unify(base_ty, Ty::Option(Box::new(inner_var.clone())), expr.span);
            let inner_ty = ctx.uf.apply(&inner_var);
            let field_ty = resolve_field_ty(inner_ty, field, *span, ctx);
            Ty::Option(Box::new(field_ty))
        }

        Expr::If { cond, then_expr, else_expr, .. } => {
            let cond_ty = infer(cond, ctx);
            ctx.unify(cond_ty, Ty::Bool, cond.span);
            let then_ty = infer(then_expr, ctx);
            let else_ty = infer(else_expr, ctx);
            // Point at the else branch when branches disagree
            ctx.unify(then_ty.clone(), else_ty, else_expr.span);
            then_ty
        }

        Expr::Match { scrutinee, arms, span } => {
            let scrut_ty = infer(scrutinee, ctx);
            let result_ty = ctx.fresh();
            for arm in arms {
                ctx.env.push();
                check_pattern(&arm.pattern.node, scrut_ty.clone(), ctx);
                if let Some(g) = &arm.guard {
                    let gty = infer(g, ctx);
                    ctx.unify(gty, Ty::Bool, *span);
                }
                let arm_ty = infer(&arm.body, ctx);
                ctx.unify(arm_ty, result_ty.clone(), *span);
                ctx.env.pop();
            }
            check_exhaustive(&scrut_ty, arms, ctx, *span);
            result_ty
        }

        Expr::Block { stmts, .. } => {
            ctx.env.push();
            let ty = infer_block(stmts, ctx);
            ctx.env.pop();
            ty
        }

        Expr::Lambda { params, body, .. } => {
            ctx.env.push();
            let param_tys: Vec<Ty> = params.iter().map(|p| {
                let ty = match &p.ty {
                    Some(ann) => type_expr_to_ty(&ann.node, ctx),
                    None      => ctx.fresh(),
                };
                ctx.env.define(p.name.node.clone(), ty.clone());
                ty
            }).collect();
            let ret_ty = infer(body, ctx);
            ctx.env.pop();
            Ty::Fn { params: param_tys, ret: Box::new(ret_ty) }
        }

        Expr::List { elements, .. } => {
            let elem_ty = ctx.fresh();
            for e in elements {
                let et = infer(e, ctx);
                ctx.unify(et, elem_ty.clone(), e.span);
            }
            Ty::List(Box::new(elem_ty))
        }

        Expr::Tuple { elements, .. } => {
            Ty::Tuple(elements.iter().map(|e| infer(e, ctx)).collect())
        }

        Expr::Record { ty_name, base, fields, span } => {
            let mut field_tys: Vec<(String, Ty)> = fields
                .iter()
                .map(|f| (f.name.node.clone(), infer(&f.value, ctx)))
                .collect();

            // Captured when `base` resolves to a nominal type, so a
            // `ty_name: None` spread (e.g. `.with(...)`'s desugar — BACKLOG
            // item 151 — which has no syntactic type name at its call site)
            // can still return the *same* nominal type as `base` below,
            // rather than degrading to a structural `Ty::Record` that would
            // no longer unify against an ordinary use of that type.
            let mut base_named: Option<Ty> = None;

            if let Some(b) = base {
                let base_ty = infer(b, ctx);
                let base_ty = ctx.uf.apply(&base_ty);
                // Accept both structural Ty::Record and nominal Ty::Named (record spread).
                let resolved = match &base_ty {
                    Ty::Named { .. } => {
                        base_named = Some(base_ty.clone());
                        if let Ty::Named { name, .. } = &base_ty {
                            ctx.env.record_fields.get(name.as_str()).cloned().map(Ty::Record)
                        } else {
                            None
                        }
                    }
                    other => Some(other.clone()),
                };
                if let Some(Ty::Record(mut base_fields)) = resolved {
                    for (name, ty) in &field_tys {
                        if let Some(bf) = base_fields.iter_mut().find(|(n, _)| n == name) {
                            bf.1 = ty.clone();
                        }
                    }
                    field_tys = base_fields;
                } else {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch { expected: Ty::Record(vec![]), found: base_ty },
                        span: *span,
                    });
                }
            }
            // Reject a field name in construction that isn't real — either
            // a `computed` property (BACKLOG item 143 — derived, never
            // settable) or not a field of the type at all (BACKLOG item
            // 174: previously neither a direct literal nor `.with(...)`
            // (item 151) validated this at all — an unknown/typo'd name
            // was silently accepted by typeck, then either surfaced two
            // stages later as a confusing raw C error `field designator
            // does not refer to any field` (a direct literal) or, for
            // `.with(...)`, silently dropped with no error anywhere,
            // confirmed directly). Checked uniformly for both a direct
            // literal (`ty_name` present) and a `.with(...)` copy-update
            // (`ty_name: None`, but `base` resolved to a nominal type) —
            // whichever of the two actually names the type here, since
            // exactly one is ever present. Skipped entirely when the type
            // isn't a known record at all (`record_fields.get` misses) —
            // matching `resolve_field_ty`'s own conservative fallback for
            // e.g. a statemachine type, not a real record.
            let construct_on_ty: Option<Ty> = base_named.clone()
                .or_else(|| ty_name.clone().map(|n| Ty::Named { name: n, args: vec![] }));
            if let Some(Ty::Named { name: tn, .. }) = &construct_on_ty {
                if let Some(declared) = ctx.env.record_fields.get(tn.as_str()) {
                    let declared_names: Vec<&str> = declared.iter().map(|(n, _)| n.as_str()).collect();
                    let computed_names: Vec<&str> = ctx.env.computed_fields.get(tn.as_str())
                        .map(|cs| cs.iter().map(|(n, _)| n.as_str()).collect())
                        .unwrap_or_default();
                    for f in fields.iter() {
                        let fname = f.name.node.as_str();
                        if computed_names.contains(&fname) {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::ComputedFieldNotSettable {
                                    field: f.name.node.clone(),
                                    type_name: tn.clone(),
                                },
                                span: f.span,
                            });
                        } else if !declared_names.contains(&fname) {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::UnknownField {
                                    field: f.name.node.clone(),
                                    on: construct_on_ty.clone().unwrap(),
                                },
                                span: f.span,
                            });
                        }
                    }
                }
            }
            // When written as `TypeName { ... }`, unify each field value against
            // the declared field type so mismatches are caught at compile time.
            if let Some(name) = ty_name {
                // A generic type's declared field types (`record_fields`)
                // reference its own type-param vars, minted *once* at the
                // type's own hoisting — every occurrence of `TypeName { .. }`
                // must instantiate them fresh (mirroring `Forall::instantiate`
                // for a generic function reference), or two occurrences with
                // different concrete field types would be forced to unify
                // with each other through the same shared var. Empty for a
                // non-generic type, matching the previous (correct) behaviour
                // exactly.
                let type_params = ctx.env.type_param_vars.get(name.as_str()).cloned().unwrap_or_default();
                let subst: HashMap<TyVar, Ty> = type_params.iter().map(|&v| {
                    *ctx.counter += 1;
                    (v, Ty::Var(*ctx.counter))
                }).collect();
                if let Some(declared) = ctx.env.record_fields.get(name.as_str()).cloned() {
                    for (fname, found_ty) in &field_tys {
                        if let Some((_, expected_ty)) = declared.iter().find(|(n, _)| n == fname) {
                            let expected_ty = expected_ty.apply_subst(&subst);
                            ctx.unify(found_ty.clone(), expected_ty, *span);
                        }
                    }
                }
                let args: Vec<Ty> = type_params.iter().map(|v| subst[v].clone()).collect();
                Ty::Named { name: name.clone(), args }
            } else if let Some(named) = base_named {
                // `.with(...)`'s desugar (BACKLOG item 151): no syntactic
                // `ty_name`, but `base` was nominal — stay that same nominal
                // type rather than degrading to a structural `Ty::Record`,
                // so the result still unifies anywhere the original type is
                // expected (e.g. passed to a `Type.method(...)` call).
                named
            } else {
                Ty::Record(field_tys)
            }
        }

        Expr::Try { expr, span } => {
            // `e?` — e must be Result<T, E>; the expression has type T
            let inner_ty = infer(expr, ctx);
            let ok_ty = ctx.fresh();
            let err_ty = ctx.fresh();
            ctx.unify(inner_ty, Ty::Result(Box::new(ok_ty.clone()), Box::new(err_ty)), *span);
            ok_ty
        }

        Expr::Await { expr, .. } => infer(expr, ctx),

        Expr::Spawn { expr, .. } => infer(expr, ctx),

        Expr::Guard { cond, else_expr, span } => {
            let cond_ty = infer(cond, ctx);
            ctx.unify(cond_ty, Ty::Bool, *span);
            infer(else_expr, ctx);
            Ty::Unit
        }

        Expr::Require { expr, span, .. } => {
            let inner = infer(expr, ctx);
            let ok_ty = ctx.fresh();
            let err_ty = ctx.fresh();
            ctx.unify(inner, Ty::Result(Box::new(ok_ty.clone()), Box::new(err_ty)), *span);
            ok_ty
        }

        Expr::Parallel { tasks, timeout, span } => {
            // `parallel(timeout: ...) { ... }` — the timeout clause was parsed
            // but never actually type-checked (or even threaded past parsing
            // at all) before BACKLOG item 81; constrain it to `Duration` like
            // any other typed value.
            if let Some(t) = timeout {
                let ty = infer(t, ctx);
                ctx.unify(ty, Ty::Named { name: "Duration".into(), args: vec![] }, *span);
            }
            Ty::Tuple(tasks.iter().map(|t| infer(t, ctx)).collect())
        }

        // `withTimeout(d) { body }` (BACKLOG item 122) — cooperative-cancellation
        // timeout: `Some(value)` if `body` finishes before `d` elapses, `None`
        // if the deadline passes first (the caller is never blocked past it).
        Expr::WithTimeout { duration, body, span } => {
            let dur_ty = infer(duration, ctx);
            ctx.unify(dur_ty, Ty::Named { name: "Duration".into(), args: vec![] }, *span);
            let body_ty = infer(body, ctx);
            Ty::Option(Box::new(body_ty))
        }

        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => infer(body, ctx),

        Expr::Ascribe { expr, ty, span } => {
            let inferred = infer(expr, ctx);
            let annotated = type_expr_to_ty(&ty.node, ctx);
            ctx.unify(inferred, annotated.clone(), *span);
            annotated
        }

        Expr::For { binding, iter, body, .. } => {
            let iter_ty = infer(iter, ctx);
            let elem_ty = ctx.fresh();
            ctx.unify(iter_ty, Ty::List(Box::new(elem_ty.clone())), iter.span);
            ctx.env.push();
            ctx.env.define(binding.node.clone(), elem_ty);
            infer(body, ctx);
            ctx.env.pop();
            Ty::Unit
        }

        Expr::While { cond, body, span } => {
            // cond must be Bool; body is evaluated for side effects; result is Unit
            let cond_ty = infer(cond, ctx);
            ctx.unify(cond_ty, Ty::Bool, *span);
            infer(body, ctx);
            Ty::Unit
        }
        Expr::Age { expr, span } => {
            let base_ty = infer(expr, ctx);
            let base_ty = ctx.uf.apply(&base_ty);
            let is_timestamp = match &base_ty {
                Ty::Named { name, .. } if name == "Timestamp" => true,
                Ty::Option(inner) => matches!(inner.as_ref(), Ty::Named { name, .. } if name == "Timestamp"),
                Ty::Var(_) => true, // unresolved — optimistically allow
                _ => false,
            };
            if is_timestamp {
                Ty::Named { name: "Duration".to_string(), args: vec![] }
            } else {
                ctx.errors.push(TypeError {
                    kind: TypeErrorKind::AgeOnNonTimestamp { found: base_ty },
                    span: *span,
                });
                Ty::Error
            }
        }
        // BACKLOG item 165 — `expect(x).toBe(y)` and friends. Every failure
        // mode reuses the existing generic `unify` machinery (same E0200 a
        // plain `==`/type-mismatch would produce) rather than a dedicated
        // error kind: `.toBe` unifies exactly like `BinOp::Eq` does (so it's
        // only as capable as `==` already is — Int/Bool/Text/Float work,
        // Decimal/records hit the same pre-existing codegen gap `a == b`
        // would, not a new one); `.toBeSome`/`.toBeNone`/`.toBeOk`/
        // `.toBeErr` unify against a fresh `Option<_>`/`Result<_,_>`, so a
        // non-Option/Result receiver is reported the same way any other
        // shape mismatch is.
        Expr::ExpectAssertion { actual, matcher, span } => {
            let actual_ty = infer(actual, ctx);
            match matcher {
                ExpectMatcher::ToBe(y) => {
                    let y_ty = infer(y, ctx);
                    ctx.unify(actual_ty, y_ty, *span);
                }
                ExpectMatcher::ToBeTrue | ExpectMatcher::ToBeFalse => {
                    ctx.unify(actual_ty, Ty::Bool, *span);
                }
                ExpectMatcher::ToBeSome | ExpectMatcher::ToBeNone => {
                    let inner = ctx.fresh();
                    ctx.unify(actual_ty, Ty::Option(Box::new(inner)), *span);
                }
                ExpectMatcher::ToBeOk | ExpectMatcher::ToBeErr => {
                    let ok = ctx.fresh();
                    let err = ctx.fresh();
                    ctx.unify(actual_ty, Ty::Result(Box::new(ok), Box::new(err)), *span);
                }
            }
            Ty::Unit
        }
    }
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

/// Types that can be interpolated into an f-string. Must stay in sync with the
/// conversions codegen emits in `coerce_to_text` (Int/Float/Bool/Decimal/Text).
fn is_displayable(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Decimal(_) | Ty::Text)
}

/// Types `List.sortBy`/`minBy`/`maxBy`/`sumBy`'s key/numeric projection can
/// resolve to (BACKLOG item 162b) — the real, closed set this codebase's
/// `<`/`>`/`+` C operators are correct for (`emit_binop`,
/// `crates/codegen/src/emit_mir.rs`). Must stay in sync with
/// `crates/mir/src/lower.rs`'s per-call-site comparator/adder synthesis,
/// which trusts this check has already ruled out everything else.
fn is_supported_key_type(ty: &Ty) -> bool {
    matches!(ty,
        Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt | Ty::Float | Ty::Float32)
}

// ------------------------------------------------------------------ //
// Row-polymorphism bounds (`R: { name: Text }`)
// ------------------------------------------------------------------ //

/// A row check to run once a call's arguments have been unified: the bounds
/// themselves (each as the bound type param's *original* `TyVar` plus its
/// required fields), and the substitution mapping those original vars to the
/// fresh vars this particular call instantiated them to.
type RowCheck = (Vec<(TyVar, Vec<(String, Ty)>)>, HashMap<TyVar, Ty>);

/// A bare-name path (`fn(x)`) resolves to `fn`'s row bounds by that name;
/// `Type.method(x)` resolves by qualified name via `qualified_call_name`,
/// matching how impl methods are hoisted into `row_bounds` (`"Type.method"`).
///
/// Resolve a call's callee expression, instantiating its type — and if it names a
/// row-bounded function, returning the row check to run once the call's arguments
/// are unified. Shared by `Expr::App` and both branches of `Expr::Pipe`, since a
/// row-bounded function can be called either way (`f(x)` or `x |> f`).
///
/// `infer(func, ctx)`'s generic `Expr::Path` case instantiates the callee's type
/// too, but discards the substitution it used, which we need here to know which
/// fresh var a bound type param became for *this* call.
fn resolve_callee(func: &S<Expr>, ctx: &mut Ctx<'_>) -> (Ty, Option<RowCheck>) {
    let lookup_name = match &func.node {
        Expr::Path { path, .. } => path.segments.last().map(|s| s.node.clone()),
        // `Type.method(...)` — same uppercase-first-segment heuristic the
        // Expr::Field arm below uses to resolve the call's type in the first
        // place; without this, the call itself still type-checks (via that
        // arm), but any row bound on `method` was silently never checked.
        Expr::Field { expr, field, .. } => qualified_call_name(expr, field),
        _ => None,
    };

    let resolved = lookup_name.as_deref().and_then(|name| {
        let bounds = ctx.env.get_row_bounds(name)?.clone();
        let raw = ctx.env.lookup(name)?.clone();
        let (instantiated, subst) = raw.instantiate_with_subst(ctx.counter);
        Some((instantiated, bounds, subst))
    });

    match resolved {
        Some((ty, bounds, subst)) => (ty, Some((bounds, subst))),
        None => (infer(func, ctx), None),
    }
}

/// If `expr.field` is a module/type-qualified reference — `expr` is a bare
/// `Path` whose first segment starts uppercase, e.g. `Text.join`, `List.map`,
/// `Order.getName` — return its qualified lookup key (`"Text.join"`, etc).
/// Returns `None` for an ordinary value field access (`record.name`), where
/// the first segment is a lowercase local/param, not a type name.
fn qualified_call_name(expr: &S<Expr>, field: &S<String>) -> Option<String> {
    let Expr::Path { path, .. } = &expr.node else { return None };
    let first = path.segments.first()?;
    if first.node.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Some(format!("{}.{}", first.node, field.node))
    } else {
        None
    }
}

/// Run a row check produced by `resolve_callee`, after the call's arguments have
/// been unified (so the bound type param's fresh var is as resolved as it's going
/// to get).
fn apply_row_check(row_check: &Option<RowCheck>, ctx: &mut Ctx<'_>, span: Span) {
    let Some((bounds, subst)) = row_check else { return };
    for (orig_var, required_fields) in bounds {
        if let Some(bound_ty) = subst.get(orig_var) {
            let resolved = ctx.uf.apply(bound_ty);
            check_row_bound(&resolved, required_fields, ctx, span);
        }
    }
}

/// Check that `resolved_ty` structurally has at least `required_fields`, with
/// compatible types — the actual row-polymorphism satisfaction check.
fn check_row_bound(resolved_ty: &Ty, required_fields: &[(String, Ty)], ctx: &mut Ctx<'_>, span: Span) {
    // Still an unresolved variable — the bound param was never pinned down to a
    // concrete type by the call (e.g. an unused argument), nothing to check yet.
    if matches!(resolved_ty, Ty::Var(_) | Ty::Error) { return; }

    let actual_fields: Vec<(String, Ty)> = match resolved_ty {
        Ty::Record(fields) => fields.clone(),
        Ty::Named { name, args } if args.is_empty() => {
            match ctx.env.record_fields.get(name) {
                Some(fields) => fields.clone(),
                None => {
                    push_row_shape_mismatch(resolved_ty, required_fields, ctx, span);
                    return;
                }
            }
        }
        _ => {
            push_row_shape_mismatch(resolved_ty, required_fields, ctx, span);
            return;
        }
    };

    for (field_name, required_ty) in required_fields {
        match actual_fields.iter().find(|(n, _)| n == field_name) {
            // Field exists — check its type the same way any other value is
            // checked, so a mismatch here gets the same well-formatted E0200.
            Some((_, actual_ty)) => ctx.unify(actual_ty.clone(), required_ty.clone(), span),
            None => ctx.errors.push(TypeError {
                kind: TypeErrorKind::MissingRowField {
                    ty:       resolved_ty.clone(),
                    field:    field_name.clone(),
                    required: required_ty.clone(),
                },
                span,
            }),
        }
    }
}

fn push_row_shape_mismatch(resolved_ty: &Ty, required_fields: &[(String, Ty)], ctx: &mut Ctx<'_>, span: Span) {
    ctx.errors.push(TypeError {
        kind: TypeErrorKind::Mismatch {
            expected: Ty::Record(required_fields.to_vec()),
            found:    resolved_ty.clone(),
        },
        span,
    });
}

fn infer_lit(lit: &Lit) -> Ty {
    match lit {
        Lit::Int(_)     => Ty::Int,
        Lit::Float(_)   => Ty::Float,
        Lit::Decimal(_) => Ty::Decimal(None),
        Lit::Bool(_)    => Ty::Bool,
        Lit::String(_)  => Ty::Text,
        Lit::FString(_) => Ty::Text,
        Lit::Uuid(_)    => Ty::Uuid,
        Lit::Unit       => Ty::Unit,
    }
}

/// Render a `BinOp` as its real Certo source symbol, for diagnostic messages.
fn binop_symbol(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
        BinOp::Div => "/", BinOp::Rem => "%", BinOp::Pow => "**",
        BinOp::Eq => "==", BinOp::NotEq => "!=",
        BinOp::Lt => "<", BinOp::LtEq => "<=", BinOp::Gt => ">", BinOp::GtEq => ">=",
        BinOp::And => "and", BinOp::Or => "or",
        BinOp::RangeInclusive => "..", BinOp::RangeExclusive => "...",
        BinOp::NullCoalesce => "??", BinOp::Concat => "++",
    }
}

fn infer_binop(op: &BinOp, left: &S<Expr>, right: &S<Expr>, span: Span, ctx: &mut Ctx<'_>) -> Ty {
    let lt = infer(left, ctx);
    let rt = infer(right, ctx);
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow => {
            ctx.unify(lt.clone(), rt, span);
            // BACKLOG item 214 — `Timestamp`/`DateTime`/`Date` are opaque,
            // non-unifying nominal types with no operator overloading (see
            // item 85); same-type arithmetic between them previously
            // unified fine and silently returned that same type, e.g.
            // `timestamp - timestamp` "worked" and produced another
            // `Timestamp`, not a `Duration`. `Duration` itself is
            // deliberately excluded — see `OpaqueTemporalArithmetic`'s own
            // doc comment for why.
            let resolved = ctx.uf.apply(&lt);
            if let Ty::Named { name, args } = &resolved {
                if args.is_empty() && matches!(name.as_str(), "Timestamp" | "DateTime" | "Date") {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::OpaqueTemporalArithmetic {
                            ty: resolved.clone(),
                            op: binop_symbol(op).to_string(),
                        },
                        span,
                    });
                }
            }
            lt
        }
        BinOp::Eq | BinOp::NotEq => {
            ctx.unify(lt, rt, span);
            Ty::Bool
        }
        BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
            ctx.unify(lt, rt, span);
            Ty::Bool
        }
        BinOp::And | BinOp::Or => {
            ctx.unify(lt, Ty::Bool, span);
            ctx.unify(rt, Ty::Bool, span);
            Ty::Bool
        }
        BinOp::RangeInclusive | BinOp::RangeExclusive => {
            ctx.unify(lt.clone(), rt, span);
            Ty::List(Box::new(lt))
        }
        BinOp::NullCoalesce => {
            // `a ?? b` — a must be Option<T>, b must be T
            let inner = ctx.fresh();
            ctx.unify(lt, Ty::Option(Box::new(inner.clone())), span);
            ctx.unify(rt, inner.clone(), span);
            inner
        }
        BinOp::Concat => {
            // `a ++ b` — both operands and result are Text
            ctx.unify(lt, Ty::Text, span);
            ctx.unify(rt, Ty::Text, span);
            Ty::Text
        }
    }
}

/// Infer the type of a block — the type of the last statement if it's an
/// expression, otherwise Unit.
pub fn infer_block(stmts: &[Stmt], ctx: &mut Ctx<'_>) -> Ty {
    let mut last_ty = Ty::Unit;
    for stmt in stmts {
        last_ty = infer_stmt(stmt, ctx);
    }
    last_ty
}

pub fn infer_stmt(stmt: &Stmt, ctx: &mut Ctx<'_>) -> Ty {
    match stmt {
        Stmt::Val { pattern, ty, value, span } => {
            let val_ty = infer(value, ctx);
            if let Some(ann) = ty {
                let ann_ty = type_expr_to_ty(&ann.node, ctx);
                // Point at the value expression, not the whole statement
                ctx.unify(val_ty.clone(), ann_ty, value.span);
            }
            let generalised = ctx.env.generalise(val_ty, ctx.uf);
            check_pattern(&pattern.node, generalised, ctx);
            let _ = span;
            Ty::Unit
        }
        Stmt::Var { name, ty, value, span } => {
            let val_ty = infer(value, ctx);
            if let Some(ann) = ty {
                let ann_ty = type_expr_to_ty(&ann.node, ctx);
                ctx.unify(val_ty.clone(), ann_ty, value.span);
            }
            ctx.env.define(name.node.clone(), val_ty);
            let _ = span;
            Ty::Unit
        }
        Stmt::Assign { target, value, span } => {
            let val_ty = infer(value, ctx);
            match ctx.env.lookup(&target.node) {
                Some(existing) => {
                    let existing = existing.clone();
                    ctx.unify(val_ty, existing, value.span);
                }
                None => {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::UnboundName(target.node.clone()),
                        span: *span,
                    });
                }
            }
            Ty::Unit
        }
        Stmt::Defer { body, .. } => {
            infer(body, ctx);
            Ty::Unit
        }
        Stmt::Expr { expr, .. } => infer(expr, ctx),
    }
}

/// Check that a `match`'s arms cover every possible value of the scrutinee's
/// type. An unguarded wildcard/binding arm always makes a match exhaustive,
/// regardless of the scrutinee's type — checked first, before looking at the
/// type at all. Beyond that, this is fully correct for `Bool`, `Option`,
/// `Result`, and user-declared sum types (their variant sets are finite and
/// known); every other type (`Int`, `Text`, `Float`, tuples, lists, records,
/// opaque named types, ...) has no finite, checkable set of literal/shape
/// patterns that could ever be exhaustive on its own, so those require an
/// unguarded catch-all arm — there's no partial "structural" exhaustiveness
/// check for compound patterns like Rust's usefulness algorithm has.
///
/// A guarded arm (`pat if cond => ...`) never counts as covering `pat`'s
/// shape: the guard could fail at runtime, so the arm might not actually
/// handle that case.
fn check_exhaustive(scrut_ty: &Ty, arms: &[certo_ast::expr::MatchArm], ctx: &mut Ctx<'_>, span: Span) {
    if arms.iter().any(|arm| arm.guard.is_none() && is_catch_all(&arm.pattern.node)) {
        return;
    }

    let resolved = ctx.uf.apply(scrut_ty);
    let missing: Vec<String> = match &resolved {
        Ty::Var(_) | Ty::Error => return, // unresolved/erroneous — don't cascade a second error

        Ty::Bool => {
            let mut covered = HashSet::new();
            for arm in arms.iter().filter(|a| a.guard.is_none()) {
                collect_bool_literals(&arm.pattern.node, &mut covered);
            }
            let mut missing = Vec::new();
            if !covered.contains(&true)  { missing.push("true".to_string()); }
            if !covered.contains(&false) { missing.push("false".to_string()); }
            missing
        }

        Ty::Option(_) => missing_variants(arms, &["Some", "None"], |n| match n {
            "Some" => "Some(_)".to_string(),
            other  => other.to_string(),
        }),

        Ty::Result(_, _) => missing_variants(arms, &["Ok", "Err"], |n| format!("{n}(_)")),

        Ty::Named { name, .. } => {
            match ctx.env.sum_variants.get(name).cloned() {
                Some(variants) => {
                    let refs: Vec<&str> = variants.iter().map(|v| v.as_str()).collect();
                    missing_variants(arms, &refs, |n| n.to_string())
                }
                // A plain record / opaque named type has no finite variant set —
                // only a catch-all (already ruled out above) can be exhaustive.
                None => vec!["_".to_string()],
            }
        }

        // Int, Text, Float, Decimal, Tuple, List, Map, Uuid, Unit, Fn, ... —
        // same reasoning as the plain-named-type case above.
        _ => vec!["_".to_string()],
    };

    if !missing.is_empty() {
        ctx.errors.push(TypeError {
            kind: TypeErrorKind::NonExhaustiveMatch { ty: resolved, missing },
            span,
        });
    }
}

/// Coverage check shared by `Option`/`Result`/user sum types: every named
/// variant in `variants` must be matched by some unguarded arm's top-level
/// constructor pattern.
fn missing_variants(
    arms:     &[certo_ast::expr::MatchArm],
    variants: &[&str],
    label:    impl Fn(&str) -> String,
) -> Vec<String> {
    let mut covered = HashSet::new();
    for arm in arms.iter().filter(|a| a.guard.is_none()) {
        collect_constructor_names(&arm.pattern.node, &mut covered);
    }
    variants.iter().filter(|v| !covered.contains(**v)).map(|v| label(v)).collect()
}

/// True for a pattern that matches any value unconditionally: a bare
/// wildcard/binding, an alias/or-pattern that reduces to one, or a tuple
/// pattern whose every element is itself unconditional — a tuple type has
/// exactly one shape, so e.g. `(a, b)` alone is genuinely exhaustive with no
/// need for a trailing wildcard arm. The same isn't recognized for
/// `Record`/`Constructor` patterns even though a plain (non-sum) record type
/// has the same one-shape property: distinguishing "the one shape of a
/// record type" from "one variant of a sum type" needs the type environment,
/// which this purely-structural check doesn't have — see BACKLOG (match
/// exhaustiveness item).
fn is_catch_all(pat: &certo_ast::pattern::Pattern) -> bool {
    use certo_ast::pattern::Pattern;
    match pat {
        Pattern::Wildcard { .. } | Pattern::Ident { .. } => true,
        Pattern::As { pattern, .. } => is_catch_all(&pattern.node),
        Pattern::Or { left, right, .. } => is_catch_all(&left.node) || is_catch_all(&right.node),
        Pattern::Guard { pattern, .. } => is_catch_all(&pattern.node),
        Pattern::Tuple { elements, .. } => elements.iter().all(|e| is_catch_all(&e.node)),
        _ => false,
    }
}

fn collect_bool_literals(pat: &certo_ast::pattern::Pattern, out: &mut HashSet<bool>) {
    use certo_ast::pattern::{Pattern, LitPat};
    match pat {
        Pattern::Literal { value: LitPat::Bool(b), .. } => { out.insert(*b); }
        Pattern::As { pattern, .. } | Pattern::Guard { pattern, .. } => collect_bool_literals(&pattern.node, out),
        Pattern::Or { left, right, .. } => {
            collect_bool_literals(&left.node, out);
            collect_bool_literals(&right.node, out);
        }
        _ => {}
    }
}

fn collect_constructor_names(pat: &certo_ast::pattern::Pattern, out: &mut HashSet<String>) {
    use certo_ast::pattern::Pattern;
    match pat {
        Pattern::Constructor { path, .. } => {
            if let Some(s) = path.segments.last() { out.insert(s.node.clone()); }
        }
        Pattern::Record { path: Some(p), .. } => {
            if let Some(s) = p.segments.last() { out.insert(s.node.clone()); }
        }
        Pattern::As { pattern, .. } | Pattern::Guard { pattern, .. } => collect_constructor_names(&pattern.node, out),
        Pattern::Or { left, right, .. } => {
            collect_constructor_names(&left.node, out);
            collect_constructor_names(&right.node, out);
        }
        _ => {}
    }
}

/// Bind pattern variables to fresh type variables (simplified — full
/// pattern matching would also unify the pattern with the scrutinee type).
fn bind_pattern_vars(pat: &certo_ast::pattern::Pattern, ctx: &mut Ctx<'_>) {
    use certo_ast::pattern::Pattern;
    match pat {
        Pattern::Ident { name, .. } => {
            let ty = ctx.fresh();
            ctx.env.define(name.node.clone(), ty);
        }
        Pattern::As { pattern, name, .. } => {
            bind_pattern_vars(&pattern.node, ctx);
            let ty = ctx.fresh();
            ctx.env.define(name.node.clone(), ty);
        }
        Pattern::Tuple { elements, .. } => {
            for e in elements { bind_pattern_vars(&e.node, ctx); }
        }
        Pattern::List { head, tail, .. } => {
            for e in head { bind_pattern_vars(&e.node, ctx); }
            if let Some(r) = tail { bind_pattern_vars(&r.node, ctx); }
        }
        Pattern::Constructor { fields, .. } => {
            for f in fields { bind_pattern_vars(&f.node, ctx); }
        }
        Pattern::Record { fields, .. } => {
            for f in fields {
                if let Some(p) = &f.pattern { bind_pattern_vars(&p.node, ctx); }
            }
        }
        Pattern::Guard { pattern, .. } => bind_pattern_vars(&pattern.node, ctx),
        Pattern::Or { left, right, .. } => {
            bind_pattern_vars(&left.node, ctx);
            bind_pattern_vars(&right.node, ctx);
        }
        Pattern::Wildcard { .. } | Pattern::Literal { .. } => {}
    }
}

/// Type-check a pattern against its scrutinee/expected type: unify each
/// structural piece (literal, tuple element, constructor field, record
/// field, list element) against the corresponding part of `expected`, and
/// bind pattern variables with their precise inferred types rather than
/// fresh, totally unconstrained ones. Shared by `match` arms and `val`
/// destructuring, since both are "a pattern must describe a value of a
/// known type."
///
/// Falls back to `bind_pattern_vars` (fresh vars, no real check) only where
/// there's genuinely no type information to check against — an unresolved
/// constructor name (resolve should already flag that separately), or a
/// named-field sum-variant pattern (`Circle { radius: r }`): variant fields
/// are registered as a positional constructor `Fn`, not individually by
/// name, so there's no per-field type to look up for that specific form.
fn check_pattern(pat: &certo_ast::pattern::Pattern, expected: Ty, ctx: &mut Ctx<'_>) {
    use certo_ast::pattern::{Pattern, LitPat};
    match pat {
        Pattern::Wildcard { .. } => {}

        Pattern::Ident { name, .. } => {
            ctx.env.define(name.node.clone(), expected);
        }

        Pattern::As { pattern, name, .. } => {
            check_pattern(&pattern.node, expected.clone(), ctx);
            ctx.env.define(name.node.clone(), expected);
        }

        Pattern::Guard { pattern, .. } => check_pattern(&pattern.node, expected, ctx),

        Pattern::Or { left, right, .. } => {
            check_pattern(&left.node, expected.clone(), ctx);
            check_pattern(&right.node, expected, ctx);
        }

        Pattern::Literal { value, span } => {
            let lit_ty = match value {
                LitPat::Int(_)    => Ty::Int,
                LitPat::Float(_)  => Ty::Float,
                LitPat::Bool(_)   => Ty::Bool,
                LitPat::String(_) => Ty::Text,
                LitPat::Unit      => Ty::Unit,
            };
            let inst = ctx.instantiate(expected);
            ctx.unify(lit_ty, inst, *span);
        }

        Pattern::Tuple { elements, span } => {
            let elem_tys: Vec<Ty> = elements.iter().map(|_| ctx.fresh()).collect();
            let inst = ctx.instantiate(expected);
            ctx.unify(Ty::Tuple(elem_tys.clone()), inst, *span);
            for (e, ety) in elements.iter().zip(elem_tys) {
                check_pattern(&e.node, ety, ctx);
            }
        }

        Pattern::List { head, tail, span } => {
            let elem_ty = ctx.fresh();
            let list_ty = Ty::List(Box::new(elem_ty.clone()));
            let inst = ctx.instantiate(expected);
            ctx.unify(list_ty.clone(), inst, *span);
            for h in head { check_pattern(&h.node, elem_ty.clone(), ctx); }
            if let Some(t) = tail { check_pattern(&t.node, list_ty, ctx); }
        }

        Pattern::Constructor { path, fields, span } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            match ctx.env.lookup(name).cloned() {
                Some(ctor_ty) => {
                    match ctx.instantiate(ctor_ty) {
                        Ty::Fn { params, ret } => {
                            let inst = ctx.instantiate(expected);
                            ctx.unify(*ret, inst, *span);
                            if params.len() != fields.len() {
                                ctx.errors.push(TypeError {
                                    kind: TypeErrorKind::ArityMismatch { expected: params.len(), found: fields.len() },
                                    span: *span,
                                });
                                for f in fields { bind_pattern_vars(&f.node, ctx); }
                            } else {
                                for (f, pty) in fields.iter().zip(params) {
                                    check_pattern(&f.node, pty, ctx);
                                }
                            }
                        }
                        other => {
                            // Unit variant / 0-arg constructor: `other` IS the parent type.
                            let inst = ctx.instantiate(expected);
                            ctx.unify(other, inst, *span);
                            if !fields.is_empty() {
                                ctx.errors.push(TypeError {
                                    kind: TypeErrorKind::ArityMismatch { expected: 0, found: fields.len() },
                                    span: *span,
                                });
                            }
                            for f in fields { bind_pattern_vars(&f.node, ctx); }
                        }
                    }
                }
                None => {
                    // Unknown constructor — resolve should already flag this; don't
                    // cascade a spurious type error, just bind fields defensively.
                    for f in fields { bind_pattern_vars(&f.node, ctx); }
                }
            }
        }

        Pattern::Record { path, fields, span, .. } => {
            let type_name = path.as_ref()
                .and_then(|p| p.segments.last())
                .map(|s| s.node.clone())
                .or_else(|| match ctx.uf.apply(&expected) {
                    Ty::Named { name, .. } => Some(name),
                    _ => None,
                });
            let declared = type_name.as_ref().and_then(|n| ctx.env.record_fields.get(n).cloned());

            if let (Some(name), Some(declared)) = (&type_name, &declared) {
                // A generic record's declared field types (`record_fields`)
                // reference its own type-param vars, minted *once* at the
                // type's own hoisting — each pattern occurrence must
                // instantiate them fresh and unify against the record's
                // *real* instantiation args, not a permanently empty
                // `args` (BACKLOG item 177, found while verifying item
                // 145's own match-arm record-pattern fix) — mirrors
                // `Expr::Record`'s own identical, already-correct fix for
                // record-*literal* construction just above in this same
                // function. Empty for a non-generic type, matching the
                // previous (correct) behaviour exactly.
                let type_params = ctx.env.type_param_vars.get(name.as_str()).cloned().unwrap_or_default();
                let subst: HashMap<TyVar, Ty> = type_params.iter().map(|&v| {
                    *ctx.counter += 1;
                    (v, Ty::Var(*ctx.counter))
                }).collect();
                let args: Vec<Ty> = type_params.iter().map(|v| subst[v].clone()).collect();
                let inst = ctx.instantiate(expected);
                ctx.unify(Ty::Named { name: name.clone(), args }, inst, *span);
                for f in fields {
                    let field_ty = declared.iter().find(|(n, _)| n == &f.name.node)
                        .map(|(_, t)| t.apply_subst(&subst))
                        .unwrap_or_else(|| ctx.fresh());
                    match &f.pattern {
                        Some(p) => check_pattern(&p.node, field_ty, ctx),
                        None    => ctx.env.define(f.name.node.clone(), field_ty), // shorthand `{ x }`
                    }
                }
            } else {
                // Best-effort fallback — see doc comment above.
                for f in fields {
                    match &f.pattern {
                        Some(p) => bind_pattern_vars(&p.node, ctx),
                        None    => { let ty = ctx.fresh(); ctx.env.define(f.name.node.clone(), ty); }
                    }
                }
            }
        }
    }
}
