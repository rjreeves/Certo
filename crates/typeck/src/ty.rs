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

    // ---------------------------------------------------------------- //
    // Higher-kinded types (BACKLOG item 76) — deliberately minimal: Certo
    // has no general kind system, only enough to let a 1-ary type
    // constructor (`Box<T>`, `List<T>`, `Option<T>`, a user's own
    // single-param generic type) be *itself* a type parameter, e.g.
    // `fn map<F<_>, A, B>(fa: F<A>, ...)`. A 2-ary type (`Map<K,V>`,
    // `Result<A,E>`) can never satisfy an `F<_>` parameter — there is no
    // partial-application/currying support, unlike a real kind system.
    // ---------------------------------------------------------------- //

    /// A bare, unapplied type constructor by name (`Box`, `List`, `Option`,
    /// ...) — the value a constructor-kind type variable (`Ty::Var` bound
    /// via an `F<_>` parameter) resolves to once its *identity* is known,
    /// before its one argument is known. Purely an internal unification
    /// intermediate — never appears in surface syntax and never survives
    /// into a fully-resolved type (`App(Ctor(name), arg)` always collapses
    /// to `Named { name, args: [arg] }`, or the builtin equivalent, the
    /// moment both sides are known — see `apply_subst`).
    Ctor(String),

    /// A type constructor applied to one argument: `F<A>` where `F` is
    /// itself potentially still an unresolved type variable. Unifies
    /// against a concrete single-argument type (`Named{args:[_]}`,
    /// `List(_)`, `Option(_)`) by binding the constructor position to a
    /// `Ctor` recovered from that type's own name, exactly like any other
    /// type-variable binding — see `unify.rs`.
    App(Box<Ty>, Box<Ty>),
}

impl Ty {
    /// Does this type need real heap allocation (malloc + copy) when boxed
    /// into a pointer-sized slot (a `List<T>`/`Tuple` element, an explicit
    /// box/unbox, or a generic `void* (*)(void*)`-style callback parameter
    /// or return), rather than a cheap same-width cast? True for any type
    /// whose C representation (`crates/codegen/src/ty_to_c.rs`) is a real,
    /// value-sized struct rather than an already-pointer-or-scalar-sized
    /// type — shared between `crates/mir` and `crates/codegen` (both need
    /// the identical answer, e.g. for `List.getOrPanic`'s raw-`void*`
    /// return vs. a boxed one) so it lives here on `Ty` itself rather than
    /// risking two independently-maintained copies drifting apart — BACKLOG
    /// item 134.
    ///
    /// `Decimal` (`certo_decimal_t = { int64_t value; int8_t scale; }`, 16
    /// bytes with padding) and `UUID` (`certo_uuid_t = struct { uint8_t
    /// bytes[16]; }`) both need this — previously `Decimal` was bit-cast
    /// like an 8-byte `double` and `UUID` went through the same raw
    /// pointer-cast as everything else, both wrong for a 16-byte struct.
    pub fn needs_heap_box(&self) -> bool {
        match self {
            Ty::Decimal(_) | Ty::Uuid => true,
            // A function value is a 2-word `{fn, env}` closure struct
            // (BACKLOG item 140), not pointer-sized — must be heap-boxed
            // like any other multi-word value flowing through a generic
            // void* slot (list/tuple element, Option/Result payload).
            Ty::Fn { .. } => true,
            Ty::Named { name, args } => {
                // Mirrors `ty_to_c`'s own opaque-handle carve-outs exactly —
                // those compile to `int64_t`/`void*` already and need no
                // boxing; every other `Named` (a user's own `type`/generic
                // record or sum type, or a multi-arg generic like `Box<T>`)
                // falls to `ty_to_c`'s catch-all, a real, named C struct.
                let opaque_handle = (args.is_empty() && matches!(name.as_str(),
                    "HttpRequest" | "HttpResponse" | "Bytes" | "DbResult" | "Query" | "Mutation"
                    | "DateTime" | "Date" | "Duration" | "JsonValue" | "Timezone" | "ProcessResult"))
                    || name == "__CertoTask" || name == "Channel";
                !opaque_handle
            }
            _ => false,
        }
    }

    /// Does this type structurally contain `Secret<_>` anywhere — itself,
    /// or nested inside an `Option`/`Result`/`List`/`Map`/`Tuple`/`Record`/
    /// another `Named`'s own type args? Used to reject passing a secret
    /// (however deeply wrapped) to a logging/serialization sink — BACKLOG
    /// item 78. `Secret<T>` isn't a dedicated `Ty::` variant (no such
    /// builtin exists — a user declares `type Secret<T> = priv Secret(T)`
    /// themselves, the same generic-sum-type shape item 135 already proved
    /// out via `Box<T>`), so this matches by name on `Ty::Named` rather than
    /// a dedicated pattern.
    pub fn contains_secret(&self) -> bool {
        match self {
            Ty::Named { name, args } => name == "Secret" || args.iter().any(Ty::contains_secret),
            Ty::Option(inner) => inner.contains_secret(),
            Ty::Result(ok, err) => ok.contains_secret() || err.contains_secret(),
            Ty::List(inner) => inner.contains_secret(),
            Ty::Map(k, v) => k.contains_secret() || v.contains_secret(),
            Ty::Tuple(elems) => elems.iter().any(Ty::contains_secret),
            Ty::Record(fields) => fields.iter().any(|(_, t)| t.contains_secret()),
            _ => false,
        }
    }

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
    ///
    /// A `Ty::Var(v)` entry recurses into whatever `v` maps to, rather than
    /// stopping after one lookup — `subst` (built from the union-find's own
    /// binding map) routinely chains a var to *another* var that only
    /// resolves further on a second hop (e.g. a generic call's own `T`
    /// getting unified through a synthesized "expected function type"'s
    /// fresh return var before that var is separately bound to the real
    /// argument type). A single-hop lookup left such a var only
    /// half-resolved, which `TypeEnv::generalise` then misread as "still
    /// free" and wrongly wrapped in a `Forall` — silently re-instantiating
    /// a *fresh*, unconstrained variable on every later reference to a
    /// `val`, discarding the real type entirely. Confirmed as a genuine,
    /// pre-existing soundness gap by direct testing (not theorized): with
    /// only the one-hop lookup, `Box<Text>` passed where a `Box<Int>`
    /// parameter was declared type-checked clean with no error at all.
    /// Occurs-check (`UnionFind::occurs_check`) already guarantees `subst`
    /// contains no cycles, so this recursion is guaranteed to terminate.
    pub fn apply_subst(&self, subst: &HashMap<TyVar, Ty>) -> Ty {
        match self {
            Ty::Var(v) => match subst.get(v) {
                Some(next) => next.apply_subst(subst),
                None => Ty::Var(*v),
            },

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

            // Once the constructor position resolves to a concrete `Ctor`,
            // collapse into an ordinary fully-applied type — the whole
            // point of `App` is to be a transient intermediate, never a
            // final answer (BACKLOG item 76).
            Ty::App(f, a) => {
                let f = f.apply_subst(subst);
                let a = a.apply_subst(subst);
                match f {
                    Ty::Ctor(name) => match name.as_str() {
                        "List"   => Ty::List(Box::new(a)),
                        "Option" => Ty::Option(Box::new(a)),
                        _        => Ty::Named { name, args: vec![a] },
                    },
                    f => Ty::App(Box::new(f), Box::new(a)),
                }
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
            Ty::App(f, a) => { f.collect_free(acc); a.collect_free(acc); }
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
            Ty::Ctor(name) => name.clone(),
            Ty::App(f, a) => format!("{}<{}>", f.display_named(names), a.display_named(names)),
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

#[cfg(test)]
mod contains_secret_tests {
    use super::Ty;

    #[test]
    fn direct_secret_is_detected() {
        let ty = Ty::Named { name: "Secret".into(), args: vec![Ty::Text] };
        assert!(ty.contains_secret());
    }

    #[test]
    fn unrelated_named_type_is_not_detected() {
        let ty = Ty::Named { name: "Box".into(), args: vec![Ty::Text] };
        assert!(!ty.contains_secret());
    }

    #[test]
    fn plain_scalar_is_not_detected() {
        assert!(!Ty::Text.contains_secret());
        assert!(!Ty::Int.contains_secret());
    }

    #[test]
    fn nested_inside_option_list_record_is_detected() {
        let secret = Ty::Named { name: "Secret".into(), args: vec![Ty::Text] };
        assert!(Ty::Option(Box::new(secret.clone())).contains_secret());
        assert!(Ty::List(Box::new(secret.clone())).contains_secret());
        assert!(Ty::Record(vec![("password".into(), secret.clone())]).contains_secret());
        assert!(Ty::Tuple(vec![Ty::Int, secret.clone()]).contains_secret());
    }

    #[test]
    fn nested_inside_another_generic_wrapper_is_detected() {
        // Secret<T> wrapped inside a second, unrelated generic wrapper
        // (Box<Secret<Text>>) must still be caught via the wrapper's own
        // type args, not just a top-level match.
        let secret = Ty::Named { name: "Secret".into(), args: vec![Ty::Text] };
        let boxed = Ty::Named { name: "Box".into(), args: vec![secret] };
        assert!(boxed.contains_secret());
    }
}

#[cfg(test)]
mod apply_subst_chain_tests {
    use super::Ty;
    use std::collections::HashMap;

    #[test]
    fn single_hop_still_resolves() {
        let mut subst = HashMap::new();
        subst.insert(1u32, Ty::Text);
        assert_eq!(Ty::Var(1).apply_subst(&subst), Ty::Text);
    }

    #[test]
    fn two_hop_chain_resolves_to_the_concrete_type() {
        // Var(1) -> Var(2) -> Text: a single-hop lookup would stop at
        // Var(2), which is exactly the bug this fix closes (confirmed via
        // direct testing: it let `Box<Text>` satisfy a `Box<Int>` parameter
        // with no type error at all).
        let mut subst = HashMap::new();
        subst.insert(1u32, Ty::Var(2));
        subst.insert(2u32, Ty::Text);
        assert_eq!(Ty::Var(1).apply_subst(&subst), Ty::Text);
    }

    #[test]
    fn chain_resolves_when_nested_inside_a_compound_type() {
        // The same chain, but as a generic wrapper's own type argument
        // (Box<Var(1)> where Var(1) -> Var(2) -> Int) — matches how this
        // bug actually manifested: a call's synthesized return-type var
        // chained through an intermediate fresh var before reaching the
        // real argument type.
        let mut subst = HashMap::new();
        subst.insert(1u32, Ty::Var(2));
        subst.insert(2u32, Ty::Int);
        let boxed = Ty::Named { name: "Box".into(), args: vec![Ty::Var(1)] };
        assert_eq!(boxed.apply_subst(&subst), Ty::Named { name: "Box".into(), args: vec![Ty::Int] });
    }

    #[test]
    fn three_hop_chain_also_resolves() {
        let mut subst = HashMap::new();
        subst.insert(1u32, Ty::Var(2));
        subst.insert(2u32, Ty::Var(3));
        subst.insert(3u32, Ty::Bool);
        assert_eq!(Ty::Var(1).apply_subst(&subst), Ty::Bool);
    }

    #[test]
    fn unbound_var_stays_a_var() {
        let subst: HashMap<u32, Ty> = HashMap::new();
        assert_eq!(Ty::Var(1).apply_subst(&subst), Ty::Var(1));
    }

    #[test]
    fn chain_ending_in_an_unbound_var_stays_that_var() {
        let mut subst = HashMap::new();
        subst.insert(1u32, Ty::Var(2));
        // Var(2) is not in subst — the chain should stop there, not panic
        // or loop.
        assert_eq!(Ty::Var(1).apply_subst(&subst), Ty::Var(2));
    }
}
