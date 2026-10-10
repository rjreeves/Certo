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

    /// E0218 — a bare arithmetic operator (`+`/`-`/`*`/`/`/`%`/`**`) was used
    /// between two values of the same opaque date/time type (`Timestamp`,
    /// `DateTime`, `Date`) — BACKLOG item 214. These types deliberately have
    /// no operator overloading (this codebase's own established convention —
    /// see item 85's own note: named functions like `.diff(...)`/
    /// `.addDuration(...)` instead), but the generic same-type arithmetic
    /// rule previously let this through silently, unifying fine and
    /// producing a nonsensical same-typed result — `timestamp - timestamp`
    /// "worked" and produced another `Timestamp`, not a `Duration`, with no
    /// error at all. `Duration` itself is deliberately not included here —
    /// summing/differencing two spans is a real, sensible operation (unlike
    /// two absolute points in time), it's just also only exposed via
    /// `.add`/`.sub` rather than operators; nothing about that is wrong or
    /// silently nonsensical the way same-type `Timestamp`/`DateTime`/`Date`
    /// arithmetic is, so it's out of this check's scope.
    OpaqueTemporalArithmetic { ty: Ty, op: String },

    /// E0219 — `name = expr` where `name` was never declared with `var`
    /// (BACKLOG item 255). Found while scoping item 208's own parallel-task
    /// shared-mutable-state check: confirmed via a direct repro that
    /// reassigning a `val` or an ordinary function parameter was accepted
    /// silently everywhere in this compiler before now — no mutability
    /// tracking existed at all.
    AssignToImmutable { name: String },

    /// E0220 — inside a row-polymorphic generic function's own body, a field
    /// access on the row-bound param names a field its own bound never
    /// declared (BACKLOG item 257). Found while implementing item 200's
    /// codegen fix: `resolve_field_ty`'s `Ty::Var(_)` arm returned an
    /// unconstrained fresh var for *any* field access on the erased receiver,
    /// bound or not — so `fn f<R: {name: Text}>(r: R): Int = r.age` (`age`
    /// isn't in the bound) typechecked exactly as cleanly as a legitimate
    /// `r.name`, purely because the fresh var happened to unify against the
    /// function's own declared return type. No concrete type satisfying only
    /// `{name: Text}` is guaranteed to have an `age` field at all, so this
    /// was a real, silent soundness gap, not just a missing convenience check.
    FieldNotInRowBound { type_param: String, field: String, bound_fields: Vec<String> },

    /// E0221 — inside a trait-bounded generic function's own body, a
    /// method call on the bound param names a method none of its bound
    /// traits declare (BACKLOG item 309). Same soundness gap as
    /// `FieldNotInRowBound` just above, for trait bounds instead of row
    /// bounds: `resolve_field_ty`'s `Ty::Var(_)` arm had no table to
    /// consult for a trait-bounded param's own method calls at all before
    /// this item, so `fn f<T: Serializable>(v: T): Text = v.toJsonx()` (a
    /// typo — `Serializable` only declares `toJson`) typechecked exactly as
    /// cleanly as the real `v.toJson()`, only failing two build stages
    /// later at the C-compiler stage.
    MethodNotInTraitBound { type_param: String, method: String, bound_methods: Vec<String> },

    /// E0703 — a validator rule's `else` clause produces a type other than
    /// the validator's own declared `errors` type (BACKLOG item 219).
    /// Deliberately its own variant, not a reuse of `Mismatch`/E0200 —
    /// matches this codebase's established convention for validator-
    /// specific diagnostics (E0700-E0702 in `crates/resolve`, E0708-E0710
    /// below), and names the offending rule for a more actionable message
    /// than a bare type mismatch would.
    ElseTypeMismatch { rule_name: String, expected: Ty, found: Ty },

    /// E0704 — a named `constraint` referenced from a validator rule's
    /// `require`/`else` accesses a field on a base name that isn't the
    /// validator's own entity variable or one of its declared `context`
    /// fields (BACKLOG item 225, split out of item 220's own investigation).
    /// A constraint's body is deliberately never checked at its own
    /// declaration site (`Decl::Constraint` hoists its name as a bare
    /// `Ty::Bool` with no body inference — spec §16.8's own "deferred
    /// resolution") — this is the direct, non-expanded check that catches
    /// a scope violation at every validator that actually *uses* the
    /// constraint, reported at the `require`/`else` clause referencing it,
    /// not the constraint's own declaration (matching the spec's own
    /// E0704 example exactly).
    ConstraintFieldNotInScope { constraint_name: String, field_name: String },

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

    /// E0700 — cycle in a validator's `after` rule dependency graph
    /// (BACKLOG item 252). Ported directly from `crates/resolve` — fully
    /// implemented there, but `crates/resolve` is dead code for the real
    /// `certo` CLI binary (its only real consumer is the LSP), so this
    /// never actually ran for `certo check`/`certo build` before now.
    RuleCycle { validator: String, cycle: Vec<String> },

    /// E0701 — a rule's `after <name>` names a rule that doesn't exist in
    /// the same validator (BACKLOG item 252, ported from `crates/resolve`
    /// — see `RuleCycle`'s own doc comment for why it never ran).
    AfterRuleNotFound { rule_name: String, after_name: String },

    /// E0702 — a rule's `overrides <name>` names a rule that doesn't
    /// exist in the same validator (BACKLOG item 252, ported from
    /// `crates/resolve` — see `RuleCycle`'s own doc comment for why it
    /// never ran).
    OverridesRuleNotFound { rule_name: String, overrides_name: String },

    /// E0711 — `db.<table>.<method>(...)` (BACKLOG item 226) named a
    /// table/method combination with no matching generated accessor
    /// function in scope. Only the three sugared methods (`find`/`all`/
    /// `delete`, mapping to `{table}FindById`/`{table}FindAll`/
    /// `{table}DeleteById`) are recognized at all; an unrecognized method
    /// name reports the same way as a recognized method whose generated
    /// function is simply missing, since from this check's perspective
    /// both are "no such accessor exists" — the fix is the same either way
    /// (run `certo db pull`, or check the spelling).
    DbAccessorNotFound { table: String, method: String, expected_fn: String },

    /// E0222 — `guard cond else e` (BACKLOG item 342) returns `e` from the
    /// enclosing function or method, so it can only appear directly in a
    /// function/method body — not inside a lambda, a `spawn`/`parallel`/
    /// `withTimeout` block, or any other body with no function return type
    /// to return into (a test block, a computed property, a state-machine
    /// hook). Rejected rather than silently returning from the wrong
    /// function.
    GuardOutsideFunction,
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
            TypeErrorKind::OpaqueTemporalArithmetic { ty, op } =>
                format!("E0218: `{}` does not support the `{}` operator — it has no operator overloading, use a named function instead (e.g. `.diff(...)`, `.addDuration(...)`)", ty.display(), op),
            TypeErrorKind::AssignToImmutable { name } =>
                format!("E0219: cannot assign to `{}` — it was never declared with `var`", name),
            TypeErrorKind::FieldNotInRowBound { type_param, field, bound_fields } =>
                format!("E0220: `{}` is not declared in `{}`'s own row bound (only {} may be accessed)",
                    field, type_param,
                    bound_fields.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")),
            TypeErrorKind::MethodNotInTraitBound { type_param, method, bound_methods } =>
                format!("E0221: `{}` is not declared by `{}`'s own trait bound (only {} may be called)",
                    method, type_param,
                    bound_methods.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ")),
            TypeErrorKind::ElseTypeMismatch { rule_name, expected, found } =>
                format!("E0703: rule `{}`'s `else` branch produces `{}`, but this validator declares `errors {}`",
                    rule_name, found.display(), expected.display()),
            TypeErrorKind::ConstraintFieldNotInScope { constraint_name, field_name } =>
                format!("E0704: constraint `{}` references field `{}` not in validator context",
                    constraint_name, field_name),
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
                if fn_name == "List.sumBy" {
                    format!("E0710: `{}`'s key/numeric projection resolved to `{}`, which isn't supported — only Int/Int8/Int16/Int32/UInt/Float/Float32/Decimal are, or a struct type declaring both `{{Type}}.add(a, b): {{Type}}` and `{{Type}}.zero(): {{Type}}`", fn_name, found.display())
                } else {
                    format!("E0710: `{}`'s key/numeric projection resolved to `{}`, which isn't supported — only Int/Int8/Int16/Int32/UInt/Float/Float32/Decimal are", fn_name, found.display())
                },
            TypeErrorKind::RuleCycle { validator, cycle } =>
                format!("E0700: cycle in rule dependency graph in validator `{}`: {}", validator, cycle.join(" → ")),
            TypeErrorKind::AfterRuleNotFound { rule_name, after_name } =>
                format!("E0701: rule `{}` has `after {}` but `{}` does not exist in this validator", rule_name, after_name, after_name),
            TypeErrorKind::OverridesRuleNotFound { rule_name, overrides_name } =>
                format!("E0702: rule `{}` has `overrides {}` but `{}` does not exist in this validator", rule_name, overrides_name, overrides_name),
            TypeErrorKind::GuardOutsideFunction =>
                "E0222: `guard` returns from the enclosing function, so it can only be used directly in a function or method body — not inside a lambda, a `spawn`/`parallel`/`withTimeout` block, or a body with no function to return from".to_string(),
            TypeErrorKind::DbAccessorNotFound { table, method, expected_fn } =>
                format!("E0711: no `db.{}.{}` — did you run `certo db pull`? (expected a generated function named `{}`)", table, method, expected_fn),
        }
    }
}

// ------------------------------------------------------------------ //
// Warnings (BACKLOG item 230) — non-fatal diagnostics. Unlike `TypeError`
// above, a warning never aborts compilation on its own; `certo check`/
// `certo build` print every one collected here and continue. The first
// three codes (W0100-W0102, validator-specific — spec §16.3/§16.4/§16.6)
// are the initial use of this mechanism; anything future that wants a
// non-fatal diagnostic reuses this same `Warning`/`WarningKind` machinery
// rather than inventing its own.
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct Warning {
    pub kind: WarningKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum WarningKind {
    /// W0100 — a rule declares `overrides` against another rule, but
    /// neither rule has an explicit `priority` — the `overrides` clause
    /// alone determines evaluation order (mechanically unambiguous), but
    /// the spec's own recommended pattern (§16.4) pairs every `overrides`
    /// with an explicit `priority` to state that intent, so a rule pair
    /// relying on `overrides` alone gets flagged.
    AmbiguousOverridePriority { rule_name: String, overridden_name: String },

    /// W0101 — an overriding rule's own `require` condition is a
    /// compile-time-obvious tautology (a bare `true` literal), which makes
    /// the rule it `overrides` permanently unreachable — it is never
    /// evaluated, since the overriding rule's condition always passes.
    UnreachableOverriddenRule { overriding_name: String, overridden_name: String },

    /// W0102 — a validator's `context` field has no `loaded by` clause, so
    /// `{Validator}.validateWithDb` is not generated for it (mirrors
    /// `crates/codegen/src/emit_validator.rs`'s own `all_have_loaded_by`
    /// gate exactly — `validateWithDb` requires every context field to
    /// have `loaded by`, not just some).
    ContextFieldMissingLoadedBy { validator_name: String, field_name: String },

    /// W0103 — BACKLOG item 208. A name assigned to somewhere in this
    /// function (`Stmt::Assign`) is referenced by two or more of a
    /// `parallel { ... }` block's own sibling task expressions — those
    /// tasks run concurrently on real OS threads, so this is a genuine,
    /// unsynchronized shared-mutable-state risk, exactly what spec §7.2's
    /// own "compiler verifies tasks do not share mutable state" promises.
    ParallelSharedMutableState { name: String },
}

impl Warning {
    pub fn message(&self) -> String {
        match &self.kind {
            WarningKind::AmbiguousOverridePriority { rule_name, overridden_name } =>
                format!("W0100: rule `{}` overrides `{}` with no explicit `priority` on either rule — add `priority` to state evaluation order explicitly", rule_name, overridden_name),
            WarningKind::UnreachableOverriddenRule { overriding_name, overridden_name } =>
                format!("W0101: rule `{}`'s condition is always true — rule `{}` (which it overrides) is never evaluated", overriding_name, overridden_name),
            WarningKind::ContextFieldMissingLoadedBy { validator_name, field_name } =>
                format!("W0102: context field `{}` has no `loaded by` — `{}.validateWithDb` will not be generated", field_name, validator_name),
            WarningKind::ParallelSharedMutableState { name } =>
                format!("W0103: `{}` is referenced by more than one task in this `parallel {{ }}` block — these tasks run concurrently, so this is unsynchronized shared mutable state", name),
        }
    }
}
