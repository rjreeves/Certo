mod effect_env;
mod infer_effects;
pub mod check_effects;
mod error;

pub use effect_env::{EffectEnv, DeclaredEffects, build_env, build_env_seeded};
pub use error::{EffectError, EffectErrorKind, effect_name};

use certo_ast::module::Module;

/// Build the effect environment and check all function bodies in a module.
pub fn check_module(module: &Module) -> Result<EffectEnv, Vec<EffectError>> {
    check_module_seeded(module, EffectEnv::new())
}

/// Like `check_module`, but starts from a pre-seeded environment — use this
/// when stdlib function effects (via `certo_stdlib::seed_stdlib_effects`)
/// need to be visible to calls made from `module`.
pub fn check_module_seeded(module: &Module, seed: EffectEnv) -> Result<EffectEnv, Vec<EffectError>> {
    let env    = build_env_seeded(module, seed);
    let errors = check_effects::check_module(module, &env);
    if errors.is_empty() { Ok(env) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
