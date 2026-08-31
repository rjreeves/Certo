//! Certo type → LLVM IR type strings.

use certo_typeck::Ty;

/// Map a Certo type to its LLVM IR type.
///
/// Complex heap types (List, Map, Named records, Decimal, UUID, …) are all
/// represented as opaque pointers (`ptr`), relying on LLVM 14+ opaque-pointer
/// mode.  The runtime library (certo_runtime.h + stdlib C sources) provides
/// the actual struct layouts.
pub fn llvm_ty(ty: &Ty) -> String {
    match ty {
        Ty::Int        => "i64".into(),
        Ty::Int8       => "i8".into(),
        Ty::Int16      => "i16".into(),
        Ty::Int32      => "i32".into(),
        Ty::UInt       => "i64".into(),  // LLVM has no unsigned; use i64 with zext where needed
        Ty::Float      => "double".into(),
        Ty::Float32    => "float".into(),
        Ty::Bool       => "i1".into(),
        // BACKLOG item 258 — must match the C backend's own `Ty::Char =>
        // "int32_t"` (`crates/codegen/src/ty_to_c.rs`), since the LLVM
        // backend calls into that same compiled C runtime — a mismatched
        // width here would be a real ABI mismatch at every Char-typed call.
        Ty::Char       => "i32".into(),
        Ty::Text       => "ptr".into(),    // const char*
        Ty::BoundedText(_) => "ptr".into(), // same runtime representation as Text
        Ty::Unit       => "i8".into(),     // placeholder; void used for return type
        Ty::Uuid       => "ptr".into(),
        Ty::Decimal(_) => "ptr".into(),    // certo_decimal_t*
        Ty::Option(_)  => "ptr".into(),
        Ty::Result(_, _) => "ptr".into(),
        Ty::List(_)    => "ptr".into(),
        Ty::Map(_, _)  => "ptr".into(),
        Ty::Tuple(ts)  => if ts.is_empty() { "i8".into() } else { "ptr".into() },
        Ty::Named { .. } => "ptr".into(),
        Ty::Record(_)  => "ptr".into(),
        Ty::Fn { .. }  => "ptr".into(),
        Ty::Var(_)     => "ptr".into(),
        Ty::Forall { body, .. } => llvm_ty(body),
        Ty::Error      => "ptr".into(),
        // Higher-kinded type param (BACKLOG item 76) — same opaque-pointer
        // erasure as any other still-generic type reaching this point.
        Ty::Ctor(_) | Ty::App(_, _) => "ptr".into(),
    }
}

/// Return type for a function — `Unit` becomes `void`.
pub fn llvm_ret_ty(ty: &Ty) -> String {
    if matches!(ty, Ty::Unit) {
        "void".into()
    } else {
        llvm_ty(ty)
    }
}

/// Convert an AST `TypeExpr` to its LLVM IR type string.
///
/// Used when the HIR types are not yet resolved (pre-typeck).
pub fn ast_ty_to_llvm(te: &certo_ast::types::TypeExpr) -> String {
    use certo_ast::types::TypeExpr;
    match te {
        TypeExpr::Named { path, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("Unit");
            match name {
                "Int"     => "i64".into(),
                "Int8"    => "i8".into(),
                "Int16"   => "i16".into(),
                "Int32"   => "i32".into(),
                "UInt"    => "i64".into(),
                "Float"   => "double".into(),
                "Float32" => "float".into(),
                "Bool"    => "i1".into(),
                "Char"    => "i8".into(),
                "Text"    => "ptr".into(),
                "BoundedText" => "ptr".into(),
                "Unit"    => "i8".into(),
                "Decimal" => "ptr".into(),
                "UUID"    => "ptr".into(),
                _         => "ptr".into(),
            }
        }
        TypeExpr::Tuple { elements, .. } if elements.is_empty() => "i8".into(),
        TypeExpr::Tuple { .. }  => "ptr".into(),
        TypeExpr::Option { .. } => "ptr".into(),
        TypeExpr::Fn { .. }     => "ptr".into(),
        TypeExpr::Record { .. } => "ptr".into(),
        TypeExpr::Ptr { .. }    => "ptr".into(),
        TypeExpr::Param { .. }  => "ptr".into(),
        TypeExpr::DecimalParam { .. } => "ptr".into(),
        TypeExpr::BoundedTextParam { .. } => "ptr".into(),
    }
}

/// Return type from an AST annotation: `None` → `"void"` (Unit); Unit → `"void"`.
pub fn ast_ret_ty_to_llvm(ret_ty: Option<&certo_ast::types::TypeExpr>) -> String {
    use certo_ast::types::TypeExpr;
    match ret_ty {
        None => "void".into(),
        Some(TypeExpr::Named { path, .. })
            if path.segments.last().map(|s| s.node.as_str()) == Some("Unit") => "void".into(),
        Some(TypeExpr::Tuple { elements, .. }) if elements.is_empty() => "void".into(),
        Some(te) => ast_ty_to_llvm(te),
    }
}

/// Mangle a `certo_typeck::Ty` name to a valid LLVM identifier fragment.
pub fn mangle(ty: &Ty) -> String {
    match ty {
        Ty::Int     => "i64".into(),
        Ty::Int8    => "i8".into(),
        Ty::Int16   => "i16".into(),
        Ty::Int32   => "i32".into(),
        Ty::UInt    => "i64".into(),
        Ty::Float   => "f64".into(),
        Ty::Float32 => "f32".into(),
        Ty::Bool    => "i1".into(),
        Ty::Char    => "i32".into(),
        Ty::Text    => "ptr".into(),
        Ty::Unit    => "void".into(),
        _           => "ptr".into(),
    }
}

/// Convert a Certo qualified name (`List.len`, `add`) to an LLVM global
/// identifier: `@certo_List_len`, `@certo_add`.
pub fn llvm_fn_name(name: &str) -> String {
    format!("certo_{}", name.replace('.', "_").replace('-', "_"))
}

/// Escape a UTF-8 string for use inside an LLVM `c"…"` constant.
///
/// Non-printable bytes and special characters (`"`, `\`) are encoded as
/// `\XX` two-hex-digit escapes.
pub fn llvm_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\5C"),
            b'"'  => out.push_str("\\22"),
            b'\n' => out.push_str("\\0A"),
            b'\r' => out.push_str("\\0D"),
            b'\t' => out.push_str("\\09"),
            0x20..=0x7E => out.push(b as char),
            other => out.push_str(&format!("\\{:02X}", other)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_types() {
        assert_eq!(llvm_ty(&Ty::Int),   "i64");
        assert_eq!(llvm_ty(&Ty::Float), "double");
        assert_eq!(llvm_ty(&Ty::Bool),  "i1");
        assert_eq!(llvm_ty(&Ty::Text),  "ptr");
        assert_eq!(llvm_ty(&Ty::Unit),  "i8");
    }

    #[test]
    fn ret_ty_unit_is_void() {
        assert_eq!(llvm_ret_ty(&Ty::Unit), "void");
        assert_eq!(llvm_ret_ty(&Ty::Int),  "i64");
    }

    #[test]
    fn fn_name_dots_replaced() {
        assert_eq!(llvm_fn_name("List.len"), "certo_List_len");
        assert_eq!(llvm_fn_name("add"),      "certo_add");
    }

    #[test]
    fn escape_special_chars() {
        assert_eq!(llvm_escape("hello\nworld"), "hello\\0Aworld");
        assert_eq!(llvm_escape("say \"hi\""),   "say \\22hi\\22");
        assert_eq!(llvm_escape("back\\slash"),  "back\\5Cslash");
    }
}
