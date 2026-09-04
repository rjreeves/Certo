use std::collections::{HashMap, HashSet};
use certo_ast::types::TypeExpr;
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
    /// Row bounds for the function/method body *currently being checked*
    /// (BACKLOG item 257), keyed by the **fresh** `TyVar` `check_decl`
    /// allocates for each type param right before checking that body —
    /// deliberately *not* `row_bounds` above, whose own keys are the
    /// hoisting pass's own, entirely different `TyVar`s (`hoist_decl`
    /// allocates one set of vars to build the function's registered
    /// `Ty::Forall` signature for callers; body-checking allocates a brand
    /// new, unrelated set via `ctx.fresh()` for checking the body itself —
    /// confirmed directly these never correspond). Each entry also carries
    /// the type param's own source name (`"R"`), purely so a rejected
    /// out-of-bound field access can name it in a real diagnostic instead of
    /// an internal `?t7`-style var id. Set right after `check_decl`
    /// allocates a function's fresh type-param vars, cleared once its body
    /// is fully checked; consulted by `resolve_field_ty`'s own `Ty::Var(v)`
    /// arm so a field access on a row-bound param is validated against the
    /// bound instead of always returning an unconstrained fresh var,
    /// regardless of whether the field was ever declared in it.
    current_body_row_bounds: HashMap<TyVar, (String, Vec<(String, Ty)>)>,
    /// Trait method signatures (BACKLOG item 309): trait_name → method_name
    /// → (declared param type exprs *excluding* the receiver/`self` param,
    /// declared return type expr, or `None` for an inferred/`Unit` return).
    /// Kept as raw, unconverted `TypeExpr`s (not `Ty`) rather than eagerly
    /// resolved — a trait method's own signature can reference `Self`
    /// (`fn toJson(self): Self`, say), which only has a *concrete* meaning
    /// once substituted with whichever specific type param a given generic
    /// function's own bound instantiates it to; converting eagerly at
    /// registration time (before any such bound exists) would have nothing
    /// correct to substitute `Self` with. Populated once, during hoisting,
    /// from every `Decl::Trait` in the module (traits themselves are never
    /// otherwise hoisted by typeck at all — only concrete `impl` blocks
    /// are — confirmed by grep before this item).
    pub trait_defs: HashMap<String, HashMap<String, (Vec<TypeExpr>, Option<TypeExpr>)>>,
    /// Trait bounds for the function/method body *currently being checked*
    /// (BACKLOG item 309), keyed by the same *fresh* body-check `TyVar`
    /// `current_body_row_bounds` is keyed by — see that field's own doc
    /// comment for why body-checking's fresh vars, not hoisting's. Each
    /// entry: the type param's own source name (for diagnostics) plus the
    /// *merged* method table across every trait bound on it (`T: A + B`
    /// exposes both `A`'s and `B`'s methods), with any `Self` in a method's
    /// own signature already substituted for this specific `TyVar` — so a
    /// hit here resolves directly to a concrete, callable `Ty::Fn`, no
    /// further substitution needed at the call site.
    current_body_trait_bounds: HashMap<TyVar, (String, HashMap<String, (Vec<Ty>, Ty)>)>,
    /// Sum type variant names: type_name → [variant_name, ...], in declaration
    /// order. Used for match exhaustiveness checking.
    pub sum_variants: HashMap<String, Vec<String>>,
    /// Real `type X = Y` type aliases (spec §3.3, BACKLOG item 281):
    /// alias_name → (its own declared type-param names, in order; the raw
    /// target `TypeExpr` it stands for). Populated during hoisting
    /// (`hoist_decl`'s `Decl::Type` arm, `TypeBody::Alias` case) — flat, not
    /// scope-stacked like `frames`, since a type alias is always module-level,
    /// same as `record_fields`/`sum_variants`. Consulted by
    /// `type_expr_to_ty`'s `Named` catch-all so a reference to the alias
    /// expands to its real target type instead of becoming an opaque,
    /// unrelated nominal `Ty::Named` (the bug this item fixes).
    pub type_aliases: HashMap<String, (Vec<String>, TypeExpr)>,
    /// Alias names currently being expanded, used only as a cycle guard by
    /// `type_expr_to_ty`'s alias-expansion path — a self-referential alias
    /// (`type A = A`, or `type A = B; type B = A`) must be a bounded error,
    /// not an infinite recursion / stack overflow.
    pub(crate) alias_expand_stack: Vec<String>,
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
            current_body_row_bounds: HashMap::new(),
            trait_defs: HashMap::new(),
            current_body_trait_bounds: HashMap::new(),
            sum_variants: HashMap::new(),
            type_aliases: HashMap::new(),
            alias_expand_stack: Vec::new(),
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

    /// Set the row bounds in scope for the body currently being checked
    /// (BACKLOG item 257) — see `current_body_row_bounds`'s own doc comment.
    /// Replaces whatever was set for the previous body outright (bodies are
    /// checked one at a time, never nested), rather than merging.
    pub fn set_current_body_row_bounds(&mut self, bounds: HashMap<TyVar, (String, Vec<(String, Ty)>)>) {
        self.current_body_row_bounds = bounds;
    }

    /// Clear the current body's row bounds once it's done being checked, so
    /// a *different* function's field access can never accidentally consult
    /// a stale entry (harmless in practice, since `TyVar`s are never reused
    /// across functions either, but keeps the invariant explicit).
    pub fn clear_current_body_row_bounds(&mut self) {
        self.current_body_row_bounds.clear();
    }

    /// Look up the type param's own name and required fields for `var`, if
    /// it's a row-bound type param of the function/method body currently
    /// being checked.
    pub fn lookup_current_body_row_bound(&self, var: TyVar) -> Option<&(String, Vec<(String, Ty)>)> {
        self.current_body_row_bounds.get(&var)
    }

    /// Set the trait bounds in scope for the body currently being checked
    /// (BACKLOG item 309) — see `current_body_trait_bounds`'s own doc
    /// comment. Same replace-outright convention as its row-bound sibling.
    pub fn set_current_body_trait_bounds(&mut self, bounds: HashMap<TyVar, (String, HashMap<String, (Vec<Ty>, Ty)>)>) {
        self.current_body_trait_bounds = bounds;
    }

    /// Clear the current body's trait bounds once it's done being checked.
    pub fn clear_current_body_trait_bounds(&mut self) {
        self.current_body_trait_bounds.clear();
    }

    /// Look up the type param's own name and merged method table for `var`,
    /// if it's a trait-bound type param of the function/method body
    /// currently being checked.
    pub fn lookup_current_body_trait_bound(&self, var: TyVar) -> Option<&(String, HashMap<String, (Vec<Ty>, Ty)>)> {
        self.current_body_trait_bounds.get(&var)
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

        // panic :: ∀a. Text -> a
        {
            *counter += 1;
            let ret = Ty::Var(*counter);
            self.define("panic", Ty::Forall {
                vars: vec![*counter],
                body: Box::new(Ty::Fn { params: vec![Ty::Text], ret: Box::new(ret) }),
            });
        }

        // unreachable / todo :: ∀a. () -> a — BACKLOG item 210. Previously
        // registered identically to `panic` (`Text -> a`), but the spec's
        // own table (§9.1) documents both as zero-arg (`fn(): Nothing`),
        // matching what `crates/codegen/src/emit_module.rs` already emits
        // (`#define certo_unreachable() certo_panic("unreachable")` —
        // a zero-arg C macro). The 1-arg typeck signature meant the *only*
        // call form that typechecked (`unreachable("msg")`) failed to
        // compile ("too many arguments provided to function-like macro
        // invocation"), while the form that *would* compile (`unreachable()`)
        // never passed typeck.
        for name in &["unreachable", "todo"] {
            *counter += 1;
            let ret = Ty::Var(*counter);
            self.define(*name, Ty::Forall {
                vars: vec![*counter],
                body: Box::new(Ty::Fn { params: vec![], ret: Box::new(ret) }),
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
