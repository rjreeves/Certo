use certo_typeck::{Ty, TypeEnv};
use crate::seed::seed_stdlib;
use crate::{CORE_C, BYTES_C, CREDENTIAL_C, COLLECTIONS_C, TEXT_C, DATETIME_C, MONEY_C,
            ENV_C, FILE_C, PATH_C, PROCESS_C, JSON_C, HTTP_C, DB_C,
            full_c_runtime};

fn seeded_env() -> TypeEnv {
    let mut env = TypeEnv::new();
    let mut counter = 0u32;
    seed_stdlib(&mut env, &mut counter);
    env
}

// ------------------------------------------------------------------ //
// seed_stdlib registers expected names
// ------------------------------------------------------------------ //

#[test]
fn core_print_registered() {
    let env = seeded_env();
    assert!(env.lookup("print").is_some());
    assert!(env.lookup("println").is_some());
}

#[test]
fn print_type_is_text_to_unit() {
    let env = seeded_env();
    let ty = env.lookup("print").unwrap();
    match ty {
        Ty::Fn { params, ret } => {
            assert_eq!(params.len(), 1);
            assert_eq!(&params[0], &Ty::Text);
            assert_eq!(ret.as_ref(), &Ty::Unit);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn range_returns_list_int() {
    let env = seeded_env();
    let ty = env.lookup("range").unwrap();
    match ty {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int, Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::List(Box::new(Ty::Int)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn list_len_is_polymorphic() {
    let env = seeded_env();
    let ty = env.lookup("List.len").unwrap();
    assert!(matches!(ty, Ty::Forall { .. }), "List.len should be polymorphic");
}

#[test]
fn list_map_registered() {
    let env = seeded_env();
    assert!(env.lookup("List.map").is_some());
    assert!(matches!(env.lookup("List.map").unwrap(), Ty::Forall { .. }));
}

#[test]
fn result_combinators_registered() {
    let env = seeded_env();
    for name in ["flatMap", "mapErr", "getOrElse", "recover", "Result.all", "Result.allSettled"] {
        assert!(env.lookup(name).is_some(), "{} not registered", name);
        assert!(matches!(env.lookup(name).unwrap(), Ty::Forall { .. }),
            "{} should be polymorphic", name);
    }
}

#[test]
fn result_c_runtime_included() {
    let rt = full_c_runtime();
    for f in ["certo_flat_map", "certo_map_err", "certo_get_or_else", "certo_recover",
              "certo_result_all", "certo_result_all_settled"] {
        assert!(rt.contains(f), "missing {} in runtime", f);
    }
}

#[test]
fn list_fold_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.fold").unwrap(), Ty::Forall { .. }));
}

#[test]
fn map_empty_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("Map.empty").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_empty_is_a_zero_arg_function_not_a_bare_value() {
    // Regression test: `List.empty` was registered as a bare `Forall<T>. List<T>`
    // value instead of `Forall<T>. () => List<T>`. Every call site is `List.empty()`
    // — an App with zero args — which always unifies the callee's type against
    // `Fn{params: [], ret}`, so a bare (non-Fn) body made every use of `List.empty()`
    // fail to type-check, misreported as "found `() => U`".
    let env = seeded_env();
    match env.lookup("List.empty").unwrap() {
        Ty::Forall { body, .. } => assert!(
            matches!(body.as_ref(), Ty::Fn { params, .. } if params.is_empty()),
            "List.empty's body should be a zero-arg Fn, got {:?}", body
        ),
        other => panic!("List.empty should be polymorphic, got {:?}", other),
    }
}

#[test]
fn map_empty_is_a_zero_arg_function_not_a_bare_value() {
    // Same bug, same fix, for Map.empty — see list_empty's test above.
    let env = seeded_env();
    match env.lookup("Map.empty").unwrap() {
        Ty::Forall { body, .. } => assert!(
            matches!(body.as_ref(), Ty::Fn { params, .. } if params.is_empty()),
            "Map.empty's body should be a zero-arg Fn, got {:?}", body
        ),
        other => panic!("Map.empty should be polymorphic, got {:?}", other),
    }
}

#[test]
fn map_insert_registered() {
    let env = seeded_env();
    assert!(matches!(env.lookup("Map.insert").unwrap(), Ty::Forall { .. }));
}

#[test]
fn text_functions_registered() {
    let env = seeded_env();
    for name in &["Text.len", "Text.byteLength", "Text.concat", "Text.contains", "Text.toUpper",
                  "Text.toLower", "Text.trim", "Text.split", "Text.join",
                  "Text.replace", "Text.indexOf"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn text_len_type() {
    let env = seeded_env();
    match env.lookup("Text.len").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn text_byte_length_type() {
    let env = seeded_env();
    match env.lookup("Text.byteLength").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn datetime_functions_registered() {
    let env = seeded_env();
    for name in &["DateTime.now", "DateTime.fromUnix", "DateTime.toUnix",
                  "DateTime.format", "DateTime.toIso", "DateTime.addDays",
                  "DateTime.diffDays", "DateTime.year"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn decimal_functions_registered() {
    let env = seeded_env();
    for name in &["Decimal.add", "Decimal.sub", "Decimal.mul", "Decimal.div",
                  "Decimal.round", "Decimal.toText", "Money.fromCents"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn decimal_add_type() {
    let env = seeded_env();
    match env.lookup("Decimal.add").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Decimal, Ty::Decimal]);
            assert_eq!(ret.as_ref(), &Ty::Decimal);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// C runtime strings are non-empty and contain expected symbols
// ------------------------------------------------------------------ //

#[test]
fn core_c_contains_certo_print() {
    assert!(CORE_C.contains("certo_print"), "missing certo_print");
    assert!(CORE_C.contains("certo_range"), "missing certo_range");
    assert!(CORE_C.contains("certo_pow"),   "missing certo_pow");
}

#[test]
fn certo_panic_flushes_stdout_before_aborting() {
    // Confirmed bug: output written via println before a panic() call was
    // silently lost, because certo_panic called abort() without first
    // flushing stdio buffers — abort() skips the normal atexit-time flush,
    // so anything sitting in stdout's (fully-buffered-when-not-a-tty) buffer
    // was dropped. certo_panic must flush stdout (and stderr, for symmetry)
    // before it aborts.
    let panic_fn = CORE_C.split("/* ---- panic ---- */").nth(1)
        .expect("CORE_C must have a panic section");
    let flush_pos = panic_fn.find("fflush(stdout)").expect("certo_panic must fflush(stdout) before aborting");
    let abort_pos = panic_fn.find("abort()").expect("certo_panic must call abort()");
    assert!(flush_pos < abort_pos, "fflush(stdout) must happen before abort()");
}

#[test]
fn certo_panic_returns_int64_not_void() {
    // A user-level `panic("msg")` call has Certo type `forall a. Text -> a`,
    // so codegen may need to assign its result to a temp of whatever type the
    // call site expects (e.g. the tail expression of a `Unit`-returning
    // function). certo_panic must therefore return int64_t like the other
    // Unit-typed builtins (certo_println, certo_flush, ...) rather than a
    // real C `void`, or `_tmp = certo_panic(...)` fails to compile with
    // "assigning to 'int64_t' from incompatible type 'void'".
    assert!(
        CORE_C.contains("int64_t certo_panic(certo_text_t msg)"),
        "certo_panic must return int64_t, not void"
    );
}

#[test]
fn collections_c_contains_list_and_map() {
    assert!(COLLECTIONS_C.contains("certo_list_len"),      "missing list_len");
    assert!(COLLECTIONS_C.contains("certo_list_map"),      "missing list_map");
    assert!(COLLECTIONS_C.contains("certo_list_filter"),   "missing list_filter");
    assert!(COLLECTIONS_C.contains("certo_list_fold"),     "missing list_fold");
    assert!(COLLECTIONS_C.contains("certo_map_insert"),    "missing map_insert");
    assert!(COLLECTIONS_C.contains("certo_map_get"),       "missing map_get");
    assert!(COLLECTIONS_C.contains("certo_list_sort"),     "missing list_sort");
    assert!(COLLECTIONS_C.contains("certo_list_find"),     "missing list_find");
    assert!(COLLECTIONS_C.contains("certo_list_any"),      "missing list_any");
    assert!(COLLECTIONS_C.contains("certo_list_all"),      "missing list_all");
    assert!(COLLECTIONS_C.contains("certo_list_zip"),      "missing list_zip");
    assert!(COLLECTIONS_C.contains("certo_list_distinct"),  "missing list_distinct");
    assert!(COLLECTIONS_C.contains("certo_list_partition"), "missing list_partition");
    assert!(COLLECTIONS_C.contains("certo_list_chunked"),   "missing list_chunked");
    assert!(COLLECTIONS_C.contains("certo_list_group_by"),  "missing list_group_by");
    assert!(COLLECTIONS_C.contains("certo_map_from_list"), "missing map_from_list");
}

#[test]
fn text_c_contains_key_functions() {
    assert!(TEXT_C.contains("certo_text_concat"),     "missing text_concat");
    assert!(TEXT_C.contains("certo_text_trim"),       "missing text_trim");
    assert!(TEXT_C.contains("certo_text_split"),      "missing text_split");
    assert!(TEXT_C.contains("certo_text_replace"),    "missing text_replace");
    assert!(TEXT_C.contains("certo_text_byte_length"),"missing text_byte_length");
}

#[test]
fn money_c_contains_parse_decimal() {
    assert!(MONEY_C.contains("certo_parse_decimal"), "missing certo_parse_decimal");
}

#[test]
fn datetime_c_contains_key_functions() {
    assert!(DATETIME_C.contains("certo_datetime_now"),    "missing datetime_now");
    assert!(DATETIME_C.contains("certo_datetime_format"), "missing datetime_format");
    assert!(DATETIME_C.contains("certo_datetime_add_days"),"missing datetime_add_days");
}

#[test]
fn money_c_contains_decimal_arithmetic() {
    assert!(MONEY_C.contains("certo_decimal_add"),    "missing decimal_add");
    assert!(MONEY_C.contains("certo_decimal_mul"),    "missing decimal_mul");
    assert!(MONEY_C.contains("certo_decimal_round"),  "missing decimal_round");
    assert!(MONEY_C.contains("certo_decimal_to_text"),"missing decimal_to_text");
}

#[test]
fn full_c_runtime_combines_all_modules() {
    let rt = full_c_runtime();
    assert!(rt.contains("certo_print"),         "missing core");
    assert!(rt.contains("certo_list_len"),      "missing collections");
    assert!(rt.contains("certo_text_concat"),   "missing text");
    assert!(rt.contains("certo_datetime_now"),  "missing datetime");
    assert!(rt.contains("certo_decimal_add"),   "missing money");
}

// ------------------------------------------------------------------ //
// Counter advances: seed_stdlib doesn't trample the caller's counter
// ------------------------------------------------------------------ //

#[test]
fn seed_advances_counter() {
    let mut env = TypeEnv::new();
    let mut counter = 1000u32;
    seed_stdlib(&mut env, &mut counter);
    assert!(counter > 1000, "counter should have advanced");
}

// ------------------------------------------------------------------ //
// Core — conversions
// ------------------------------------------------------------------ //

#[test]
fn conversion_functions_registered() {
    let env = seeded_env();
    for name in &["intToText", "floatToText", "boolToText",
                  "floatToInt", "intToFloat", "parseInt", "parseFloat", "parseDecimal"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn int_to_text_type() {
    let env = seeded_env();
    match env.lookup("intToText").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::Text);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn parse_int_returns_option_int() {
    let env = seeded_env();
    match env.lookup("parseInt").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Int)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn parse_float_returns_option_float() {
    let env = seeded_env();
    match env.lookup("parseFloat").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Float)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn parse_decimal_returns_option_decimal() {
    let env = seeded_env();
    match env.lookup("parseDecimal").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Decimal)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Core — math
// ------------------------------------------------------------------ //

#[test]
fn math_functions_registered() {
    let env = seeded_env();
    for name in &["pow", "absInt", "absFloat",
                  "minInt", "maxInt", "minFloat", "maxFloat",
                  "floor", "ceil", "round", "sqrt",
                  "rangeInclusive"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn pow_type() {
    let env = seeded_env();
    match env.lookup("pow").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int, Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn sqrt_type() {
    let env = seeded_env();
    match env.lookup("sqrt").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Float]);
            assert_eq!(ret.as_ref(), &Ty::Float);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn range_inclusive_type() {
    let env = seeded_env();
    match env.lookup("rangeInclusive").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int, Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::List(Box::new(Ty::Int)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Core — argv / IO
// ------------------------------------------------------------------ //

#[test]
fn argv_functions_registered() {
    let env = seeded_env();
    assert!(env.lookup("argCount").is_some(), "missing argCount");
    assert!(env.lookup("arg").is_some(),      "missing arg");
    assert!(env.lookup("readLine").is_some(), "missing readLine");
    assert!(env.lookup("readAll").is_some(),  "missing readAll");
}

#[test]
fn arg_returns_option_text() {
    let env = seeded_env();
    match env.lookup("arg").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Text)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Collections — List extras
// ------------------------------------------------------------------ //

#[test]
fn list_extra_functions_registered() {
    let env = seeded_env();
    for name in &["List.get", "List.getOrPanic", "List.push", "List.concat",
                  "List.first", "List.last", "List.slice", "List.reverse",
                  "List.filter", "List.contains"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn list_get_returns_option() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.get").unwrap(), Ty::Forall { .. }),
            "List.get should be polymorphic");
}

#[test]
fn list_filter_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.filter").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_contains_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.contains").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_new_functions_registered() {
    let env = seeded_env();
    for name in &["List.find", "List.any", "List.all", "List.sort", "List.zip"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn list_find_returns_option() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.find").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_any_all_are_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.any").unwrap(), Ty::Forall { .. }));
    assert!(matches!(env.lookup("List.all").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_sort_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.sort").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_zip_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.zip").unwrap(), Ty::Forall { .. }));
}

#[test]
fn map_from_list_registered() {
    let env = seeded_env();
    assert!(matches!(env.lookup("Map.fromList").unwrap(), Ty::Forall { .. }));
}

#[test]
fn list_new_collection_functions_registered() {
    let env = seeded_env();
    for name in &["List.distinct", "List.partition", "List.chunked", "List.groupBy"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn list_distinct_partition_chunked_group_by_are_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("List.distinct").unwrap(),  Ty::Forall { .. }));
    assert!(matches!(env.lookup("List.partition").unwrap(), Ty::Forall { .. }));
    assert!(matches!(env.lookup("List.chunked").unwrap(),   Ty::Forall { .. }));
    assert!(matches!(env.lookup("List.groupBy").unwrap(),   Ty::Forall { .. }));
}

// ------------------------------------------------------------------ //
// Collections — Map extras
// ------------------------------------------------------------------ //

#[test]
fn map_extra_functions_registered() {
    let env = seeded_env();
    for name in &["Map.get", "Map.contains", "Map.remove",
                  "Map.len", "Map.keys", "Map.values"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn map_get_is_polymorphic() {
    let env = seeded_env();
    assert!(matches!(env.lookup("Map.get").unwrap(),      Ty::Forall { .. }));
    assert!(matches!(env.lookup("Map.remove").unwrap(),   Ty::Forall { .. }));
    assert!(matches!(env.lookup("Map.keys").unwrap(),     Ty::Forall { .. }));
    assert!(matches!(env.lookup("Map.values").unwrap(),   Ty::Forall { .. }));
}

// ------------------------------------------------------------------ //
// Text extras
// ------------------------------------------------------------------ //

#[test]
fn text_extra_functions_registered() {
    let env = seeded_env();
    for name in &["Text.eq", "Text.startsWith", "Text.endsWith",
                  "Text.trimStart", "Text.trimEnd", "Text.slice", "Text.repeat"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn text_eq_type() {
    let env = seeded_env();
    match env.lookup("Text.eq").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Bool);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn text_repeat_type() {
    let env = seeded_env();
    match env.lookup("Text.repeat").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::Text);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn text_index_of_returns_option_int() {
    let env = seeded_env();
    match env.lookup("Text.indexOf").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Int)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// DateTime — component accessors and comparisons
// ------------------------------------------------------------------ //

#[test]
fn datetime_component_accessors_registered() {
    let env = seeded_env();
    for name in &["DateTime.month", "DateTime.day", "DateTime.hour",
                  "DateTime.minute", "DateTime.second",
                  "DateTime.addSeconds", "DateTime.addMinutes", "DateTime.addHours",
                  "DateTime.diffSeconds", "DateTime.before", "DateTime.after",
                  "DateTime.eq", "Date.today", "Date.format"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn datetime_before_type() {
    let env = seeded_env();
    let dt = Ty::Named { name: "DateTime".into(), args: vec![] };
    match env.lookup("DateTime.before").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[dt.clone(), dt]);
            assert_eq!(ret.as_ref(), &Ty::Bool);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn datetime_diff_seconds_type() {
    let env = seeded_env();
    let dt = Ty::Named { name: "DateTime".into(), args: vec![] };
    match env.lookup("DateTime.diffSeconds").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[dt.clone(), dt]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Duration
// ------------------------------------------------------------------ //

#[test]
fn duration_functions_registered() {
    let env = seeded_env();
    for name in &["Duration.seconds", "Duration.minutes", "Duration.hours", "Duration.days",
                  "Duration.toSeconds", "Duration.toMinutes", "Duration.toHours", "Duration.toDays",
                  "Duration.add", "Duration.sub", "Duration.negate",
                  "Duration.eq", "Duration.lt", "Duration.gt",
                  "DateTime.addDuration", "DateTime.diff", "Date.addDuration"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn duration_days_type() {
    let env = seeded_env();
    let dur = Ty::Named { name: "Duration".into(), args: vec![] };
    match env.lookup("Duration.days").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int]);
            assert_eq!(ret.as_ref(), &dur);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn duration_add_type() {
    let env = seeded_env();
    let dur = Ty::Named { name: "Duration".into(), args: vec![] };
    match env.lookup("Duration.add").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[dur.clone(), dur.clone()]);
            assert_eq!(ret.as_ref(), &dur);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn datetime_add_duration_type() {
    let env = seeded_env();
    let dt = Ty::Named { name: "DateTime".into(), args: vec![] };
    let dur = Ty::Named { name: "Duration".into(), args: vec![] };
    match env.lookup("DateTime.addDuration").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[dt.clone(), dur]);
            assert_eq!(ret.as_ref(), &dt);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn datetime_diff_returns_duration() {
    let env = seeded_env();
    let dt = Ty::Named { name: "DateTime".into(), args: vec![] };
    let dur = Ty::Named { name: "Duration".into(), args: vec![] };
    match env.lookup("DateTime.diff").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[dt.clone(), dt]);
            assert_eq!(ret.as_ref(), &dur);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn date_add_duration_type() {
    let env = seeded_env();
    let date = Ty::Named { name: "Date".into(), args: vec![] };
    let dur = Ty::Named { name: "Duration".into(), args: vec![] };
    match env.lookup("Date.addDuration").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[date.clone(), dur]);
            assert_eq!(ret.as_ref(), &date);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn datetime_c_contains_duration_functions() {
    assert!(DATETIME_C.contains("certo_duration_seconds"),      "missing duration_seconds");
    assert!(DATETIME_C.contains("certo_duration_add"),          "missing duration_add");
    assert!(DATETIME_C.contains("certo_datetime_add_duration"), "missing datetime_add_duration");
    assert!(DATETIME_C.contains("certo_datetime_diff"),         "missing datetime_diff");
    assert!(DATETIME_C.contains("certo_date_add_duration"),     "missing date_add_duration");
}

// ------------------------------------------------------------------ //
// Money / Decimal extras
// ------------------------------------------------------------------ //

#[test]
fn decimal_comparison_functions_registered() {
    let env = seeded_env();
    for name in &["Decimal.eq", "Decimal.lt", "Decimal.gt",
                  "Decimal.lte", "Decimal.gte",
                  "Decimal.abs", "Decimal.negate",
                  "Decimal.toInt", "Decimal.fromInt",
                  "Money.toCents", "Money.fromDecimal"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn decimal_lt_type() {
    let env = seeded_env();
    match env.lookup("Decimal.lt").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Decimal, Ty::Decimal]);
            assert_eq!(ret.as_ref(), &Ty::Bool);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn decimal_to_int_type() {
    let env = seeded_env();
    match env.lookup("Decimal.toInt").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Decimal]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Env
// ------------------------------------------------------------------ //

#[test]
fn env_functions_registered() {
    let env = seeded_env();
    assert!(env.lookup("getEnv").is_some(),   "missing getEnv");
    assert!(env.lookup("setEnv").is_some(),   "missing setEnv");
    assert!(env.lookup("unsetEnv").is_some(), "missing unsetEnv");
}

#[test]
fn get_env_returns_option_text() {
    let env = seeded_env();
    match env.lookup("getEnv").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Text)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn env_c_contains_key_functions() {
    assert!(ENV_C.contains("certo_get_env"),   "missing get_env");
    assert!(ENV_C.contains("certo_set_env"),   "missing set_env");
    assert!(ENV_C.contains("certo_unset_env"), "missing unset_env");
}

// ------------------------------------------------------------------ //
// File
// ------------------------------------------------------------------ //

#[test]
fn file_functions_registered() {
    let env = seeded_env();
    for name in &["readFile", "writeFile", "appendFile",
                  "fileExists", "deleteFile", "listDir"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn read_file_returns_option_text() {
    let env = seeded_env();
    match env.lookup("readFile").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Text)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn write_file_returns_bool() {
    let env = seeded_env();
    match env.lookup("writeFile").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Bool);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn list_dir_returns_option_list_text() {
    let env = seeded_env();
    match env.lookup("listDir").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(),
                &Ty::Option(Box::new(Ty::List(Box::new(Ty::Text)))));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn option_producers_heap_box_their_results() {
    // Every stdlib Option producer must heap-box its Some value (via
    // __certo_opt_box), so `match`/`??` can safely dereference it. Returning a
    // value inline caused silent crashes (e.g. `readFile(...) ?? ""`).
    assert!(CORE_C.contains("void* certo_arg("), "arg must return a boxed Option");
    assert!(CORE_C.contains("__certo_opt_box((int64_t)__certo_argv"), "arg must heap-box");
    assert!(FILE_C.contains("void* certo_read_file("), "read_file must return a boxed Option");
    assert!(FILE_C.contains("__certo_opt_box((int64_t)buf)"), "read_file must heap-box");
    assert!(ENV_C.contains("void* certo_get_env("), "get_env must return a boxed Option");
}

#[test]
fn file_c_contains_key_functions() {
    assert!(FILE_C.contains("certo_read_file"),   "missing read_file");
    assert!(FILE_C.contains("certo_write_file"),  "missing write_file");
    assert!(FILE_C.contains("certo_file_exists"), "missing file_exists");
    assert!(FILE_C.contains("certo_list_dir"),    "missing list_dir");
    assert!(FILE_C.contains("certo_append_file"), "missing append_file");
    assert!(FILE_C.contains("certo_delete_file"), "missing delete_file");
}

// ------------------------------------------------------------------ //
// Path
// ------------------------------------------------------------------ //

#[test]
fn path_functions_registered() {
    let env = seeded_env();
    for name in &["Path.join", "Path.basename", "Path.dirname",
                  "Path.extension", "Path.stem"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn path_join_type() {
    let env = seeded_env();
    match env.lookup("Path.join").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Text);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn path_extension_returns_option_text() {
    let env = seeded_env();
    match env.lookup("Path.extension").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Text)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn path_c_contains_key_functions() {
    assert!(PATH_C.contains("certo_path_join"),      "missing path_join");
    assert!(PATH_C.contains("certo_path_basename"),  "missing path_basename");
    assert!(PATH_C.contains("certo_path_dirname"),   "missing path_dirname");
    assert!(PATH_C.contains("certo_path_extension"), "missing path_extension");
    assert!(PATH_C.contains("certo_path_stem"),      "missing path_stem");
}

// ------------------------------------------------------------------ //
// Process
// ------------------------------------------------------------------ //

#[test]
fn process_functions_registered() {
    let env = seeded_env();
    assert!(env.lookup("Process.exec").is_some(),           "missing Process.exec");
    assert!(env.lookup("ProcessResult.exitCode").is_some(), "missing ProcessResult.exitCode");
    assert!(env.lookup("ProcessResult.stdout").is_some(),   "missing ProcessResult.stdout");
    assert!(env.lookup("ProcessResult.stderr").is_some(),   "missing ProcessResult.stderr");
}

#[test]
fn process_exec_return_type_is_process_result() {
    let env = seeded_env();
    match env.lookup("Process.exec").unwrap() {
        Ty::Fn { ret, .. } => {
            assert!(matches!(ret.as_ref(),
                Ty::Named { name, .. } if name == "ProcessResult"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn process_result_exit_code_returns_int() {
    let env = seeded_env();
    match env.lookup("ProcessResult.exitCode").unwrap() {
        Ty::Fn { ret, .. } => assert_eq!(ret.as_ref(), &Ty::Int),
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn process_c_contains_key_functions() {
    assert!(PROCESS_C.contains("certo_process_exec"),              "missing process_exec");
    assert!(PROCESS_C.contains("certo_process_result_exit_code"),  "missing exit_code");
    assert!(PROCESS_C.contains("certo_process_result_stdout"),     "missing stdout");
    assert!(PROCESS_C.contains("certo_process_result_stderr"),     "missing stderr");
}

// ------------------------------------------------------------------ //
// full_c_runtime includes new modules
// ------------------------------------------------------------------ //

#[test]
fn full_c_runtime_includes_new_modules() {
    let rt = full_c_runtime();
    assert!(rt.contains("certo_get_env"),      "missing env");
    assert!(rt.contains("certo_read_file"),    "missing file");
    assert!(rt.contains("certo_path_join"),    "missing path");
    assert!(rt.contains("certo_process_exec"), "missing process");
}

#[test]
fn money_to_cents_type() {
    let env = seeded_env();
    match env.lookup("Money.toCents").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Decimal]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

// ------------------------------------------------------------------ //
// Json
// ------------------------------------------------------------------ //

#[test]
fn json_parse_stringify_registered() {
    let env = seeded_env();
    assert!(env.lookup("Json.parse").is_some(),     "missing Json.parse");
    assert!(env.lookup("Json.stringify").is_some(), "missing Json.stringify");
}

#[test]
fn json_parse_returns_json_value() {
    let env = seeded_env();
    match env.lookup("Json.parse").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert!(matches!(ret.as_ref(), Ty::Named { name, .. } if name == "JsonValue"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn json_stringify_takes_json_value() {
    let env = seeded_env();
    match env.lookup("Json.stringify").unwrap() {
        Ty::Fn { params, ret } => {
            assert!(matches!(&params[0], Ty::Named { name, .. } if name == "JsonValue"));
            assert_eq!(ret.as_ref(), &Ty::Text);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn json_constructors_registered() {
    let env = seeded_env();
    for name in &["Json.null", "Json.bool", "Json.int", "Json.float",
                  "Json.string", "Json.array", "Json.object"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn json_type_predicates_registered() {
    let env = seeded_env();
    for name in &["JsonValue.isNull", "JsonValue.isBool", "JsonValue.isInt",
                  "JsonValue.isFloat", "JsonValue.isString",
                  "JsonValue.isArray", "JsonValue.isObject"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn json_value_extractors_registered() {
    let env = seeded_env();
    for name in &["JsonValue.asBool", "JsonValue.asInt",
                  "JsonValue.asFloat", "JsonValue.asText"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn json_as_int_returns_int() {
    let env = seeded_env();
    let jv = Ty::Named { name: "JsonValue".into(), args: vec![] };
    match env.lookup("JsonValue.asInt").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[jv]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn json_structural_accessors_registered() {
    let env = seeded_env();
    assert!(env.lookup("JsonValue.length").is_some(), "missing length");
    assert!(env.lookup("JsonValue.at").is_some(),     "missing at");
    assert!(env.lookup("JsonValue.get").is_some(),    "missing get");
    assert!(env.lookup("JsonValue.keys").is_some(),   "missing keys");
}

#[test]
fn json_at_takes_int_index() {
    let env = seeded_env();
    match env.lookup("JsonValue.at").unwrap() {
        Ty::Fn { params, .. } => {
            assert!(matches!(&params[0], Ty::Named { name, .. } if name == "JsonValue"));
            assert_eq!(&params[1], &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn json_get_takes_text_key() {
    let env = seeded_env();
    match env.lookup("JsonValue.get").unwrap() {
        Ty::Fn { params, .. } => {
            assert!(matches!(&params[0], Ty::Named { name, .. } if name == "JsonValue"));
            assert_eq!(&params[1], &Ty::Text);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn json_mutation_functions_registered() {
    let env = seeded_env();
    assert!(env.lookup("JsonValue.push").is_some(), "missing push");
    assert!(env.lookup("JsonValue.set").is_some(),  "missing set");
}

#[test]
fn json_c_contains_key_functions() {
    assert!(JSON_C.contains("certo_json_parse"),      "missing json_parse");
    assert!(JSON_C.contains("certo_json_stringify"),  "missing json_stringify");
    assert!(JSON_C.contains("certo_json_get"),        "missing json_get");
    assert!(JSON_C.contains("certo_json_at"),         "missing json_at");
    assert!(JSON_C.contains("certo_json_array_push"), "missing json_array_push");
    assert!(JSON_C.contains("certo_json_object_set"), "missing json_object_set");
}

// ------------------------------------------------------------------ //
// Http
// ------------------------------------------------------------------ //

#[test]
fn http_functions_registered() {
    let env = seeded_env();
    for name in &["Http.get", "Http.post", "Http.put", "Http.delete",
                  "Http.request", "Http.requestBytes"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn http_request_bytes_takes_bytes_body() {
    let env = seeded_env();
    let list_hdr = Ty::List(Box::new(Ty::List(Box::new(Ty::Text))));
    let bytes = Ty::Named { name: "Bytes".into(), args: vec![] };
    match env.lookup("Http.requestBytes").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text, list_hdr, bytes]);
            assert!(matches!(ret.as_ref(), Ty::Named { name, .. } if name == "HttpResponse"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    assert!(HTTP_C.contains("certo_http_request_bytes"), "missing http_request_bytes");
}

#[test]
fn http_request_takes_method_url_headers_body() {
    let env = seeded_env();
    let list_hdr = Ty::List(Box::new(Ty::List(Box::new(Ty::Text))));
    match env.lookup("Http.request").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text, list_hdr, Ty::Text]);
            assert!(matches!(ret.as_ref(), Ty::Named { name, .. } if name == "HttpResponse"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn http_get_returns_http_response() {
    let env = seeded_env();
    match env.lookup("Http.get").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert!(matches!(ret.as_ref(), Ty::Named { name, .. } if name == "HttpResponse"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn http_post_takes_three_text_args() {
    let env = seeded_env();
    match env.lookup("Http.post").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text, Ty::Text, Ty::Text]);
            assert!(matches!(ret.as_ref(), Ty::Named { name, .. } if name == "HttpResponse"));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn http_response_accessors_registered() {
    let env = seeded_env();
    for name in &["HttpResponse.status", "HttpResponse.body",
                  "HttpResponse.contentType", "HttpResponse.ok"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn http_response_status_returns_int() {
    let env = seeded_env();
    let hr = Ty::Named { name: "HttpResponse".into(), args: vec![] };
    match env.lookup("HttpResponse.status").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[hr]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn http_response_ok_returns_bool() {
    let env = seeded_env();
    match env.lookup("HttpResponse.ok").unwrap() {
        Ty::Fn { ret, .. } => assert_eq!(ret.as_ref(), &Ty::Bool),
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn bytes_functions_registered() {
    let env = seeded_env();
    for name in &["Bytes.length", "Bytes.empty", "Bytes.slice", "Bytes.concat",
                  "Bytes.toHex", "Bytes.fromText", "readFileBytes", "writeFileBytes"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
}

#[test]
fn bytes_slice_takes_bytes_int_int_returns_bytes() {
    let env = seeded_env();
    let bytes = Ty::Named { name: "Bytes".into(), args: vec![] };
    match env.lookup("Bytes.slice").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[bytes.clone(), Ty::Int, Ty::Int]);
            assert_eq!(ret.as_ref(), &bytes);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn read_file_bytes_returns_option_bytes() {
    let env = seeded_env();
    let bytes = Ty::Named { name: "Bytes".into(), args: vec![] };
    match env.lookup("readFileBytes").unwrap() {
        Ty::Fn { ret, .. } => {
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(bytes)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn sha256_bytes_registered_bytes_to_bytes() {
    let env = seeded_env();
    let bytes = Ty::Named { name: "Bytes".into(), args: vec![] };
    match env.lookup("Crypto.sha256Bytes").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[bytes.clone()]);
            assert_eq!(ret.as_ref(), &bytes);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    assert!(full_c_runtime().contains("certo_crypto_sha256_bytes"),
            "sha256_bytes missing from runtime");
}

#[test]
fn runsql_support_functions_registered() {
    let env = seeded_env();
    // dbRunScript: (Int, Text) -> Int
    match env.lookup("dbRunScript").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int, Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Int);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    // monotonicMillis: () -> Int, flush: () -> Unit
    match env.lookup("monotonicMillis").unwrap() {
        Ty::Fn { params, ret } => { assert!(params.is_empty()); assert_eq!(ret.as_ref(), &Ty::Int); }
        other => panic!("expected Fn, got {:?}", other),
    }
    assert!(env.lookup("flush").is_some(), "flush not registered");
    assert!(DB_C.contains("certo_db_run_script"), "missing db_run_script in DB_C");
    assert!(CORE_C.contains("certo_monotonic_millis"), "missing monotonic_millis");
    assert!(CORE_C.contains("certo_flush"), "missing flush");
}

#[test]
fn sleep_registered() {
    // sleep(ms: Int): Unit — added as the test harness for BACKLOG item 81's
    // parallel(timeout:) enforcement, but a generally useful primitive in
    // its own right.
    let env = seeded_env();
    match env.lookup("sleep").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Int]);
            assert_eq!(ret.as_ref(), &Ty::Unit);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    assert!(CORE_C.contains("certo_sleep"), "missing certo_sleep in CORE_C");
}

#[test]
fn dbresult_functions_registered() {
    let env = seeded_env();
    for name in &["dbRunScriptResult", "DbResult.ok", "DbResult.error",
                  "DbResult.columns", "DbResult.rows"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
    assert!(DB_C.contains("certo_db_run_script_result"), "missing db_run_script_result");
}

#[test]
fn db_query_boxes_option_cells() {
    // Regression: dbQuery/dbQueryOne/dbStream/DbResult.rows must box Some(text)
    // as a heap Option via __certo_opt_box — storing a raw char* makes `?? `
    // dereference a string as an Option box and crash.
    assert!(DB_C.contains("__certo_opt_box"),
            "db Option<Text> cells must be boxed with __certo_opt_box");
}

#[test]
fn make_dir_registered() {
    let env = seeded_env();
    match env.lookup("makeDir").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Bool);
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    assert!(FILE_C.contains("certo_make_dir"), "missing make_dir in FILE_C");
}

#[test]
fn credential_functions_registered() {
    let env = seeded_env();
    let bytes = Ty::Named { name: "Bytes".into(), args: vec![] };
    for name in &["Credential.getBytes", "Credential.get", "Credential.set", "Credential.delete"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
    }
    match env.lookup("Credential.getBytes").unwrap() {
        Ty::Fn { params, ret } => {
            assert_eq!(params, &[Ty::Text]);
            assert_eq!(ret.as_ref(), &Ty::Option(Box::new(bytes)));
        }
        other => panic!("expected Fn, got {:?}", other),
    }
    match env.lookup("Credential.get").unwrap() {
        Ty::Fn { ret, .. } => assert_eq!(ret.as_ref(), &Ty::Option(Box::new(Ty::Text))),
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn credential_c_contains_key_functions() {
    assert!(CREDENTIAL_C.contains("certo_credential_get_bytes"), "missing credential_get_bytes");
    assert!(CREDENTIAL_C.contains("certo_credential_get"),       "missing credential_get");
    assert!(CREDENTIAL_C.contains("certo_credential_set"),       "missing credential_set");
    assert!(CREDENTIAL_C.contains("certo_credential_delete"),    "missing credential_delete");
    assert!(full_c_runtime().contains("certo_credential_get"),   "credential missing from runtime");
}

#[test]
fn bytes_c_contains_key_functions() {
    assert!(BYTES_C.contains("certo_bytes_slice"),      "missing bytes_slice");
    assert!(BYTES_C.contains("certo_bytes_concat"),     "missing bytes_concat");
    assert!(BYTES_C.contains("certo_bytes_to_hex"),     "missing bytes_to_hex");
    assert!(BYTES_C.contains("certo_read_file_bytes"),  "missing read_file_bytes");
    assert!(BYTES_C.contains("certo_write_file_bytes"), "missing write_file_bytes");
    assert!(full_c_runtime().contains("certo_bytes_slice"), "bytes missing from runtime");
}

#[test]
fn http_c_contains_key_functions() {
    assert!(HTTP_C.contains("certo_http_get"),             "missing http_get");
    assert!(HTTP_C.contains("certo_http_post"),            "missing http_post");
    assert!(HTTP_C.contains("certo_http_request"),         "missing http_request");
    assert!(HTTP_C.contains("certo_http_response_status"), "missing response_status");
    assert!(HTTP_C.contains("certo_http_response_body"),   "missing response_body");
    assert!(HTTP_C.contains("certo_http_response_ok"),     "missing response_ok");
}

#[test]
fn full_c_runtime_includes_json_and_http() {
    let rt = full_c_runtime();
    assert!(rt.contains("certo_json_parse"), "missing json in runtime");
    assert!(rt.contains("certo_http_get"),   "missing http in runtime");
}

// ------------------------------------------------------------------ //
// seed_stdlib_effects
// ------------------------------------------------------------------ //

#[test]
fn seed_stdlib_effects_registers_known_io_functions() {
    use certo_ast::types::Effect;
    let mut env = certo_effects::EffectEnv::new();
    crate::seed_stdlib_effects(&mut env);

    for name in ["println", "eprintln", "readLine", "readAll", "dbConnect", "dbQuery", "setEnv",
                 "Http.get", "Query.list", "Mutation.run"] {
        let declared = env.get(name).unwrap_or_else(|| panic!("{} not registered", name));
        assert!(declared.effects.contains(&Effect::Io),
            "{} registered without Io: {:?}", name, declared);
    }
}

#[test]
fn seed_stdlib_effects_leaves_pure_functions_unregistered() {
    // Absence from the env is exactly what lets a pure function call these
    // without a [pure] violation — a *wrong* effect entry would be worse
    // than no entry at all.
    let mut env = certo_effects::EffectEnv::new();
    crate::seed_stdlib_effects(&mut env);

    for name in ["intToText", "absInt", "pow", "range", "List.len", "Text.len"] {
        assert!(env.get(name).is_none(), "{} should not be registered as effectful", name);
    }
}

// ------------------------------------------------------------------ //
// Result combinators — full type-check against real source
// ------------------------------------------------------------------ //

/// Unlike `seeded_env`, this also seeds the typeck-builtin constructors
/// (`Ok`/`Err`/`Some`/`None`/`true`/`false`) via `seed_builtins`, matching
/// exactly what the real CLI does (`crates/cli/src/main.rs`'s `run_typeck`)
/// — needed here since these tests exercise real source using `Ok`/`Err`.
fn check_full(src: &str) -> Result<(), Vec<certo_typeck::TypeError>> {
    let module = certo_parser::parse(src).expect("parse error");
    let mut env = TypeEnv::new();
    let mut counter = 0u32;
    env.seed_builtins(&mut counter);
    seed_stdlib(&mut env, &mut counter);
    certo_typeck::check_module_seeded(&module, env, counter)
}

#[test]
fn flat_map_chains_fallible_ops_ok() {
    check_full(
        "module A
fn parsePositive(s: Text): Result<Int, Text> = Ok(1)
fn doubleIt(n: Int): Result<Int, Text> = Ok(n * 2)
fn f(): Result<Int, Text> = parsePositive(\"5\") |> flatMap(doubleIt)"
    ).unwrap();
}

#[test]
fn map_err_transforms_error_side_ok() {
    check_full(
        "module A
fn f(): Result<Int, Text> = Err(\"boom\") |> mapErr((e) => \"wrapped: \" ++ e)"
    ).unwrap();
}

#[test]
fn get_or_else_unwraps_to_payload_type_ok() {
    check_full("module A\nfn f(): Int = Ok(1) |> getOrElse(0)").unwrap();
}

#[test]
fn recover_unwraps_to_payload_type_ok() {
    check_full("module A\nfn f(): Int = Err(\"e\") |> recover((e) => 0)").unwrap();
}

#[test]
fn result_all_wraps_list_of_payloads_ok() {
    check_full("module A\nfn f(): Result<List<Int>, Text> = Result.all([Ok(1), Ok(2)])").unwrap();
}

#[test]
fn result_all_settled_returns_list_of_results_ok() {
    check_full("module A\nfn f(): List<Result<Int, Text>> = Result.allSettled([Ok(1), Err(\"e\")])").unwrap();
}

#[test]
fn flat_map_with_non_result_returning_fn_is_type_error() {
    // flatMap's callback must itself return a Result — a plain Int-returning
    // lambda must be rejected, not silently accepted.
    let errs = check_full(
        "module A\nfn f(): Result<Int, Text> = Ok(1) |> flatMap((x) => x + 1)"
    ).unwrap_err();
    assert!(!errs.is_empty(), "expected a type error for flatMap's callback not returning a Result");
}

// ------------------------------------------------------------------ //
// Collections — distinct/partition/chunked/groupBy — full type-check
// ------------------------------------------------------------------ //

#[test]
fn distinct_preserves_list_type_ok() {
    check_full("module A\nfn f(xs: List<Int>): List<Int> = List.distinct(xs)").unwrap();
}

#[test]
fn partition_returns_tuple_of_lists_ok() {
    check_full(
        "module A\nfn f(xs: List<Int>): (List<Int>, List<Int>) = List.partition(xs, (x) => x > 0)"
    ).unwrap();
}

#[test]
fn chunked_returns_list_of_lists_ok() {
    check_full("module A\nfn f(xs: List<Int>): List<List<Int>> = List.chunked(xs, 2)").unwrap();
}

#[test]
fn group_by_returns_map_of_key_to_list_ok() {
    check_full(
        "module A\nfn f(xs: List<Int>): Map<Int, List<Int>> = List.groupBy(xs, (x) => x % 2)"
    ).unwrap();
}

#[test]
fn partition_pred_must_return_bool_is_type_error() {
    let errs = check_full(
        "module A\nfn f(xs: List<Int>): (List<Int>, List<Int>) = List.partition(xs, (x) => x)"
    ).unwrap_err();
    assert!(!errs.is_empty(), "expected a type error for a non-Bool partition predicate");
}
