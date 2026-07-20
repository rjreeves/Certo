use crate::span::{S, Span};
use crate::types::ModulePath;
use crate::decl::Decl;

// ------------------------------------------------------------------ //
// A parsed source file
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// `module MyApp.Orders.Processing`
    pub path:    ModulePath,
    pub imports: Vec<Import>,
    pub decls:   Vec<S<Decl>>,
    pub span:    Span,
}

// ------------------------------------------------------------------ //
// Import declarations
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub is_pub:  bool,
    pub path:    ModulePath,
    pub kind:    ImportKind,
    pub when:    Option<ImportCondition>,
    pub span:    Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    /// `import Stdlib.DateTime`  — import whole module
    Whole,

    /// `import Stdlib.Collections.{ List, Map }`  — named imports
    Named(Vec<ImportedName>),

    /// `import MyApp.Models.Order as O`  — aliased import
    Aliased(S<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedName {
    pub name:  S<String>,
    pub alias: Option<S<String>>,
    pub span:  Span,
}

/// `import when [target = "wasm"] Stdlib.WASM.{ Memory }`
#[derive(Debug, Clone, PartialEq)]
pub struct ImportCondition {
    pub key:   String,
    pub value: String,
    pub span:  Span,
}
