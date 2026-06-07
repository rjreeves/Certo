/// Byte-offset span within a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end:   u32,
}

impl Span {
    pub const DUMMY: Span = Span { start: 0, end: 0 };

    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start, end: other.end }
    }
}

impl From<logos::Span> for Span {
    fn from(s: logos::Span) -> Self {
        Span::new(s.start, s.end)
    }
}

/// Any AST node that carries a source span.
pub trait Spanned {
    fn span(&self) -> Span;
}

/// Wraps a value `T` with a source `Span`.
#[derive(Debug, Clone, PartialEq)]
pub struct S<T> {
    pub node: T,
    pub span: Span,
}

impl<T> S<T> {
    pub fn new(node: T, span: Span) -> Self {
        S { node, span }
    }
}

impl<T> Spanned for S<T> {
    fn span(&self) -> Span { self.span }
}
