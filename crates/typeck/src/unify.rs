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
    pub fn apply(&self, ty: &Ty) -> Ty {
        let walked = self.find(ty.clone());
        let subst  = self.to_subst();
        walked.apply_subst(&subst)
    }

    fn to_subst(&self) -> HashMap<TyVar, Ty> {
        self.map.clone()
    }

    /// Unify two types, recording the necessary bindings.
    /// Returns `Err` for type mismatches or occurs-check failures.
    pub fn unify(&mut self, a: Ty, b: Ty, span: Span) -> Result<(), TypeError> {
        let a = self.find(a);
        let b = self.find(b);

        match (a, b) {
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
            (Ty::Decimal, Ty::Decimal) => Ok(()),
            (Ty::Bool,    Ty::Bool)    => Ok(()),
            (Ty::Text,    Ty::Text)    => Ok(()),
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
                            expected: Ty::Tuple(as_),
                            found:    Ty::Tuple(bs),
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
                            expected: Ty::Named { name: na, args: aa },
                            found:    Ty::Named { name: nb, args: ab },
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
                            expected: Ty::Record(afs),
                            found:    Ty::Record(bfs),
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

            (Ty::Fn { params: ap, ret: ar }, Ty::Fn { params: bp, ret: br }) => {
                if ap.len() != bp.len() {
                    return Err(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: Ty::Fn { params: ap, ret: ar },
                            found:    Ty::Fn { params: bp, ret: br },
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
            (expected, found) => Err(TypeError {
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
