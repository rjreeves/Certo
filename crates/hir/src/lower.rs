use std::collections::HashMap;
use certo_ast::module::Module;
use certo_ast::decl::Decl;
use certo_ast::expr::{Expr, Stmt, Lit, BinOp as AstBinOp, UnOp as AstUnOp, FStringPart};
use certo_ast::pattern::Pattern;
use certo_ast::span::{S, Span};
use certo_typeck::Ty;
use crate::hir::*;
use crate::error::{LowerError, LowerErrorKind};

// ------------------------------------------------------------------ //
// Lowering context
// ------------------------------------------------------------------ //

struct Cx {
    /// Next LocalId to assign.
    next_local: LocalId,
    /// Next FnId to assign.
    next_fn:    FnId,
    /// Name → LocalId for the current scope stack.
    locals:     Vec<HashMap<String, LocalId>>,
    /// Name → FnId for top-level functions.
    globals:    HashMap<String, FnId>,
    errors:     Vec<LowerError>,
}

impl Cx {
    fn new() -> Self {
        Cx {
            next_local: 0,
            next_fn:    0,
            locals:     vec![HashMap::new()],
            globals:    HashMap::new(),
            errors:     Vec::new(),
        }
    }

    fn fresh_local(&mut self) -> LocalId {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    fn fresh_fn(&mut self) -> FnId {
        let id = self.next_fn;
        self.next_fn += 1;
        id
    }

    fn define_local(&mut self, name: &str) -> LocalId {
        let id = self.fresh_local();
        self.locals.last_mut().unwrap().insert(name.to_string(), id);
        id
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        for frame in self.locals.iter().rev() {
            if let Some(&id) = frame.get(name) { return Some(id); }
        }
        None
    }

    fn push_scope(&mut self) { self.locals.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.locals.pop(); }

    fn err(&mut self, kind: LowerErrorKind, span: Span) {
        self.errors.push(LowerError { kind, span });
    }
}

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

pub fn lower_module(module: &Module) -> Result<HirModule, Vec<LowerError>> {
    let mut cx = Cx::new();

    // Register all top-level fn names first (for mutual recursion).
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            let id = cx.fresh_fn();
            cx.globals.insert(f.name.node.clone(), id);
        }
    }

    let mut items = Vec::new();
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                let id = cx.globals[&f.name.node];
                cx.push_scope();

                let params: Vec<HirParam> = f.params.iter().map(|p| {
                    let local = cx.define_local(&p.name.node);
                    HirParam { local, name: p.name.node.clone(), ty: Ty::Error, span: p.span }
                }).collect();

                let body = f.body.as_ref().map(|b| lower_expr(b, &mut cx));

                cx.pop_scope();

                items.push(HirItem::Fn(HirFn {
                    id,
                    name:   f.name.node.clone(),
                    params,
                    ret_ty: Ty::Error, // filled by type checker later
                    body,
                    span:   f.span,
                }));
            }

            Decl::Val(v) => {
                let value = lower_expr(&v.value, &mut cx);
                let name = match &v.pattern.node {
                    Pattern::Ident { name, .. } => name.node.clone(),
                    _ => "<pattern>".to_string(),
                };
                items.push(HirItem::Const(HirConst {
                    name,
                    ty:    Ty::Error,
                    value,
                    span:  v.span,
                }));
            }

            _ => {} // type decls, migrations, etc. don't lower to HIR items
        }
    }

    let name = module.path.segments.last()
        .map(|s| s.node.clone())
        .unwrap_or_default();

    if cx.errors.is_empty() {
        Ok(HirModule { name, items })
    } else {
        Err(cx.errors)
    }
}

// ------------------------------------------------------------------ //
// Expression lowering
// ------------------------------------------------------------------ //

fn lower_expr(expr: &S<Expr>, cx: &mut Cx) -> HirExpr {
    let span = expr.span;
    match &expr.node {
        // Desugar f-string to ++ chain before generic lit handling
        Expr::Lit { value: Lit::FString(parts), .. } => {
            let mut segments: Vec<HirExpr> = parts.iter().map(|p| match p {
                FStringPart::Literal(s) => HirExpr {
                    kind: HirExprKind::Str(s.clone()), ty: Ty::Text, span,
                },
                FStringPart::Interpolated(e) => lower_expr(e, cx),
            }).collect();
            if segments.is_empty() {
                return HirExpr { kind: HirExprKind::Str(String::new()), ty: Ty::Text, span };
            }
            let first = segments.remove(0);
            return segments.into_iter().fold(first, |acc, seg| HirExpr {
                kind: HirExprKind::BinOp {
                    op:  BinOp::Concat,
                    lhs: Box::new(acc),
                    rhs: Box::new(seg),
                },
                ty: Ty::Text, span,
            });
        }

        Expr::Lit { value, .. } => lower_lit(value, span),

        Expr::Path { path, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            if let Some(local) = cx.lookup_local(name) {
                HirExpr { kind: HirExprKind::Local(local), ty: Ty::Error, span }
            } else {
                HirExpr { kind: HirExprKind::Global(name.to_string()), ty: Ty::Error, span }
            }
        }

        // Desugar `a |> f` → `f(a)`
        Expr::Pipe { left, right, .. } => {
            let lhs = lower_expr(left, cx);
            let func = lower_expr(right, cx);
            HirExpr {
                kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs] },
                ty: Ty::Error,
                span,
            }
        }

        Expr::App { func, args, .. } => {
            let func = lower_expr(func, cx);
            let args = args.iter().map(|a| lower_expr(&a.value, cx)).collect();
            HirExpr { kind: HirExprKind::Call { func: Box::new(func), args }, ty: Ty::Error, span }
        }

        Expr::BinOp { op, left, right, .. } => {
            // Desugar range ops to stdlib calls; keep primitives as BinOp.
            match op {
                AstBinOp::RangeInclusive | AstBinOp::RangeExclusive => {
                    let fn_name = if *op == AstBinOp::RangeInclusive { "range_inclusive" } else { "range" };
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    let func = HirExpr { kind: HirExprKind::Global(fn_name.into()), ty: Ty::Error, span };
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs, rhs] }, ty: Ty::Error, span }
                }
                _ => {
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    HirExpr {
                        kind: HirExprKind::BinOp { op: lower_binop(op), lhs: Box::new(lhs), rhs: Box::new(rhs) },
                        ty: Ty::Error,
                        span,
                    }
                }
            }
        }

        Expr::UnOp { op, expr, .. } => {
            let arg = lower_expr(expr, cx);
            let op = match op { AstUnOp::Neg => UnOp::Neg, AstUnOp::Not => UnOp::Not };
            HirExpr { kind: HirExprKind::UnOp { op, arg: Box::new(arg) }, ty: Ty::Error, span }
        }

        Expr::Field { expr, field, .. } => {
            // If the base is a module/type path (starts with an uppercase letter and
            // resolves to no local), treat `Module.fn` as a global function reference
            // rather than a struct field access.
            let is_module_path = match &expr.node {
                Expr::Path { path, .. } => {
                    let first = path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
                    let is_upper = first.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                    let last = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                    is_upper && cx.lookup_local(last).is_none()
                }
                _ => false,
            };
            if is_module_path {
                // Build a dotted name: "Text.indexOf"
                let base_name = match &expr.node {
                    Expr::Path { path, .. } => path.segments.iter()
                        .map(|s| s.node.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                    _ => unreachable!(),
                };
                let global_name = format!("{}.{}", base_name, field.node);
                HirExpr { kind: HirExprKind::Global(global_name), ty: Ty::Error, span }
            } else {
                let base = lower_expr(expr, cx);
                HirExpr { kind: HirExprKind::Field { base: Box::new(base), field: field.node.clone() }, ty: Ty::Error, span }
            }
        }

        // SafeField `e?.f` → `match e { Some(v) => Some(v.f), None => None }`
        Expr::SafeField { expr, field, .. } => {
            let base = lower_expr(expr, cx);
            let tmp = cx.fresh_local();
            let bind = HirPat::Bind { local: tmp, name: "_safe_tmp".into() };
            let field_access = HirExpr {
                kind: HirExprKind::Field {
                    base:  Box::new(HirExpr { kind: HirExprKind::Local(tmp), ty: Ty::Error, span }),
                    field: field.node.clone(),
                },
                ty: Ty::Error, span,
            };
            let some_arm = HirArm {
                pat: bind,
                body: HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr { kind: HirExprKind::Global("Some".into()), ty: Ty::Error, span }),
                        args: vec![field_access],
                    },
                    ty: Ty::Error, span,
                },
            };
            let none_arm = HirArm {
                pat: HirPat::Constructor { name: "None".into(), fields: vec![] },
                body: HirExpr { kind: HirExprKind::Global("None".into()), ty: Ty::Error, span },
            };
            HirExpr {
                kind: HirExprKind::Match { scrutinee: Box::new(base), arms: vec![some_arm, none_arm] },
                ty: Ty::Error, span,
            }
        }

        Expr::If { cond, then_expr, else_expr, .. } => {
            let cond = lower_expr(cond, cx);
            let then_ = lower_expr(then_expr, cx);
            let else_ = lower_expr(else_expr, cx);
            HirExpr { kind: HirExprKind::If { cond: Box::new(cond), then_expr: Box::new(then_), else_expr: Box::new(else_) }, ty: Ty::Error, span }
        }

        Expr::Match { scrutinee, arms, .. } => {
            let scrut = lower_expr(scrutinee, cx);
            let arms = arms.iter().map(|arm| {
                cx.push_scope();
                let pat = lower_pat(&arm.pattern, cx);
                let body = lower_expr(&arm.body, cx);
                cx.pop_scope();
                HirArm { pat, body }
            }).collect();
            HirExpr { kind: HirExprKind::Match { scrutinee: Box::new(scrut), arms }, ty: Ty::Error, span }
        }

        Expr::Block { stmts, .. } => lower_block(stmts, span, cx),

        Expr::Lambda { params, body, .. } => {
            cx.push_scope();
            let hir_params: Vec<HirParam> = params.iter().map(|p| {
                let local = cx.define_local(&p.name.node);
                HirParam { local, name: p.name.node.clone(), ty: Ty::Error, span: p.span }
            }).collect();
            let body = lower_expr(body, cx);
            cx.pop_scope();
            HirExpr { kind: HirExprKind::Lambda { params: hir_params, body: Box::new(body) }, ty: Ty::Error, span }
        }

        Expr::List { elements, .. } => {
            let elems = elements.iter().map(|e| lower_expr(e, cx)).collect();
            HirExpr { kind: HirExprKind::List(elems), ty: Ty::Error, span }
        }

        Expr::Tuple { elements, .. } => {
            let elems = elements.iter().map(|e| lower_expr(e, cx)).collect();
            HirExpr { kind: HirExprKind::Tuple(elems), ty: Ty::Error, span }
        }

        Expr::Record { base, fields, .. } => {
            let mut hir_fields: Vec<(String, HirExpr)> = fields.iter()
                .map(|f| (f.name.node.clone(), lower_expr(&f.value, cx)))
                .collect();
            if let Some(b) = base {
                // `base with { f: v }` — lower base and prepend as a spread.
                // In HIR we model this as calling a compiler builtin `record_update`.
                let base_expr = lower_expr(b, cx);
                let update_fn = HirExpr { kind: HirExprKind::Global("__record_update".into()), ty: Ty::Error, span };
                let record_expr = HirExpr { kind: HirExprKind::Record(hir_fields), ty: Ty::Error, span };
                return HirExpr {
                    kind: HirExprKind::Call { func: Box::new(update_fn), args: vec![base_expr, record_expr] },
                    ty: Ty::Error, span,
                };
            }
            HirExpr { kind: HirExprKind::Record(hir_fields), ty: Ty::Error, span }
        }

        Expr::Try { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Try(Box::new(inner)), ty: Ty::Error, span }
        }

        Expr::Unsafe { body, .. } => {
            let inner = lower_expr(body, cx);
            HirExpr { kind: HirExprKind::Unsafe(Box::new(inner)), ty: Ty::Error, span }
        }

        // `await expr` — at HIR level async is transparent; scheduling handled by runtime.
        Expr::Await { expr, .. } => lower_expr(expr, cx),

        // `guard cond else e` → `if !cond { e }; unit`
        Expr::Guard { cond, else_expr, .. } => {
            let cond = lower_expr(cond, cx);
            let else_ = lower_expr(else_expr, cx);
            let not_cond = HirExpr { kind: HirExprKind::UnOp { op: UnOp::Not, arg: Box::new(cond) }, ty: Ty::Bool, span };
            let unit = HirExpr { kind: HirExprKind::Unit, ty: Ty::Unit, span };
            let if_expr = HirExpr {
                kind: HirExprKind::If { cond: Box::new(not_cond), then_expr: Box::new(else_), else_expr: Box::new(unit.clone()) },
                ty: Ty::Unit, span,
            };
            HirExpr { kind: HirExprKind::Block { stmts: vec![HirStmt::Expr(if_expr)], tail: Box::new(unit) }, ty: Ty::Unit, span }
        }

        // `require e (Err(..))` → `match e { Ok(v) => v, _ => return Err(..) }`
        // At HIR level we lower this to a Try on the expression.
        Expr::Require { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Try(Box::new(inner)), ty: Ty::Error, span }
        }

        Expr::Parallel { tasks, .. } => {
            // parallel { a, b, c } — lower as tuple construction (runtime handles scheduling)
            let lowered: Vec<HirExpr> = tasks.iter().map(|t| lower_expr(t, cx)).collect();
            HirExpr { kind: HirExprKind::Tuple(lowered), ty: Ty::Error, span }
        }

        Expr::Transaction { body, .. } => {
            // db.transaction { body } — lower as a call to __db_transaction(|| body)
            let inner = lower_expr(body, cx);
            let thunk = HirExpr { kind: HirExprKind::Lambda { params: vec![], body: Box::new(inner) }, ty: Ty::Error, span };
            let func  = HirExpr { kind: HirExprKind::Global("__db_transaction".into()), ty: Ty::Error, span };
            HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![thunk] }, ty: Ty::Error, span }
        }

        Expr::Ascribe { expr, .. } => lower_expr(expr, cx),

        Expr::For { binding, iter, body, .. } => {
            let iter_hir = lower_expr(iter, cx);
            cx.push_scope();
            let local = cx.define_local(&binding.node);
            let body_hir = lower_expr(body, cx);
            cx.pop_scope();
            HirExpr {
                kind: HirExprKind::For {
                    binding:      local,
                    binding_name: binding.node.clone(),
                    iter:         Box::new(iter_hir),
                    body:         Box::new(body_hir),
                },
                ty: Ty::Unit,
                span,
            }
        }
    }
}

fn lower_lit(lit: &Lit, span: Span) -> HirExpr {
    use certo_ast::expr::FStringPart;
    let (kind, ty) = match lit {
        Lit::Int(n)     => (HirExprKind::Int(*n),           Ty::Int),
        Lit::Float(f)   => (HirExprKind::Float(*f),         Ty::Float),
        Lit::Decimal(s) => (HirExprKind::Decimal(s.clone()), Ty::Decimal),
        Lit::Bool(b)    => (HirExprKind::Bool(*b),          Ty::Bool),
        Lit::String(s)  => (HirExprKind::Str(s.clone()),    Ty::Text),
        Lit::FString(parts) => {
            // Parts with interpolations are desugared in lower_expr before reaching here.
            // If we still have interpolated parts, join literal segments as a fallback.
            let joined = parts.iter().filter_map(|p| {
                if let FStringPart::Literal(s) = p { Some(s.clone()) } else { None }
            }).collect::<Vec<_>>().join("");
            (HirExprKind::Str(joined), Ty::Text)
        }
        Lit::Uuid(u) => (HirExprKind::Uuid(u.clone()), Ty::Uuid),
        Lit::Unit    => (HirExprKind::Unit, Ty::Unit),
    };
    HirExpr { kind, ty, span }
}

fn lower_binop(op: &AstBinOp) -> BinOp {
    match op {
        AstBinOp::Add => BinOp::Add,
        AstBinOp::Sub => BinOp::Sub,
        AstBinOp::Mul => BinOp::Mul,
        AstBinOp::Div => BinOp::Div,
        AstBinOp::Rem => BinOp::Rem,
        AstBinOp::Pow => BinOp::Pow,
        AstBinOp::Eq  => BinOp::Eq,
        AstBinOp::NotEq => BinOp::NotEq,
        AstBinOp::Lt  => BinOp::Lt,
        AstBinOp::LtEq => BinOp::LtEq,
        AstBinOp::Gt  => BinOp::Gt,
        AstBinOp::GtEq => BinOp::GtEq,
        AstBinOp::And => BinOp::And,
        AstBinOp::Or  => BinOp::Or,
        AstBinOp::NullCoalesce => BinOp::NullCoalesce,
        AstBinOp::Concat       => BinOp::Concat,
        AstBinOp::RangeInclusive | AstBinOp::RangeExclusive => unreachable!("handled above"),
    }
}

fn lower_block(stmts: &[Stmt], span: Span, cx: &mut Cx) -> HirExpr {
    cx.push_scope();
    let mut hir_stmts = Vec::new();
    let mut tail: Option<HirExpr> = None;

    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i == stmts.len() - 1;
        match stmt {
            Stmt::Val { pattern, value, .. } => {
                let init = lower_expr(value, cx);
                match &pattern.node {
                    Pattern::Ident { name, .. } => {
                        let local = cx.define_local(&name.node);
                        hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: Ty::Error, init });
                    }
                    Pattern::Wildcard { .. } => {
                        hir_stmts.push(HirStmt::Expr(init));
                    }
                    Pattern::Tuple { elements, .. } => {
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_tup".into(), ty: Ty::Error, init });
                        for (i, elem) in elements.iter().enumerate() {
                            if let Pattern::Ident { name, .. } = &elem.node {
                                let local = cx.define_local(&name.node);
                                let base = HirExpr { kind: HirExprKind::Local(tmp), ty: Ty::Error, span };
                                let field_expr = HirExpr {
                                    kind: HirExprKind::Field { base: Box::new(base), field: i.to_string() },
                                    ty: Ty::Error, span,
                                };
                                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: Ty::Error, init: field_expr });
                            }
                        }
                    }
                    Pattern::Record { fields, .. } => {
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_rec".into(), ty: Ty::Error, init });
                        for pf in fields {
                            let binding_name = if let Some(sub) = &pf.pattern {
                                if let Pattern::Ident { name, .. } = &sub.node { name.node.clone() } else { continue }
                            } else {
                                pf.name.node.clone()
                            };
                            let local = cx.define_local(&binding_name);
                            let base = HirExpr { kind: HirExprKind::Local(tmp), ty: Ty::Error, span };
                            let field_expr = HirExpr {
                                kind: HirExprKind::Field { base: Box::new(base), field: pf.name.node.clone() },
                                ty: Ty::Error, span,
                            };
                            hir_stmts.push(HirStmt::Let { local, name: binding_name, ty: Ty::Error, init: field_expr });
                        }
                    }
                    _ => {
                        // Complex patterns: lower to match + let
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_pat".into(), ty: Ty::Error, init });
                    }
                }
            }
            Stmt::Var { name, value, .. } => {
                let init = lower_expr(value, cx);
                let local = cx.define_local(&name.node);
                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: Ty::Error, init });
            }
            Stmt::Assign { target, value, .. } => {
                let v = lower_expr(value, cx);
                if let Some(local) = cx.lookup_local(&target.node) {
                    hir_stmts.push(HirStmt::Assign { local, value: v });
                } else {
                    cx.err(LowerErrorKind::UnresolvedName(target.node.clone()), target.span);
                }
            }
            Stmt::Defer { body, .. } => {
                // `defer` — emit as a statement; the C backend will handle cleanup.
                let e = lower_expr(body, cx);
                hir_stmts.push(HirStmt::Expr(e));
            }
            Stmt::Expr { expr, .. } => {
                let e = lower_expr(expr, cx);
                if is_last {
                    tail = Some(e);
                } else {
                    hir_stmts.push(HirStmt::Expr(e));
                }
            }
        }
    }

    cx.pop_scope();

    let tail = tail.unwrap_or(HirExpr { kind: HirExprKind::Unit, ty: Ty::Unit, span });
    if hir_stmts.is_empty() {
        tail
    } else {
        HirExpr { kind: HirExprKind::Block { stmts: hir_stmts, tail: Box::new(tail) }, ty: Ty::Error, span }
    }
}

fn lower_pat(pat: &S<certo_ast::pattern::Pattern>, cx: &mut Cx) -> HirPat {
    use certo_ast::pattern::{Pattern, LitPat};
    match &pat.node {
        Pattern::Wildcard { .. } => HirPat::Wildcard,
        Pattern::Ident { name, .. } => {
            let local = cx.define_local(&name.node);
            HirPat::Bind { local, name: name.node.clone() }
        }
        Pattern::Literal { value, .. } => HirPat::Lit(match value {
            LitPat::Int(n)    => HirLitPat::Int(*n),
            LitPat::Bool(b)   => HirLitPat::Bool(*b),
            LitPat::String(s) => HirLitPat::Str(s.clone()),
            LitPat::Float(_)  => HirLitPat::Int(0), // float patterns unusual — placeholder
            LitPat::Unit      => HirLitPat::Bool(true), // placeholder
        }),
        Pattern::Tuple { elements, .. } => HirPat::Tuple(elements.iter().map(|e| lower_pat(e, cx)).collect()),
        Pattern::Constructor { path, fields, .. } => {
            let name = path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            HirPat::Constructor { name, fields: fields.iter().map(|f| lower_pat(f, cx)).collect() }
        }
        Pattern::Or { left, right, .. } => HirPat::Or(
            Box::new(lower_pat(left, cx)),
            Box::new(lower_pat(right, cx)),
        ),
        // Record / Guard / As — flatten to wildcard for now (full pattern compilation later)
        _ => HirPat::Wildcard,
    }
}
