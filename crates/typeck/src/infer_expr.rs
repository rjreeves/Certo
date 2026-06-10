use certo_ast::expr::{Expr, Stmt, Lit, BinOp, UnOp};
use certo_ast::span::{S, Span};
use certo_ast::types::TypeExpr;
use crate::ty::Ty;
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

    /// Unify, recording errors instead of returning them.
    pub fn unify(&mut self, a: Ty, b: Ty, span: Span) {
        if let Err(e) = self.uf.unify(a, b, span) {
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
                "Decimal" => Ty::Decimal,
                "Bool"    => Ty::Bool,
                "Text"    => Ty::Text,
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
                _ => Ty::Named { name, args: targs },
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
            // Generic type parameter — treat as fresh var if not in env; this
            // is a simplification for now (proper polymorphic defs need skolems).
            let fresh = ctx.fresh();
            ctx.env.define(name.node.clone(), fresh.clone());
            fresh
        }
        TE::Ptr { inner, .. } => type_expr_to_ty(&inner.node, ctx),
    }
}

// ------------------------------------------------------------------ //
// Infer the type of an expression.
// ------------------------------------------------------------------ //

pub fn infer(expr: &S<Expr>, ctx: &mut Ctx<'_>) -> Ty {
    match &expr.node {
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
            let func_ty = infer(func, ctx);

            let has_labels = args.iter().any(|a| a.label.is_some());
            // Build both a full-qualified path (for stdlib) and a short name (for user-defined).
            let (fn_full_path, fn_short_name) = if let Expr::Path { path, .. } = &func.node {
                let full = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
                let short = path.segments.last().map(|s| s.node.clone());
                (Some(full), short)
            } else {
                (None, None)
            };
            // Prefer full-path lookup (stdlib), fall back to short name (user-defined).
            let fn_name: Option<String> = fn_full_path
                .as_ref()
                .filter(|fp| ctx.env.get_param_meta(fp).is_some())
                .cloned()
                .or(fn_short_name);

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
            ret_ty
        }

        Expr::Pipe { left, right, span } => {
            let left_ty = infer(left, ctx);
            match &right.node {
                // `a |> f(b, c)` — type-check as `f(a, b, c)`
                Expr::App { func, args, .. } => {
                    let func_ty = infer(func, ctx);
                    let mut param_tys = vec![left_ty];
                    param_tys.extend(args.iter().map(|a| infer(&a.value, ctx)));
                    let ret_ty = ctx.fresh();
                    let expected = Ty::Fn { params: param_tys, ret: Box::new(ret_ty.clone()) };
                    ctx.unify(func_ty, expected, *span);
                    ret_ty
                }
                // `a |> f` — type-check as `f(a)`
                _ => {
                    let right_ty = infer(right, ctx);
                    let ret_ty = ctx.fresh();
                    let expected_fn = Ty::Fn { params: vec![left_ty], ret: Box::new(ret_ty.clone()) };
                    ctx.unify(right_ty, expected_fn, *span);
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
            if let Expr::Path { path, .. } = &expr.node {
                let first = path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
                let is_upper = first.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if is_upper {
                    let qualified = format!("{}.{}", first, field.node);
                    if let Some(ty) = ctx.env.lookup(&qualified) {
                        let ty = ty.clone();
                        return ctx.instantiate(ty);
                    }
                }
            }

            let obj_ty = infer(expr, ctx);
            let obj_ty = ctx.uf.apply(&obj_ty);
            match &obj_ty {
                Ty::Record(fields) => {
                    match fields.iter().find(|(n, _)| n == &field.node) {
                        Some((_, ty)) => ty.clone(),
                        None => {
                            ctx.errors.push(TypeError {
                                kind: TypeErrorKind::UnknownField { field: field.node.clone(), on: obj_ty },
                                span: *span,
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
                            None => {
                                ctx.errors.push(TypeError {
                                    kind: TypeErrorKind::UnknownField { field: field.node.clone(), on: obj_ty },
                                    span: *span,
                                });
                                Ty::Error
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
                        span: *span,
                    });
                    Ty::Error
                }
            }
        }

        Expr::SafeField { expr, field, span } => {
            // expr?.field : Option<T> where expr has field : T
            let inner = infer(&S { node: Expr::Field {
                expr: expr.clone(),
                field: field.clone(),
                span: *span,
            }, span: *span }, ctx);
            Ty::Option(Box::new(inner))
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
                // Bind pattern variables as fresh vars (simplified — full pattern inference is complex)
                bind_pattern_vars(&arm.pattern.node, ctx);
                if let Some(g) = &arm.guard {
                    let gty = infer(g, ctx);
                    ctx.unify(gty, Ty::Bool, *span);
                }
                let arm_ty = infer(&arm.body, ctx);
                ctx.unify(arm_ty, result_ty.clone(), *span);
                ctx.env.pop();
            }
            // Unify scrutinee with pattern scrutinee type (simplified for now)
            let _ = scrut_ty;
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

            if let Some(b) = base {
                let base_ty = infer(b, ctx);
                let base_ty = ctx.uf.apply(&base_ty);
                // Accept both structural Ty::Record and nominal Ty::Named (record spread).
                let resolved = match &base_ty {
                    Ty::Named { name, .. } => ctx.env.record_fields.get(name.as_str()).cloned()
                        .map(Ty::Record),
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
            // When written as `TypeName { ... }`, return the named type so callers
            // that expect `Ty::Named { name: "TypeName" }` unify correctly.
            if let Some(name) = ty_name {
                // Still infer field types to surface any errors, but discard the structural type.
                Ty::Named { name: name.clone(), args: vec![] }
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

        Expr::Parallel { tasks, .. } => {
            Ty::Tuple(tasks.iter().map(|t| infer(t, ctx)).collect())
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
    }
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

fn infer_lit(lit: &Lit) -> Ty {
    match lit {
        Lit::Int(_)     => Ty::Int,
        Lit::Float(_)   => Ty::Float,
        Lit::Decimal(_) => Ty::Decimal,
        Lit::Bool(_)    => Ty::Bool,
        Lit::String(_)  => Ty::Text,
        Lit::FString(_) => Ty::Text,
        Lit::Uuid(_)    => Ty::Uuid,
        Lit::Unit       => Ty::Unit,
    }
}

fn infer_binop(op: &BinOp, left: &S<Expr>, right: &S<Expr>, span: Span, ctx: &mut Ctx<'_>) -> Ty {
    let lt = infer(left, ctx);
    let rt = infer(right, ctx);
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow => {
            ctx.unify(lt.clone(), rt, span);
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
            bind_pattern_tys(&pattern.node, generalised, ctx);
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

fn bind_pattern_tys(pat: &certo_ast::pattern::Pattern, ty: Ty, ctx: &mut Ctx<'_>) {
    use certo_ast::pattern::Pattern;
    match pat {
        Pattern::Ident { name, .. } => {
            ctx.env.define(name.node.clone(), ty);
        }
        Pattern::Wildcard { .. } => {}
        _ => bind_pattern_vars(pat, ctx),
    }
}
