use certo_typeck::Ty;

/// Convert a Certo type to its C representation.
pub fn ty_to_c(ty: &Ty) -> String {
    match ty {
        Ty::Int     => "int64_t".into(),
        Ty::Int8    => "int8_t".into(),
        Ty::Int16   => "int16_t".into(),
        Ty::Int32   => "int32_t".into(),
        Ty::UInt    => "uint64_t".into(),
        Ty::Float   => "double".into(),
        Ty::Decimal => "certo_decimal_t".into(),
        Ty::Bool    => "bool".into(),
        Ty::Text    => "certo_text_t".into(),
        Ty::Unit    => "int64_t".into(), // Unit locals stored as 0; certo_unit_t only in function sigs
        Ty::Uuid    => "certo_uuid_t".into(),

        Ty::Option(_)   => "void*".into(),
        Ty::Result(_, _) => "void*".into(), // heap-allocated certo_result_t
        Ty::List(_)     => "void*".into(),
        // `CertoMap*` is opaque and untyped regardless of K/V (same as
        // `CertoList*` for List<T> above) — no per-instantiation typedef is
        // ever emitted, so naming one here produced an undeclared C
        // identifier the moment a Map value crossed a function boundary.
        Ty::Map(_, _)   => "void*".into(),
        Ty::Tuple(ts)   => {
            if ts.is_empty() { "int64_t".into() } else { "void*".into() }
        }

        // Http/Db opaque handles — passed as int64_t (pointer-sized) through the runtime ABI.
        Ty::Named { name, args } if args.is_empty() && matches!(name.as_str(), "HttpRequest" | "HttpResponse" | "Bytes" | "DbResult" | "Query" | "Mutation") => {
            "int64_t".into()
        }
        // A spawned-task handle is an opaque heap pointer.
        Ty::Named { name, .. } if name == "__CertoTask" => "void*".into(),
        Ty::Named { name, args } if args.is_empty() => c_ident(name),
        Ty::Named { name, args } => {
            format!("{}_{}_t", c_ident(name), args.iter().map(mangle).collect::<Vec<_>>().join("_"))
        }

        Ty::Record(_) => "void*".into(), // anonymous records become void* until struct is emitted
        Ty::Fn { .. } => "certo_fn_t".into(),

        // Type variables reach codegen only as unerased generic type params.
        // Represent them as void* — consistent with List<T>, Option<T>, etc.
        Ty::Var(_)    => "void*".into(),
        Ty::Forall { body, .. } => ty_to_c(body),
        Ty::Error     => "int64_t".into(),
    }
}

/// Mangle a type into a valid C identifier fragment.
pub fn mangle(ty: &Ty) -> String {
    match ty {
        Ty::Int     => "int".into(),
        Ty::Int8    => "int8".into(),
        Ty::Int16   => "int16".into(),
        Ty::Int32   => "int32".into(),
        Ty::UInt    => "uint".into(),
        Ty::Float   => "float".into(),
        Ty::Decimal => "decimal".into(),
        Ty::Bool    => "bool".into(),
        Ty::Text    => "text".into(),
        Ty::Unit    => "unit".into(),
        Ty::Uuid    => "uuid".into(),
        Ty::Option(t)   => format!("opt_{}", mangle(t)),
        Ty::List(t)     => format!("list_{}", mangle(t)),
        Ty::Tuple(ts)   => format!("tup_{}", ts.iter().map(mangle).collect::<Vec<_>>().join("_")),
        Ty::Named { name, .. } => c_ident(name).to_lowercase(),
        _ => "any".into(),
    }
}

/// Convert a Certo identifier to a valid C identifier (replace dots, spaces).
pub fn c_ident(s: &str) -> String {
    s.replace('.', "_").replace('-', "_")
}

/// The C return type for a function (Unit → void).
pub fn ret_ty_to_c(ty: &Ty) -> String {
    // Unit functions return int64_t(0) so call sites can always capture the result
    // without needing to know the callee's return type at the call site.
    if matches!(ty, Ty::Unit) { "int64_t".into() } else { ty_to_c(ty) }
}
