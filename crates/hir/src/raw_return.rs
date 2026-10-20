//! Return-provenance classification for user-defined generic functions —
//! BACKLOG items 350 and 340.
//!
//! A generic function's bare-`T` return crosses an erasure boundary as a
//! `void*`, but this compiler has two incompatible representations for such
//! an erased value, and the call site has to undo the *same* one the callee
//! produced:
//!
//! * **Boxed** — a `BoxSome`-style heap cell holding the value (what a
//!   generic function's own parameter already is when the caller passes a
//!   concrete value in, and what `Box.unwrap<T>(...)` and friends hand back).
//!   Undone with `Rvalue::UnboxSome` (always dereferences).
//! * **Raw element** — the bare word a container stores for an element:
//!   a scalar bit-packed straight into the slot, a `Text` pointer as-is, a
//!   struct as a pointer to a malloc'd copy (`Rvalue::Box`/`box_value`).
//!   This is what `List.getOrPanic` returns and what a `Some(v)` pattern
//!   extracts from an `Option<T>` cell (`List.first`, `List.get`, ...).
//!   Undone with `Rvalue::Unbox` (driven by the concrete type).
//!
//! MIR used to assume Boxed for *every* generic return, so
//! `fn head<T>(l: List<T>): T = match l.first() { Some(v) => v ... }` (the
//! spec's own §5.6 example) and `fn firstOf<T>(xs: List<T>): T =
//! List.getOrPanic(xs, 0)` made the call site dereference a bit-packed
//! scalar / a `Text` pointer as if it were a heap cell. This pass walks each
//! generic function's result expression, decides whether *every* path
//! produces a raw-element word, and if so marks the function in
//! `fn_ret_tys` by giving its erased return the distinguished type variable
//! [`RAW_ELEM_RET_VAR`] instead of `Ty::Var(0)`; MIR's call-site unboxing
//! picks `Unbox` for those. A function whose paths disagree (or that it
//! cannot analyse) is left exactly as before.

use std::collections::{HashMap, HashSet};

use certo_typeck::Ty;

use crate::hir::*;

/// The type variable id marking a generic function whose erased return is a
/// raw container-element word (see the module docs). Chosen far outside the
/// ids ordinary generalisation hands out, which start at 0.
pub const RAW_ELEM_RET_VAR: u32 = 0x7fff_0001;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    /// A raw container-element word.
    Raw,
    /// A `BoxSome`-style heap cell.
    Boxed,
    /// Diverges (`panic`) — contributes nothing to the join.
    Never,
    /// Can't tell.
    Unknown,
}

fn join(a: Origin, b: Origin) -> Origin {
    match (a, b) {
        (Origin::Never, x) | (x, Origin::Never) => x,
        (x, y) if x == y => x,
        _ => Origin::Unknown,
    }
}

struct Walker<'a> {
    locals: HashMap<LocalId, Origin>,
    raw_fns: &'a HashSet<String>,
    erased_fns: &'a HashSet<String>,
}

impl Walker<'_> {
    fn origin(&mut self, e: &HirExpr) -> Origin {
        match &e.kind {
            HirExprKind::Local(id) => self.locals.get(id).copied().unwrap_or(Origin::Unknown),
            HirExprKind::Call { func, .. } => match &func.kind {
                HirExprKind::Global(name) => {
                    if name == "List.getOrPanic" || self.raw_fns.contains(name) {
                        Origin::Raw
                    } else if name == "panic" || name == "unreachable" {
                        Origin::Never
                    } else if self.erased_fns.contains(name) {
                        Origin::Boxed
                    } else {
                        Origin::Unknown
                    }
                }
                _ => Origin::Unknown,
            },
            HirExprKind::If { then_expr, else_expr, .. } => {
                let t = self.origin(then_expr);
                let f = self.origin(else_expr);
                join(t, f)
            }
            HirExprKind::Match { scrutinee, arms } => {
                let mut acc = Origin::Never;
                for arm in arms {
                    self.bind_pattern(&arm.pat, &scrutinee.ty);
                    let o = self.origin(&arm.body);
                    acc = join(acc, o);
                }
                acc
            }
            HirExprKind::Block { stmts, tail } => {
                for s in stmts {
                    match s {
                        HirStmt::Let { local, init, .. } => {
                            let o = self.origin(init);
                            self.locals.insert(*local, o);
                        }
                        HirStmt::Assign { local, .. } => {
                            self.locals.insert(*local, Origin::Unknown);
                        }
                        _ => {}
                    }
                }
                self.origin(tail)
            }
            HirExprKind::Unsafe(inner) => self.origin(inner),
            _ => Origin::Unknown,
        }
    }

    /// Record the provenance of every local a pattern binds. Only the plain
    /// `Some(v)` extraction from an `Option<T>` whose payload is itself an
    /// erased type variable is known to yield a raw element word; any other
    /// binding is `Unknown` (never claimed raw).
    fn bind_pattern(&mut self, pat: &HirPat, scrut_ty: &Ty) {
        match pat {
            HirPat::Constructor { name, fields, .. }
                if name == "Some"
                    && matches!(scrut_ty, Ty::Option(inner) if matches!(**inner, Ty::Var(_)))
                    && matches!(fields.as_slice(), [HirPat::Bind { .. }]) =>
            {
                if let [HirPat::Bind { local, .. }] = fields.as_slice() {
                    self.locals.insert(*local, Origin::Raw);
                }
            }
            other => mark_unknown(other, &mut self.locals),
        }
    }
}

fn mark_unknown(pat: &HirPat, locals: &mut HashMap<LocalId, Origin>) {
    match pat {
        HirPat::Bind { local, .. } => { locals.insert(*local, Origin::Unknown); }
        HirPat::Tuple(ps) => ps.iter().for_each(|p| mark_unknown(p, locals)),
        HirPat::Constructor { fields, .. } => fields.iter().for_each(|p| mark_unknown(p, locals)),
        HirPat::Record { fields, .. } => fields.iter().for_each(|p| mark_unknown(p, locals)),
        HirPat::Or(a, b) => { mark_unknown(a, locals); mark_unknown(b, locals); }
        _ => {}
    }
}

/// Does any expression reachable from `e` (outside nested lambdas) early-
/// return? Such a function has an exit path this pass doesn't follow, so it
/// is never classified.
fn contains_return(e: &HirExpr) -> bool {
    match &e.kind {
        HirExprKind::Return(_) => true,
        HirExprKind::Call { func, args } => contains_return(func) || args.iter().any(contains_return),
        HirExprKind::BinOp { lhs, rhs, .. } => contains_return(lhs) || contains_return(rhs),
        HirExprKind::UnOp { arg, .. } => contains_return(arg),
        HirExprKind::Field { base, .. } => contains_return(base),
        HirExprKind::Record { fields, .. } => fields.iter().any(|(_, v)| contains_return(v)),
        HirExprKind::Tuple(xs) | HirExprKind::List(xs) => xs.iter().any(contains_return),
        HirExprKind::If { cond, then_expr, else_expr } =>
            contains_return(cond) || contains_return(then_expr) || contains_return(else_expr),
        HirExprKind::Block { stmts, tail } => {
            stmts.iter().any(|s| match s {
                HirStmt::Let { init, .. } => contains_return(init),
                HirStmt::Assign { value, .. } => contains_return(value),
                HirStmt::Expr(x) => contains_return(x),
                HirStmt::Defer { body } => contains_return(body),
            }) || contains_return(tail)
        }
        HirExprKind::Match { scrutinee, arms } =>
            contains_return(scrutinee)
                || arms.iter().any(|a| a.guard.as_ref().is_some_and(contains_return) || contains_return(&a.body)),
        HirExprKind::Try(x) | HirExprKind::Unsafe(x) | HirExprKind::Await(x) => contains_return(x),
        HirExprKind::For { iter, body, .. } => contains_return(iter) || contains_return(body),
        HirExprKind::While { cond, body } => contains_return(cond) || contains_return(body),
        HirExprKind::Spawn { args, .. } => args.iter().any(contains_return),
        _ => false,
    }
}

/// How many times `Local(id)` is the value of the expression (in result position).
fn tail_uses(e: &HirExpr, id: LocalId) -> usize {
    match &e.kind {
        HirExprKind::Local(l) => usize::from(*l == id),
        HirExprKind::If { then_expr, else_expr, .. } => tail_uses(then_expr, id) + tail_uses(else_expr, id),
        HirExprKind::Match { arms, .. } => arms.iter().map(|a| tail_uses(&a.body, id)).sum(),
        HirExprKind::Block { tail, .. } => tail_uses(tail, id),
        HirExprKind::Unsafe(inner) => tail_uses(inner, id),
        _ => 0,
    }
}

/// Every reference to `id` anywhere in `e` (counted on the debug text: `Local(7)` is only ever
/// printed for a reference, and over-counting can only make the caller more conservative).
fn all_uses(e: &HirExpr, id: LocalId) -> usize {
    format!("{:?}", e).matches(&format!("Local({id})")).count()
}

/// Classify every generic function whose declared return is an erased type
/// variable, rewriting its entry in `fn_ret_tys` to
/// `Ty::Var(RAW_ELEM_RET_VAR)` when all of its result paths yield a raw
/// container-element word. Iterated to a fixpoint so a generic that merely
/// forwards another raw-returning generic is classified too.
///
/// BACKLOG item 425 — a function whose result paths mix a raw element word with one of its own
/// bare-`T` parameters (`match xs.first() { Some(v) => v  None => d }`) is also classified raw
/// when each such parameter is used *only* as a result value: the caller is then told (by
/// `Ty::Var(RAW_ELEM_RET_VAR)` in that parameter's slot of `fn_param_tys`) to pass the raw
/// element word instead of a boxed cell, so every result path agrees.
pub fn classify_raw_returns(items: &[HirItem], fn_ret_tys: &mut HashMap<String, Ty>, fn_param_tys: &mut HashMap<String, Vec<Ty>>) {
    let erased_fns: HashSet<String> = fn_ret_tys.iter()
        .filter(|(_, t)| matches!(t, Ty::Var(_)))
        .map(|(n, _)| n.clone())
        .collect();
    if erased_fns.is_empty() { return; }

    let fns: Vec<&HirFn> = items.iter().filter_map(|it| match it {
        HirItem::Fn(f) if erased_fns.contains(&f.name) && f.body.is_some() => Some(f),
        _ => None,
    }).collect();

    let mut raw_fns: HashSet<String> = HashSet::new();
    let mut raw_params: HashMap<String, Vec<usize>> = HashMap::new();
    loop {
        let mut changed = false;
        for f in &fns {
            if raw_fns.contains(&f.name) { continue; }
            let body = f.body.as_ref().unwrap();
            if contains_return(body) { continue; }
            let run = |raw_idx: &[usize], raw_fns: &HashSet<String>| {
                let mut w = Walker { locals: HashMap::new(), raw_fns, erased_fns: &erased_fns };
                for (i, p) in f.params.iter().enumerate() {
                    if matches!(p.ty, Ty::Var(_)) {
                        w.locals.insert(p.local, if raw_idx.contains(&i) { Origin::Raw } else { Origin::Boxed });
                    }
                }
                w.origin(body)
            };
            let plain = run(&[], &raw_fns);
            if plain == Origin::Raw {
                raw_fns.insert(f.name.clone());
                changed = true;
                continue;
            }
            // Only a genuine mix is rescued; an all-boxed function keeps its boxed scheme.
            if plain != Origin::Unknown { continue; }
            let candidates: Vec<usize> = f.params.iter().enumerate()
                .filter(|(_, p)| matches!(p.ty, Ty::Var(_)))
                .filter(|(_, p)| { let t = tail_uses(body, p.local); t > 0 && t == all_uses(body, p.local) })
                .map(|(i, _)| i).collect();
            if !candidates.is_empty() && run(&candidates, &raw_fns) == Origin::Raw {
                raw_fns.insert(f.name.clone());
                raw_params.insert(f.name.clone(), candidates);
                changed = true;
            }
        }
        if !changed { break; }
    }

    for name in raw_fns {
        fn_ret_tys.insert(name, Ty::Var(RAW_ELEM_RET_VAR));
    }
    for (name, idxs) in raw_params {
        if let Some(tys) = fn_param_tys.get_mut(&name) {
            for i in idxs { if i < tys.len() { tys[i] = Ty::Var(RAW_ELEM_RET_VAR); } }
        }
    }
}
