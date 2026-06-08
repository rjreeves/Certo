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
    def!("range",   fn2(Ty::Int, Ty::Int, Ty::List(Box::new(Ty::Int))));
    def!("rangeInclusive", fn2(Ty::Int, Ty::Int, Ty::List(Box::new(Ty::Int))));

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
}

// ---------------------------------------------------------------- //
// Helpers for building Ty values
// ---------------------------------------------------------------- //

fn fn1(a: Ty, ret: Ty) -> Ty {
    Ty::Fn { params: vec![a], ret: Box::new(ret) }
}

fn fn2(a: Ty, b: Ty, ret: Ty) -> Ty {
    Ty::Fn { params: vec![a, b], ret: Box::new(ret) }
}

fn poly1(var: u32, body: Ty) -> Ty {
    Ty::Forall { vars: vec![var], body: Box::new(body) }
}
