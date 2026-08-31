use certo_ast::span::Span;
use certo_ast::types::Effect;

#[derive(Debug, Clone)]
pub struct EffectError {
    pub kind: EffectErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EffectErrorKind {
    /// E0400 — function uses an effect it didn't declare.
    UndeclaredEffect {
        fn_name: String,
        effect:  Effect,
    },

    /// E0401 — pure function calls a non-pure function.
    ImpureCallInPure {
        caller: String,
        callee: String,
        effect: Effect,
    },

    /// E0402 — async function not declared [async].
    MissingAsyncAnnotation { fn_name: String },

    /// E0403 — `db.transaction` block used outside a [db.write] function.
    TransactionOutsideDbWrite { fn_name: String },

    /// E0404 — `unsafe` block used outside an [unsafe] function.
    UnsafeOutsideUnsafe { fn_name: String },

    /// E0405 — `[fallible]` declared on a function whose own return type
    /// isn't `Result<T, E>` (BACKLOG item 261, spec §4.6's own documented
    /// restriction).
    FallibleReturnMustBeResult { fn_name: String },
}

impl EffectError {
    pub fn message(&self) -> String {
        match &self.kind {
            EffectErrorKind::UndeclaredEffect { fn_name, effect } =>
                format!("E0400: function `{}` uses effect `{}` without declaring it", fn_name, effect_name(effect)),
            EffectErrorKind::ImpureCallInPure { caller, callee, effect } =>
                format!("E0401: pure function `{}` calls `{}` which requires `{}`", caller, callee, effect_name(effect)),
            EffectErrorKind::MissingAsyncAnnotation { fn_name } =>
                format!("E0402: function `{}` uses `await` but is not declared [async]", fn_name),
            EffectErrorKind::TransactionOutsideDbWrite { fn_name } =>
                format!("E0403: `db.transaction` in `{}` requires [db.write] annotation", fn_name),
            EffectErrorKind::UnsafeOutsideUnsafe { fn_name } =>
                format!("E0404: `unsafe` block in `{}` requires [unsafe] annotation", fn_name),
            EffectErrorKind::FallibleReturnMustBeResult { fn_name } =>
                format!("E0405: function `{}` is declared [fallible] but its return type is not `Result<T, E>`", fn_name),
        }
    }
}

pub fn effect_name(e: &Effect) -> &'static str {
    match e {
        Effect::Pure     => "pure",
        Effect::DbRead   => "db.read",
        Effect::DbWrite  => "db.write",
        Effect::Io       => "io",
        Effect::Async    => "async",
        Effect::Fallible => "fallible",
        Effect::Unsafe   => "unsafe",
    }
}
