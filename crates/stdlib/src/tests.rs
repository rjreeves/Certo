use certo_typeck::{Ty, TypeEnv};
use crate::seed::seed_stdlib;
use crate::{CORE_C, COLLECTIONS_C, TEXT_C, DATETIME_C, MONEY_C,
            ENV_C, FILE_C, PATH_C, PROCESS_C, JSON_C, HTTP_C,
            full_c_runtime, certo_sources};

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
fn map_insert_registered() {
    let env = seeded_env();
    assert!(matches!(env.lookup("Map.insert").unwrap(), Ty::Forall { .. }));
}

#[test]
fn text_functions_registered() {
    let env = seeded_env();
    for name in &["Text.len", "Text.concat", "Text.contains", "Text.toUpper",
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
    assert!(COLLECTIONS_C.contains("certo_map_from_list"), "missing map_from_list");
}

#[test]
fn text_c_contains_key_functions() {
    assert!(TEXT_C.contains("certo_text_concat"),     "missing text_concat");
    assert!(TEXT_C.contains("certo_text_trim"),       "missing text_trim");
    assert!(TEXT_C.contains("certo_text_split"),      "missing text_split");
    assert!(TEXT_C.contains("certo_text_replace"),    "missing text_replace");
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

#[test]
fn certo_sources_covers_all_modules() {
    let srcs = certo_sources();
    let names: Vec<&str> = srcs.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"Stdlib.Core"));
    assert!(names.contains(&"Stdlib.Collections"));
    assert!(names.contains(&"Stdlib.Text"));
    assert!(names.contains(&"Stdlib.DateTime"));
    assert!(names.contains(&"Stdlib.Money"));
    // every module has non-empty source
    for (name, src) in &srcs {
        assert!(!src.is_empty(), "{} source is empty", name);
    }
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
                  "floatToInt", "intToFloat", "parseInt", "parseFloat"] {
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
fn certo_sources_includes_new_modules() {
    let srcs = certo_sources();
    let names: Vec<&str> = srcs.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"Stdlib.Env"),     "missing Stdlib.Env");
    assert!(names.contains(&"Stdlib.File"),    "missing Stdlib.File");
    assert!(names.contains(&"Stdlib.Path"),    "missing Stdlib.Path");
    assert!(names.contains(&"Stdlib.Process"), "missing Stdlib.Process");
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
    for name in &["Http.get", "Http.post", "Http.put", "Http.delete"] {
        assert!(env.lookup(name).is_some(), "missing: {}", name);
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
fn http_c_contains_key_functions() {
    assert!(HTTP_C.contains("certo_http_get"),             "missing http_get");
    assert!(HTTP_C.contains("certo_http_post"),            "missing http_post");
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

#[test]
fn certo_sources_includes_json_and_http() {
    let srcs = certo_sources();
    let names: Vec<&str> = srcs.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"Stdlib.Json"), "missing Stdlib.Json");
    assert!(names.contains(&"Stdlib.Http"), "missing Stdlib.Http");
}
