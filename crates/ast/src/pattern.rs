use crate::span::{S, Span};
use crate::types::ModulePath;
use crate::expr::Expr;

// ------------------------------------------------------------------ //
// Patterns
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`  — wildcard, matches anything and binds nothing
    Wildcard { span: Span },

    /// A bare name: binds the matched value, e.g. `x`
    Ident { name: S<String>, span: Span },

    /// A constructor pattern: `Some(x)`, `Circle(r)`, `Ok(v)`
    Constructor {
        path:   ModulePath,
        fields: Vec<S<Pattern>>,
        span:   Span,
    },

    /// A named-field constructor: `Circle { radius: r }`
    Record {
        path:   Option<ModulePath>,
        fields: Vec<PatternField>,
        rest:   bool,          // true when `..` appears at the end
        span:   Span,
    },

    /// `(p1, p2, p3)` — tuple pattern
    Tuple {
        elements: Vec<S<Pattern>>,
        span:     Span,
    },

    /// `[head, ...tail]` — list pattern
    List {
        head: Vec<S<Pattern>>,
        tail: Option<Box<S<Pattern>>>,  // the `...rest` binding if present
        span: Span,
    },

    /// A literal: `42`, `"hello"`, `true`
    Literal { value: LitPat, span: Span },

    /// `pat if guard` — guarded pattern (used in match arms)
    Guard {
        pattern: Box<S<Pattern>>,
        guard:   Box<S<Expr>>,
        span:    Span,
    },

    /// `pat as name` — alias pattern
    As {
        pattern: Box<S<Pattern>>,
        name:    S<String>,
        span:    Span,
    },

    /// `p1 | p2` — or-pattern
    Or {
        left:  Box<S<Pattern>>,
        right: Box<S<Pattern>>,
        span:  Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternField {
    pub name:    S<String>,
    pub pattern: Option<S<Pattern>>,  // None means shorthand `{ x }` = `{ x: x }`
    pub span:    Span,
}

/// Literal values legal in patterns.
#[derive(Debug, Clone, PartialEq)]
pub enum LitPat {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Unit,
}
