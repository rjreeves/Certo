//! Text-by-content collections — BACKLOG item 392.
//!
//! The list/map runtime is untyped (`void*` elements), so it compares elements by pointer — right
//! for `Int`/`Bool`/`Float` (stored in the word), wrong for `Text`, where two equal strings are
//! different pointers. The compiler knows the element/key type at each call, so a call over
//! `Text` is renamed to a runtime variant that compares by content.

use certo_typeck::Ty;

use crate::generic_kind::visit_mut;
use crate::hir::*;

fn is_text(t: &Ty) -> bool { matches!(t, Ty::Text | Ty::BoundedText(_)) }

fn list_of_text(t: &Ty) -> bool { matches!(t, Ty::List(inner) if is_text(inner)) }

const EQ_LOCAL_BASE: LocalId = 0x7ffc_0000;

/// Elements the untyped runtime already compares correctly by their word (and `Text`, which has
/// its own variants above).
fn word_comparable(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt | Ty::Bool | Ty::Char | Ty::Unit | Ty::Text | Ty::BoundedText(_))
}

fn concrete(t: &Ty) -> bool {
    let s = format!("{t:?}");
    !(s.contains("Var(") || s.contains("Error"))
}

/// BACKLOG item 391 — `List.contains(xs, x)` over Float, Decimal, records and sum types becomes
/// `{ val needle = x  List.any(xs, (e) => e == needle) }`, which uses the language's own `==`
/// for the element type.
fn contains_by_eq(e: &mut HirExpr, next: &mut u32) {
    let span = e.span;
    let HirExprKind::Call { func, args } = &mut e.kind else { return };
    if !matches!(&func.kind, HirExprKind::Global(n) if n == "List.contains") || args.len() != 2 { return; }
    let Ty::List(elem) = &args[0].ty else { return };
    let elem = (**elem).clone();
    if word_comparable(&elem) || !concrete(&elem) { return; }
    let needle = EQ_LOCAL_BASE + *next; let el = EQ_LOCAL_BASE + *next + 1; *next += 2;
    let item = args.pop().unwrap();
    let list = args.pop().unwrap();
    let local = |id, ty: &Ty| HirExpr { kind: HirExprKind::Local(id), ty: ty.clone(), span };
    let body = HirExpr {
        kind: HirExprKind::BinOp { op: BinOp::Eq, lhs: Box::new(local(el, &elem)), rhs: Box::new(local(needle, &elem)) },
        ty: Ty::Bool, span,
    };
    let lambda = HirExpr {
        kind: HirExprKind::Lambda {
            params: vec![HirParam { local: el, name: "__e".into(), ty: elem.clone(), span }],
            body: Box::new(body),
            captures: vec![needle],
            ret_hint: Ty::Error,
        },
        ty: Ty::Fn { params: vec![elem.clone()], ret: Box::new(Ty::Bool) },
        span,
    };
    let any = HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(HirExpr { kind: HirExprKind::Global("List.any".into()), ty: Ty::Error, span }),
            args: vec![list, lambda],
        },
        ty: Ty::Bool, span,
    };
    e.kind = HirExprKind::Block {
        stmts: vec![HirStmt::Let { local: needle, name: "__needle".into(), ty: elem, init: item }],
        tail: Box::new(any),
    };
}

/// `List.distinct` over Decimal, records and sum types:
/// `List.fold(xs, [], (acc, e) => if List.any(acc, (y) => y == e) then acc else List.push(acc, e))`.
fn distinct_by_eq(e: &mut HirExpr, next: &mut u32) {
    let span = e.span;
    let HirExprKind::Call { func, args } = &mut e.kind else { return };
    if !matches!(&func.kind, HirExprKind::Global(n) if n == "List.distinct") || args.len() != 1 { return; }
    let Ty::List(elem) = &args[0].ty else { return };
    let elem = (**elem).clone();
    if word_comparable(&elem) || matches!(elem, Ty::Float | Ty::Float32) || !concrete(&elem) { return; }
    let (acc, el, y) = (EQ_LOCAL_BASE + *next, EQ_LOCAL_BASE + *next + 1, EQ_LOCAL_BASE + *next + 2);
    *next += 3;
    let list_ty = Ty::List(Box::new(elem.clone()));
    let local = |id, ty: &Ty| HirExpr { kind: HirExprKind::Local(id), ty: ty.clone(), span };
    let global = |n: &str| Box::new(HirExpr { kind: HirExprKind::Global(n.into()), ty: Ty::Error, span });
    let same = HirExpr {
        kind: HirExprKind::BinOp { op: BinOp::Eq, lhs: Box::new(local(y, &elem)), rhs: Box::new(local(el, &elem)) },
        ty: Ty::Bool, span,
    };
    let inner = HirExpr {
        kind: HirExprKind::Lambda {
            params: vec![HirParam { local: y, name: "__y".into(), ty: elem.clone(), span }],
            body: Box::new(same), captures: vec![el], ret_hint: Ty::Error,
        },
        ty: Ty::Fn { params: vec![elem.clone()], ret: Box::new(Ty::Bool) }, span,
    };
    let seen = HirExpr {
        kind: HirExprKind::Call { func: global("List.any"), args: vec![local(acc, &list_ty), inner] },
        ty: Ty::Bool, span,
    };
    let push = HirExpr {
        kind: HirExprKind::Call { func: global("List.push"), args: vec![local(acc, &list_ty), local(el, &elem)] },
        ty: list_ty.clone(), span,
    };
    let step = HirExpr {
        kind: HirExprKind::If { cond: Box::new(seen), then_expr: Box::new(local(acc, &list_ty)), else_expr: Box::new(push) },
        ty: list_ty.clone(), span,
    };
    let outer = HirExpr {
        kind: HirExprKind::Lambda {
            params: vec![
                HirParam { local: acc, name: "__acc".into(), ty: list_ty.clone(), span },
                HirParam { local: el, name: "__e".into(), ty: elem.clone(), span },
            ],
            body: Box::new(step), captures: vec![], ret_hint: Ty::Error,
        },
        ty: Ty::Fn { params: vec![list_ty.clone(), elem.clone()], ret: Box::new(list_ty.clone()) }, span,
    };
    let list = args.pop().unwrap();
    let empty = HirExpr { kind: HirExprKind::List(vec![]), ty: list_ty.clone(), span };
    e.kind = HirExprKind::Call { func: global("List.fold"), args: vec![list, empty, outer] };
}

pub fn rewrite_text_calls(items: &mut [HirItem]) {
    let mut next = 0u32;
    for it in items.iter_mut() {
        let body = match it {
            HirItem::Fn(f) => f.body.as_mut(),
            HirItem::Const(c) => Some(&mut c.value),
        };
        let Some(body) = body else { continue };
        visit_mut(body, &mut |e| {
            contains_by_eq(e, &mut next);
            distinct_by_eq(e, &mut next);
            let call_ty = e.ty.clone();
            let HirExprKind::Call { func, args } = &mut e.kind else { return };
            let HirExprKind::Global(name) = &mut func.kind else { return };
            let renamed = match name.as_str() {
                "List.contains" | "List.distinct" if args.first().is_some_and(|a| list_of_text(&a.ty)) =>
                    Some(format!("{name}Text")),
                "Map.insert" if args.get(1).is_some_and(|a| is_text(&a.ty)) => Some("Map.insertText".to_string()),
                "List.groupBy" if matches!(&call_ty, Ty::Map(k, _) if is_text(k)) => Some("List.groupByText".to_string()),
                _ => None,
            };
            if let Some(n) = renamed { *name = n; }
        });
    }
}
