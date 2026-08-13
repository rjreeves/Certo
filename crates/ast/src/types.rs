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

    /// `Decimal(19, 4)` — precision/scale on the one builtin type that needs
    /// compile-time-constant (not type) parameters. Deliberately not a
    /// general mechanism: the parser only ever produces this for the exact
    /// single-segment name `Decimal`, not a reusable "any type can take
    /// parenthesized literal args" grammar rule.
    DecimalParam {
        precision: u8,
        scale:     u8,
        span:      Span,
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

/// A row-polymorphism bound: `R: { name: Text }` — satisfied by any type that has
/// *at least* these fields (with compatible types); extra fields are fine, unlike
/// `TypeExpr::Record`'s use as a concrete anonymous record type.
#[derive(Debug, Clone, PartialEq)]
pub struct RowBound {
    pub fields: Vec<RecordTypeField>,
    pub span:   Span,
}

/// One bound in a type parameter's `+`-separated bound list: either a trait name
/// (`DbModel`) or an inline record shape (`{ name: Text }`).
#[derive(Debug, Clone, PartialEq)]
pub enum Bound {
    Trait(TraitBound),
    Row(RowBound),
}

/// A single generic parameter with optional bounds: `T: DbModel + Serializable`,
/// or `R: { name: Text }` (row polymorphism).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name:   Ident,
    pub bounds: Vec<Bound>,
    /// `true` for `F<_>` — a higher-kinded, 1-ary type-constructor parameter
    /// (BACKLOG item 76), e.g. `fn map<F<_>, A, B>(fa: F<A>, ...)`, distinct
    /// from an ordinary type parameter like `T`. Certo has no kind
    /// polymorphism beyond this: a constructor parameter is always exactly
    /// 1-ary (matching every real single-type-argument type in the
    /// language — `Box<T>`, `List<T>`, `Option<T>`, a user's own
    /// single-param generic record/sum type); a 2-ary constructor like
    /// `Map`/`Result` can never satisfy one.
    pub is_constructor: bool,
    pub span:   Span,
}
