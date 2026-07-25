use std::collections::{HashMap, HashSet};
use certo_ast::decl::Decl;
use certo_ast::module::Module;
use certo_ast::types::Effect;

/// The declared effect set for one function (empty = unconstrained / inferred).
#[derive(Debug, Clone, Default)]
pub struct DeclaredEffects {
    pub effects: HashSet<Effect>,
    /// True when the function has an explicit `[pure]` or `[]` annotation
    /// (i.e. we must enforce the empty set, not just leave it open).
    pub is_pure: bool,
}

impl DeclaredEffects {
    pub fn allows(&self, e: &Effect) -> bool {
        if self.is_pure { false } else { self.effects.contains(e) || self.effects.is_empty() }
    }
}

/// Maps function names to their declared effect sets.
pub type EffectEnv = HashMap<String, DeclaredEffects>;

/// Build an effect environment from a module's top-level fn declarations.
pub fn build_env(module: &Module) -> EffectEnv {
    build_env_seeded(module, EffectEnv::new())
}

/// Like `build_env`, but merges the module's declarations into a pre-existing
/// environment (e.g. one already populated with stdlib function effects via
/// `certo_stdlib::seed_stdlib_effects`) instead of starting from empty. A
/// function the module declares under a name that also exists in `env`
/// overrides the seeded entry, matching how `certo_typeck`'s stdlib seeding
/// lets user code shadow a builtin.
pub fn build_env_seeded(module: &Module, mut env: EffectEnv) -> EffectEnv {
    for sdecl in &module.decls {
        collect_fn_effects(&sdecl.node, &mut env);
    }
    env
}

fn collect_fn_effects(decl: &Decl, env: &mut EffectEnv) {
    // Impl methods are keyed by "Type.method" — matching how a Type.method(...)
    // dot-call is resolved (see infer_effects.rs) — so two impls declaring a
    // same-named method don't collide under one bare-name entry. Top-level fns
    // and trait method *signatures* (abstract, never directly callable) keep
    // their bare name.
    let fns: Vec<(String, &certo_ast::decl::FnDecl)> = match decl {
        Decl::Fn(f)    => vec![(f.name.node.clone(), f)],
        Decl::Trait(t) => t.methods.iter().map(|f| (f.name.node.clone(), f)).collect(),
        Decl::Impl(i)  => {
            let type_name = i.type_path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            i.methods.iter().map(|f| (format!("{}.{}", type_name, f.name.node), f)).collect()
        }
        _              => vec![],
    };

    for (key, f) in fns {
        let declared = match &f.effects {
            None => DeclaredEffects { effects: HashSet::new(), is_pure: false },
            Some(es) => {
                let mut set = HashSet::new();
                let mut is_pure = false;
                for se in &es.effects {
                    if se.node == Effect::Pure {
                        is_pure = true;
                    } else {
                        set.insert(se.node.clone());
                    }
                }
                // An empty annotation `[]` also means pure.
                if es.effects.is_empty() { is_pure = true; }
                DeclaredEffects { effects: set, is_pure }
            }
        };

        // `async fn` implicitly requires [async]
        let mut declared = declared;
        if f.is_async {
            declared.effects.insert(Effect::Async);
        }

        env.insert(key, declared);
    }
}
