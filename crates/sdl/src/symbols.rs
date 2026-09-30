//! Symbol table: which top-level names exist and what they are.
//!
//! Tables, enums and composite types share one namespace (and may not
//! shadow a builtin type). Index and constraint names are global within
//! their own namespaces. The first declaration of a name wins; later ones
//! are reported and ignored by the semantic pass (see `is_canonical`).

use crate::ast::*;
use crate::ir::Builtin;
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKind {
    Sequence,
    Table,
    Enum,
    Type,
}

impl SymKind {
    fn article(self) -> &'static str {
        match self {
            SymKind::Sequence => "sequence",
            SymKind::Table => "table",
            SymKind::Enum => "enum",
            SymKind::Type => "type",
        }
    }
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    decls: HashMap<String, (SymKind, Span)>,
    indexes: HashMap<String, Span>,
    constraints: HashMap<String, Span>,
}

impl SymbolTable {
    pub fn collect(file: &SdlFile, diags: &mut Vec<Diagnostic>) -> SymbolTable {
        let mut t = SymbolTable::default();
        for d in &file.decls {
            match d {
                Decl::Sequence(x) => t.declare(&x.name, SymKind::Sequence, diags),
                Decl::Table(x) => t.declare(&x.name, SymKind::Table, diags),
                Decl::Enum(x) => t.declare(&x.name, SymKind::Enum, diags),
                Decl::Type(x) => t.declare(&x.name, SymKind::Type, diags),
                Decl::Index(x) => dup_check(&mut t.indexes, &x.name, "index", diags),
                Decl::Constraint(x) => dup_check(&mut t.constraints, &x.name, "constraint", diags),
            }
        }
        t
    }

    fn declare(&mut self, name: &Ident, kind: SymKind, diags: &mut Vec<Diagnostic>) {
        if Builtin::is_type_name(&name.name) {
            diags.push(
                Diagnostic::error("SDL200", format!("`{}` is a builtin type and cannot be redeclared", name.name))
                    .with_span(name.span),
            );
            return;
        }
        match self.decls.get(&name.name) {
            Some(&(prev_kind, prev_span)) => diags.push(
                Diagnostic::error("SDL200", format!("`{}` is already declared", name.name))
                    .with_span(name.span)
                    .with_label(format!("previously declared as {} at byte {}", prev_kind.article(), prev_span.start)),
            ),
            None => { self.decls.insert(name.name.clone(), (kind, name.span)); }
        }
    }

    pub fn kind(&self, name: &str) -> Option<SymKind> {
        self.decls.get(name).map(|&(k, _)| k)
    }

    /// True if `id` is the declaration that owns its name (not a duplicate,
    /// and not a builtin-shadowing declaration that was rejected).
    pub fn is_canonical(&self, id: &Ident) -> bool {
        self.decls.get(&id.name).is_some_and(|&(_, s)| s == id.span)
    }

    pub fn is_canonical_index(&self, id: &Ident) -> bool {
        self.indexes.get(&id.name).is_some_and(|&s| s == id.span)
    }

    pub fn is_canonical_constraint(&self, id: &Ident) -> bool {
        self.constraints.get(&id.name).is_some_and(|&s| s == id.span)
    }
}

fn dup_check(map: &mut HashMap<String, Span>, name: &Ident, what: &str, diags: &mut Vec<Diagnostic>) {
    if map.contains_key(&name.name) {
        diags.push(
            Diagnostic::error("SDL200", format!("{what} `{}` is already declared", name.name))
                .with_span(name.span),
        );
    } else {
        map.insert(name.name.clone(), name.span);
    }
}
