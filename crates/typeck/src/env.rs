use std::collections::{HashMap, HashSet};
use crate::ty::{Ty, TyVar};
use crate::unify::UnionFind;

/// A type environment: maps names to their (possibly polymorphic) types.
#[derive(Default, Clone)]
pub struct TypeEnv {
    frames: Vec<HashMap<String, Ty>>,
    /// `TyVar`s bound to a 1-ary type-constructor parameter (`F<_>`,
    /// BACKLOG item 76) — a global set, not scope-stacked like `frames`,
    /// since a `TyVar` id is globally unique (minted from the same counter
    /// as every other fresh var) and remains meaningful after its
    /// declaring scope is popped (e.g. once captured inside a `Forall`).
    /// Consulted by `type_expr_to_ty` to tell `F<A>` (higher-kinded
    /// application) apart from an ordinary `Named { name: "F", args }`
    /// when it sees a type-param name applied to an argument.
    pub constructor_vars: HashSet<TyVar>,
    /// Parameter metadata for user-defined functions: name → [(param_name, has_default)].
    /// Stored flat (not scope-stacked) since function decls are always at module level.
    pub param_meta: HashMap<String, Vec<(String, bool)>>,
    /// Record type definitions: type_name → [(field_name, field_type)].
    pub record_fields: HashMap<String, Vec<(String, Ty)>>,
    /// `computed` record properties (BACKLOG item 143): type_name →
    /// [(computed_name, declared_type)]. Deliberately kept separate from
    /// `record_fields` rather than merged in — a computed name has a type
    /// for field-*access* purposes but is never a real stored field, so a
    /// record literal (`TypeName { ... }`) or `.with(...)` copy-update
    /// (BACKLOG item 151) must keep rejecting it exactly as before; only
    /// `resolve_field_ty`'s read-access lookup checks this map, as a
    /// fallback after `record_fields` itself misses.
    pub computed_fields: HashMap<String, Vec<(String, Ty)>>,
    /// A type's own declared type-parameter vars: type_name → [TyVar, ...],
    /// in declaration order — the *same* vars `record_fields`'s stored field
    /// types (for a generic record) reference. A `TypeName { field: val }`
    /// literal must instantiate these fresh per occurrence (mirroring how
    /// `Ty::Forall::instantiate` works for a generic *function* reference)
    /// before unifying field values against them, and must report the
    /// literal's own inferred type as `Named { name, args: <those vars> }`
    /// — not a bare, argument-less `Named`, which silently discarded the
    /// concrete instantiation entirely and made every generic record type
    /// unusable together with an explicit type annotation anywhere.
    pub type_param_vars: HashMap<String, Vec<TyVar>>,
    /// Row-polymorphism bounds on generic functions: fn_name → [(the bound type
    /// param's original quantified TyVar, its required [(field_name, field_type)])].
    /// Populated during hoisting (see `infer_decl::hoist_decl`); checked at each
    /// call site once the function's type params are instantiated and the call's
    /// arguments are unified (see `infer_expr`'s `Expr::App` handling).
    pub row_bounds: HashMap<String, Vec<(TyVar, Vec<(String, Ty)>)>>,
    /// Sum type variant names: type_name → [variant_name, ...], in declaration
    /// order. Used for match exhaustiveness checking.
    pub sum_variants: HashMap<String, Vec<String>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            frames: vec![HashMap::new()],
            constructor_vars: HashSet::new(),
            param_meta: HashMap::new(),
            record_fields: HashMap::new(),
            computed_fields: HashMap::new(),
            type_param_vars: HashMap::new(),
            row_bounds: HashMap::new(),
            sum_variants: HashMap::new(),
        }
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

    /// Every name currently defined in any frame. Used to hand the same set
    /// of stdlib/builtin names typeck knows about to other passes (e.g.
    /// `certo_resolve`) without hand-maintaining a second list.
    pub fn names(&self) -> Vec<String> {
        self.frames.iter().flat_map(|f| f.keys().cloned()).collect()
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

    /// Register a function's row-polymorphism bounds (see `row_bounds` field doc).
    /// A no-op when `bounds` is empty, so callers can call this unconditionally.
    pub fn define_row_bounds(&mut self, fn_name: impl Into<String>, bounds: Vec<(TyVar, Vec<(String, Ty)>)>) {
        if !bounds.is_empty() {
            self.row_bounds.insert(fn_name.into(), bounds);
        }
    }

    /// Retrieve a function's row-polymorphism bounds, if it has any.
    pub fn get_row_bounds(&self, fn_name: &str) -> Option<&Vec<(TyVar, Vec<(String, Ty)>)>> {
        self.row_bounds.get(fn_name)
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

        // Some :: ∀a. a → Option<a>
        // None :: ∀a. Option<a>
        {
            *counter += 1; let a = *counter;
            self.define("Some", Ty::Forall {
                vars: vec![a],
                body: Box::new(Ty::Fn { params: vec![Ty::Var(a)], ret: Box::new(Ty::Option(Box::new(Ty::Var(a)))) }),
            });
        }
        {
            *counter += 1; let a = *counter;
            self.define("None", Ty::Forall {
                vars: vec![a],
                body: Box::new(Ty::Option(Box::new(Ty::Var(a)))),
            });
        }

        // Duration constructors: Duration.days / .hours / .minutes / .seconds :: Int → Duration
        {
            let dur = Ty::Named { name: "Duration".into(), args: vec![] };
            for method in &["days", "hours", "minutes", "seconds", "milliseconds"] {
                self.define(
                    format!("Duration.{}", method),
                    Ty::Fn { params: vec![Ty::Int], ret: Box::new(dur.clone()) },
                );
            }
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
