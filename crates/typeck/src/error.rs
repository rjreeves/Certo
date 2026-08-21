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

    /// E0217 — a record literal or `.with(...)` copy-update (BACKLOG item
    /// 151) tried to set a `computed` property (item 143) directly. A
    /// dedicated check, not a reuse of E0205/`UnknownField`: without it,
    /// direct literal construction surfaces this as a confusing raw C
    /// compile error two stages later (`field designator does not refer
    /// to any field`), and `.with(...)` doesn't surface it at all —
    /// `.with(...)`'s own field-name validation silently drops any name
    /// it doesn't recognize, confirmed directly (a real, separate,
    /// pre-existing gap in `.with(...)`'s own validation, not fixed here).
    ComputedFieldNotSettable { field: String, type_name: String },

    /// E0210 — a call to an `extern "C"` (FFI) function must be inside an `unsafe { }` block.
    FfiCallOutsideUnsafe { name: String },

    /// E0211 — an f-string interpolation `{ }` holds a value with no text representation.
    NonDisplayableInterpolation { ty: Ty },

    /// E0212 — an argument passed for a row-polymorphism-bounded type parameter
    /// (`R: { name: Text }`) is missing a required field.
    MissingRowField { ty: Ty, field: String, required: Ty },

    /// E0213 — a `match` doesn't cover every possible value of the scrutinee's type.
    NonExhaustiveMatch { ty: Ty, missing: Vec<String> },

    /// E0214 — a `type X = priv X(...)` smart-constructor's raw constructor
    /// was called outside an `impl X { ... }` block for the same type.
    PrivConstructorCall { type_name: String },

    /// E0215 — a value whose type structurally contains `Secret<_>` was
    /// passed to a logging/serialization sink (`println`/`print`/`eprint`/
    /// `Json.stringify`) — BACKLOG item 78. The spec (§13.2) suggests E0601
    /// for this, but that code was already in use (HIR lowering errors,
    /// item 136) before this was implemented; E0215 continues typeck's own
    /// E02xx sequence instead of colliding with an unrelated existing code.
    SecretInSensitiveContext { fn_name: String, ty: Ty },

    /// E0216 — an f-string with live interpolation was passed directly as
    /// the `sql` argument to a raw-SQL sink (`dbQuery`/`dbExec`/etc) —
    /// BACKLOG item 159. The spec (§13.1) claims this is architecturally
    /// impossible; this check is what makes that claim true for the
    /// direct, syntactically-visible case. Parameterize with `?`/`params`
    /// instead.
    SqlInjectionRisk { fn_name: String },

    /// E0703 — a validator rule's `else` clause produces a type other than
    /// the validator's own declared `errors` type (BACKLOG item 219).
    /// Deliberately its own variant, not a reuse of `Mismatch`/E0200 —
    /// matches this codebase's established convention for validator-
    /// specific diagnostics (E0700-E0702 in `crates/resolve`, E0708-E0710
    /// below), and names the offending rule for a more actionable message
    /// than a bare type mismatch would.
    ElseTypeMismatch { rule_name: String, expected: Ty, found: Ty },

    /// E0705 — a `loaded by` expression's own inferred type does not match
    /// the context field's declared type (BACKLOG item 220). Compares
    /// already-resolved types directly, mirroring `ElseTypeMismatch`/
    /// `TemporalNotDuration` above, rather than `ctx.unify`'s generic E0200.
    LoadedByTypeMismatch { field_name: String, expected: Ty, found: Ty },

    /// E0706 — a validator's `trigger on ... when <field> == <value>`
    /// condition names a field that doesn't exist on the entity type
    /// (BACKLOG item 220).
    TriggerFieldNotFound { field: String, entity: String },

    /// E0707 — a `trigger ... when <field> == <value>` condition's value is
    /// not a valid variant of the named field's own type (BACKLOG item 220).
    TriggerValueNotVariant { value: String, field: String, field_ty: Ty },

    /// E0708 — temporal declaration body does not resolve to Duration.
    TemporalNotDuration { found: Ty },

    /// E0709 — `.age` used on a non-Timestamp expression.
    AgeOnNonTimestamp { found: Ty },

    /// E0710 — `List.sortBy`/`minBy`/`maxBy`/`sumBy`'s key/numeric
    /// projection resolved to a type these functions don't support
    /// (BACKLOG item 162b). Not a generic type mismatch: the projected
    /// type is perfectly valid on its own, just not one of the types this
    /// codebase's `<`/`>`/`+` C operators are correct for — `Text`'s `<`
    /// is pointer comparison (not lexicographic) and `Decimal`'s `+`/`<`
    /// don't compile at all (struct operands). Confirmed real numeric/
    /// comparable set: `Int`/`Int8`/`Int16`/`Int32`/`UInt`/`Float`/`Float32`.
    UnsupportedKeyType { fn_name: String, found: Ty },
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
            TypeErrorKind::ComputedFieldNotSettable { field, type_name } =>
                format!("E0217: `{}` is a computed property of `{}` — it's derived, not stored, and can't be set directly", field, type_name),
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
            TypeErrorKind::PrivConstructorCall { type_name } =>
                format!("E0214: constructor `{type_name}` is private — call it only from within `impl {type_name} {{ ... }}`, e.g. via a validating `{type_name}.new` factory"),
            TypeErrorKind::SecretInSensitiveContext { fn_name, ty } =>
                format!("E0215: `{}` is not Loggable/Serializable — passed to `{}`", ty.display(), fn_name),
            TypeErrorKind::SqlInjectionRisk { fn_name } =>
                format!("E0216: an interpolated f-string was passed directly as the `sql` argument to `{}` — use `?` placeholders and pass values via `params` instead", fn_name),
            TypeErrorKind::ElseTypeMismatch { rule_name, expected, found } =>
                format!("E0703: rule `{}`'s `else` branch produces `{}`, but this validator declares `errors {}`",
                    rule_name, found.display(), expected.display()),
            TypeErrorKind::LoadedByTypeMismatch { field_name, expected, found } =>
                format!("E0705: context field `{}`'s `loaded by` expression produces `{}`, but the field is declared `{}`",
                    field_name, found.display(), expected.display()),
            TypeErrorKind::TriggerFieldNotFound { field, entity } =>
                format!("E0706: trigger condition references field `{}`, which does not exist on `{}`", field, entity),
            TypeErrorKind::TriggerValueNotVariant { value, field, field_ty } =>
                format!("E0707: `{}` is not a variant of `{}` (the type of field `{}`)", value, field_ty.display(), field),
            TypeErrorKind::TemporalNotDuration { found } =>
                format!("E0708: temporal body must resolve to Duration, found `{}`", found.display()),
            TypeErrorKind::AgeOnNonTimestamp { found } =>
                format!("E0709: `.age` is only valid on Timestamp or Timestamp?, found `{}`", found.display()),
            TypeErrorKind::UnsupportedKeyType { fn_name, found } =>
                format!("E0710: `{}`'s key/numeric projection resolved to `{}`, which isn't supported — only Int/Int8/Int16/Int32/UInt/Float/Float32 are", fn_name, found.display()),
        }
    }
}
