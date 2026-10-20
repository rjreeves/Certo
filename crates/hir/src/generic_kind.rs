//! Hidden representation argument for generic functions — BACKLOG item 426.
//!
//! Inside a generic body `T` is erased, so `Some(raw)` cannot tell whether the raw container
//! element word it was handed needs a fresh Option cell (a scalar, a `Text`, a `Float`: the
//! word *is* the value) or already is one (a struct: the word is a pointer to the value). The
//! caller does know `T`. So a generic function that builds an Option from a raw element —
//! `List.map(xs, Some)` or `List.map(xs, x => Some(x))` over a `List<T>` — gets one extra
//! trailing `Int` parameter, `__kind`: the byte size of a non-struct `T`, or 0 for a
//! pointer-like one. The sites inside the body become `__certo_some_raw(x, __kind)`, and every
//! call of such a function passes the kind it computes from the concrete argument types.
//!
//! A generic function that calls one of these with a still-erased `T` forwards its own kind (and
//! so gets the parameter too). A call whose `T` cannot be determined outside any generic
//! function passes 0, and a flagged function used as a value (not called) is not supported.

use std::collections::{HashMap, HashSet};

use certo_typeck::Ty;

use crate::hir::*;

/// Fresh local ids, far above anything the lowering hands out.
const KIND_LOCAL_BASE: LocalId = 0x7ffe_0000;
const LAMBDA_LOCAL_BASE: LocalId = 0x7ffd_0000;

fn has_var(t: &Ty) -> bool {
    let s = format!("{t:?}");
    s.contains("Var(") || s.contains("Error")
}

fn size_kind(t: &Ty) -> i64 {
    if t.needs_heap_box() { return 0; }
    match t {
        Ty::Bool | Ty::Int8 => 1,
        Ty::Int16 => 2,
        Ty::Int32 | Ty::Char | Ty::Float32 => 4,
        _ => 8,
    }
}

/// Find what the (single) erased type parameter is, by walking a declared type against an
/// actual one. Returns the first fully concrete match.
fn find_param(decl: &Ty, actual: &Ty) -> Option<Ty> {
    match (decl, actual) {
        (Ty::Var(_), a) => (!has_var(a)).then(|| a.clone()),
        (Ty::List(d), Ty::List(a)) | (Ty::Option(d), Ty::Option(a)) => find_param(d, a),
        (Ty::Result(d1, d2), Ty::Result(a1, a2)) | (Ty::Map(d1, d2), Ty::Map(a1, a2)) =>
            find_param(d1, a1).or_else(|| find_param(d2, a2)),
        (Ty::Tuple(ds), Ty::Tuple(as_)) => ds.iter().zip(as_).find_map(|(d, a)| find_param(d, a)),
        (Ty::Fn { params: dp, ret: dr }, Ty::Fn { params: ap, ret: ar }) =>
            dp.iter().zip(ap).find_map(|(d, a)| find_param(d, a)).or_else(|| find_param(dr, ar)),
        _ => None,
    }
}

/// Apply `f` to every expression in `e`, children first (including lambda bodies).
pub(crate) fn visit_mut(e: &mut HirExpr, f: &mut dyn FnMut(&mut HirExpr)) {
    match &mut e.kind {
        HirExprKind::Call { func, args } => { visit_mut(func, f); args.iter_mut().for_each(|a| visit_mut(a, f)); }
        HirExprKind::BinOp { lhs, rhs, .. } => { visit_mut(lhs, f); visit_mut(rhs, f); }
        HirExprKind::UnOp { arg, .. } => visit_mut(arg, f),
        HirExprKind::Field { base, .. } => visit_mut(base, f),
        HirExprKind::Record { fields, .. } => fields.iter_mut().for_each(|(_, v)| visit_mut(v, f)),
        HirExprKind::Tuple(xs) | HirExprKind::List(xs) => xs.iter_mut().for_each(|x| visit_mut(x, f)),
        HirExprKind::If { cond, then_expr, else_expr } => { visit_mut(cond, f); visit_mut(then_expr, f); visit_mut(else_expr, f); }
        HirExprKind::Block { stmts, tail } => {
            for s in stmts {
                match s {
                    HirStmt::Let { init, .. } => visit_mut(init, f),
                    HirStmt::Assign { value, .. } => visit_mut(value, f),
                    HirStmt::Expr(x) => visit_mut(x, f),
                    HirStmt::Defer { body } => visit_mut(body, f),
                }
            }
            visit_mut(tail, f);
        }
        HirExprKind::Lambda { body, .. } => visit_mut(body, f),
        HirExprKind::Match { scrutinee, arms } => {
            visit_mut(scrutinee, f);
            for a in arms {
                if let Some(g) = &mut a.guard { visit_mut(g, f); }
                visit_mut(&mut a.body, f);
            }
        }
        HirExprKind::Try(x) | HirExprKind::Return(x) | HirExprKind::Unsafe(x) | HirExprKind::Await(x) => visit_mut(x, f),
        HirExprKind::For { iter, body, .. } => { visit_mut(iter, f); visit_mut(body, f); }
        HirExprKind::While { cond, body } => { visit_mut(cond, f); visit_mut(body, f); }
        HirExprKind::Spawn { args, .. } => args.iter_mut().for_each(|a| visit_mut(a, f)),
        HirExprKind::AwaitTimed { task, .. } | HirExprKind::JoinTimedCancel { task, .. } => visit_mut(task, f),
        _ => {}
    }
    f(e);
}

/// Is `e` a `List.map` call over a list of erased elements whose callback builds `Some` from the
/// element — `Some` itself, or `x => Some(x)`?
fn is_raw_some_map(e: &HirExpr) -> bool {
    let HirExprKind::Call { func, args } = &e.kind else { return false };
    if !matches!(&func.kind, HirExprKind::Global(n) if n == "List.map") || args.len() != 2 { return false; }
    if !matches!(&args[0].ty, Ty::List(inner) if matches!(**inner, Ty::Var(_))) { return false; }
    match &args[1].kind {
        HirExprKind::Global(n) => n == "Some",
        HirExprKind::Lambda { params, body, .. } => {
            let [p] = params.as_slice() else { return false };
            matches!(&body.kind, HirExprKind::Call { func, args: a }
                if matches!(&func.kind, HirExprKind::Global(n) if n == "Some")
                    && matches!(a.as_slice(), [x] if matches!(x.kind, HirExprKind::Local(l) if l == p.local)))
        }
        _ => false,
    }
}

/// The kind a call to `callee` passes, or `None` if its type parameter cannot be determined here.
fn call_kind(callee: &str, args: &[HirExpr], call_ty: &Ty, fn_param_tys: &HashMap<String, Vec<Ty>>, fn_ret_tys: &HashMap<String, Ty>) -> Option<i64> {
    let decl = fn_param_tys.get(callee)?;
    let found = decl.iter().zip(args).find_map(|(d, a)| find_param(d, &a.ty))
        .or_else(|| fn_ret_tys.get(callee).and_then(|r| find_param(r, call_ty)))?;
    Some(size_kind(&found))
}

fn is_generic(f: &HirFn) -> bool {
    f.params.iter().any(|p| has_var(&p.ty)) || has_var(&f.ret_ty)
}

pub fn add_kind_arguments(items: &mut [HirItem], fn_param_tys: &mut HashMap<String, Vec<Ty>>, fn_ret_tys: &HashMap<String, Ty>) {
    // Phase 1: functions with a raw-Some site.
    let mut flagged: HashSet<String> = HashSet::new();
    for it in items.iter() {
        if let HirItem::Fn(f) = it {
            let Some(body) = &f.body else { continue };
            let mut found = false;
            visit_mut(&mut body.clone(), &mut |e| { if is_raw_some_map(e) { found = true; } });
            if found && is_generic(f) { flagged.insert(f.name.clone()); }
        }
    }
    if flagged.is_empty() { return; }

    // Phase 2: a generic function that calls a flagged one with an undetermined `T` forwards it.
    loop {
        let mut grew = false;
        for it in items.iter() {
            let HirItem::Fn(f) = it else { continue };
            let Some(body) = &f.body else { continue };
            if flagged.contains(&f.name) || !is_generic(f) { continue; }
            let mut needs = false;
            visit_mut(&mut body.clone(), &mut |e| {
                if let HirExprKind::Call { func, args } = &e.kind {
                    if let HirExprKind::Global(n) = &func.kind {
                        if flagged.contains(n) && call_kind(n, args, &e.ty, fn_param_tys, fn_ret_tys).is_none() { needs = true; }
                    }
                }
            });
            if needs { flagged.insert(f.name.clone()); grew = true; }
        }
        if !grew { break; }
    }

    // Phase 3: rewrite. Give every flagged function its kind parameter first, so that the
    // call-site arguments below see the final signatures' original (pre-append) parameter lists.
    let original_params = fn_param_tys.clone();
    let mut kind_local: HashMap<String, LocalId> = HashMap::new();
    let mut n = 0u32;
    for it in items.iter_mut() {
        if let HirItem::Fn(f) = it {
            if !flagged.contains(&f.name) { continue; }
            let id = KIND_LOCAL_BASE + n; n += 1;
            kind_local.insert(f.name.clone(), id);
            f.params.push(HirParam { local: id, name: "__kind".into(), ty: Ty::Int, span: f.span });
        }
    }
    for name in &flagged {
        if let Some(tys) = fn_param_tys.get_mut(name) { tys.push(Ty::Int); }
    }

    let mut lambda_n = 0u32;
    for it in items.iter_mut() {
        let HirItem::Fn(f) = it else { continue };
        let own_kind = kind_local.get(&f.name).copied();
        let Some(body) = &mut f.body else { continue };
        let mut rewrite = |e: &mut HirExpr| {
            // Raw-Some sites: replace the callback with a lambda that builds the cell.
            if own_kind.is_some() && is_raw_some_map(e) {
                let kid = own_kind.unwrap();
                let HirExprKind::Call { args, .. } = &mut e.kind else { unreachable!() };
                let span = args[1].span;
                let x = LAMBDA_LOCAL_BASE + lambda_n; lambda_n += 1;
                let x_ref = HirExpr { kind: HirExprKind::Local(x), ty: Ty::Var(0), span };
                let k_ref = HirExpr { kind: HirExprKind::Local(kid), ty: Ty::Int, span };
                let call = HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr { kind: HirExprKind::Global("__certo_some_raw".into()), ty: Ty::Error, span }),
                        args: vec![x_ref, k_ref],
                    },
                    ty: Ty::Option(Box::new(Ty::Var(0))),
                    span,
                };
                let lam_ty = args[1].ty.clone();
                args[1] = HirExpr {
                    kind: HirExprKind::Lambda {
                        params: vec![HirParam { local: x, name: "__x".into(), ty: Ty::Var(0), span }],
                        body: Box::new(call),
                        captures: vec![kid],
                        ret_hint: Ty::Error,
                    },
                    ty: lam_ty,
                    span,
                };
            }
            // Calls of flagged functions: pass the kind.
            if let HirExprKind::Call { func, args } = &mut e.kind {
                if let HirExprKind::Global(callee) = &func.kind {
                    if flagged.contains(callee) {
                        let callee = callee.clone();
                        let k = call_kind(&callee, args, &e.ty, &original_params, fn_ret_tys);
                        let span = e.span;
                        let arg = match (k, own_kind) {
                            (Some(k), _) => HirExpr { kind: HirExprKind::Int(k), ty: Ty::Int, span },
                            (None, Some(kid)) => HirExpr { kind: HirExprKind::Local(kid), ty: Ty::Int, span },
                            (None, None) => HirExpr { kind: HirExprKind::Int(0), ty: Ty::Int, span },
                        };
                        args.push(arg);
                    }
                }
            }
        };
        visit_mut(body, &mut rewrite);
    }
}
