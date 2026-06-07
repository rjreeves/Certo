use certo_ast::pattern::{Pattern, PatternField};
use certo_ast::span::{S, Span};
use crate::scope::{ScopeChain, Res};

/// Walk a pattern and define every binding it introduces into `scope`.
/// Lookups for constructor names are verified against the scope.
pub fn define_pattern_bindings(pat: &S<Pattern>, scope: &mut ScopeChain) {
    match &pat.node {
        Pattern::Wildcard { .. } => {}

        Pattern::Ident { name, span } => {
            // A plain lowercase ident in a pattern is always a new binding.
            scope.define(&name.node, Res::Local { span: *span }, *span);
        }

        Pattern::Constructor { path, fields, .. } => {
            // Verify the constructor name is in scope.
            let ctor = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            let ctor_span = path.span;
            scope.lookup(ctor, ctor_span);
            for f in fields {
                define_pattern_bindings(f, scope);
            }
        }

        Pattern::Record { path, fields, .. } => {
            if let Some(p) = path {
                let ctor = p.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                scope.lookup(ctor, p.span);
            }
            for PatternField { name, pattern, span } in fields {
                match pattern {
                    Some(p) => define_pattern_bindings(p, scope),
                    // Shorthand `{ x }` — x is both the field name and the binding
                    None => scope.define(&name.node, Res::Local { span: *span }, *span),
                }
            }
        }

        Pattern::Tuple { elements, .. } => {
            for e in elements { define_pattern_bindings(e, scope); }
        }

        Pattern::List { head, tail, .. } => {
            for h in head { define_pattern_bindings(h, scope); }
            if let Some(t) = tail { define_pattern_bindings(t, scope); }
        }

        Pattern::Literal { .. } => {}

        Pattern::Guard { pattern, guard, .. } => {
            define_pattern_bindings(pattern, scope);
            // The guard expression is resolved by the caller after bindings are introduced.
            let _ = guard;
        }

        Pattern::As { pattern, name, span } => {
            define_pattern_bindings(pattern, scope);
            scope.define(&name.node, Res::Local { span: *span }, *span);
        }

        Pattern::Or { left, right, .. } => {
            // Both sides must bind the same names (checked by the type checker).
            // For now resolve both sides independently.
            define_pattern_bindings(left, scope);
            define_pattern_bindings(right, scope);
        }
    }
}
