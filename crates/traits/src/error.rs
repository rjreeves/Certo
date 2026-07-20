use certo_ast::span::Span;

#[derive(Debug, Clone)]
pub struct TraitError {
    pub kind: TraitErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TraitErrorKind {
    /// E0300 — impl declares a method not in the trait.
    UnknownMethod { trait_name: String, method: String },

    /// E0301 — impl is missing a required trait method.
    MissingMethod { trait_name: String, method: String },

    /// E0302 — impl method has wrong number of parameters.
    ParamCountMismatch { method: String, expected: usize, found: usize },

    /// E0303 — impl method return type doesn't match the trait signature.
    ReturnTypeMismatch { method: String, expected: String, found: String },

    /// E0304 — impl method param type doesn't match the trait signature.
    ParamTypeMismatch { method: String, param: usize, expected: String, found: String },

    /// E0305 — no impl of `trait_name` found for `ty`.
    UnsatisfiedBound { ty: String, trait_name: String },

    /// E0306 — duplicate impl of the same trait for the same type.
    DuplicateImpl { trait_name: String, ty: String },
}

impl TraitError {
    pub fn message(&self) -> String {
        match &self.kind {
            TraitErrorKind::UnknownMethod { trait_name, method } =>
                format!("E0300: method `{}` is not declared in trait `{}`", method, trait_name),
            TraitErrorKind::MissingMethod { trait_name, method } =>
                format!("E0301: impl is missing method `{}` required by trait `{}`", method, trait_name),
            TraitErrorKind::ParamCountMismatch { method, expected, found } =>
                format!("E0302: method `{}` expects {} parameter(s), impl has {}", method, expected, found),
            TraitErrorKind::ReturnTypeMismatch { method, expected, found } =>
                format!("E0303: method `{}` return type mismatch — trait `{}`, impl `{}`", method, expected, found),
            TraitErrorKind::ParamTypeMismatch { method, param, expected, found } =>
                format!("E0304: method `{}` param {} type mismatch — trait `{}`, impl `{}`", method, param, expected, found),
            TraitErrorKind::UnsatisfiedBound { ty, trait_name } =>
                format!("E0305: type `{}` does not implement trait `{}`", ty, trait_name),
            TraitErrorKind::DuplicateImpl { trait_name, ty } =>
                format!("E0306: duplicate impl of `{}` for `{}`", trait_name, ty),
        }
    }
}
