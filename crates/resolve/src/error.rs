use certo_ast::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolveError {
    pub kind: ResolveErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveErrorKind {
    /// E0100 — name used but never declared
    UndefinedIdent(String),
    /// E0101 — two imports bring the same name into scope
    AmbiguousImport { name: String, first: Span, second: Span },
    /// E0102 — two declarations in the same scope share a name
    DuplicateDefinition { name: String, first: Span },
    /// E0700 — cycle in the `after` rule dependency graph inside a validator
    RuleCycle { validator: String, cycle: Vec<String> },
    /// E0701 — `after` references a rule that does not exist in this validator
    AfterRuleNotFound { rule_name: String, after_name: String },
    /// E0702 — `overrides` references a rule that does not exist in this validator
    OverridesRuleNotFound { rule_name: String, overrides_name: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ResolveErrorKind::UndefinedIdent(n) =>
                write!(f, "E0100: undefined identifier `{n}`"),
            ResolveErrorKind::AmbiguousImport { name, .. } =>
                write!(f, "E0101: ambiguous import — `{name}` is imported more than once"),
            ResolveErrorKind::DuplicateDefinition { name, .. } =>
                write!(f, "E0102: duplicate definition of `{name}` in this scope"),
            ResolveErrorKind::RuleCycle { validator, cycle } =>
                write!(f, "E0700: cycle in rule dependency graph in validator `{validator}`: {}", cycle.join(" → ")),
            ResolveErrorKind::AfterRuleNotFound { rule_name, after_name } =>
                write!(f, "E0701: rule `{rule_name}` has `after {after_name}` but `{after_name}` does not exist in this validator"),
            ResolveErrorKind::OverridesRuleNotFound { rule_name, overrides_name } =>
                write!(f, "E0702: rule `{rule_name}` has `overrides {overrides_name}` but `{overrides_name}` does not exist in this validator"),
        }
    }
}

impl std::error::Error for ResolveError {}
