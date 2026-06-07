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
        }
    }
}

impl std::error::Error for ResolveError {}
