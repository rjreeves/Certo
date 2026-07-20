use crate::span::{S, Span};

/// An interned identifier (name as a string slice, span where it appeared).
pub type Ident = S<String>;

/// A dot-separated module path, e.g. `Stdlib.Collections`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModulePath {
    pub segments: Vec<Ident>,
    pub span:     Span,
}

// ------------------------------------------------------------------ //
// Type expressions
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// A named type, possibly with generic arguments: `List<Int>`, `Result<T, E>`
    Named {
        path: ModulePath,
        args: Vec<S<TypeExpr>>,
        span: Span,
    },

    /// `T?`  — shorthand for `Option<T>`
    Option {
        inner: Box<S<TypeExpr>>,
        span:  Span,
    },

    /// `(A, B, C)` — tuple type
    Tuple {
        elements: Vec<S<TypeExpr>>,
        span:     Span,
    },

    /// `A => B` — function type
    Fn {
        params: Vec<S<TypeExpr>>,
        ret:    Box<S<TypeExpr>>,
        span:   Span,
    },

    /// `{ name: Text, age: Int }` — anonymous record / row type
    Record {
        fields: Vec<RecordTypeField>,
        span:   Span,
    },

    /// `*Byte` — raw pointer (FFI / unsafe only)
    Ptr {
        inner: Box<S<TypeExpr>>,
        span:  Span,
    },

    /// A single type parameter name: `T`, `E`, `Key`
    Param {
        name: Ident,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeField {
    pub name:     Ident,
    pub ty:       S<TypeExpr>,
    pub optional: bool,   // true when the field is declared with `?`
    pub span:     Span,
}

// ------------------------------------------------------------------ //
// Effect annotations  [pure]  [db.read, db.write]  etc.
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Effect {
    Pure,
    DbRead,
    DbWrite,
    Io,
    Async,
    Fallible,
    Unsafe,
}

/// The bracketed effect list on a function: `[db.read, async]`
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSet {
    pub effects: Vec<S<Effect>>,
    pub span:    Span,
}

// ------------------------------------------------------------------ //
// Trait constraints  `T: DbModel + Serializable`
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct TraitBound {
    pub name: ModulePath,
    pub span: Span,
}

/// A single generic parameter with optional bounds: `T: DbModel + Serializable`
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name:   Ident,
    pub bounds: Vec<TraitBound>,
    pub span:   Span,
}
