//! Match exhaustiveness over nested patterns (BACKLOG item 349).
//!
//! The classic matrix algorithm: a set of rows (one per unguarded arm) is exhaustive when, for
//! the first column, either every constructor of its type is present and each specialised matrix
//! is exhaustive, or the rows whose head is a wildcard (the default matrix) are exhaustive.
//! Constructors are recognised from the patterns themselves, so a nested pattern needs no type
//! information beyond what a variant name already says.

use certo_ast::pattern::{LitPat, Pattern};

/// A pattern reduced to what exhaustiveness needs.
#[derive(Debug, Clone)]
pub enum P {
    Wild,
    /// A constructor with positional arguments: `Some(x)`, `Circle(r)`, `true`, `[]`, `::`.
    Ctor(String, Vec<P>),
    /// A literal from an infinite domain: an Int, Float, Text or Unit.
    Lit(String),
    Tuple(Vec<P>),
    /// A record pattern without a type name: the fields it names.
    Rec(Vec<(String, P)>),
    Or(Vec<P>),
}

/// What the type environment says about a constructor name.
pub struct CtorInfo {
    /// Every constructor of the same type, in declaration order, with its arity.
    pub siblings: Vec<(String, usize)>,
}

pub trait Env {
    /// The constructors of the type that has this variant or record, if it is one.
    fn ctor_info(&self, name: &str) -> Option<CtorInfo>;
    /// A variant's field count.
    fn arity(&self, name: &str) -> usize;
}

pub fn lower(pat: &Pattern, env: &dyn Env) -> P {
    match pat {
        Pattern::Wildcard { .. } => P::Wild,
        Pattern::Ident { name, .. } => {
            let n = name.node.as_str();
            let upper = n.chars().next().is_some_and(|c| c.is_uppercase());
            if upper && (n == "None" || env.ctor_info(n).is_some()) { P::Ctor(n.to_string(), vec![]) } else { P::Wild }
        }
        Pattern::Constructor { path, fields, .. } => {
            let name = path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            P::Ctor(name, fields.iter().map(|f| lower(&f.node, env)).collect())
        }
        Pattern::Record { path: Some(p), fields, .. } => {
            // Named fields of a variant are not tracked individually: assume the pattern
            // covers its variant, as before.
            let name = p.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            let n = env.arity(&name).max(fields.len());
            P::Ctor(name, vec![P::Wild; n])
        }
        Pattern::Record { path: None, fields, .. } => P::Rec(fields.iter().map(|f| (
            f.name.node.clone(),
            f.pattern.as_ref().map(|p| lower(&p.node, env)).unwrap_or(P::Wild),
        )).collect()),
        Pattern::Tuple { elements, .. } => P::Tuple(elements.iter().map(|e| lower(&e.node, env)).collect()),
        Pattern::List { head, tail, .. } => {
            let mut acc = match tail {
                Some(t) => lower(&t.node, env),
                None => P::Ctor("[]".into(), vec![]),
            };
            for h in head.iter().rev() {
                acc = P::Ctor("::".into(), vec![lower(&h.node, env), acc]);
            }
            acc
        }
        Pattern::Literal { value, .. } => match value {
            LitPat::Bool(b) => P::Ctor(b.to_string(), vec![]),
            LitPat::Int(i) => P::Lit(i.to_string()),
            LitPat::Float(f) => P::Lit(f.to_string()),
            LitPat::String(s) => P::Lit(format!("{s:?}")),
            LitPat::Unit => P::Lit("()".into()),
        },
        Pattern::Guard { pattern, .. } | Pattern::As { pattern, .. } => lower(&pattern.node, env),
        Pattern::Or { left, right, .. } => P::Or(vec![lower(&left.node, env), lower(&right.node, env)]),
    }
}

/// Is a `match` with these unguarded rows exhaustive? `None` means yes; otherwise a description
/// of a value no row matches.
pub fn missing(rows: Vec<P>, env: &dyn Env) -> Option<String> {
    let matrix: Vec<Vec<P>> = rows.into_iter().map(|r| vec![r]).collect();
    check(matrix, 1, env).map(|w| w.into_iter().next().unwrap_or_else(|| "_".into()))
}

fn expand_or(rows: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let mut out = Vec::new();
    for row in rows {
        match row.first() {
            Some(P::Or(alts)) => {
                let alts = alts.clone();
                let mut more: Vec<Vec<P>> = alts.into_iter().map(|a| {
                    let mut r = row.clone();
                    r[0] = a;
                    r
                }).collect();
                more = expand_or(more);
                out.extend(more);
            }
            _ => out.push(row),
        }
    }
    out
}

fn arity_of(rows: &[Vec<P>], name: &str, env: &dyn Env) -> usize {
    let from_patterns = rows.iter().filter_map(|r| match &r[0] {
        P::Ctor(n, args) if n == name => Some(args.len()),
        _ => None,
    }).max().unwrap_or(0);
    from_patterns.max(env.arity(name))
}

/// Constructors that make up the whole type of a column whose heads include `name`.
fn complete_set(name: &str, rows: &[Vec<P>], env: &dyn Env) -> Option<Vec<(String, usize)>> {
    match name {
        "true" | "false" => Some(vec![("true".into(), 0), ("false".into(), 0)]),
        "Some" | "None" => Some(vec![("Some".into(), 1), ("None".into(), 0)]),
        "Ok" | "Err" => Some(vec![("Ok".into(), 1), ("Err".into(), 1)]),
        "[]" | "::" => Some(vec![("[]".into(), 0), ("::".into(), 2)]),
        _ => env.ctor_info(name).map(|info| {
            info.siblings.into_iter().map(|(n, a)| {
                let a = a.max(arity_of(rows, &n, env));
                (n, a)
            }).collect()
        }),
    }
}

fn show(name: &str, arity: usize) -> String {
    match (name, arity) {
        ("[]", _) => "[]".into(),
        ("::", _) => "[_, ..._]".into(),
        (n, 0) => n.to_string(),
        (n, a) => format!("{n}({})", vec!["_"; a].join(", ")),
    }
}

fn check(rows: Vec<Vec<P>>, cols: usize, env: &dyn Env) -> Option<Vec<String>> {
    if cols == 0 {
        return if rows.is_empty() { Some(vec![]) } else { None };
    }
    let rows = expand_or(rows);
    let rest = |w: Option<Vec<String>>, head: String| w.map(|mut w| { w.insert(0, head); w });

    let heads: Vec<&P> = rows.iter().map(|r| &r[0]).filter(|p| !matches!(p, P::Wild)).collect();

    let default = |rows: &Vec<Vec<P>>| -> Vec<Vec<P>> {
        rows.iter().filter(|r| matches!(r[0], P::Wild)).map(|r| r[1..].to_vec()).collect()
    };

    // Tuples have one constructor.
    if let Some(P::Tuple(first)) = heads.iter().find(|p| matches!(p, P::Tuple(_))) {
        let k = first.len();
        let spec: Vec<Vec<P>> = rows.iter().map(|r| {
            let mut v: Vec<P> = match &r[0] {
                P::Tuple(xs) => xs.iter().cloned().chain(std::iter::repeat(P::Wild)).take(k).collect(),
                _ => vec![P::Wild; k],
            };
            v.extend(r[1..].iter().cloned());
            v
        }).collect();
        return check(spec, k + cols - 1, env).map(|w| {
            let (inner, tail) = w.split_at(k.min(w.len()));
            let mut out = vec![format!("({})", inner.join(", "))];
            out.extend(tail.iter().cloned());
            out
        });
    }

    // A record pattern has one constructor with a field per name any row mentions.
    if heads.iter().any(|p| matches!(p, P::Rec(_))) {
        let mut names: Vec<String> = Vec::new();
        for p in &heads {
            if let P::Rec(fs) = p {
                for (n, _) in fs { if !names.contains(n) { names.push(n.clone()); } }
            }
        }
        let k = names.len();
        let spec: Vec<Vec<P>> = rows.iter().map(|r| {
            let mut v: Vec<P> = match &r[0] {
                P::Rec(fs) => names.iter().map(|n| fs.iter().find(|(m, _)| m == n).map(|(_, p)| p.clone()).unwrap_or(P::Wild)).collect(),
                _ => vec![P::Wild; k],
            };
            v.extend(r[1..].iter().cloned());
            v
        }).collect();
        return check(spec, k + cols - 1, env).map(|w| {
            let (inner, tail) = w.split_at(k.min(w.len()));
            let body = names.iter().zip(inner).map(|(n, v)| format!("{n}: {v}")).collect::<Vec<_>>().join(", ");
            let mut out = vec![format!("{{ {body} }}")];
            out.extend(tail.iter().cloned());
            out
        });
    }

    let ctor_names: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for p in &heads {
            if let P::Ctor(n, _) = p { if !v.contains(n) { v.push(n.clone()); } }
        }
        v
    };

    let complete = ctor_names.first().and_then(|n| complete_set(n, &rows, env));
    if let Some(all) = complete {
        let all_present = all.iter().all(|(n, _)| ctor_names.contains(n));
        if all_present {
            for (name, arity) in &all {
                let spec: Vec<Vec<P>> = rows.iter().filter_map(|r| match &r[0] {
                    P::Ctor(n, args) if n == name => {
                        let mut v: Vec<P> = args.iter().cloned().chain(std::iter::repeat(P::Wild)).take(*arity).collect();
                        v.extend(r[1..].iter().cloned());
                        Some(v)
                    }
                    P::Wild => {
                        let mut v = vec![P::Wild; *arity];
                        v.extend(r[1..].iter().cloned());
                        Some(v)
                    }
                    _ => None,
                }).collect();
                if let Some(w) = check(spec, arity + cols - 1, env) {
                    let (inner, tail) = w.split_at((*arity).min(w.len()));
                    let head = match (name.as_str(), inner.len()) {
                        (_, 0) => name.clone(),
                        ("::", _) => format!("[{}, ...{}]", inner[0], inner.get(1).cloned().unwrap_or_else(|| "_".into())),
                        (n, _) => format!("{n}({})", inner.join(", ")),
                    };
                    let mut out = vec![head];
                    out.extend(tail.iter().cloned());
                    return Some(out);
                }
            }
            return None;
        }
        // Some constructor is not mentioned: only the wildcard rows can cover it.
        let missing_ctor = all.iter().find(|(n, _)| !ctor_names.contains(n)).map(|(n, a)| show(n, *a)).unwrap_or_else(|| "_".into());
        return rest(check(default(&rows), cols - 1, env), missing_ctor);
    }

    // Literals and constructors of an unknown type: infinite, so only wildcard rows cover the rest.
    rest(check(default(&rows), cols - 1, env), "_".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct E;
    impl Env for E {
        fn ctor_info(&self, name: &str) -> Option<CtorInfo> {
            match name {
                "Circle" | "Sq" => Some(CtorInfo { siblings: vec![("Circle".into(), 1), ("Sq".into(), 1)] }),
                _ => None,
            }
        }
        fn arity(&self, name: &str) -> usize { if name == "Circle" || name == "Sq" { 1 } else { 0 } }
    }

    fn c(n: &str, args: Vec<P>) -> P { P::Ctor(n.into(), args) }

    #[test]
    fn nested_constructor_missing_is_found() {
        // Ok(Some(_)) | Err(_) misses Ok(None)
        let rows = vec![c("Ok", vec![c("Some", vec![P::Wild])]), c("Err", vec![P::Wild])];
        assert_eq!(missing(rows, &E).as_deref(), Some("Ok(None)"));
    }

    #[test]
    fn nested_constructors_all_covered() {
        let rows = vec![c("Ok", vec![c("Some", vec![P::Wild])]), c("Ok", vec![c("None", vec![])]), c("Err", vec![P::Wild])];
        assert_eq!(missing(rows, &E), None);
    }

    #[test]
    fn literal_inside_some_needs_a_catch_all() {
        let rows = vec![c("Some", vec![P::Lit("0".into())]), c("None", vec![])];
        assert_eq!(missing(rows, &E).as_deref(), Some("Some(_)"));
    }

    #[test]
    fn tuple_of_bools_is_exact() {
        let t = |a: &str, b: &str| P::Tuple(vec![c(a, vec![]), c(b, vec![])]);
        assert_eq!(missing(vec![t("true", "true"), t("true", "false"), t("false", "true"), t("false", "false")], &E), None);
        assert!(missing(vec![t("true", "true"), t("false", "false")], &E).is_some());
    }

    #[test]
    fn list_nil_and_cons_cover_every_list() {
        let rows = vec![c("[]", vec![]), c("::", vec![P::Wild, P::Wild])];
        assert_eq!(missing(rows, &E), None);
        assert!(missing(vec![c("::", vec![P::Wild, P::Wild])], &E).is_some());
    }
}
