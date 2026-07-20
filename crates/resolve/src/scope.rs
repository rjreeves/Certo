use std::collections::HashMap;
use certo_ast::span::Span;
use crate::error::{ResolveError, ResolveErrorKind};

// ------------------------------------------------------------------ //
// Resolution result — what a name refers to
// ------------------------------------------------------------------ //

#[derive(Debug, Clone, PartialEq)]
pub enum Res {
    /// A local binding introduced by `val`, `var`, `fn` param, or pattern.
    Local { span: Span },
    /// A module-level declaration in the current module.
    Decl { name: String, span: Span },
    /// A name brought in by an `import` statement.
    Import { from: String, name: String, span: Span },
    /// A built-in type or value (Int, Bool, Ok, Err, Some, None, …).
    Builtin,
}

// ------------------------------------------------------------------ //
// Scope chain
// ------------------------------------------------------------------ //

/// A single lexical scope frame — maps names to their resolution.
#[derive(Debug, Default)]
struct Frame {
    bindings: HashMap<String, Res>,
}

/// The scope stack used during a single name-resolution pass.
pub struct ScopeChain {
    frames: Vec<Frame>,
    pub errors: Vec<ResolveError>,
}

impl ScopeChain {
    pub fn new() -> Self {
        let mut sc = ScopeChain { frames: vec![Frame::default()], errors: Vec::new() };
        sc.seed_builtins();
        sc
    }

    // ---------------------------------------------------------------- //
    // Frame management
    // ---------------------------------------------------------------- //

    pub fn push(&mut self) {
        self.frames.push(Frame::default());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    // ---------------------------------------------------------------- //
    // Defining names
    // ---------------------------------------------------------------- //

    /// Define a name in the innermost frame. Reports E0102 on duplicate.
    pub fn define(&mut self, name: &str, res: Res, span: Span) {
        let frame = self.frames.last_mut().unwrap();
        if let Some(existing) = frame.bindings.get(name) {
            let first = match existing {
                Res::Local { span } | Res::Decl { span, .. } | Res::Import { span, .. } => *span,
                Res::Builtin => Span::DUMMY,
            };
            self.errors.push(ResolveError {
                kind: ResolveErrorKind::DuplicateDefinition { name: name.to_string(), first },
                span,
            });
            return;
        }
        frame.bindings.insert(name.to_string(), res);
    }

    /// Define a name in the module-level (outermost) frame, used for
    /// top-level declarations. Allows shadowing builtins.
    pub fn define_top_level(&mut self, name: &str, res: Res, span: Span) {
        let frame = self.frames.first_mut().unwrap();
        if let Some(Res::Builtin) = frame.bindings.get(name) {
            // Silently allow shadowing builtins
            frame.bindings.insert(name.to_string(), res);
            return;
        }
        if let Some(existing) = frame.bindings.get(name) {
            let first = match existing {
                Res::Local { span } | Res::Decl { span, .. } | Res::Import { span, .. } => *span,
                Res::Builtin => Span::DUMMY,
            };
            self.errors.push(ResolveError {
                kind: ResolveErrorKind::DuplicateDefinition { name: name.to_string(), first },
                span,
            });
            return;
        }
        frame.bindings.insert(name.to_string(), res);
    }

    /// Define an import in the outermost frame. Reports E0101 on conflict
    /// with another import (but not with a local decl — locals win).
    pub fn define_import(&mut self, name: &str, from: &str, import_span: Span) {
        let frame = self.frames.first_mut().unwrap();
        if let Some(existing) = frame.bindings.get(name) {
            match existing {
                Res::Import { span: first_span, .. } => {
                    let first = *first_span;
                    self.errors.push(ResolveError {
                        kind: ResolveErrorKind::AmbiguousImport {
                            name: name.to_string(),
                            first,
                            second: import_span,
                        },
                        span: import_span,
                    });
                    return;
                }
                // Decl or builtin takes precedence over import — silently skip
                _ => return,
            }
        }
        frame.bindings.insert(name.to_string(), Res::Import {
            from: from.to_string(),
            name: name.to_string(),
            span: import_span,
        });
    }

    // ---------------------------------------------------------------- //
    // Looking up names
    // ---------------------------------------------------------------- //

    /// Resolve a name, searching from innermost to outermost frame.
    /// Records E0100 and returns `None` if not found.
    pub fn lookup(&mut self, name: &str, use_span: Span) -> Option<Res> {
        for frame in self.frames.iter().rev() {
            if let Some(res) = frame.bindings.get(name) {
                return Some(res.clone());
            }
        }
        self.errors.push(ResolveError {
            kind: ResolveErrorKind::UndefinedIdent(name.to_string()),
            span: use_span,
        });
        None
    }

    // ---------------------------------------------------------------- //
    // Builtins
    // ---------------------------------------------------------------- //

    fn seed_builtins(&mut self) {
        let builtins = [
            // Primitive types
            "Int", "Int8", "Int16", "Int32", "Int64",
            "UInt", "Float", "Float32", "Decimal",
            "Bool", "Char", "Text", "Unit",
            // Common stdlib types
            "UUID", "Money", "Timestamp", "Date", "Duration", "Timezone",
            "List", "Map", "Set", "Option", "Result",
            "Page", "Json", "Path", "URL", "Email", "PhoneNumber",
            // Result/Option constructors
            "Ok", "Err", "Some", "None",
            // Built-in values
            "true", "false",
            // Built-in functions
            "panic", "unreachable", "todo",
            "messageBox",
        ];
        let frame = self.frames.first_mut().unwrap();
        for b in builtins {
            frame.bindings.insert(b.to_string(), Res::Builtin);
        }
    }
}
