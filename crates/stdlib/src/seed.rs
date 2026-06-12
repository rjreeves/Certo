use certo_typeck::{Ty, TypeEnv};

/// Register all stdlib function types into the type environment.
///
/// Names are registered both qualified (`"List.len"`) and, for commonly
/// imported names, unqualified (`"len"` is not registered to avoid clashes,
/// but the qualified forms are always present).
pub fn seed_stdlib(env: &mut TypeEnv, counter: &mut u32) {
    macro_rules! def {
        ($name:expr, $ty:expr) => { env.define($name, $ty); };
    }
    let mut fresh = || { *counter += 1; *counter };

    // ---------------------------------------------------------------- //
    // Core
    // ---------------------------------------------------------------- //

    def!("print",    fn1(Ty::Text, Ty::Unit));
    def!("println",  fn1(Ty::Text, Ty::Unit));
    def!("eprint",   fn1(Ty::Text, Ty::Unit));
    def!("eprintln", fn1(Ty::Text, Ty::Unit));

    def!("intToText",   fn1(Ty::Int,   Ty::Text));
    def!("floatToText", fn1(Ty::Float, Ty::Text));
    def!("boolToText",  fn1(Ty::Bool,  Ty::Text));
    def!("floatToInt",  fn1(Ty::Float, Ty::Int));
    def!("intToFloat",  fn1(Ty::Int,   Ty::Float));

    def!("parseInt",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Int))));
    def!("parseFloat", fn1(Ty::Text, Ty::Option(Box::new(Ty::Float))));

    // Option constructors — Some(x) / None
    {
        let a = fresh();
        env.define("Some", poly1(a, fn1(Ty::Var(a), Ty::Option(Box::new(Ty::Var(a))))));
    }
    {
        let a = fresh();
        env.define("None", poly1(a, Ty::Option(Box::new(Ty::Var(a)))));
    }
    // Result constructors — Ok(x) / Err(e)
    {
        let a = fresh(); let b = fresh();
        env.define("Ok",  poly1(a, fn1(Ty::Var(a), Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))))));
    }
    {
        let a = fresh(); let b = fresh();
        env.define("Err", poly1(b, fn1(Ty::Var(b), Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))))));
    }

    def!("messageBox",      fn2(Ty::Text, Ty::Text, Ty::Unit)); // messageBox(title, message)
    def!("assert",  fn2(Ty::Bool, Ty::Text, Ty::Unit));
    def!("pow",     fn2(Ty::Int, Ty::Int, Ty::Int));
    def!("absInt",  fn1(Ty::Int, Ty::Int));
    def!("absFloat",fn1(Ty::Float, Ty::Float));
    def!("minInt",  fn2(Ty::Int, Ty::Int, Ty::Int));
    def!("maxInt",  fn2(Ty::Int, Ty::Int, Ty::Int));
    def!("minFloat",fn2(Ty::Float, Ty::Float, Ty::Float));
    def!("maxFloat",fn2(Ty::Float, Ty::Float, Ty::Float));
    def!("floor",   fn1(Ty::Float, Ty::Float));
    def!("ceil",    fn1(Ty::Float, Ty::Float));
    def!("round",   fn1(Ty::Float, Ty::Float));
    def!("sqrt",    fn1(Ty::Float, Ty::Float));
    def!("range",         fn2(Ty::Int, Ty::Int, Ty::List(Box::new(Ty::Int))));
    def!("rangeInclusive",fn2(Ty::Int, Ty::Int, Ty::List(Box::new(Ty::Int))));
    def!("readLine",  Ty::Fn { params: vec![], ret: Box::new(Ty::Option(Box::new(Ty::Text))) });
    def!("readAll",   Ty::Fn { params: vec![], ret: Box::new(Ty::Text) });
    def!("argCount",  Ty::Fn { params: vec![], ret: Box::new(Ty::Int) });
    def!("arg",       fn1(Ty::Int, Ty::Option(Box::new(Ty::Text))));

    // ---------------------------------------------------------------- //
    // Collections — List<T>
    // ---------------------------------------------------------------- //

    {
        let a = fresh();
        env.define("List.empty", poly1(a, Ty::List(Box::new(Ty::Var(a)))));
    }
    {
        let a = fresh();
        env.define("List.len", poly1(a, fn1(Ty::List(Box::new(Ty::Var(a))), Ty::Int)));
    }
    {
        let a = fresh();
        env.define("List.get", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), Ty::Int,
                Ty::Option(Box::new(Ty::Var(a))))));
    }
    {
        let a = fresh();
        env.define("List.getOrPanic", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), Ty::Int, Ty::Var(a))));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.push", poly1(a,
            fn2(list_a.clone(), Ty::Var(a), list_a)));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.concat", poly1(a,
            fn2(list_a.clone(), list_a.clone(), list_a)));
    }
    {
        let a = fresh();
        env.define("List.first", poly1(a,
            fn1(Ty::List(Box::new(Ty::Var(a))), Ty::Option(Box::new(Ty::Var(a))))));
    }
    {
        let a = fresh();
        env.define("List.last", poly1(a,
            fn1(Ty::List(Box::new(Ty::Var(a))), Ty::Option(Box::new(Ty::Var(a))))));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.slice", poly1(a,
            Ty::Fn { params: vec![list_a.clone(), Ty::Int, Ty::Int], ret: Box::new(list_a) }));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.reverse", poly1(a, fn1(list_a.clone(), list_a)));
    }
    {
        let a = fresh(); let b = fresh();
        let f_ty = fn1(Ty::Var(a), Ty::Var(b));
        env.define("List.map", Ty::Forall {
            vars: vec![a, b],
            body: Box::new(fn2(Ty::List(Box::new(Ty::Var(a))), f_ty,
                              Ty::List(Box::new(Ty::Var(b))))),
        });
    }
    {
        let a = fresh();
        let pred = fn1(Ty::Var(a), Ty::Bool);
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.filter", poly1(a, fn2(list_a.clone(), pred, list_a)));
    }
    {
        let t = fresh(); let acc = fresh();
        env.define("List.fold", Ty::Forall {
            vars: vec![t, acc],
            body: Box::new(Ty::Fn {
                params: vec![
                    Ty::List(Box::new(Ty::Var(t))),
                    Ty::Var(acc),
                    fn2(Ty::Var(acc), Ty::Var(t), Ty::Var(acc)),
                ],
                ret: Box::new(Ty::Var(acc)),
            }),
        });
    }
    {
        let a = fresh();
        env.define("List.contains", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), Ty::Var(a), Ty::Bool)));
    }
    {
        let a = fresh();
        let pred = fn1(Ty::Var(a), Ty::Bool);
        env.define("List.find", poly1(a, fn2(
            Ty::List(Box::new(Ty::Var(a))),
            pred,
            Ty::Option(Box::new(Ty::Var(a))),
        )));
    }
    {
        let a = fresh();
        let pred = fn1(Ty::Var(a), Ty::Bool);
        env.define("List.any", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), pred, Ty::Bool)));
    }
    {
        let a = fresh();
        let pred = fn1(Ty::Var(a), Ty::Bool);
        env.define("List.all", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), pred, Ty::Bool)));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        let cmp    = Ty::Fn { params: vec![Ty::Var(a), Ty::Var(a)], ret: Box::new(Ty::Int) };
        env.define("List.sort", poly1(a, fn2(list_a.clone(), cmp, list_a)));
    }
    {
        let a = fresh(); let b = fresh();
        let pair = Ty::Tuple(vec![Ty::Var(a), Ty::Var(b)]);
        env.define("List.zip", Ty::Forall {
            vars: vec![a, b],
            body: Box::new(fn2(
                Ty::List(Box::new(Ty::Var(a))),
                Ty::List(Box::new(Ty::Var(b))),
                Ty::List(Box::new(pair)),
            )),
        });
    }
    {
        let k = fresh(); let v = fresh();
        let pair = Ty::Tuple(vec![Ty::Var(k), Ty::Var(v)]);
        env.define("Map.fromList", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn1(
                Ty::List(Box::new(pair)),
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
            )),
        });
    }

    // ---------------------------------------------------------------- //
    // Collections — Map<K, V>
    // ---------------------------------------------------------------- //

    {
        let k = fresh(); let v = fresh();
        env.define("Map.empty", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v)))),
        });
    }
    {
        let k = fresh(); let v = fresh();
        let map = Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v)));
        env.define("Map.insert", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(Ty::Fn {
                params: vec![map.clone(), Ty::Var(k), Ty::Var(v)],
                ret:    Box::new(map),
            }),
        });
    }
    {
        let k = fresh(); let v = fresh();
        env.define("Map.get", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn2(
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
                Ty::Var(k),
                Ty::Option(Box::new(Ty::Var(v))),
            )),
        });
    }
    {
        let k = fresh(); let v = fresh();
        env.define("Map.contains", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn2(
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
                Ty::Var(k),
                Ty::Bool,
            )),
        });
    }
    {
        let k = fresh(); let v = fresh();
        let map = Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v)));
        env.define("Map.remove", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn2(map.clone(), Ty::Var(k), map)),
        });
    }
    {
        let k = fresh(); let v = fresh();
        env.define("Map.len", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn1(
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
                Ty::Int,
            )),
        });
    }
    {
        let k = fresh(); let v = fresh();
        env.define("Map.keys", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn1(
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
                Ty::List(Box::new(Ty::Var(k))),
            )),
        });
    }
    {
        let k = fresh(); let v = fresh();
        env.define("Map.values", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(fn1(
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v))),
                Ty::List(Box::new(Ty::Var(v))),
            )),
        });
    }

    // ---------------------------------------------------------------- //
    // Text
    // ---------------------------------------------------------------- //

    def!("Text.len",        fn1(Ty::Text, Ty::Int));
    def!("Text.concat",     fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Text.eq",         fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.contains",   fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.startsWith", fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.endsWith",   fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.toUpper",    fn1(Ty::Text, Ty::Text));
    def!("Text.toLower",    fn1(Ty::Text, Ty::Text));
    def!("Text.trim",       fn1(Ty::Text, Ty::Text));
    def!("Text.trimStart",  fn1(Ty::Text, Ty::Text));
    def!("Text.trimEnd",    fn1(Ty::Text, Ty::Text));
    def!("Text.slice",      Ty::Fn { params: vec![Ty::Text, Ty::Int, Ty::Int], ret: Box::new(Ty::Text) });
    def!("Text.indexOf",    fn2(Ty::Text, Ty::Text, Ty::Option(Box::new(Ty::Int))));
    def!("Text.replace",    Ty::Fn { params: vec![Ty::Text, Ty::Text, Ty::Text], ret: Box::new(Ty::Text) });
    def!("Text.split",      fn2(Ty::Text, Ty::Text, Ty::List(Box::new(Ty::Text))));
    def!("Text.join",       fn2(Ty::List(Box::new(Ty::Text)), Ty::Text, Ty::Text));
    def!("Text.repeat",     fn2(Ty::Text, Ty::Int, Ty::Text));

    // ---------------------------------------------------------------- //
    // DateTime  (represented as Int / Named types in the type system)
    // ---------------------------------------------------------------- //

    let dt = || Ty::Named { name: "DateTime".into(), args: vec![] };
    let date = || Ty::Named { name: "Date".into(), args: vec![] };

    def!("DateTime.now",        Ty::Fn { params: vec![], ret: Box::new(dt()) });
    def!("Date.today",          Ty::Fn { params: vec![], ret: Box::new(date()) });
    def!("DateTime.fromUnix",   fn1(Ty::Int, dt()));
    def!("DateTime.toUnix",     fn1(dt(), Ty::Int));
    def!("DateTime.format",     fn2(dt(), Ty::Text, Ty::Text));
    def!("Date.format",         fn2(date(), Ty::Text, Ty::Text));
    def!("DateTime.toIso",      fn1(dt(), Ty::Text));
    def!("DateTime.parseIso",   fn1(Ty::Text, dt()));
    def!("DateTime.addSeconds", fn2(dt(), Ty::Int, dt()));
    def!("DateTime.addMinutes", fn2(dt(), Ty::Int, dt()));
    def!("DateTime.addHours",   fn2(dt(), Ty::Int, dt()));
    def!("DateTime.addDays",    fn2(dt(), Ty::Int, dt()));
    def!("DateTime.diffSeconds",fn2(dt(), dt(), Ty::Int));
    def!("DateTime.diffDays",   fn2(dt(), dt(), Ty::Int));
    def!("DateTime.before",     fn2(dt(), dt(), Ty::Bool));
    def!("DateTime.after",      fn2(dt(), dt(), Ty::Bool));
    def!("DateTime.eq",         fn2(dt(), dt(), Ty::Bool));
    def!("DateTime.year",       fn1(dt(), Ty::Int));
    def!("DateTime.month",      fn1(dt(), Ty::Int));
    def!("DateTime.day",        fn1(dt(), Ty::Int));
    def!("DateTime.hour",       fn1(dt(), Ty::Int));
    def!("DateTime.minute",     fn1(dt(), Ty::Int));
    def!("DateTime.second",     fn1(dt(), Ty::Int));

    // ---------------------------------------------------------------- //
    // Money / Decimal
    // ---------------------------------------------------------------- //

    def!("Decimal.add",        fn2(Ty::Decimal, Ty::Decimal, Ty::Decimal));
    def!("Decimal.sub",        fn2(Ty::Decimal, Ty::Decimal, Ty::Decimal));
    def!("Decimal.mul",        fn2(Ty::Decimal, Ty::Decimal, Ty::Decimal));
    def!("Decimal.div",        fn2(Ty::Decimal, Ty::Decimal, Ty::Decimal));
    def!("Decimal.eq",         fn2(Ty::Decimal, Ty::Decimal, Ty::Bool));
    def!("Decimal.lt",         fn2(Ty::Decimal, Ty::Decimal, Ty::Bool));
    def!("Decimal.gt",         fn2(Ty::Decimal, Ty::Decimal, Ty::Bool));
    def!("Decimal.lte",        fn2(Ty::Decimal, Ty::Decimal, Ty::Bool));
    def!("Decimal.gte",        fn2(Ty::Decimal, Ty::Decimal, Ty::Bool));
    def!("Decimal.abs",        fn1(Ty::Decimal, Ty::Decimal));
    def!("Decimal.negate",     fn1(Ty::Decimal, Ty::Decimal));
    def!("Decimal.round",      fn2(Ty::Decimal, Ty::Int, Ty::Decimal));
    def!("Decimal.toInt",      fn1(Ty::Decimal, Ty::Int));
    def!("Decimal.fromInt",    fn1(Ty::Int, Ty::Decimal));
    def!("Decimal.toText",     fn1(Ty::Decimal, Ty::Text));
    def!("Money.fromCents",    fn1(Ty::Int, Ty::Decimal));
    def!("Money.toCents",      fn1(Ty::Decimal, Ty::Int));
    def!("Money.fromDecimal",  fn1(Ty::Decimal, Ty::Decimal));

    // ---------------------------------------------------------------- //
    // Env
    // ---------------------------------------------------------------- //

    def!("getEnv",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
    def!("setEnv",   fn2(Ty::Text, Ty::Text, Ty::Unit));
    def!("unsetEnv", fn1(Ty::Text, Ty::Unit));

    // ---------------------------------------------------------------- //
    // File
    // ---------------------------------------------------------------- //

    def!("readFile",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
    def!("writeFile",  fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("appendFile", fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("fileExists", fn1(Ty::Text, Ty::Bool));
    def!("deleteFile", fn1(Ty::Text, Ty::Bool));
    {
        let list_text = Ty::List(Box::new(Ty::Text));
        def!("listDir", fn1(Ty::Text, Ty::Option(Box::new(list_text))));
    }

    // ---------------------------------------------------------------- //
    // Path
    // ---------------------------------------------------------------- //

    def!("Path.join",      fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Path.basename",  fn1(Ty::Text, Ty::Text));
    def!("Path.dirname",   fn1(Ty::Text, Ty::Text));
    def!("Path.extension", fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
    def!("Path.stem",      fn1(Ty::Text, Ty::Text));

    // ---------------------------------------------------------------- //
    // Process
    // ---------------------------------------------------------------- //

    {
        let pr = Ty::Named { name: "ProcessResult".into(), args: vec![] };
        let list_text = Ty::List(Box::new(Ty::Text));
        def!("Process.exec",           fn2(Ty::Text, list_text, pr.clone()));
        def!("ProcessResult.exitCode", fn1(pr.clone(), Ty::Int));
        def!("ProcessResult.stdout",   fn1(pr.clone(), Ty::Text));
        def!("ProcessResult.stderr",   fn1(pr.clone(), Ty::Text));
    }

    // ---------------------------------------------------------------- //
    // Json
    // ---------------------------------------------------------------- //

    {
        let jv   = || Ty::Named { name: "JsonValue".into(), args: vec![] };
        let list_text = Ty::List(Box::new(Ty::Text));

        def!("Json.parse",     fn1(Ty::Text, jv()));
        def!("Json.stringify", fn1(jv(), Ty::Text));

        def!("Json.null",   Ty::Fn { params: vec![], ret: Box::new(jv()) });
        def!("Json.bool",   fn1(Ty::Bool,  jv()));
        def!("Json.int",    fn1(Ty::Int,   jv()));
        def!("Json.float",  fn1(Ty::Float, jv()));
        def!("Json.string", fn1(Ty::Text,  jv()));
        def!("Json.array",  Ty::Fn { params: vec![], ret: Box::new(jv()) });
        def!("Json.object", Ty::Fn { params: vec![], ret: Box::new(jv()) });

        def!("JsonValue.isNull",   fn1(jv(), Ty::Bool));
        def!("JsonValue.isBool",   fn1(jv(), Ty::Bool));
        def!("JsonValue.isInt",    fn1(jv(), Ty::Bool));
        def!("JsonValue.isFloat",  fn1(jv(), Ty::Bool));
        def!("JsonValue.isString", fn1(jv(), Ty::Bool));
        def!("JsonValue.isArray",  fn1(jv(), Ty::Bool));
        def!("JsonValue.isObject", fn1(jv(), Ty::Bool));

        def!("JsonValue.asBool",   fn1(jv(), Ty::Bool));
        def!("JsonValue.asInt",    fn1(jv(), Ty::Int));
        def!("JsonValue.asFloat",  fn1(jv(), Ty::Float));
        def!("JsonValue.asText",   fn1(jv(), Ty::Text));

        def!("JsonValue.length", fn1(jv(), Ty::Int));
        def!("JsonValue.at",     fn2(jv(), Ty::Int,  jv()));
        def!("JsonValue.get",    fn2(jv(), Ty::Text, jv()));
        def!("JsonValue.keys",   fn1(jv(), list_text));

        def!("JsonValue.push", fn2(jv(), jv(), Ty::Unit));
        def!("JsonValue.set",  Ty::Fn { params: vec![jv(), Ty::Text, jv()], ret: Box::new(Ty::Unit) });
    }

    // ---------------------------------------------------------------- //
    // Db (PostgreSQL via libpq)
    // ---------------------------------------------------------------- //

    {
        let conn      = Ty::Int; // connection handle
        let list_text = Ty::List(Box::new(Ty::Text));
        let list_row  = Ty::List(Box::new(list_text.clone()));

        // Null sentinel
        def!("dbNull", Ty::Fn { params: vec![], ret: Box::new(Ty::Text) });

        // Connection
        def!("dbConnect",       fn1(Ty::Text, conn.clone()));
        def!("dbClose",         fn1(conn.clone(), Ty::Unit));
        def!("dbError",         fn1(conn.clone(), Ty::Text));

        // Server info
        def!("dbServerVersion", fn1(conn.clone(), Ty::Int));
        def!("dbVersionString", fn1(conn.clone(), Ty::Text));

        // Exec
        def!("dbExec", Ty::Fn {
            params: vec![conn.clone(), Ty::Text, list_text.clone()],
            ret: Box::new(Ty::Int),
        });

        // Query
        def!("dbQuery", Ty::Fn {
            params: vec![conn.clone(), Ty::Text, list_text.clone()],
            ret: Box::new(list_row.clone()),
        });

        // dbQueryTyped :: ∀T. (Int, Text, List<Text>, List<Text> -> T) -> List<T>
        {
            let t = fresh();
            let mapper = fn1(list_text.clone(), Ty::Var(t));
            env.define("dbQueryTyped", Ty::Forall {
                vars: vec![t],
                body: Box::new(Ty::Fn {
                    params: vec![conn.clone(), Ty::Text, list_text.clone(), mapper],
                    ret: Box::new(Ty::List(Box::new(Ty::Var(t)))),
                }),
            });
        }
        def!("dbQueryRow", Ty::Fn {
            params: vec![conn.clone(), Ty::Text, list_text.clone()],
            ret: Box::new(Ty::Option(Box::new(list_text.clone()))),
        });
        def!("dbQueryOne",  fn2(conn.clone(), Ty::Text, Ty::Text));
        def!("dbColumns",   fn2(conn.clone(), Ty::Text, list_text.clone()));

        // Transactions
        def!("dbBegin",    fn1(conn.clone(), Ty::Int));
        def!("dbCommit",   fn1(conn.clone(), Ty::Int));
        def!("dbRollback", fn1(conn.clone(), Ty::Int));
    }

    // ---------------------------------------------------------------- //
    // Http
    // ---------------------------------------------------------------- //

    {
        let hr  = || Ty::Named { name: "HttpResponse".into(), args: vec![] };
        let req = || Ty::Named { name: "HttpRequest".into(),  args: vec![] };
        let list_text = Ty::List(Box::new(Ty::Text));
        let list_hdr  = Ty::List(Box::new(Ty::List(Box::new(Ty::Text))));

        // Client
        def!("Http.get",    fn1(Ty::Text, hr()));
        def!("Http.delete", fn1(Ty::Text, hr()));
        def!("Http.post",   Ty::Fn { params: vec![Ty::Text, Ty::Text, Ty::Text], ret: Box::new(hr()) });
        def!("Http.put",    Ty::Fn { params: vec![Ty::Text, Ty::Text, Ty::Text], ret: Box::new(hr()) });

        // HttpResponse accessors
        def!("HttpResponse.status",      fn1(hr(), Ty::Int));
        def!("HttpResponse.body",        fn1(hr(), Ty::Text));
        def!("HttpResponse.contentType", fn1(hr(), Ty::Text));
        def!("HttpResponse.ok",          fn1(hr(), Ty::Bool));

        // Server
        let handler_ty = Ty::Fn { params: vec![req()], ret: Box::new(hr()) };
        def!("Http.serve", Ty::Fn {
            params: vec![Ty::Int, handler_ty],
            ret:    Box::new(Ty::Unit),
        });

        // Response constructors
        def!("Http.respond",     Ty::Fn { params: vec![Ty::Int, Ty::Text, Ty::Text], ret: Box::new(hr()) });
        def!("Http.ok",          fn2(Ty::Text, Ty::Text, hr()));
        def!("Http.notFound",    fn1(Ty::Text, hr()));
        def!("Http.badRequest",  fn1(Ty::Text, hr()));
        def!("Http.serverError", fn1(Ty::Text, hr()));

        // HttpRequest accessors
        def!("HttpRequest.method",  fn1(req(), Ty::Text));
        def!("HttpRequest.path",    fn1(req(), Ty::Text));
        def!("HttpRequest.query",   fn1(req(), Ty::Text));
        def!("HttpRequest.body",    fn1(req(), Ty::Text));
        def!("HttpRequest.header",  fn2(req(), Ty::Text, Ty::Text));
        def!("HttpRequest.headers", fn1(req(), list_hdr));
        let _ = list_text; // may be used later
    }

    // ---------------------------------------------------------------- //
    // Math
    // ---------------------------------------------------------------- //

    def!("Math.pi",    Ty::Fn { params: vec![], ret: Box::new(Ty::Float) });
    def!("Math.e",     Ty::Fn { params: vec![], ret: Box::new(Ty::Float) });

    def!("Math.sin",   fn1(Ty::Float, Ty::Float));
    def!("Math.cos",   fn1(Ty::Float, Ty::Float));
    def!("Math.tan",   fn1(Ty::Float, Ty::Float));
    def!("Math.asin",  fn1(Ty::Float, Ty::Float));
    def!("Math.acos",  fn1(Ty::Float, Ty::Float));
    def!("Math.atan",  fn1(Ty::Float, Ty::Float));
    def!("Math.atan2", fn2(Ty::Float, Ty::Float, Ty::Float));

    def!("Math.log",   fn1(Ty::Float, Ty::Float));
    def!("Math.log2",  fn1(Ty::Float, Ty::Float));
    def!("Math.log10", fn1(Ty::Float, Ty::Float));
    def!("Math.exp",   fn1(Ty::Float, Ty::Float));
    def!("Math.hypot", fn2(Ty::Float, Ty::Float, Ty::Float));

    def!("Math.clamp",    Ty::Fn { params: vec![Ty::Float, Ty::Float, Ty::Float], ret: Box::new(Ty::Float) });
    def!("Math.clampInt", Ty::Fn { params: vec![Ty::Int, Ty::Int, Ty::Int],       ret: Box::new(Ty::Int) });

    def!("Math.pow",     fn2(Ty::Float, Ty::Float, Ty::Float));
    def!("Math.sign",    fn1(Ty::Float, Ty::Float));
    def!("Math.signInt", fn1(Ty::Int,   Ty::Int));
    def!("Math.trunc",   fn1(Ty::Float, Ty::Float));
    def!("Math.random",  Ty::Fn { params: vec![], ret: Box::new(Ty::Float) });

    // ---------------------------------------------------------------- //
    // Crypto
    // ---------------------------------------------------------------- //

    def!("Crypto.sha256",       fn1(Ty::Text, Ty::Text));
    def!("Crypto.md5",          fn1(Ty::Text, Ty::Text));
    def!("Crypto.base64Encode", fn1(Ty::Text, Ty::Text));
    def!("Crypto.base64Decode", fn1(Ty::Text, Ty::Text));

    // ---------------------------------------------------------------- //
    // Regex
    // ---------------------------------------------------------------- //

    def!("Regex.match",    fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Regex.find",     fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Regex.captures", fn2(Ty::Text, Ty::Text, Ty::List(Box::new(Ty::Text))));
    def!("Regex.replace",  Ty::Fn { params: vec![Ty::Text, Ty::Text, Ty::Text], ret: Box::new(Ty::Text) });
    def!("Regex.split",    fn2(Ty::Text, Ty::Text, Ty::List(Box::new(Ty::Text))));

    // ---------------------------------------------------------------- //
    // Csv
    // ---------------------------------------------------------------- //

    {
        let list_text      = || Ty::List(Box::new(Ty::Text));
        let list_list_text = || Ty::List(Box::new(list_text()));
        def!("Csv.parse",     fn1(Ty::Text,        list_list_text()));
        def!("Csv.serialize", fn1(list_list_text(), Ty::Text));
        def!("Csv.header",    fn1(list_list_text(), list_text()));
        def!("Csv.rows",      fn1(list_list_text(), list_list_text()));
    }

    // ---------------------------------------------------------------- //
    // Named-arg metadata (full-qualified keys to match call-site lookup)
    // ---------------------------------------------------------------- //

    macro_rules! pm {
        ($name:expr, $($p:expr),+) => {
            env.define_param_meta($name, vec![$( ($p.to_string(), false) ),+]);
        };
    }

    // Core
    pm!("assert",          "cond", "msg");
    pm!("pow",             "base", "exp");
    pm!("minInt",          "a", "b");
    pm!("maxInt",          "a", "b");
    pm!("minFloat",        "a", "b");
    pm!("maxFloat",        "a", "b");
    pm!("range",           "from", "to");
    pm!("rangeInclusive",  "from", "to");

    // List
    pm!("List.get",        "list", "index");
    pm!("List.getOrPanic", "list", "index");
    pm!("List.push",       "list", "item");
    pm!("List.concat",     "a", "b");
    pm!("List.slice",      "list", "from", "to");
    pm!("List.contains",   "list", "item");
    pm!("List.map",        "list", "f");
    pm!("List.filter",     "list", "pred");
    pm!("List.fold",       "list", "init", "f");
    pm!("List.find",       "list", "pred");
    pm!("List.any",        "list", "pred");
    pm!("List.all",        "list", "pred");
    pm!("List.sort",       "list", "cmp");
    pm!("List.zip",        "a", "b");

    // Map
    pm!("Map.insert",      "map", "key", "value");
    pm!("Map.get",         "map", "key");
    pm!("Map.contains",    "map", "key");
    pm!("Map.remove",      "map", "key");

    // Text
    pm!("Text.concat",     "a", "b");
    pm!("Text.contains",   "text", "sub");
    pm!("Text.startsWith", "text", "prefix");
    pm!("Text.endsWith",   "text", "suffix");
    pm!("Text.slice",      "text", "from", "to");
    pm!("Text.indexOf",    "text", "sub");
    pm!("Text.replace",    "text", "from", "to");
    pm!("Text.split",      "text", "sep");
    pm!("Text.join",       "parts", "sep");
    pm!("Text.repeat",     "text", "n");

    // DateTime
    pm!("DateTime.format",      "dt", "fmt");
    pm!("DateTime.addSeconds",  "dt", "secs");
    pm!("DateTime.addMinutes",  "dt", "mins");
    pm!("DateTime.addHours",    "dt", "hours");
    pm!("DateTime.addDays",     "dt", "days");
    pm!("DateTime.diffSeconds", "a", "b");
    pm!("DateTime.diffDays",    "a", "b");
    pm!("DateTime.before",      "a", "b");
    pm!("DateTime.after",       "a", "b");
    pm!("Date.format",          "date", "fmt");

    // Decimal
    pm!("Decimal.add",    "a", "b");
    pm!("Decimal.sub",    "a", "b");
    pm!("Decimal.mul",    "a", "b");
    pm!("Decimal.div",    "a", "b");
    pm!("Decimal.round",  "d", "places");

    // File / Path
    pm!("writeFile",   "path", "content");
    pm!("appendFile",  "path", "content");
    pm!("Path.join",   "base", "part");

    // Process
    pm!("Process.exec", "cmd", "args");

    // Json
    pm!("JsonValue.at",   "value", "index");
    pm!("JsonValue.get",  "value", "key");
    pm!("JsonValue.push", "array", "item");
    pm!("JsonValue.set",  "obj", "key", "value");

    // Http
    pm!("Http.post",    "url", "content_type", "body");
    pm!("Http.put",     "url", "content_type", "body");
    pm!("Http.respond", "status", "content_type", "body");
    pm!("Http.ok",      "content_type", "body");
    pm!("Http.serve",   "port", "handler");

    // Math
    pm!("Math.atan2",    "y", "x");
    pm!("Math.clamp",    "value", "min", "max");
    pm!("Math.clampInt", "value", "min", "max");
    pm!("Math.pow",      "base", "exp");
    pm!("Math.hypot",    "x", "y");

    // Regex
    pm!("Regex.match",    "pattern", "text");
    pm!("Regex.find",     "pattern", "text");
    pm!("Regex.captures", "pattern", "text");
    pm!("Regex.replace",  "pattern", "text", "replacement");
    pm!("Regex.split",    "pattern", "text");

    // Db
    pm!("dbConnect",    "url");
    pm!("dbExec",       "conn", "sql", "params");
    pm!("dbQuery",      "conn", "sql", "params");
    pm!("dbQueryRow",   "conn", "sql", "params");
}

fn fn1(a: Ty, ret: Ty) -> Ty {
    Ty::Fn { params: vec![a], ret: Box::new(ret) }
}

fn fn2(a: Ty, b: Ty, ret: Ty) -> Ty {
    Ty::Fn { params: vec![a, b], ret: Box::new(ret) }
}

fn poly1(var: u32, body: Ty) -> Ty {
    Ty::Forall { vars: vec![var], body: Box::new(body) }
}
