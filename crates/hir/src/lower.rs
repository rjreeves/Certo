use std::collections::HashMap;
use certo_ast::module::Module;
use certo_ast::decl::{Decl, FnParam};
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
    /// Name → param list for user-defined functions (for labeled/default arg normalization).
    fn_params:  HashMap<String, Vec<FnParam>>,
    /// Full-qualified name → param names for stdlib functions (keyed as "Module.fn").
    stdlib_params: HashMap<&'static str, &'static [&'static str]>,
    /// Statemachine-generated function full names → return type.
    sm_returns:    HashMap<String, Ty>,
    /// User-defined function names → return type (from AST annotation).
    fn_ret_types:  HashMap<String, Ty>,
    /// Global value types — for sum variant constants like `Red`, `Green`.
    global_types:  HashMap<String, Ty>,
    /// Sum variant name → parent type name (e.g. "Red" → "Color").
    variant_to_type: HashMap<String, String>,
    errors:        Vec<LowerError>,
}

impl Cx {
    fn new() -> Self {
        Cx {
            next_local:    0,
            next_fn:       0,
            locals:        vec![HashMap::new()],
            globals:       HashMap::new(),
            fn_params:     HashMap::new(),
            stdlib_params: stdlib_param_names(),
            sm_returns:    HashMap::new(),
            fn_ret_types:  HashMap::new(),
            global_types:    HashMap::new(),
            variant_to_type: HashMap::new(),
            errors:          Vec::new(),
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

/// Stdlib function parameter names, keyed by fully-qualified name (e.g. "Text.split").
/// Used to reorder named args at call sites for stdlib functions.
fn stdlib_param_names() -> HashMap<&'static str, &'static [&'static str]> {
    let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();

    // Core
    m.insert("assert",          &["cond", "msg"]);
    m.insert("pow",             &["base", "exp"]);
    m.insert("minInt",          &["a", "b"]);
    m.insert("maxInt",          &["a", "b"]);
    m.insert("minFloat",        &["a", "b"]);
    m.insert("maxFloat",        &["a", "b"]);
    m.insert("range",           &["from", "to"]);
    m.insert("rangeInclusive",  &["from", "to"]);

    // List
    m.insert("List.get",        &["list", "index"]);
    m.insert("List.getOrPanic", &["list", "index"]);
    m.insert("List.push",       &["list", "item"]);
    m.insert("List.concat",     &["a", "b"]);
    m.insert("List.slice",      &["list", "from", "to"]);
    m.insert("List.contains",   &["list", "item"]);
    m.insert("List.map",        &["list", "f"]);
    m.insert("List.filter",     &["list", "pred"]);
    m.insert("List.fold",       &["list", "init", "f"]);
    m.insert("List.find",       &["list", "pred"]);
    m.insert("List.any",        &["list", "pred"]);
    m.insert("List.all",        &["list", "pred"]);
    m.insert("List.sort",       &["list", "cmp"]);
    m.insert("List.zip",        &["a", "b"]);

    // Map
    m.insert("Map.insert",      &["map", "key", "value"]);
    m.insert("Map.get",         &["map", "key"]);
    m.insert("Map.contains",    &["map", "key"]);
    m.insert("Map.remove",      &["map", "key"]);

    // Text
    m.insert("Text.concat",     &["a", "b"]);
    m.insert("Text.contains",   &["text", "sub"]);
    m.insert("Text.startsWith", &["text", "prefix"]);
    m.insert("Text.endsWith",   &["text", "suffix"]);
    m.insert("Text.slice",      &["text", "from", "to"]);
    m.insert("Text.indexOf",    &["text", "sub"]);
    m.insert("Text.replace",    &["text", "from", "to"]);
    m.insert("Text.split",      &["text", "sep"]);
    m.insert("Text.join",       &["parts", "sep"]);
    m.insert("Text.repeat",     &["text", "n"]);

    // DateTime
    m.insert("DateTime.format",      &["dt", "fmt"]);
    m.insert("DateTime.addSeconds",  &["dt", "secs"]);
    m.insert("DateTime.addMinutes",  &["dt", "mins"]);
    m.insert("DateTime.addHours",    &["dt", "hours"]);
    m.insert("DateTime.addDays",     &["dt", "days"]);
    m.insert("DateTime.diffSeconds", &["a", "b"]);
    m.insert("DateTime.diffDays",    &["a", "b"]);
    m.insert("DateTime.before",      &["a", "b"]);
    m.insert("DateTime.after",       &["a", "b"]);
    m.insert("Date.format",          &["date", "fmt"]);

    // Decimal / Money
    m.insert("Decimal.add",    &["a", "b"]);
    m.insert("Decimal.sub",    &["a", "b"]);
    m.insert("Decimal.mul",    &["a", "b"]);
    m.insert("Decimal.div",    &["a", "b"]);
    m.insert("Decimal.round",  &["d", "places"]);

    // File / Path / IO
    m.insert("writeFile",      &["path", "content"]);
    m.insert("appendFile",     &["path", "content"]);
    m.insert("Path.join",      &["base", "part"]);

    // Process
    m.insert("Process.exec",   &["cmd", "args"]);

    // Json
    m.insert("JsonValue.at",   &["value", "index"]);
    m.insert("JsonValue.get",  &["value", "key"]);
    m.insert("JsonValue.push", &["array", "item"]);
    m.insert("JsonValue.set",  &["obj", "key", "value"]);

    m
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
            cx.fn_params.insert(f.name.node.clone(), f.params.clone());
            if let Some(ret) = &f.ret_ty {
                cx.fn_ret_types.insert(f.name.node.clone(), ast_ty_to_ty(&ret.node));
            }
        }
        // Register sum variant constructors and unit values.
        if let Decl::Type(t) = &sdecl.node {
            if let certo_ast::decl::TypeBody::Sum(variants) = &t.body {
                let parent_ty = Ty::Named { name: t.name.node.clone(), args: vec![] };
                for v in variants {
                    cx.variant_to_type.insert(v.name.node.clone(), t.name.node.clone());
                    if v.fields.is_empty() {
                        cx.global_types.insert(v.name.node.clone(), parent_ty.clone());
                    } else {
                        cx.fn_ret_types.insert(v.name.node.clone(), parent_ty.clone());
                    }
                }
            }
        }
    }

    // Pre-register statemachine-generated function return types so HIR call
    // expressions get the correct type (used by MIR for C codegen).
    for sdecl in &module.decls {
        if let Decl::StateMachine(sm) = &sdecl.node {
            let machine_ty = Ty::Named { name: sm.name.node.clone(), args: vec![] };
            let state_ty   = Ty::Named { name: format!("{}State", sm.name.node), args: vec![] };
            cx.sm_returns.insert(format!("{}_new",   sm.name.node), machine_ty.clone());
            cx.sm_returns.insert(format!("{}_state", sm.name.node), state_ty);
            for t in &sm.transitions {
                cx.sm_returns.insert(format!("{}_{}", sm.name.node, t.event.node), machine_ty.clone());
            }
            for state in &sm.states {
                cx.sm_returns.insert(format!("{}_is{}", sm.name.node, state.node), Ty::Bool);
            }
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
                    let ty = ast_ty_to_ty(&p.ty.node);
                    HirParam { local, name: p.name.node.clone(), ty, span: p.span }
                }).collect();

                let body = f.body.as_ref().map(|b| lower_expr(b, &mut cx));

                cx.pop_scope();

                items.push(HirItem::Fn(HirFn {
                    id,
                    name:   f.name.node.clone(),
                    params,
                    ret_ty: f.ret_ty.as_ref().map(|t| ast_ty_to_ty(&t.node)).unwrap_or(Ty::Error),
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

            _ => {} // type decls, migrations, on_enter hooks, etc. don't lower to HIR items
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
                let ty = cx.global_types.get(name).cloned().unwrap_or(Ty::Error);
                HirExpr { kind: HirExprKind::Global(name.to_string()), ty, span }
            }
        }

        Expr::Pipe { left, right, .. } => {
            let lhs = lower_expr(left, cx);
            match &right.node {
                // `a |> f(b, c)` → `f(a, b, c)`
                Expr::App { func, args, .. } => {
                    let func = lower_expr(func, cx);
                    let mut call_args = vec![lhs];
                    call_args.extend(args.iter().map(|a| lower_expr(&a.value, cx)));
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: call_args }, ty: Ty::Error, span }
                }
                // `a |> f` → `f(a)`
                _ => {
                    let func = lower_expr(right, cx);
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs] }, ty: Ty::Error, span }
                }
            }
        }

        Expr::App { func, args, .. } => {
            let func_hir = lower_expr(func, cx);

            // Extract the call target name(s) for param-reordering lookups.
            let (fn_full_path, fn_short_name) = match &func.node {
                Expr::Path { path, .. } => {
                    let full = path.segments.iter()
                        .map(|s| s.node.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let short = path.segments.last().map(|s| s.node.clone());
                    (Some(full), short)
                }
                _ => (None, None),
            };
            let has_labels = args.iter().any(|a| a.label.is_some());

            // Resolve param names: stdlib (by full path) takes priority, then user-defined (by short name).
            let stdlib_names: Option<&[&str]> = fn_full_path.as_deref()
                .and_then(|fp| cx.stdlib_params.get(fp).copied());
            let user_params: Option<Vec<FnParam>> = fn_short_name.as_ref()
                .and_then(|s| cx.fn_params.get(s).cloned());

            let lowered_args = if let Some(snames) = stdlib_names {
                // Stdlib function: only labeled reordering (no defaults).
                if has_labels {
                    let mut slots: Vec<Option<HirExpr>> = vec![None; snames.len()];
                    let mut pos_cursor = 0usize;
                    for arg in args {
                        let expr = lower_expr(&arg.value, cx);
                        if let Some(label) = &arg.label {
                            if let Some(idx) = snames.iter().position(|&n| n == label.node.as_str()) {
                                slots[idx] = Some(expr);
                            } else {
                                while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                                if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                            }
                        } else {
                            while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                            if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                        }
                    }
                    slots.into_iter().map(|maybe| {
                        maybe.unwrap_or_else(|| HirExpr { kind: HirExprKind::Unit, ty: Ty::Error, span })
                    }).collect()
                } else {
                    args.iter().map(|a| lower_expr(&a.value, cx)).collect()
                }
            } else if let Some(params) = user_params {
                if has_labels || args.len() < params.len() {
                    // Normalize: reorder labeled args, insert defaults for missing
                    let mut slots: Vec<Option<HirExpr>> = vec![None; params.len()];
                    let mut pos_cursor = 0usize;
                    for arg in args {
                        let expr = lower_expr(&arg.value, cx);
                        if let Some(label) = &arg.label {
                            if let Some(idx) = params.iter().position(|p| p.name.node == label.node) {
                                slots[idx] = Some(expr);
                            } else {
                                while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                                if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                            }
                        } else {
                            while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                            if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                        }
                    }
                    slots.into_iter().enumerate().map(|(i, maybe)| {
                        maybe.unwrap_or_else(|| {
                            if let Some(default_expr) = &params[i].default {
                                lower_expr(default_expr, cx)
                            } else {
                                cx.err(LowerErrorKind::Unsupported(
                                    format!("missing required argument `{}`", params[i].name.node)
                                ), span);
                                HirExpr { kind: HirExprKind::Unit, ty: Ty::Error, span }
                            }
                        })
                    }).collect()
                } else {
                    args.iter().map(|a| lower_expr(&a.value, cx)).collect()
                }
            } else {
                args.iter().map(|a| lower_expr(&a.value, cx)).collect()
            };

            // Look up the return type: statemachine fns first, then user-defined fns.
            let short = fn_short_name.as_deref().unwrap_or("");
            let call_ty = fn_full_path.as_deref()
                .and_then(|fp| cx.sm_returns.get(fp).cloned())
                .or_else(|| cx.fn_ret_types.get(short).cloned())
                .unwrap_or(Ty::Error);

            HirExpr { kind: HirExprKind::Call { func: Box::new(func_hir), args: lowered_args }, ty: call_ty, span }
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
                guard: None,
                body: HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr { kind: HirExprKind::Global("Some".into()), ty: Ty::Error, span }),
                        args: vec![field_access],
                    },
                    ty: Ty::Error, span,
                },
            };
            let none_arm = HirArm {
                pat:   HirPat::Constructor { name: "None".into(), fields: vec![] },
                guard: None,
                body:  HirExpr { kind: HirExprKind::Global("None".into()), ty: Ty::Error, span },
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
            let ty = if !matches!(then_.ty, Ty::Error) { then_.ty.clone() } else { else_.ty.clone() };
            HirExpr { kind: HirExprKind::If { cond: Box::new(cond), then_expr: Box::new(then_), else_expr: Box::new(else_) }, ty, span }
        }

        Expr::Match { scrutinee, arms, .. } => {
            let scrut = lower_expr(scrutinee, cx);
            let hir_arms: Vec<HirArm> = arms.iter().map(|arm| {
                cx.push_scope();
                let pat   = lower_pat(&arm.pattern, cx);
                let guard = arm.guard.as_ref().map(|g| lower_expr(g, cx));
                let body  = lower_expr(&arm.body, cx);
                cx.pop_scope();
                HirArm { pat, guard, body }
            }).collect();
            let ty = hir_arms.iter().find_map(|a| {
                if !matches!(a.body.ty, Ty::Error) { Some(a.body.ty.clone()) } else { None }
            }).unwrap_or(Ty::Error);
            HirExpr { kind: HirExprKind::Match { scrutinee: Box::new(scrut), arms: hir_arms }, ty, span }
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

        Expr::Record { ty_name, base, fields, .. } => {
            let record_ty = ty_name.as_deref()
                .map(|n| Ty::Named { name: n.to_string(), args: vec![] })
                .unwrap_or(Ty::Error);
            let mut hir_fields: Vec<(String, HirExpr)> = fields.iter()
                .map(|f| (f.name.node.clone(), lower_expr(&f.value, cx)))
                .collect();
            if let Some(b) = base {
                let base_expr = lower_expr(b, cx);
                let update_fn = HirExpr { kind: HirExprKind::Global("__record_update".into()), ty: Ty::Error, span };
                let record_expr = HirExpr { kind: HirExprKind::Record(hir_fields), ty: record_ty.clone(), span };
                return HirExpr {
                    kind: HirExprKind::Call { func: Box::new(update_fn), args: vec![base_expr, record_expr] },
                    ty: record_ty, span,
                };
            }
            HirExpr { kind: HirExprKind::Record(hir_fields), ty: record_ty, span }
        }

        Expr::Try { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Try(Box::new(inner)), ty: Ty::Error, span }
        }

        Expr::Unsafe { body, .. } => {
            let inner = lower_expr(body, cx);
            HirExpr { kind: HirExprKind::Unsafe(Box::new(inner)), ty: Ty::Error, span }
        }

        // `await task` — join a spawned task.
        Expr::Await { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Await(Box::new(inner)), ty: Ty::Error, span }
        }

        // `spawn expr` — run expr in a new task.
        // Lower `spawn f(a, b)` as a Call node wrapped in Spawn so MIR can emit the call,
        // with Spawn being transparent at HIR (the actual threading is done by codegen/runtime).
        Expr::Spawn { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Spawn { fn_name: String::new(), args: vec![inner] }, ty: Ty::Error, span }
        }

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
            // parallel { a, b, c } — spawn each task then await all, yielding a tuple.
            // Lower as: { val t0 = spawn a; val t1 = spawn b; ...; (await t0, await t1, ...) }
            let mut stmts: Vec<HirStmt> = Vec::new();
            let mut task_locals: Vec<LocalId> = Vec::new();
            for (i, task) in tasks.iter().enumerate() {
                let spawn_inner = lower_expr(task, cx);
                let spawn_expr = HirExpr {
                    kind: HirExprKind::Spawn { fn_name: format!("__parallel_task_{i}"), args: vec![spawn_inner] },
                    ty: Ty::Error, span,
                };
                let local = cx.fresh_local();
                stmts.push(HirStmt::Let { local, name: format!("__task_{i}"), ty: Ty::Error, init: spawn_expr });
                task_locals.push(local);
            }
            let awaited: Vec<HirExpr> = task_locals.iter().map(|&l| {
                let local_expr = HirExpr { kind: HirExprKind::Local(l), ty: Ty::Error, span };
                HirExpr { kind: HirExprKind::Await(Box::new(local_expr)), ty: Ty::Error, span }
            }).collect();
            let tail = HirExpr { kind: HirExprKind::Tuple(awaited), ty: Ty::Error, span };
            HirExpr { kind: HirExprKind::Block { stmts, tail: Box::new(tail) }, ty: Ty::Error, span }
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

        Expr::While { cond, body, .. } => {
            let cond_hir = lower_expr(cond, cx);
            let body_hir = lower_expr(body, cx);
            HirExpr {
                kind: HirExprKind::While {
                    cond: Box::new(cond_hir),
                    body: Box::new(body_hir),
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
                        let ty = init.ty.clone(); // propagate init type (e.g. statemachine return type)
                        hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty, init });
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
                let ty = init.ty.clone();
                let local = cx.define_local(&name.node);
                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty, init });
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
            let variant = path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            // Build a fully qualified tag name so MIR can emit `TypeName_VariantName`.
            let name = if let Some(parent) = cx.variant_to_type.get(&variant) {
                format!("{}__{}", parent, variant)
            } else {
                variant
            };
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

// ------------------------------------------------------------------ //
// AST type expression → Ty (lightweight conversion for HIR param types)
// ------------------------------------------------------------------ //

fn ast_ty_to_ty(te: &certo_ast::types::TypeExpr) -> Ty {
    use certo_ast::types::TypeExpr;
    match te {
        TypeExpr::Named { path, args, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            let targs: Vec<Ty> = args.iter().map(|a| ast_ty_to_ty(&a.node)).collect();
            match name {
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
                "List"    => Ty::List(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Option"  => Ty::Option(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Result"  => {
                    let mut it = targs.into_iter();
                    Ty::Result(Box::new(it.next().unwrap_or(Ty::Error)), Box::new(it.next().unwrap_or(Ty::Error)))
                }
                "Map"     => {
                    let mut it = targs.into_iter();
                    Ty::Map(Box::new(it.next().unwrap_or(Ty::Error)), Box::new(it.next().unwrap_or(Ty::Error)))
                }
                other     => Ty::Named { name: other.to_string(), args: targs },
            }
        }
        TypeExpr::Option { inner, .. } => Ty::Option(Box::new(ast_ty_to_ty(&inner.node))),
        TypeExpr::Tuple { elements, .. } => Ty::Tuple(elements.iter().map(|e| ast_ty_to_ty(&e.node)).collect()),
        TypeExpr::Fn { params, ret, .. } => Ty::Fn {
            params: params.iter().map(|p| ast_ty_to_ty(&p.node)).collect(),
            ret:    Box::new(ast_ty_to_ty(&ret.node)),
        },
        _ => Ty::Error,
    }
}
