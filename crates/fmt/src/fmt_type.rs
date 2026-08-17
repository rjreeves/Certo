use certo_ast::types::{TypeExpr, TypeParam, EffectSet, Effect, Bound};
use crate::printer::group;

pub fn fmt_type(te: &TypeExpr, indent: usize) -> String {
    use TypeExpr::*;
    match te {
        Named { path, args, .. } => {
            let base = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            if args.is_empty() {
                base
            } else {
                let formatted: Vec<String> = args.iter().map(|a| fmt_type(&a.node, indent)).collect();
                format!("{}<{}>", base, formatted.join(", "))
            }
        }
        Option { inner, .. } => format!("{}?", fmt_type(&inner.node, indent)),
        Tuple  { elements, .. } => {
            let elems: Vec<String> = elements.iter().map(|e| fmt_type(&e.node, indent)).collect();
            group(&elems, "(", ")", ", ", indent, false)
        }
        Fn { params, ret, .. } => {
            let ps: Vec<String> = params.iter().map(|p| fmt_type(&p.node, indent)).collect();
            let param_str = if ps.len() == 1 { ps[0].clone() } else { format!("({})", ps.join(", ")) };
            format!("{} => {}", param_str, fmt_type(&ret.node, indent))
        }
        Record { fields, .. } => {
            let fs: Vec<String> = fields.iter()
                .map(|f| format!("{}: {}", f.name.node, fmt_type(&f.ty.node, indent + 1)))
                .collect();
            group(&fs, "{ ", " }", ", ", indent, false)
        }
        Ptr  { inner, .. } => format!("*{}", fmt_type(&inner.node, indent)),
        Param { name, .. }  => name.node.clone(),
        DecimalParam { precision, scale, .. } => format!("Decimal({}, {})", precision, scale),
        BoundedTextParam { max_len, .. } => format!("BoundedText({})", max_len),
    }
}

pub fn fmt_type_params(params: &[TypeParam]) -> String {
    if params.is_empty() { return String::new(); }
    let ps: Vec<String> = params.iter().map(|p| {
        if p.bounds.is_empty() {
            p.name.node.clone()
        } else {
            let bounds = p.bounds.iter().map(fmt_bound).collect::<Vec<_>>().join(" + ");
            format!("{}: {}", p.name.node, bounds)
        }
    }).collect();
    format!("<{}>", ps.join(", "))
}

fn fmt_bound(b: &Bound) -> String {
    match b {
        Bound::Trait(t) => t.name.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."),
        Bound::Row(r) => {
            let fs: Vec<String> = r.fields.iter()
                .map(|f| format!("{}: {}", f.name.node, fmt_type(&f.ty.node, 0)))
                .collect();
            format!("{{ {} }}", fs.join(", "))
        }
    }
}

pub fn fmt_effects(effects: &Option<EffectSet>) -> String {
    let Some(es) = effects else { return String::new(); };
    if es.effects.is_empty() { return String::new(); }
    let names: Vec<&str> = es.effects.iter().map(|e| effect_name(&e.node)).collect();
    format!(" [{}]", names.join(", "))
}

fn effect_name(e: &Effect) -> &'static str {
    match e {
        Effect::Pure     => "pure",
        Effect::DbRead   => "db.read",
        Effect::DbWrite  => "db.write",
        Effect::Io       => "io",
        Effect::Async    => "async",
        Effect::Unsafe   => "unsafe",
        Effect::Fallible => "fallible",
    }
}
