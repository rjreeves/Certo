use std::collections::HashMap;

/// A unique identifier for a type variable.
pub type TyVar = u32;

/// The type language used during inference.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    // ---------------------------------------------------------------- //
    // Primitives
    // ---------------------------------------------------------------- //
    Int,    // Int64 — default integer type
    Int8,
    Int16,
    Int32,
    UInt,   // UInt64
    Float,
    Float32,
    /// `Decimal` (unparameterized, `None`) or `Decimal(p, s)` (`Some((p, s))`).
    /// Always the same runtime representation (`certo_decimal_t`) either way —
    /// the parameter is a compile-time-only refinement, primarily for
    /// checking fidelity against a `NUMERIC(p,s)` database column (see
    /// `certo_dbschema`), not a new runtime type. See `unify.rs`: a bare
    /// `Decimal` unifies with any `Decimal(p,s)`; two different `Some(..)`
    /// parameterizations do not unify with each other.
    Decimal(Option<(u8, u8)>),
    Bool,
    Char,   // single ASCII byte — same byte-oriented convention as Text
    Text,
    Unit,
    Uuid,

    // ---------------------------------------------------------------- //
    // Compound
    // ---------------------------------------------------------------- //

    /// `Option<T>`
    Option(Box<Ty>),

    /// `Result<T, E>`
    Result(Box<Ty>, Box<Ty>),

    /// `List<T>`
    List(Box<Ty>),

    /// `Map<K, V>`
    Map(Box<Ty>, Box<Ty>),

    /// `(T1, T2, ...)`
    Tuple(Vec<Ty>),

    /// A named type with arguments: `Order`, `Result<T, E>`, `MyType<A>`
    Named {
        name: String,
        args: Vec<Ty>,
    },

    /// `{ field: T, ... }` — record type
    Record(Vec<(String, Ty)>),

    /// `A -> B` — function type (may be multi-param)
    Fn {
        params: Vec<Ty>,
        ret:    Box<Ty>,
    },

    // ---------------------------------------------------------------- //
    // Inference machinery
    // ---------------------------------------------------------------- //

    /// An unresolved type variable — filled in by unification.
    Var(TyVar),

    /// A generalised type variable in a polytype (quantified).
    /// After generalisation `∀a. a -> a` becomes `Forall([0], Fn([Var(0)], Var(0)))`.
    Forall {
        vars: Vec<TyVar>,
        body: Box<Ty>,
    },

    /// Placeholder for a type that failed to infer — suppresses cascading errors.
    Error,
}

impl Ty {
    /// Instantiate a `Forall` by replacing each quantified var with a fresh
    /// inference variable, using the provided counter.
    pub fn instantiate(&self, counter: &mut u32) -> Ty {
        self.instantiate_with_subst(counter).0
    }

    /// Same as `instantiate`, but also returns the substitution map (original
    /// quantified `TyVar` → fresh `Ty::Var`) — needed by callers that must track
    /// which fresh variable a given bound type parameter became, e.g. to check a
    /// row-polymorphism bound against whatever that variable resolves to after
    /// the call's arguments are unified.
    pub fn instantiate_with_subst(&self, counter: &mut u32) -> (Ty, HashMap<TyVar, Ty>) {
        match self {
            Ty::Forall { vars, body } => {
                let mut subst: HashMap<TyVar, Ty> = HashMap::new();
                for &v in vars {
                    *counter += 1;
                    subst.insert(v, Ty::Var(*counter));
                }
                (body.apply_subst(&subst), subst)
            }
            other => (other.clone(), HashMap::new()),
        }
    }

    /// Apply a substitution map (TyVar → Ty) to this type.
    pub fn apply_subst(&self, subst: &HashMap<TyVar, Ty>) -> Ty {
        match self {
            Ty::Var(v) => subst.get(v).cloned().unwrap_or(Ty::Var(*v)),

            Ty::Option(t)     => Ty::Option(Box::new(t.apply_subst(subst))),
            Ty::Result(t, e)  => Ty::Result(Box::new(t.apply_subst(subst)), Box::new(e.apply_subst(subst))),
            Ty::List(t)       => Ty::List(Box::new(t.apply_subst(subst))),
            Ty::Map(k, v)     => Ty::Map(Box::new(k.apply_subst(subst)), Box::new(v.apply_subst(subst))),
            Ty::Tuple(ts)     => Ty::Tuple(ts.iter().map(|t| t.apply_subst(subst)).collect()),

            Ty::Named { name, args } => Ty::Named {
                name: name.clone(),
                args: args.iter().map(|a| a.apply_subst(subst)).collect(),
            },

            Ty::Record(fields) => Ty::Record(
                fields.iter().map(|(n, t)| (n.clone(), t.apply_subst(subst))).collect()
            ),

            Ty::Fn { params, ret } => Ty::Fn {
                params: params.iter().map(|p| p.apply_subst(subst)).collect(),
                ret:    Box::new(ret.apply_subst(subst)),
            },

            Ty::Forall { vars, body } => {
                // Don't substitute over bound vars
                let mut inner = subst.clone();
                for v in vars { inner.remove(v); }
                Ty::Forall { vars: vars.clone(), body: Box::new(body.apply_subst(&inner)) }
            }

            // Ground types and Error pass through unchanged
            other => other.clone(),
        }
    }

    /// Collect all free type variables in this type.
    pub fn free_vars(&self) -> Vec<TyVar> {
        let mut out = Vec::new();
        self.collect_free(&mut out);
        out.sort_unstable();
        out.dedup();
        out
    }

    fn collect_free(&self, acc: &mut Vec<TyVar>) {
        match self {
            Ty::Var(v)          => acc.push(*v),
            Ty::Option(t)       => t.collect_free(acc),
            Ty::Result(t, e)    => { t.collect_free(acc); e.collect_free(acc); }
            Ty::List(t)         => t.collect_free(acc),
            Ty::Map(k, v)       => { k.collect_free(acc); v.collect_free(acc); }
            Ty::Tuple(ts)       => ts.iter().for_each(|t| t.collect_free(acc)),
            Ty::Named { args, .. } => args.iter().for_each(|a| a.collect_free(acc)),
            Ty::Record(fields)  => fields.iter().for_each(|(_, t)| t.collect_free(acc)),
            Ty::Fn { params, ret } => {
                params.iter().for_each(|p| p.collect_free(acc));
                ret.collect_free(acc);
            }
            Ty::Forall { vars, body } => {
                body.collect_free(acc);
                acc.retain(|v| !vars.contains(v));
            }
            _ => {}
        }
    }

    /// Pretty-print for error messages (raw — shows `?t3` for unresolved vars).
    pub fn display(&self) -> String {
        self.display_named(&HashMap::new())
    }

    /// Pretty-print with a var-name map.  Any `Var(n)` whose id is in `names`
    /// is shown as the mapped letter; unmapped vars fall back to `?t{n}`.
    pub fn display_named(&self, names: &HashMap<TyVar, String>) -> String {
        match self {
            Ty::Int     => "Int".into(),
            Ty::Int8    => "Int8".into(),
            Ty::Int16   => "Int16".into(),
            Ty::Int32   => "Int32".into(),
            Ty::UInt    => "UInt".into(),
            Ty::Float   => "Float".into(),
            Ty::Float32 => "Float32".into(),
            Ty::Decimal(None)          => "Decimal".into(),
            Ty::Decimal(Some((p, s))) => format!("Decimal({}, {})", p, s),
            Ty::Bool    => "Bool".into(),
            Ty::Char    => "Char".into(),
            Ty::Text    => "Text".into(),
            Ty::Unit    => "Unit".into(),
            Ty::Uuid    => "UUID".into(),
            Ty::Option(t)    => format!("{}?", t.display_named(names)),
            Ty::Result(t, e) => format!("Result<{}, {}>",
                t.display_named(names), e.display_named(names)),
            Ty::List(t)      => format!("List<{}>", t.display_named(names)),
            Ty::Map(k, v)    => format!("Map<{}, {}>",
                k.display_named(names), v.display_named(names)),
            Ty::Tuple(ts)    => format!("({})",
                ts.iter().map(|t| t.display_named(names)).collect::<Vec<_>>().join(", ")),
            Ty::Named { name, args } if args.is_empty() => name.clone(),
            Ty::Named { name, args } => format!("{}<{}>", name,
                args.iter().map(|a| a.display_named(names)).collect::<Vec<_>>().join(", ")),
            Ty::Record(fields) => {
                let fs = fields.iter()
                    .map(|(n, t)| format!("{}: {}", n, t.display_named(names)))
                    .collect::<Vec<_>>().join(", ");
                format!("{{ {} }}", fs)
            }
            Ty::Fn { params, ret } => {
                let ps = params.iter().map(|p| p.display_named(names)).collect::<Vec<_>>().join(", ");
                format!("({}) => {}", ps, ret.display_named(names))
            }
            Ty::Var(v) => names.get(v)
                .cloned()
                .unwrap_or_else(|| format!("?t{}", v)),
            Ty::Forall { vars, body } => {
                let vs = vars.iter()
                    .map(|v| names.get(v).cloned().unwrap_or_else(|| format!("t{}", v)))
                    .collect::<Vec<_>>().join(", ");
                format!("∀{}. {}", vs, body.display_named(names))
            }
            Ty::Error => "<error>".into(),
        }
    }

    /// Returns true if this type contains any unresolved inference variables.
    pub fn has_vars(&self) -> bool {
        !self.free_vars().is_empty()
    }

    /// Returns true if this is a function type.
    pub fn is_fn(&self) -> bool {
        matches!(self, Ty::Fn { .. })
    }
}

/// Collect all free vars from a slice of types and assign them short readable
/// names: `T`, `U`, `V`, `W`, `A`, `B`, `C`, …
/// Vars are assigned in order of first appearance (left-to-right across all tys).
pub fn assign_var_names(tys: &[&Ty]) -> HashMap<TyVar, String> {
    const NAMES: &[&str] = &[
        "T", "U", "V", "W", "A", "B", "C", "D", "E", "F",
        "G", "H", "I", "J", "K", "L", "M", "N", "P", "Q",
    ];
    let mut seen: Vec<TyVar> = Vec::new();
    for ty in tys {
        for v in ty.free_vars() {
            if !seen.contains(&v) { seen.push(v); }
        }
    }
    seen.into_iter().enumerate()
        .map(|(i, v)| {
            let name = NAMES.get(i).copied().unwrap_or("?").to_string();
            (v, name)
        })
        .collect()
}
