use certo_ast::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseErrorKind {
    Expected { expected: String, found: String },
    UnexpectedToken(String),
    UnexpectedEof,
    InvalidLiteral(String),
    Custom(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ParseErrorKind::Expected { expected, found } =>
                write!(f, "expected {expected}, found {found}"),
            ParseErrorKind::UnexpectedToken(t) =>
                write!(f, "unexpected token: {t}"),
            ParseErrorKind::UnexpectedEof =>
                write!(f, "unexpected end of file"),
            ParseErrorKind::InvalidLiteral(msg) =>
                write!(f, "invalid literal: {msg}"),
            ParseErrorKind::Custom(msg) =>
                write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ParseError {}
