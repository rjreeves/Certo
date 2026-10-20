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

pub fn rewrite_text_calls(items: &mut [HirItem]) {
    for it in items.iter_mut() {
        let body = match it {
            HirItem::Fn(f) => f.body.as_mut(),
            HirItem::Const(c) => Some(&mut c.value),
        };
        let Some(body) = body else { continue };
        visit_mut(body, &mut |e| {
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
