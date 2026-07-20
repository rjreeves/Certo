use crate::trait_db::{TraitDb, sig_of};
use crate::error::{TraitError, TraitErrorKind};
use certo_ast::decl::{Decl, ImplDecl};
use certo_ast::module::Module;

/// Check every `impl Trait for Type` in the module against the trait definition.
pub fn check_impls(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    let mut errors = Vec::new();

    // Detect duplicate impls
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    for sdecl in &module.decls {
        if let Decl::Impl(i) = &sdecl.node {
            check_impl(i, db, &mut errors, &mut seen);
        }
    }
    errors
}

fn check_impl(
    i:      &ImplDecl,
    db:     &TraitDb,
    errors: &mut Vec<TraitError>,
    seen:   &mut std::collections::HashSet<(String, String)>,
) {
    let for_type = i.type_path.segments
        .iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");

    let trait_path = match &i.trait_path {
        Some(p) => p.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."),
        // Inherent impls need no trait conformance check.
        None    => return,
    };

    // Duplicate impl check
    let key = (trait_path.clone(), for_type.clone());
    if !seen.insert(key) {
        errors.push(TraitError {
            kind: TraitErrorKind::DuplicateImpl {
                trait_name: trait_path.clone(),
                ty:         for_type.clone(),
            },
            span: i.span,
        });
        return;
    }

    // Look up the trait definition
    let trait_def = match db.traits.get(&trait_path) {
        Some(t) => t,
        None    => return, // unknown trait — name resolution would have caught it
    };

    // Build a set of impl method names
    let impl_methods: std::collections::HashMap<String, _> = i.methods.iter()
        .map(|m| (m.name.node.clone(), m))
        .collect();

    // Check: every impl method must be in the trait
    for (name, method) in &impl_methods {
        if !trait_def.methods.contains_key(name.as_str()) {
            errors.push(TraitError {
                kind: TraitErrorKind::UnknownMethod {
                    trait_name: trait_path.clone(),
                    method:     name.clone(),
                },
                span: method.span,
            });
        }
    }

    // Check: every required trait method (no default) must be in the impl
    for (name, tsig) in &trait_def.methods {
        if tsig.has_default {
            continue;
        }
        match impl_methods.get(name.as_str()) {
            None => {
                errors.push(TraitError {
                    kind: TraitErrorKind::MissingMethod {
                        trait_name: trait_path.clone(),
                        method:     name.clone(),
                    },
                    span: i.span,
                });
            }
            Some(impl_fn) => {
                let isig = sig_of(impl_fn);

                // Param count
                if isig.param_count != tsig.param_count {
                    errors.push(TraitError {
                        kind: TraitErrorKind::ParamCountMismatch {
                            method:   name.clone(),
                            expected: tsig.param_count,
                            found:    isig.param_count,
                        },
                        span: impl_fn.span,
                    });
                    continue;
                }

                // Param types (string-level comparison — full HM unification
                // across traits and impls is the trait solver, deferred to
                // the trait system extension in task #5b)
                for (idx, (tpt, ipt)) in tsig.param_types.iter().zip(&isig.param_types).enumerate() {
                    if !types_compatible(tpt, ipt) {
                        errors.push(TraitError {
                            kind: TraitErrorKind::ParamTypeMismatch {
                                method:   name.clone(),
                                param:    idx,
                                expected: tpt.clone(),
                                found:    ipt.clone(),
                            },
                            span: impl_fn.span,
                        });
                    }
                }

                // Return type
                if !tsig.ret_type.is_empty()
                    && !isig.ret_type.is_empty()
                    && !types_compatible(&tsig.ret_type, &isig.ret_type)
                {
                    errors.push(TraitError {
                        kind: TraitErrorKind::ReturnTypeMismatch {
                            method:   name.clone(),
                            expected: tsig.ret_type.clone(),
                            found:    isig.ret_type.clone(),
                        },
                        span: impl_fn.span,
                    });
                }
            }
        }
    }
}

/// Two type strings are compatible if they are equal or if at least one is a
/// generic type parameter (single uppercase-starting identifier — e.g. `T`, `E`).
fn types_compatible(a: &str, b: &str) -> bool {
    if a == b { return true; }
    is_type_param(a) || is_type_param(b)
}

fn is_type_param(s: &str) -> bool {
    s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !matches!(s, "Int" | "Bool" | "Text" | "Float" | "Decimal" | "Unit" | "UUID")
}
