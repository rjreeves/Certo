use certo_ast::span::Span;
use crate::ty::{Ty, TyVar};

#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// E0200 — types don't match.
    Mismatch { expected: Ty, found: Ty },

    /// E0201 — cannot unify two rigid types (same as Mismatch but for named types).
    CannotUnify { left: Ty, right: Ty },

    /// E0202 — occurs check: type variable appears in its own inferred type.
    OccursCheck { var: TyVar, ty: Ty },

    /// E0203 — recursive function needs an explicit return type annotation.
    MissingAnnotation { name: String },

    /// E0204 — wrong number of arguments.
    ArityMismatch { expected: usize, found: usize },

    /// E0205 — field does not exist on this type.
    UnknownField { field: String, on: Ty },

    /// E0206 — name was not found in the type environment (should be caught by resolve, but belt+suspenders).
    UnboundName(String),

    /// E0210 — a call to an `extern "C"` (FFI) function must be inside an `unsafe { }` block.
    FfiCallOutsideUnsafe { name: String },

    /// E0211 — an f-string interpolation `{ }` holds a value with no text representation.
    NonDisplayableInterpolation { ty: Ty },

    /// E0212 — an argument passed for a row-polymorphism-bounded type parameter
    /// (`R: { name: Text }`) is missing a required field.
    MissingRowField { ty: Ty, field: String, required: Ty },

    /// E0213 — a `match` doesn't cover every possible value of the scrutinee's type.
    NonExhaustiveMatch { ty: Ty, missing: Vec<String> },

    /// E0708 — temporal declaration body does not resolve to Duration.
    TemporalNotDuration { found: Ty },

    /// E0709 — `.age` used on a non-Timestamp expression.
    AgeOnNonTimestamp { found: Ty },
}

impl TypeError {
    pub fn message(&self) -> String {
        match &self.kind {
            TypeErrorKind::Mismatch { expected, found } =>
                format!("E0200: type mismatch — expected `{}`, found `{}`", expected.display(), found.display()),
            TypeErrorKind::CannotUnify { left, right } =>
                format!("E0201: cannot unify `{}` with `{}`", left.display(), right.display()),
            TypeErrorKind::OccursCheck { var, ty } =>
                format!("E0202: occurs check failed — type variable ?t{} appears in `{}`", var, ty.display()),
            TypeErrorKind::MissingAnnotation { name } =>
                format!("E0203: recursive function `{}` requires an explicit return type annotation", name),
            TypeErrorKind::ArityMismatch { expected, found } =>
                format!("E0204: expected {} argument(s), found {}", expected, found),
            TypeErrorKind::UnknownField { field, on } =>
                format!("E0205: type `{}` has no field `{}`", on.display(), field),
            TypeErrorKind::UnboundName(name) =>
                format!("E0206: unbound name `{}`", name),
            TypeErrorKind::FfiCallOutsideUnsafe { name } =>
                format!("E0210: call to extern function `{}` must be inside an `unsafe {{ }}` block", name),
            TypeErrorKind::NonDisplayableInterpolation { ty } =>
                format!("E0211: cannot interpolate a value of type `{}` into a string — convert it first", ty.display()),
            TypeErrorKind::MissingRowField { ty, field, required } =>
                format!("E0212: `{}` does not satisfy the row bound — missing field `{}: {}`",
                    ty.display(), field, required.display()),
            TypeErrorKind::NonExhaustiveMatch { ty, missing } =>
                format!("E0213: match on `{}` is not exhaustive — missing: {}",
                    ty.display(), missing.join(", ")),
            TypeErrorKind::TemporalNotDuration { found } =>
                format!("E0708: temporal body must resolve to Duration, found `{}`", found.display()),
            TypeErrorKind::AgeOnNonTimestamp { found } =>
                format!("E0709: `.age` is only valid on Timestamp or Timestamp?, found `{}`", found.display()),
        }
    }
}
