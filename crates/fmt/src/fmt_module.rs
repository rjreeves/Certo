use certo_ast::module::{Module, Import, ImportKind};
use certo_ast::decl::Decl;
use crate::fmt_decl::fmt_decl;

/// Format a complete module to canonical Certo source.
pub fn fmt_module(module: &Module) -> String {
    let mut out = String::new();

    // `module` header
    let path = module.path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
    out.push_str("module ");
    out.push_str(&path);
    out.push('\n');

    // Imports — one blank line separating from the header, then consecutive
    if !module.imports.is_empty() {
        out.push('\n');
        for imp in &module.imports {
            out.push_str(&fmt_import(imp));
            out.push('\n');
        }
    }

    // Declarations — blank line between each. Skips a synthesized trailing
    // `impl` block (BACKLOG item 175): its content — in-body `fn` methods
    // and `computed` properties — is already fully represented by the
    // preceding `type`'s own body, which `fmt_type_decl` prints directly;
    // printing the synthesized impl too would duplicate it (and for a
    // `computed` property, produce output that fails to recompile at all,
    // since both copies would define the same accessor).
    for sdecl in &module.decls {
        if let Decl::Impl(i) = &sdecl.node {
            if i.is_synthesized { continue; }
        }
        out.push('\n');
        out.push_str(&fmt_decl(&sdecl.node, 0));
        out.push('\n');
    }

    out
}

fn fmt_import(imp: &Import) -> String {
    let pub_str  = if imp.is_pub { "pub " } else { "" };
    let when_str = imp.when.as_ref()
        .map(|w| format!("when [{} = \"{}\"] ", w.key, w.value))
        .unwrap_or_default();
    let path = imp.path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");

    let kind_str = match &imp.kind {
        ImportKind::Whole        => String::new(),
        ImportKind::Aliased(a)   => format!(" as {}", a.node),
        ImportKind::Named(names) => {
            let ns: Vec<String> = names.iter().map(|n| {
                if let Some(a) = &n.alias {
                    format!("{} as {}", n.name.node, a.node)
                } else {
                    n.name.node.clone()
                }
            }).collect();
            format!(".{{ {} }}", ns.join(", "))
        }
    };

    format!("{}import {}{}{}", pub_str, when_str, path, kind_str)
}
