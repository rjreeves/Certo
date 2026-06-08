use std::collections::HashMap;
use crate::ty::{Ty, TyVar};
use crate::unify::UnionFind;

/// A type environment: maps names to their (possibly polymorphic) types.
#[derive(Default, Clone)]
pub struct TypeEnv {
    frames: Vec<HashMap<String, Ty>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv { frames: vec![HashMap::new()] }
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
    }
}
