use std::collections::HashMap;
use certo_ast::span::Span;
use crate::ty::{Ty, TyVar};
use crate::error::{TypeError, TypeErrorKind};

/// Union-find based substitution table.
/// Maps a TyVar to either another TyVar (redirect) or a concrete Ty.
#[derive(Default)]
pub struct UnionFind {
    map: HashMap<TyVar, Ty>,
}

impl UnionFind {
    /// Walk the chain until we reach a type that is not just a Var redirect.
    pub fn find(&self, mut ty: Ty) -> Ty {
        loop {
            match ty {
                Ty::Var(v) => match self.map.get(&v) {
                    Some(next) => ty = next.clone(),
                    None       => return Ty::Var(v),
                },
                other => return other,
            }
        }
    }

    /// Bind a free TyVar to a type.  Panics if the var is already bound.
    fn bind(&mut self, v: TyVar, ty: Ty) {
        debug_assert!(!self.map.contains_key(&v));
        self.map.insert(v, ty);
    }

    /// Apply everything we know to a type.
    ///
    /// BACKLOG item 327 — this used to clone the *entire* substitution map
    /// (`self.map.clone()`) on every single call, via a since-removed
    /// `to_subst()` helper, even though `apply_subst` only ever needs a
    /// borrowed `&HashMap` (it recurses purely by reference, never mutates
    /// or takes ownership — confirmed by reading its own body). For a large,
    /// real, internally cross-referencing program, `apply` is called tens
    /// of thousands of times per declaration (once per identifier/argument/
    /// field touched during inference) against a substitution map that only
    /// ever grows over the whole module — so the wasted clone's cost
    /// compounded into the cubic-to-quartic scaling this item found and
    /// filed, confirmed live via instrumentation before this fix (one
    /// single declaration in a real ~250-declaration file cost nearly 5s
    /// and 35,000+ `apply` calls against a ~4,000-entry map, almost all of
    /// it spent cloning that map over and over for calls that never
    /// mutated it).
    pub fn apply(&self, ty: &Ty) -> Ty {
        let walked = self.find(ty.clone());
        walked.apply_subst(&self.map)
    }

    /// Unify two types, recording the necessary bindings.
    /// Returns `Err` for type mismatches or occurs-check failures.
    ///
    /// BACKLOG item 196 — the two parameters are named, not just positional,
    /// specifically because the overwhelming majority of the ~50 call sites
    /// across `crates/typeck` were found passing (actual value's type,
    /// declared/required type) in that order, but every `Mismatch`
    /// construction below used to unconditionally label the *first*
    /// argument "expected" — silently swapping every basic type-error
    /// message's `expected`/`found` fields backwards. Renaming the params
    /// to match what call sites already pass fixes ~44 of them for free,
    /// with zero call-site changes; only the couple of outliers that
    /// genuinely passed (expected, actual) needed their own two arguments
    /// swapped to match this now-explicit convention.
    pub fn unify(&mut self, found: Ty, expected: Ty, span: Span) -> Result<(), TypeError> {
        let found = self.find(found);
        let expected = self.find(expected);

        match (found, expected) {
            // Same var — nothing to do.
            (Ty::Var(u), Ty::Var(v)) if u == v => Ok(()),

            // Bind var to type.
            (Ty::Var(v), ty) | (ty, Ty::Var(v)) => {
                self.occurs_check(v, &ty, span)?;
                self.bind(v, ty);
                Ok(())
            }

            // Error propagates silently.
            (Ty::Error, _) | (_, Ty::Error) => Ok(()),

            // Primitives — structural equality.
            (Ty::Int,     Ty::Int)     => Ok(()),
            (Ty::Int8,    Ty::Int8)    => Ok(()),
            (Ty::Int16,   Ty::Int16)   => Ok(()),
            (Ty::Int32,   Ty::Int32)   => Ok(()),
            (Ty::UInt,    Ty::UInt)    => Ok(()),
            (Ty::Float,   Ty::Float)   => Ok(()),
            (Ty::Float32, Ty::Float32) => Ok(()),
            // A bare `Decimal` unifies with any `Decimal(p,s)` (same runtime
            // representation, the parameter is purely a compile-time
            // refinement — see `Ty::Decimal`'s doc comment). Two *different*
            // parameterizations don't unify with each other; that falls
            // through to the generic mismatch arm below.
            (Ty::Decimal(a), Ty::Decimal(b)) if a.is_none() || b.is_none() || a == b => Ok(()),
            (Ty::Bool,    Ty::Bool)    => Ok(()),
            (Ty::Char,    Ty::Char)    => Ok(()),
            (Ty::Text,    Ty::Text)    => Ok(()),
            // `BoundedText(n)` (BACKLOG item 147) mirrors `Decimal(p,s)`'s
            // own two rules exactly, just across a separate `Ty` variant
            // instead of a parameterized `Text` itself (see `Ty::BoundedText`'s
            // doc comment for why): two different max lengths don't unify
            // with each other (falls through to the mismatch arm below), and
            // — since it's a distinct variant from `Text`, unlike `Decimal`'s
            // self-parameterization — it also needs an explicit cross-variant
            // rule so a bounded value stays usable anywhere plain `Text` is
            // expected (and vice versa), matching the spec's own newtype
            // usage (`type Email = Email(BoundedText(255))`, passed around
            // just like any other `Text`-shaped value).
            (Ty::BoundedText(a), Ty::BoundedText(b)) if a.is_none() || b.is_none() || a == b => Ok(()),
            (Ty::Text, Ty::BoundedText(_)) | (Ty::BoundedText(_), Ty::Text) => Ok(()),
            (Ty::Unit,    Ty::Unit)    => Ok(()),
            (Ty::Uuid,    Ty::Uuid)    => Ok(()),

            // Compound — recurse.
            (Ty::Option(a), Ty::Option(b)) => self.unify(*a, *b, span),

            (Ty::Result(at, ae), Ty::Result(bt, be)) => {
                self.unify(*at, *bt, span)?;
                self.unify(*ae, *be, span)
            }

            (Ty::List(a), Ty::List(b)) => self.unify(*a, *b, span),

            (Ty::Map(ak, av), Ty::Map(bk, bv)) => {
                self.unify(*ak, *bk, span)?;
                self.unify(*av, *bv, span)
            }

            (Ty::Tuple(as_), Ty::Tuple(bs)) => {
                if as_.len() != bs.len() {
                    return Err(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: Ty::Tuple(bs),
                            found:    Ty::Tuple(as_),
                        },
                        span,
                    });
                }
                for (a, b) in as_.into_iter().zip(bs) {
                    self.unify(a, b, span)?;
                }
                Ok(())
            }

            (Ty::Named { name: na, args: aa }, Ty::Named { name: nb, args: ab }) => {
                if na != nb || aa.len() != ab.len() {
                    return Err(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: Ty::Named { name: nb, args: ab },
                            found:    Ty::Named { name: na, args: aa },
                        },
                        span,
                    });
                }
                for (a, b) in aa.into_iter().zip(ab) {
                    self.unify(a, b, span)?;
                }
                Ok(())
            }

            (Ty::Record(afs), Ty::Record(bfs)) => {
                // Records unify field-by-field; extra fields are an error.
                if afs.len() != bfs.len() {
                    return Err(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: Ty::Record(bfs),
                            found:    Ty::Record(afs),
                        },
                        span,
                    });
                }
                let mut bmap: HashMap<String, Ty> = bfs.into_iter().collect();
                for (name, aty) in afs {
                    match bmap.remove(&name) {
                        Some(bty) => self.unify(aty, bty, span)?,
                        None => return Err(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: Ty::Named { name: name.clone(), args: vec![] },
                                found:    Ty::Unit,
                            },
                            span,
                        }),
                    }
                }
                Ok(())
            }

            // Higher-kinded application (BACKLOG item 76) — `F<A>` where `F`
            // may itself still be an unresolved (constructor-kind) type
            // variable. Two `App`s unify structurally, component-wise, just
            // like any other 2-argument compound type.
            (Ty::App(af, aa), Ty::App(bf, ba)) => {
                self.unify(*af, *bf, span)?;
                self.unify(*aa, *ba, span)
            }
            (Ty::Ctor(a), Ty::Ctor(b)) if a == b => Ok(()),

            // `F<A>` against a concrete single-argument type: bind the
            // constructor position to a `Ctor` recovered from that type's
            // own name/shape, then unify the argument. Deliberately only
            // 1-ary shapes (a user's own `Named{args:[_]}`, or the builtin
            // `List`/`Option`) — `Map`/`Result` are 2-ary and can never
            // satisfy an `F<_>` parameter, which is a real kind mismatch,
            // not something to special-case away.
            (Ty::App(f, a), Ty::Named { name, args }) | (Ty::Named { name, args }, Ty::App(f, a))
                if args.len() == 1 =>
            {
                self.unify(*f, Ty::Ctor(name), span)?;
                self.unify(*a, args.into_iter().next().unwrap(), span)
            }
            (Ty::App(f, a), Ty::List(elem)) | (Ty::List(elem), Ty::App(f, a)) => {
                self.unify(*f, Ty::Ctor("List".into()), span)?;
                self.unify(*a, *elem, span)
            }
            (Ty::App(f, a), Ty::Option(elem)) | (Ty::Option(elem), Ty::App(f, a)) => {
                self.unify(*f, Ty::Ctor("Option".into()), span)?;
                self.unify(*a, *elem, span)
            }

            (Ty::Fn { params: ap, ret: ar }, Ty::Fn { params: bp, ret: br }) => {
                if ap.len() != bp.len() {
                    return Err(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: Ty::Fn { params: bp, ret: br },
                            found:    Ty::Fn { params: ap, ret: ar },
                        },
                        span,
                    });
                }
                for (a, b) in ap.into_iter().zip(bp) {
                    self.unify(a, b, span)?;
                }
                self.unify(*ar, *br, span)
            }

            // Everything else is a mismatch.
            (found, expected) => Err(TypeError {
                kind: TypeErrorKind::Mismatch { expected, found },
                span,
            }),
        }
    }

    fn occurs_check(&self, v: TyVar, ty: &Ty, span: Span) -> Result<(), TypeError> {
        if ty.free_vars().contains(&v) {
            return Err(TypeError {
                kind: TypeErrorKind::OccursCheck { var: v, ty: ty.clone() },
                span,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // BACKLOG item 327 — `apply` used to clone the entire substitution map
    // (`self.map.clone()`) on every call, even though `apply_subst` only
    // ever needs a borrowed `&HashMap`. A real, large, internally
    // cross-referencing program calls `apply` tens of thousands of times
    // per declaration against a map that only grows over the whole module,
    // so that wasted clone compounded into cubic-to-quartic scaling.
    // Regression guard: with the clone still in place, 20,000 bindings x
    // 5,000 `apply` calls costs ~100M cloned-entry-equivalents (measured at
    // ~26.7M/sec against the real bug, i.e. ~3.7s) — comfortably over the
    // 1s budget below. Without the clone, `apply` on a bare `Ty::Var` costs
    // O(1) regardless of map size, so this finishes in well under a second
    // even on a slow CI box.
    #[test]
    fn apply_does_not_clone_the_whole_substitution_map() {
        let mut uf = UnionFind::default();
        for i in 0..20_000u32 {
            uf.unify(Ty::Var(i), Ty::Int, Span::DUMMY).unwrap();
        }
        let probe = Ty::Var(19_999);
        let t0 = std::time::Instant::now();
        for _ in 0..5_000 {
            assert_eq!(uf.apply(&probe), Ty::Int);
        }
        let elapsed = t0.elapsed();
        assert!(elapsed.as_secs_f64() < 1.0,
            "apply() took {:?} for 5,000 calls against a 20,000-entry map — \
             looks like the full-map-clone regression is back (item 327)", elapsed);
    }
}
