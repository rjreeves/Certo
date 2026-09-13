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
        Ty::Float32 => "float".into(),
        Ty::Decimal(_) => "certo_decimal_t".into(),
        Ty::Bool    => "bool".into(),
        // BACKLOG item 258 — spec §4.1 defines `Char` as "4 bytes, a Unicode
        // scalar value, not a byte"; a plain 1-byte C `char` can only ever
        // hold a single UTF-8 *byte*, corrupting any multi-byte character it
        // represents. `int32_t` (already used for `Ty::Int32` above — no new
        // C type needed) holds a full Unicode scalar value (max 0x10FFFF).
        Ty::Char    => "int32_t".into(),
        Ty::Text    => "certo_text_t".into(),
        Ty::BoundedText(_) => "certo_text_t".into(), // same runtime representation as Text
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
        // A lifted spawn worker's hidden cancel-token parameter (BACKLOG
        // item 186) — the worker's own `__certo_task_hdr_t*`, passed by
        // codegen's spawn trampoline so its loops can check for abandonment.
        Ty::Named { name, .. } if name == "__CertoCancelToken" => "void*".into(),
        // `Channel<T>` is a `CertoChannel*` regardless of `T` — like `Map<K,V>`
        // above, no per-instantiation typedef is ever emitted, so this must be
        // checked before the generic `Ty::Named` arm below (which assumes a
        // real struct named after the type exists).
        Ty::Named { name, .. } if name == "Channel" => "void*".into(),
        // `File` (BACKLOG item 192) is an opaque `FILE*` handle — same
        // convention as `Channel`/`__CertoTask`, must also be checked before
        // the generic `Ty::Named` catch-all below.
        Ty::Named { name, .. } if name == "File" => "void*".into(),
        // Typed Host discriminants remain distinct Certo types while using
        // immutable text constants at the native ABI boundary.
        Ty::Named { name, .. } if matches!(name.as_str(),
            "HostState" | "HostWorkerState" | "HostFailureKind"
        ) => "certo_text_t".into(),
        // Stdlib.Host values are opaque heap handles owned by the runtime.
        Ty::Named { name, .. } if matches!(name.as_str(),
            "Host" | "HostPlugin" | "HostContext" | "ServiceKey" | "RestartPolicy" |
            "HostStatusSnapshot" | "HostWorkerStatus" | "HostMetricSnapshot" |
            "HostLifecycleError"
        ) => "void*".into(),
        // These stdlib scalar types have a C-side typedef prefixed `Certo`
        // (`CertoDateTime`, etc. — see crates/stdlib/src/datetime.rs) rather
        // than matching their bare Certo name; an explicit `DateTime`/`Date`/
        // `Duration` annotation previously emitted an undeclared C identifier
        // (only `val`-inferred bindings worked, by defaulting to `int64_t` via
        // HIR's Ty::Error fallback, which happens to be layout-compatible).
        // `Timezone` (BACKLOG item 118) is `CertoTimezone` — a `certo_text_t`
        // alias (the IANA zone name itself) — same treatment. `Timestamp`
        // (BACKLOG item 164) has no separate runtime type of its own at
        // all — it's a reserved builtin name used only for `.age` postfix
        // expressions (`crates/typeck/src/infer_expr.rs`'s `Expr::Age`) —
        // so it reuses `CertoDateTime` exactly, same representation, no new
        // C type. An explicit `Timestamp`-typed function param/return
        // previously emitted the bare, undeclared C identifier `Timestamp`.
        Ty::Named { name, args } if args.is_empty() && matches!(name.as_str(), "DateTime" | "Date" | "Duration" | "Timezone" | "Timestamp") => {
            let c_name = if name == "Timestamp" { "DateTime" } else { name.as_str() };
            format!("Certo{}", c_name)
        }
        // `JsonValue` is a real C pointer (`CertoJsonValue*`, see
        // crates/stdlib/src/json.rs), not a bare struct or an int64_t
        // handle like the opaque types above — same gap as DateTime/Date/
        // Duration just above (an explicit `JsonValue`-typed function
        // param/return/local previously emitted the undeclared C
        // identifier `JsonValue`; confirmed while building item 93's REST
        // client generator, whose model decode/encode functions are the
        // first real code to ever annotate a `JsonValue` parameter type
        // explicitly instead of only ever using it via untyped `val`
        // bindings).
        Ty::Named { name, args } if args.is_empty() && name == "JsonValue" => "CertoJsonValue*".into(),
        // `ProcessResult` is a real C pointer (`CertoProcessResult*`, see
        // crates/stdlib/src/process.rs), same shape as `JsonValue` just
        // above — was missing from every one of these tables entirely
        // (unlike its Http/Db siblings), so *any* use of `Process.exec`'s
        // own return value — annotated or plain `val`-inferred, both hit
        // the same HIR path that resolves the real `Ty::Named` — emitted
        // the literal, undeclared C identifier `ProcessResult` and failed
        // to compile; confirmed via a direct repro before this fix (BACKLOG
        // item 137).
        Ty::Named { name, args } if args.is_empty() && name == "ProcessResult" => "CertoProcessResult*".into(),
        Ty::Named { name, args } if args.is_empty() && name == "CliCommand" => "CertoCliCommand*".into(),
        Ty::Named { name, args } if args.is_empty() && name == "CliMatches" => "CertoCliMatches*".into(),
        Ty::Named { name, .. } => {
            // A user-defined generic type (`Box<Int>`, `Box<Text>`, ...) isn't
            // monomorphized — there is exactly one `Box` struct, with its
            // type-param fields stored as `void*` (BACKLOG item 119), reused
            // for every instantiation. So the C type is always the bare
            // struct name regardless of `args` — a mangled per-instantiation
            // name here would reference a typedef nothing ever emits.
            c_ident(name)
        }

        Ty::Record(_) => "void*".into(), // anonymous records become void* until struct is emitted
        Ty::Fn { .. } => "certo_fn_t".into(),

        // Type variables reach codegen only as unerased generic type params.
        // Represent them as void* — consistent with List<T>, Option<T>, etc.
        Ty::Var(_)    => "void*".into(),
        Ty::Forall { body, .. } => ty_to_c(body),
        Ty::Error     => "int64_t".into(),

        // A higher-kinded type param (`F<_>`, BACKLOG item 76) reaches
        // codegen only *inside* a still-generic function's own body (any
        // concrete call site fully resolves `App`/`Ctor` away during
        // unification — see `Ty::apply_subst` — before HIR/MIR ever see
        // it). Same type-erased `void*` representation as an ordinary
        // generic type param.
        Ty::Ctor(_) | Ty::App(_, _) => "void*".into(),
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
        Ty::Float32 => "float32".into(),
        Ty::Decimal(_) => "decimal".into(),
        Ty::Bool    => "bool".into(),
        Ty::Char    => "char".into(),
        Ty::Text    => "text".into(),
        Ty::BoundedText(_) => "text".into(),
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
