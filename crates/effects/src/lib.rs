mod effect_env;
mod infer_effects;
pub mod check_effects;
mod error;

pub use effect_env::{EffectEnv, DeclaredEffects, build_env};
pub use error::{EffectError, EffectErrorKind};

use certo_ast::module::Module;

/// Build the effect environment and check all function bodies in a module.
pub fn check_module(module: &Module) -> Result<EffectEnv, Vec<EffectError>> {
    let env    = build_env(module);
    let errors = check_effects::check_module(module, &env);
    if errors.is_empty() { Ok(env) } else { Err(errors) }
}

#[cfg(test)]
mod tests;
