use certo_ast::module::{Module, Import, ImportKind};
use certo_ast::decl::Decl;
use crate::scope::{ScopeChain, Res};
use crate::error::ResolveError;
use crate::resolve_decl::resolve_decl;

/// Resolve all names in a parsed module.
///
/// Returns `Ok(())` when no errors were found, or `Err(Vec<ResolveError>)`
/// listing every E0100/E0101/E0102 that was detected.
pub fn resolve(module: &Module) -> Result<(), Vec<ResolveError>> {
    let mut scope = ScopeChain::new();

    // Pass 1: hoist all top-level declaration names so that mutually
    // recursive functions can refer to each other.
    hoist_decls(module, &mut scope);

    // Pass 2: process imports — they come after hoisting so that a local
    // decl silently shadows an import of the same name.
    process_imports(&module.imports, &mut scope);

    // Pass 3: resolve names inside every declaration.
    for decl in &module.decls {
        resolve_decl(decl, &mut scope);
    }

    if scope.errors.is_empty() {
        Ok(())
    } else {
        Err(scope.errors)
    }
}

/// Pre-declare every top-level name so forward references and mutual
/// recursion work without ordering constraints.
fn hoist_decls(module: &Module, scope: &mut ScopeChain) {
    for decl in &module.decls {
        let (name, span) = match &decl.node {
            Decl::Fn(f)           => (f.name.node.clone(), f.name.span),
            Decl::Type(t)         => (t.name.node.clone(), t.name.span),
            Decl::Val(v)          => {
                // Val can have a complex pattern — only hoist simple ident patterns.
                use certo_ast::pattern::Pattern;
                if let Pattern::Ident { name, .. } = &v.pattern.node {
                    (name.node.clone(), name.span)
                } else { continue; }
            }
            Decl::Var(v)          => (v.name.node.clone(), v.name.span),
            Decl::Trait(t)        => (t.name.node.clone(), t.name.span),
            Decl::StateMachine(s) => (s.name.node.clone(), s.name.span),
            Decl::View(v)         => (v.name.node.clone(), v.name.span),
            Decl::Constraint(c)   => (c.name.node.clone(), c.name.span),
            Decl::Temporal(t)     => (t.name.node.clone(), t.name.span),
            Decl::Validator(v)    => (v.name.node.clone(), v.name.span),
            // impl / migration / test / rule-test don't introduce a module-level name
            _ => continue,
        };
        scope.define_top_level(
            &name,
            Res::Decl { name: name.clone(), span },
            span,
        );
    }
}

/// Bring imported names into scope.
fn process_imports(imports: &[Import], scope: &mut ScopeChain) {
    for import in imports {
        let module_name = import.path.segments
            .iter()
            .map(|s| s.node.as_str())
            .collect::<Vec<_>>()
            .join(".");

        match &import.kind {
            ImportKind::Whole => {
                // `import Stdlib.DateTime` — the leaf module name is usable as a qualifier.
                if let Some(leaf) = import.path.segments.last() {
                    scope.define_import(&leaf.node, &module_name, import.span);
                }
            }

            ImportKind::Named(names) => {
                // `import Stdlib.Collections.{ List, Map }`
                for imported in names {
                    let local_name = imported.alias.as_ref()
                        .map(|a| a.node.as_str())
                        .unwrap_or(imported.name.node.as_str());
                    scope.define_import(local_name, &module_name, imported.span);
                }
            }

            ImportKind::Aliased(alias) => {
                // `import MyApp.Models.Order as O`
                scope.define_import(&alias.node, &module_name, import.span);
            }
        }
    }
}
