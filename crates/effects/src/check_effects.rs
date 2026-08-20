use certo_ast::decl::{Decl, FnParam};
use certo_ast::module::Module;
use certo_ast::types::Effect;
use crate::effect_env::{EffectEnv, DeclaredEffects};
use crate::infer_effects::{infer_expr, type_expr_simple_name, InferredEffects, LocalTypes};
use crate::error::{EffectError, EffectErrorKind};

/// Parameters are always explicitly typed in Certo (`FnParam.ty` isn't
/// optional) — BACKLOG item 178's own starting point: every parameter's
/// type is known for free, no inference needed, just reading what's
/// already in the signature.
fn params_to_locals(params: &[FnParam]) -> LocalTypes {
    params.iter()
        .filter_map(|p| type_expr_simple_name(&p.ty.node).map(|ty| (p.name.node.clone(), ty)))
        .collect()
}

pub fn check_module(module: &Module, env: &EffectEnv) -> Vec<EffectError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                if let Some(body) = &f.body {
                    let declared = env.get(&f.name.node)
                        .cloned()
                        .unwrap_or_default();
                    let locals = params_to_locals(&f.params);
                    check_fn_body(&f.name.node, &declared, body, env, &locals, &mut errors);
                }
            }
            Decl::Impl(i) => {
                let type_name = i.type_path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                for m in &i.methods {
                    if let Some(body) = &m.body {
                        let qname = format!("{}.{}", type_name, m.name.node);
                        let declared = env.get(&qname)
                            .cloned()
                            .unwrap_or_default();
                        let locals = params_to_locals(&m.params);
                        check_fn_body(&qname, &declared, body, env, &locals, &mut errors);
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
    locals:   &LocalTypes,
    errors:   &mut Vec<EffectError>,
) {
    let mut inferred = InferredEffects::default();
    infer_expr(body, env, locals, &mut inferred);

    for (effect, span, callee) in &inferred.origins {
        // `?` is allowed in pure functions (pure = no side effects, not
        // necessarily total); Pure itself is never a violation.
        if matches!(effect, Effect::Pure | Effect::Fallible) { continue; }

        // A pure function calling a *named* function that requires this
        // effect gets the most specific, actionable message — who was
        // called and what it needs. This must be checked (and the loop
        // iteration finished) before the per-kind arms below, so a call's
        // effect isn't also reported by the generic per-kind message.
        if declared.is_pure {
            if let Some(callee_name) = callee {
                errors.push(EffectError {
                    kind: EffectErrorKind::ImpureCallInPure {
                        caller: fn_name.to_string(),
                        callee: callee_name.clone(),
                        effect: effect.clone(),
                    },
                    span: *span,
                });
                continue;
            }
        }

        match effect {
            Effect::Async => {
                if !declared.effects.contains(&Effect::Async) {
                    if declared.is_pure {
                        // Direct `await`/`spawn`/`parallel` syntax — no callee to name.
                        errors.push(EffectError {
                            kind: EffectErrorKind::UndeclaredEffect {
                                fn_name: fn_name.to_string(),
                                effect:  effect.clone(),
                            },
                            span: *span,
                        });
                    } else if !declared.effects.is_empty() {
                        errors.push(EffectError {
                            kind: EffectErrorKind::MissingAsyncAnnotation {
                                fn_name: fn_name.to_string(),
                            },
                            span: *span,
                        });
                    }
                }
            }

            Effect::DbWrite => {
                if !declared.effects.contains(&Effect::DbWrite)
                    && (declared.is_pure || !declared.effects.is_empty())
                {
                    errors.push(EffectError {
                        kind: EffectErrorKind::TransactionOutsideDbWrite {
                            fn_name: fn_name.to_string(),
                        },
                        span: *span,
                    });
                }
            }

            Effect::Unsafe => {
                // Always requires an explicit [unsafe] annotation, even for an
                // otherwise-open (unannotated) function.
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
                // Reaching here with declared.is_pure means a callee-less origin
                // (in practice this doesn't happen for Io/DbRead — they're only
                // ever introduced via a named call — but handle it defensively).
                if declared.is_pure {
                    errors.push(EffectError {
                        kind: EffectErrorKind::UndeclaredEffect {
                            fn_name: fn_name.to_string(),
                            effect:  other.clone(),
                        },
                        span: *span,
                    });
                } else if !declared.effects.is_empty() && !declared.effects.contains(other) {
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
}
