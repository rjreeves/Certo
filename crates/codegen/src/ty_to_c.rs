use certo_typeck::Ty;

/// Convert a Certo type to its C representation.
pub fn ty_to_c(ty: &Ty) -> String {
    match ty {
        Ty::Int     => "int64_t".into(),
        Ty::Float   => "double".into(),
        Ty::Decimal => "certo_decimal_t".into(),
        Ty::Bool    => "bool".into(),
        Ty::Text    => "certo_text_t".into(),
        Ty::Unit    => "certo_unit_t".into(),
        Ty::Uuid    => "certo_uuid_t".into(),

        Ty::Option(inner) => format!("certo_option_{}_t", mangle(inner)),
        Ty::Result(ok, err) => format!("certo_result_{}_{}_t", mangle(ok), mangle(err)),
        Ty::List(elem)  => format!("certo_list_{}_t", mangle(elem)),
        Ty::Map(k, v)   => format!("certo_map_{}_{}_t", mangle(k), mangle(v)),
        Ty::Tuple(ts)   => {
            if ts.is_empty() { return "certo_unit_t".into(); }
            format!("certo_tuple_{}_t", ts.iter().map(mangle).collect::<Vec<_>>().join("_"))
        }

        Ty::Named { name, args } if args.is_empty() => c_ident(name),
        Ty::Named { name, args } => {
            format!("{}_{}_t", c_ident(name), args.iter().map(mangle).collect::<Vec<_>>().join("_"))
        }

        Ty::Record(_) => "void*".into(), // anonymous records become void* until struct is emitted
        Ty::Fn { .. } => "certo_fn_t".into(),

        // Inference leftovers — shouldn't reach codegen, but be safe.
        // Use int64_t (not void*): on 64-bit it can store both integers and
        // pointers, avoiding hard void-pointer↔integer conversion errors.
        Ty::Var(v)    => format!("certo_var{}_t", v),
        Ty::Forall { body, .. } => ty_to_c(body),
        Ty::Error     => "int64_t".into(),
    }
}

/// Mangle a type into a valid C identifier fragment.
pub fn mangle(ty: &Ty) -> String {
    match ty {
        Ty::Int     => "int".into(),
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
