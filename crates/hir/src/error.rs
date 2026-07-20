use certo_ast::span::Span;

#[derive(Debug, Clone)]
pub struct LowerError {
    pub kind: LowerErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum LowerErrorKind {
    /// Tried to lower a name that wasn't in the HIR environment.
    UnresolvedName(String),
    /// A feature not yet supported by the HIR lowering pass.
    Unsupported(String),
}

impl LowerError {
    pub fn message(&self) -> String {
        match &self.kind {
            LowerErrorKind::UnresolvedName(n) => format!("HIR: unresolved name `{}`", n),
            LowerErrorKind::Unsupported(f)    => format!("HIR: unsupported: {}", f),
        }
    }
}
