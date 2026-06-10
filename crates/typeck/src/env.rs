use std::collections::HashMap;
use crate::ty::{Ty, TyVar};
use crate::unify::UnionFind;

/// A type environment: maps names to their (possibly polymorphic) types.
#[derive(Default, Clone)]
pub struct TypeEnv {
    frames: Vec<HashMap<String, Ty>>,
    /// Parameter metadata for user-defined functions: name → [(param_name, has_default)].
    /// Stored flat (not scope-stacked) since function decls are always at module level.
    pub param_meta: HashMap<String, Vec<(String, bool)>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv { frames: vec![HashMap::new()], param_meta: HashMap::new() }
    }

    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    /// Define a name in the current innermost frame.
    pub fn define(&mut self, name: impl Into<String>, ty: Ty) {
        self.frames.last_mut().unwrap().insert(name.into(), ty);
    }

    /// Look up a name, searching from innermost to outermost frame.
    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        for frame in self.frames.iter().rev() {
            if let Some(ty) = frame.get(name) {
                return Some(ty);
            }
        }
        None
    }

    /// Generalise a monotype with respect to this environment.
    /// Quantifies over all free vars in `ty` that are not free in the environment.
    pub fn generalise(&self, ty: Ty, uf: &UnionFind) -> Ty {
        let ty_applied = uf.apply(&ty);
        let ty_free    = ty_applied.free_vars();
        let env_free   = self.free_vars(uf);

        let quantified: Vec<TyVar> = ty_free
            .into_iter()
            .filter(|v| !env_free.contains(v))
            .collect();

        if quantified.is_empty() {
            ty_applied
        } else {
            Ty::Forall { vars: quantified, body: Box::new(ty_applied) }
        }
    }

    fn free_vars(&self, uf: &UnionFind) -> Vec<TyVar> {
        let mut out = Vec::new();
        for frame in &self.frames {
            for ty in frame.values() {
                let applied = uf.apply(ty);
                out.extend(applied.free_vars());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Register parameter metadata for a function (names + whether each has a default).
    pub fn define_param_meta(&mut self, fn_name: impl Into<String>, params: Vec<(String, bool)>) {
        self.param_meta.insert(fn_name.into(), params);
    }

    /// Retrieve parameter metadata for a function, if available.
    pub fn get_param_meta(&self, fn_name: &str) -> Option<&Vec<(String, bool)>> {
        self.param_meta.get(fn_name)
    }

    /// Seed the environment with built-in types/values.
    pub fn seed_builtins(&mut self, counter: &mut u32) {
        // Primitive constructors (type annotations, not value-level)
        let prims: &[(&str, Ty)] = &[
            ("true",  Ty::Bool),
            ("false", Ty::Bool),
        ];
        for (name, ty) in prims {
            self.define(*name, ty.clone());
        }

        // panic / unreachable / todo :: ∀a. Text -> a
        for name in &["panic", "unreachable", "todo"] {
            *counter += 1;
            let ret = Ty::Var(*counter);
            self.define(*name, Ty::Forall {
                vars: vec![*counter],
                body: Box::new(Ty::Fn { params: vec![Ty::Text], ret: Box::new(ret) }),
            });
        }

        // Ok  :: ∀T E. T → Result<T, E>
        // Err :: ∀T E. E → Result<T, E>
        {
            *counter += 1; let t = *counter;
            *counter += 1; let e = *counter;
            let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
            self.define("Ok", Ty::Forall {
                vars: vec![t, e],
                body: Box::new(Ty::Fn { params: vec![Ty::Var(t)], ret: Box::new(result_te) }),
            });
        }
        {
            *counter += 1; let t = *counter;
            *counter += 1; let e = *counter;
            let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
            self.define("Err", Ty::Forall {
                vars: vec![t, e],
                body: Box::new(Ty::Fn { params: vec![Ty::Var(e)], ret: Box::new(result_te) }),
            });
        }
    }
}
