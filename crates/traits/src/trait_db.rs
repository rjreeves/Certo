use std::collections::HashMap;
use certo_ast::decl::{Decl, FnDecl, TraitDecl, ImplDecl};
use certo_ast::module::Module;
use certo_ast::span::Span;

// ------------------------------------------------------------------ //
// Trait definition record
// ------------------------------------------------------------------ //

/// The canonical signature of one method in a trait.
#[derive(Debug, Clone)]
pub struct MethodSig {
    pub name:       String,
    pub param_count: usize,
    /// Stringified param type expressions (for display / comparison).
    pub param_types: Vec<String>,
    /// Stringified return type (empty string = unspecified / Unit).
    pub ret_type:   String,
    pub has_default: bool,
    pub span:       Span,
}

/// Everything we know about a trait.
#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name:    String,
    pub methods: HashMap<String, MethodSig>,
    pub span:    Span,
}

// ------------------------------------------------------------------ //
// Impl record
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct ImplRecord {
    /// The trait being implemented (None = inherent impl).
    pub trait_name: Option<String>,
    /// The concrete type this impl is for, e.g. `"Order"`.
    pub for_type:   String,
    /// Methods provided by this impl.
    pub methods:    HashMap<String, MethodSig>,
    pub span:       Span,
}

// ------------------------------------------------------------------ //
// The registry
// ------------------------------------------------------------------ //

#[derive(Default, Debug)]
pub struct TraitDb {
    /// trait name → definition
    pub traits: HashMap<String, TraitDef>,
    /// (trait name, type name) → impl record
    pub impls:  HashMap<(String, String), ImplRecord>,
    /// type name → list of inherent impls
    pub inherent: HashMap<String, Vec<ImplRecord>>,
}

impl TraitDb {
    /// Build the registry from a parsed module.
    pub fn build(module: &Module) -> Self {
        let mut db = TraitDb::default();
        for sdecl in &module.decls {
            match &sdecl.node {
                Decl::Trait(t) => db.register_trait(t),
                Decl::Impl(i)  => db.register_impl(i),
                _              => {}
            }
        }
        db
    }

    fn register_trait(&mut self, t: &TraitDecl) {
        let methods = t.methods.iter()
            .map(|m| (m.name.node.clone(), sig_of(m)))
            .collect();
        self.traits.insert(t.name.node.clone(), TraitDef {
            name:    t.name.node.clone(),
            methods,
            span:    t.span,
        });
    }

    fn register_impl(&mut self, i: &ImplDecl) {
        let for_type = path_to_string(&i.type_path.segments
            .iter().map(|s| s.node.as_str()).collect::<Vec<_>>());

        let methods: HashMap<String, MethodSig> = i.methods.iter()
            .map(|m| (m.name.node.clone(), sig_of(m)))
            .collect();

        let record = ImplRecord {
            trait_name: i.trait_path.as_ref().map(|p| {
                path_to_string(&p.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>())
            }),
            for_type:   for_type.clone(),
            methods,
            span:       i.span,
        };

        match &record.trait_name {
            Some(tn) => {
                self.impls.insert((tn.clone(), for_type), record);
            }
            None => {
                self.inherent.entry(for_type).or_default().push(record);
            }
        }
    }

    /// Does `ty` implement `trait_name`?
    pub fn implements(&self, ty: &str, trait_name: &str) -> bool {
        self.impls.contains_key(&(trait_name.to_string(), ty.to_string()))
    }
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

fn path_to_string(segs: &[&str]) -> String {
    segs.join(".")
}

pub fn sig_of(f: &FnDecl) -> MethodSig {
    use certo_ast::types::TypeExpr;

    fn te_str(te: &TypeExpr) -> String {
        match te {
            TypeExpr::Named { path, args, .. } => {
                let name = path.segments.last()
                    .map(|s| s.node.as_str()).unwrap_or("_");
                if args.is_empty() {
                    name.to_string()
                } else {
                    format!("{}<{}>", name, args.iter().map(|a| te_str(&a.node)).collect::<Vec<_>>().join(", "))
                }
            }
            TypeExpr::Option { inner, .. } => format!("{}?", te_str(&inner.node)),
            TypeExpr::Tuple { elements, .. } =>
                format!("({})", elements.iter().map(|e| te_str(&e.node)).collect::<Vec<_>>().join(", ")),
            TypeExpr::Fn { params, ret, .. } =>
                format!("({}) => {}", params.iter().map(|p| te_str(&p.node)).collect::<Vec<_>>().join(", "), te_str(&ret.node)),
            TypeExpr::Record { .. } => "{ .. }".into(),
            TypeExpr::Ptr { inner, .. } => format!("*{}", te_str(&inner.node)),
            TypeExpr::Param { name, .. } => name.node.clone(),
        }
    }

    MethodSig {
        name:        f.name.node.clone(),
        param_count: f.params.len(),
        param_types: f.params.iter().map(|p| te_str(&p.ty.node)).collect(),
        ret_type:    f.ret_ty.as_ref().map(|t| te_str(&t.node)).unwrap_or_default(),
        has_default: f.body.is_some(),
        span:        f.span,
    }
}
