use certo_ast::decl::Decl;
use certo_ast::module::Module;
use certo_ast::types::Effect;
use crate::effect_env::{EffectEnv, DeclaredEffects};
use crate::infer_effects::{infer_expr, InferredEffects};
use crate::error::{EffectError, EffectErrorKind};

pub fn check_module(module: &Module, env: &EffectEnv) -> Vec<EffectError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                if let Some(body) = &f.body {
                    let declared = env.get(&f.name.node)
                        .cloned()
                        .unwrap_or_default();
                    check_fn_body(&f.name.node, &declared, body, env, &mut errors);
                }
            }
            Decl::Impl(i) => {
                for m in &i.methods {
                    if let Some(body) = &m.body {
                        let declared = env.get(&m.name.node)
                            .cloned()
                            .unwrap_or_default();
                        check_fn_body(&m.name.node, &declared, body, env, &mut errors);
                    }
                }
            }
            _ => {}
        }
    }
    errors
}

fn check_fn_body(
    fn_name:  &str,
    declared: &DeclaredEffects,
    body:     &certo_ast::span::S<certo_ast::expr::Expr>,
    env:      &EffectEnv,
    errors:   &mut Vec<EffectError>,
) {
    let mut inferred = InferredEffects::default();
    infer_expr(body, env, &mut inferred);

    for (effect, span) in &inferred.origins {
        match effect {
            Effect::Async => {
                if !declared.is_pure && !declared.effects.contains(&Effect::Async) {
                    // If the function has no effects annotation at all — open set — skip.
                    // Only flag when there IS a non-async annotation or when it's pure.
                    if declared.is_pure || !declared.effects.is_empty() {
                        if declared.is_pure {
                            errors.push(EffectError {
                                kind: EffectErrorKind::UndeclaredEffect {
                                    fn_name: fn_name.to_string(),
                                    effect:  effect.clone(),
                                },
                                span: *span,
                            });
                        } else {
                            errors.push(EffectError {
                                kind: EffectErrorKind::MissingAsyncAnnotation {
                                    fn_name: fn_name.to_string(),
                                },
                                span: *span,
                            });
                        }
                    }
                }
            }

            Effect::DbWrite => {
                if !declared.effects.contains(&Effect::DbWrite) {
                    if declared.is_pure || !declared.effects.is_empty() {
                        errors.push(EffectError {
                            kind: EffectErrorKind::TransactionOutsideDbWrite {
                                fn_name: fn_name.to_string(),
                            },
                            span: *span,
                        });
                    }
                }
            }

            Effect::Unsafe => {
                if !declared.effects.contains(&Effect::Unsafe) {
                    errors.push(EffectError {
                        kind: EffectErrorKind::UnsafeOutsideUnsafe {
                            fn_name: fn_name.to_string(),
                        },
                        span: *span,
                    });
                }
            }

            other => {
                // For all other effects: flag only when the function is explicitly
                // annotated as pure, or has a declared set that doesn't include them.
                if declared.is_pure {
                    errors.push(EffectError {
                        kind: EffectErrorKind::UndeclaredEffect {
                            fn_name: fn_name.to_string(),
                            effect:  other.clone(),
                        },
                        span: *span,
                    });
                } else if !declared.effects.is_empty() && !declared.effects.contains(other) {
                    // Check if this effect came from a callee — emit ImpureCallInPure only
                    // when the caller is pure.
                    errors.push(EffectError {
                        kind: EffectErrorKind::UndeclaredEffect {
                            fn_name: fn_name.to_string(),
                            effect:  other.clone(),
                        },
                        span: *span,
                    });
                }
            }
        }
    }

    // Special case: pure function calling any effectful callee
    if declared.is_pure {
        for (effect, span) in &inferred.origins {
            if *effect != Effect::Pure && *effect != Effect::Fallible {
                // Fallible is allowed in pure functions (pure = no side effects,
                // not necessarily total). All others are not.
                errors.push(EffectError {
                    kind: EffectErrorKind::ImpureCallInPure {
                        caller: fn_name.to_string(),
                        callee: String::new(),
                        effect: effect.clone(),
                    },
                    span: *span,
                });
            }
        }
    }
}
