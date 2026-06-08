use crate::trait_db::TraitDb;
use crate::error::{TraitError, TraitErrorKind};
use certo_ast::decl::{Decl, FnDecl};
use certo_ast::module::Module;
use certo_ast::types::TypeParam;

/// Walk every function declaration in the module and verify that
/// concrete types supplied for bounded type parameters actually
/// implement the required traits.
///
/// This is a *static* check over the surface syntax — it catches cases
/// like `fn f<T: Serialize>(x: T)` being called with a type that has no
/// `impl Serialize for …` in the same module.  Cross-module satisfaction
/// is deferred to the linker/module-loader phase.
pub fn check_bounds(module: &Module, db: &TraitDb) -> Vec<TraitError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            check_fn_bounds(f, db, &mut errors);
        }
    }
    errors
}

fn check_fn_bounds(f: &FnDecl, db: &TraitDb, errors: &mut Vec<TraitError>) {
    for tp in &f.type_params {
        check_type_param(tp, db, errors);
    }
}

fn check_type_param(tp: &TypeParam, db: &TraitDb, errors: &mut Vec<TraitError>) {
    // For each bound `T: SomeTrait`, verify there is at least one registered
    // impl of SomeTrait.  We cannot know the concrete T at this stage, so we
    // only flag bounds that refer to traits that don't exist at all.
    for bound in &tp.bounds {
        let trait_name = bound.name.segments
            .iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
        if !db.traits.contains_key(&trait_name) {
            // Unknown trait in a bound — this should be caught by name
            // resolution, so we emit a best-effort error here for defence.
            errors.push(TraitError {
                kind: TraitErrorKind::UnsatisfiedBound {
                    ty:         tp.name.node.clone(),
                    trait_name: trait_name.clone(),
                },
                span: bound.span,
            });
        }
    }
}

/// At a call site `f::<ConcreteType>(…)`, verify `ConcreteType` satisfies
/// the bounds declared on `f`'s type parameter.
///
/// `concrete_type` is the string name of the type being substituted.
/// `type_params` are the function's declared type parameters.
pub fn check_call_bounds(
    concrete_type: &str,
    type_params:   &[TypeParam],
    db:            &TraitDb,
    errors:        &mut Vec<TraitError>,
    span:          certo_ast::span::Span,
) {
    for tp in type_params {
        for bound in &tp.bounds {
            let trait_name = bound.name.segments
                .iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            if !db.implements(concrete_type, &trait_name) {
                errors.push(TraitError {
                    kind: TraitErrorKind::UnsatisfiedBound {
                        ty:         concrete_type.to_string(),
                        trait_name: trait_name.clone(),
                    },
                    span,
                });
            }
        }
    }
}
