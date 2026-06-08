//! Certo → C type mapping for header generation.

use certo_ast::types::TypeExpr;

/// Map a Certo `TypeExpr` to a C type string suitable for a header file.
pub fn ty_to_c(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Named { path, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("void");
            match name {
                "Int"     => "int64_t".into(),
                "Float"   => "double".into(),
                "Decimal" => "certo_decimal_t".into(),
                "Bool"    => "bool".into(),
                "Text"    => "const char*".into(),
                "Unit"    => "void".into(),
                "UUID"    => "certo_uuid_t".into(),
                "Byte"    => "uint8_t".into(),
                other     => c_ident(other),
            }
        }
        TypeExpr::Option { .. } => "certo_option_t".into(),
        TypeExpr::Tuple { elements, .. } if elements.is_empty() => "void".into(),
        TypeExpr::Tuple { .. }  => "certo_tuple_t".into(),
        TypeExpr::Fn { .. }     => "certo_fn_t".into(),
        TypeExpr::Record { .. } => "void*".into(),
        TypeExpr::Ptr { inner, .. } => format!("{}*", ty_to_c(&inner.node)),
        TypeExpr::Param { .. }  => "void*".into(),
    }
}

/// Return type variant: `Unit` / empty tuple → `void` for function return positions.
pub fn ret_ty_to_c(ret: Option<&TypeExpr>) -> String {
    match ret {
        None     => "void".into(),
        Some(te) => {
            let s = ty_to_c(te);
            if s == "void" { "void".into() } else { s }
        }
    }
}

/// Convert a Certo identifier to a safe C identifier (replace `.`/`-` with `_`).
pub fn c_ident(s: &str) -> String {
    s.replace('.', "_").replace('-', "_")
}
