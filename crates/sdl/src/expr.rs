//! Expression type checking for defaults and constraints. Produces the
//! paren-free `ExprIR` together with its type. Returns `None` after
//! reporting an error so callers don't emit cascading diagnostics.

use crate::ast::*;
use crate::ir::*;
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use std::collections::{HashMap, HashSet};

pub type Typed = (ExprIR, TypeIR);

pub struct ExprCx<'a> {
    pub enums: &'a HashMap<String, Vec<String>>,
    /// Columns visible to bare identifiers (constraints). `None` in defaults.
    pub columns: Option<&'a HashMap<String, TypeIR>>,
    /// Declared sequences, for `nextval(name)`.
    pub sequences: &'a HashSet<String>,
    pub diags: &'a mut Vec<Diagnostic>,
}

fn builtin(b: Builtin) -> TypeIR { TypeIR::Builtin(b) }

pub fn is_text(t: &TypeIR) -> bool {
    matches!(t, TypeIR::Builtin(Builtin::Text | Builtin::Varchar(_) | Builtin::Char(_)))
}

/// Numeric promotion order; bounded and unbounded decimals share a rank.
pub fn rank(t: &TypeIR) -> Option<u8> {
    match t {
        TypeIR::Builtin(Builtin::SmallInt) => Some(0),
        TypeIR::Builtin(Builtin::Int) => Some(1),
        TypeIR::Builtin(Builtin::BigInt) => Some(2),
        TypeIR::Builtin(Builtin::Decimal | Builtin::Numeric(..)) => Some(3),
        TypeIR::Builtin(Builtin::Real) => Some(4),
        TypeIR::Builtin(Builtin::Float) => Some(5),
        _ => None,
    }
}

pub fn from_rank(r: u8) -> TypeIR {
    builtin(match r {
        0 => Builtin::SmallInt,
        1 => Builtin::Int,
        2 => Builtin::BigInt,
        3 => Builtin::Decimal,
        4 => Builtin::Real,
        _ => Builtin::Float,
    })
}

pub fn ordered(t: &TypeIR) -> bool {
    rank(t).is_some()
        || is_text(t)
        || matches!(t, TypeIR::Builtin(Builtin::Timestamp | Builtin::TimestampNaive | Builtin::Date))
}

/// date / timestamp / timestamp_naive convert implicitly into one another in
/// PostgreSQL (`now()` is a valid default for a plain timestamp column).
pub fn is_time(t: &TypeIR) -> bool {
    matches!(t, TypeIR::Builtin(Builtin::Timestamp | Builtin::TimestampNaive | Builtin::Date))
}

/// Can values of these two types be compared or mixed?
pub fn compatible(a: &TypeIR, b: &TypeIR) -> bool {
    a == b
        || (rank(a).is_some() && rank(b).is_some())
        || (is_text(a) && is_text(b))
        || (is_time(a) && is_time(b))
}

pub fn describe(t: &TypeIR) -> String {
    match t {
        TypeIR::Builtin(b) => b.sdl_name(),
        TypeIR::Enum(n) | TypeIR::Composite(n) => n.clone(),
    }
}

impl ExprCx<'_> {
    fn err(&mut self, code: &str, msg: String, span: Span) {
        self.diags.push(Diagnostic::error(code, msg).with_span(span));
    }

    fn bare_unresolved(&self, e: &Expr) -> bool {
        matches!(e, Expr::Ident(id) if !self.columns.is_some_and(|c| c.contains_key(&id.name)))
    }

    /// `hint` lets a bare identifier resolve as an enum variant of that enum.
    pub fn check(&mut self, e: &Expr, hint: Option<&TypeIR>) -> Option<Typed> {
        match e {
            Expr::Literal(l, _) => Some(match l {
                Literal::Number(n) => (ExprIR::Number { value: *n }, builtin(Builtin::Int)),
                Literal::Decimal(s) => (ExprIR::Decimal { value: s.clone() }, builtin(Builtin::Decimal)),
                Literal::String(s) => (ExprIR::String { value: s.clone() }, builtin(Builtin::Text)),
                Literal::Bool(b) => (ExprIR::Bool { value: *b }, builtin(Builtin::Bool)),
            }),
            Expr::Paren(inner, _) => self.check(inner, hint),
            Expr::Ident(id) => {
                if let Some(ty) = self.columns.and_then(|c| c.get(&id.name)) {
                    return Some((ExprIR::Column { name: id.name.clone() }, ty.clone()));
                }
                if let Some(TypeIR::Enum(en)) = hint
                    && self.enums.get(en).is_some_and(|v| v.contains(&id.name))
                {
                    return Some((
                        ExprIR::EnumVariant { enum_name: en.clone(), variant: id.name.clone() },
                        TypeIR::Enum(en.clone()),
                    ));
                }
                self.err("SDL210", format!("unknown identifier `{}`", id.name), id.span);
                None
            }
            Expr::Call { func, args, span } => self.call(func, args, *span),
            Expr::Binary { op, lhs, rhs, span } => self.binary(*op, lhs, rhs, *span),
            Expr::Not(inner, span) => {
                let (e, t) = self.check(inner, None)?;
                if t != builtin(Builtin::Bool) {
                    self.err("SDL211", format!("`not` requires a boolean, found {}", describe(&t)), *span);
                    return None;
                }
                Some((ExprIR::Not { expr: Box::new(e) }, builtin(Builtin::Bool)))
            }
            Expr::IsNull { expr, negated, .. } => {
                let (e, _) = self.check(expr, None)?;
                Some((ExprIR::IsNull { expr: Box::new(e), negated: *negated }, builtin(Builtin::Bool)))
            }
            Expr::In { expr, list, negated, span } => {
                let (e, lt) = self.check(expr, None)?;
                let mut items = Vec::new();
                let mut failed = false;
                for item in list {
                    match self.check(item, Some(&lt)) {
                        Some((ie, it)) if compatible(&lt, &it) => items.push(ie),
                        Some((_, it)) => {
                            self.err("SDL211", format!("cannot compare {} with {}", describe(&lt), describe(&it)), item.span());
                            failed = true;
                        }
                        None => failed = true,
                    }
                }
                if failed {
                    return None;
                }
                let _ = span;
                Some((ExprIR::In { expr: Box::new(e), list: items, negated: *negated }, builtin(Builtin::Bool)))
            }
        }
    }

    fn call(&mut self, func: &Ident, args: &[Expr], span: Span) -> Option<Typed> {
        // `nextval(seq)` takes a sequence NAME, not a value
        if func.name == "nextval" {
            return match args {
                [Expr::Ident(id)] if self.sequences.contains(&id.name) => {
                    Some((ExprIR::NextVal { sequence: id.name.clone() }, builtin(Builtin::BigInt)))
                }
                _ => {
                    self.err("SDL209", "`nextval` takes the name of one declared sequence, e.g. `nextval(order_seq)`".into(), span);
                    None
                }
            };
        }
        let mut typed = Vec::new();
        let mut failed = false;
        for a in args {
            match self.check(a, None) {
                Some(t) => typed.push(t),
                None => failed = true,
            }
        }
        // (min arity, max arity)
        let arity = match func.name.as_str() {
            "now" | "gen_uuid" | "today" => (0, 0),
            "lower" | "upper" | "length" | "abs" | "is_null" | "trim" | "round" => (1, 1),
            "nullif" => (2, 2),
            "coalesce" => (1, usize::MAX),
            other => {
                self.err("SDL209", format!("unknown function `{other}`"), func.span);
                return None;
            }
        };
        if args.len() < arity.0 || args.len() > arity.1 {
            self.err(
                "SDL209",
                format!("function `{}` called with {} argument(s)", func.name, args.len()),
                span,
            );
            return None;
        }
        if failed { return None; }
        let name = func.name.as_str();
        let text_arg = |cx: &mut Self, typed: &[Typed]| -> bool {
            if is_text(&typed[0].1) { return true; }
            cx.err("SDL209", format!("`{name}` expects text, found {}", describe(&typed[0].1)), args[0].span());
            false
        };
        let ret = match name {
            // `is_null(x)` is the function spelling of `x is null`
            "is_null" => {
                let (e, _) = typed.remove(0);
                return Some((ExprIR::IsNull { expr: Box::new(e), negated: false }, builtin(Builtin::Bool)));
            }
            "now" => builtin(Builtin::Timestamp),
            "today" => builtin(Builtin::Date),
            "gen_uuid" => builtin(Builtin::Uuid),
            "lower" | "upper" | "trim" => {
                if !text_arg(self, &typed) { return None; }
                builtin(Builtin::Text)
            }
            "length" => {
                if !text_arg(self, &typed) { return None; }
                builtin(Builtin::Int)
            }
            "abs" | "round" => {
                if rank(&typed[0].1).is_none() {
                    self.err("SDL209", format!("`{name}` expects a number, found {}", describe(&typed[0].1)), args[0].span());
                    return None;
                }
                typed[0].1.clone()
            }
            _ => {
                // coalesce / nullif: every argument must be compatible with the first
                let first = typed[0].1.clone();
                if let Some((i, _)) = typed.iter().enumerate().find(|(_, t)| !compatible(&first, &t.1)) {
                    self.err("SDL209", format!("`{name}` arguments must all have the same type"), args[i].span());
                    return None;
                }
                first
            }
        };
        Some((
            ExprIR::Call { func: func.name.clone(), args: typed.into_iter().map(|t| t.0).collect() },
            ret,
        ))
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, span: Span) -> Option<Typed> {
        use BinaryOp::*;
        let cmp = matches!(op, Eq | Ne | Lt | Le | Gt | Ge);
        let (l, r) = if cmp && self.bare_unresolved(lhs) && !self.bare_unresolved(rhs) {
            // `admin == role`: type the right side first so `admin` can resolve.
            let r = self.check(rhs, None);
            let l = self.check(lhs, r.as_ref().map(|t| &t.1));
            (l, r)
        } else {
            let l = self.check(lhs, None);
            let r = self.check(rhs, l.as_ref().map(|t| &t.1).filter(|_| cmp));
            (l, r)
        };
        let ((le, lt), (re, rt)) = (l?, r?);

        let ty = match op {
            And | Or => {
                if lt != builtin(Builtin::Bool) || rt != builtin(Builtin::Bool) {
                    self.err("SDL211", "`and` / `or` require boolean operands".into(), span);
                    return None;
                }
                builtin(Builtin::Bool)
            }
            Add | Sub | Mul | Div => match (rank(&lt), rank(&rt)) {
                (Some(a), Some(b)) => from_rank(a.max(b)),
                _ => {
                    self.err(
                        "SDL211",
                        format!("arithmetic requires numbers, found {} and {}", describe(&lt), describe(&rt)),
                        span,
                    );
                    return None;
                }
            },
            Eq | Ne | Lt | Le | Gt | Ge => {
                let order_ok = matches!(op, Eq | Ne) || ordered(&lt);
                if !compatible(&lt, &rt) || !order_ok {
                    self.err(
                        "SDL211",
                        format!("cannot compare {} with {}", describe(&lt), describe(&rt)),
                        span,
                    );
                    return None;
                }
                builtin(Builtin::Bool)
            }
        };
        Some((ExprIR::Binary { op, lhs: Box::new(le), rhs: Box::new(re) }, ty))
    }
}

/// Can a default of type `got` (from `expr`) be stored in a column of `want`?
pub fn default_assignable(want: &TypeIR, got: &TypeIR, expr: &Expr) -> bool {
    // a decimal literal only fits a column that can hold a fraction
    if matches!(expr, Expr::Literal(Literal::Decimal(_), _)) {
        return matches!(
            want,
            TypeIR::Builtin(Builtin::Decimal | Builtin::Numeric(..) | Builtin::Real | Builtin::Float)
        );
    }
    if compatible(want, got) {
        return true;
    }
    // String literals may initialise these non-text columns.
    matches!(expr, Expr::Literal(Literal::String(_), _))
        && matches!(
            want,
            TypeIR::Builtin(
                Builtin::Uuid | Builtin::Timestamp | Builtin::TimestampNaive | Builtin::Date | Builtin::Json
            )
        )
}
