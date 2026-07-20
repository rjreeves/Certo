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
    let mut env = EffectEnv::new();

    for sdecl in &module.decls {
        collect_fn_effects(&sdecl.node, &mut env);
    }
    env
}

fn collect_fn_effects(decl: &Decl, env: &mut EffectEnv) {
    let fns: Vec<&certo_ast::decl::FnDecl> = match decl {
        Decl::Fn(f)    => vec![f],
        Decl::Trait(t) => t.methods.iter().collect(),
        Decl::Impl(i)  => i.methods.iter().collect(),
        _              => vec![],
    };

    for f in fns {
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

        env.insert(f.name.node.clone(), declared);
    }
}
