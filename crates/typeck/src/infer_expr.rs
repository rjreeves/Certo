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
            let arg_tys: Vec<Ty> = args.iter().map(|a| infer(&a.value, ctx)).collect();
            let ret_ty = ctx.fresh();
            let expected_fn = Ty::Fn { params: arg_tys, ret: Box::new(ret_ty.clone()) };
            ctx.unify(func_ty, expected_fn, *span);
            ret_ty
        }

        Expr::Pipe { left, right, span } => {
            // `a |> f` is equivalent to `f(a)`
            let left_ty = infer(left, ctx);
            let right_ty = infer(right, ctx);
            let ret_ty = ctx.fresh();
            let expected_fn = Ty::Fn { params: vec![left_ty], ret: Box::new(ret_ty.clone()) };
            ctx.unify(right_ty, expected_fn, *span);
            ret_ty
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

        Expr::If { cond, then_expr, else_expr, span } => {
            let cond_ty = infer(cond, ctx);
            ctx.unify(cond_ty, Ty::Bool, *span);
            let then_ty = infer(then_expr, ctx);
            let else_ty = infer(else_expr, ctx);
            ctx.unify(then_ty.clone(), else_ty, *span);
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

        Expr::Block { stmts, span } => {
            ctx.env.push();
            let ty = infer_block(stmts, *span, ctx);
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

        Expr::List { elements, span } => {
            let elem_ty = ctx.fresh();
            for e in elements {
                let et = infer(e, ctx);
                ctx.unify(et, elem_ty.clone(), *span);
            }
            Ty::List(Box::new(elem_ty))
        }

        Expr::Tuple { elements, .. } => {
            Ty::Tuple(elements.iter().map(|e| infer(e, ctx)).collect())
        }

        Expr::Record { base, fields, span } => {
            let mut field_tys: Vec<(String, Ty)> = fields
                .iter()
                .map(|f| (f.name.node.clone(), infer(&f.value, ctx)))
                .collect();

            if let Some(b) = base {
                let base_ty = infer(b, ctx);
                let base_ty = ctx.uf.apply(&base_ty);
                if let Ty::Record(mut base_fields) = base_ty {
                    // Override base fields with the update
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
            Ty::Record(field_tys)
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

        Expr::For { iter, body, .. } => {
            // iter must be a list; body is evaluated for side effects; result is Unit
            let iter_ty  = infer(iter, ctx);
            let elem_ty  = ctx.fresh();
            ctx.unify(iter_ty, Ty::List(Box::new(elem_ty)), iter.span);
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
pub fn infer_block(stmts: &[Stmt], span: Span, ctx: &mut Ctx<'_>) -> Ty {
    let mut last_ty = Ty::Unit;
    for stmt in stmts {
        last_ty = infer_stmt(stmt, span, ctx);
    }
    last_ty
}

pub fn infer_stmt(stmt: &Stmt, span: Span, ctx: &mut Ctx<'_>) -> Ty {
    match stmt {
        Stmt::Val { pattern, ty, value, .. } => {
            let val_ty = infer(value, ctx);
            if let Some(ann) = ty {
                let ann_ty = type_expr_to_ty(&ann.node, ctx);
                ctx.unify(val_ty.clone(), ann_ty, span);
            }
            // Generalise and bind pattern
            let generalised = ctx.env.generalise(val_ty, ctx.uf);
            bind_pattern_tys(&pattern.node, generalised, ctx);
            Ty::Unit
        }
        Stmt::Var { name, ty, value, .. } => {
            let val_ty = infer(value, ctx);
            if let Some(ann) = ty {
                let ann_ty = type_expr_to_ty(&ann.node, ctx);
                ctx.unify(val_ty.clone(), ann_ty, span);
            }
            ctx.env.define(name.node.clone(), val_ty);
            Ty::Unit
        }
        Stmt::Assign { target, value, .. } => {
            let val_ty = infer(value, ctx);
            match ctx.env.lookup(&target.node) {
                Some(existing) => {
                    let existing = existing.clone();
                    ctx.unify(val_ty, existing, span);
                }
                None => {
                    ctx.errors.push(TypeError {
                        kind: TypeErrorKind::UnboundName(target.node.clone()),
                        span,
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
