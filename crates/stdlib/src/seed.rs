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
    def!("flush",    Ty::Fn { params: vec![], ret: Box::new(Ty::Unit) });
    def!("eprint",   fn1(Ty::Text, Ty::Unit));
    def!("eprintln", fn1(Ty::Text, Ty::Unit));

    def!("intToText",   fn1(Ty::Int,   Ty::Text));
    def!("floatToText", fn1(Ty::Float, Ty::Text));
    def!("boolToText",  fn1(Ty::Bool,  Ty::Text));
    def!("floatToInt",  fn1(Ty::Float, Ty::Int));
    def!("intToFloat",  fn1(Ty::Int,   Ty::Float));
    def!("textToIntUnsafe", fn1(Ty::Text, Ty::Int));

    // Float32 — a distinct type from Float (does not unify), so it needs its
    // own full conversion set rather than reusing Float's.
    def!("float32ToText",  fn1(Ty::Float32, Ty::Text));
    def!("float32ToInt",   fn1(Ty::Float32, Ty::Int));
    def!("intToFloat32",   fn1(Ty::Int,     Ty::Float32));
    def!("float32ToFloat", fn1(Ty::Float32, Ty::Float));
    def!("floatToFloat32", fn1(Ty::Float,   Ty::Float32));

    // Int8/Int16/Int32/UInt — BACKLOG item 235. Each is a distinct C type
    // from Int (see crates/typeck/src/unify.rs), so — mirroring Float32's
    // own conversion set immediately above — each needs its own
    // construction/conversion path rather than reusing Int's.
    def!("int8ToText",  fn1(Ty::Int8,  Ty::Text));
    def!("int8ToInt",   fn1(Ty::Int8,  Ty::Int));
    def!("intToInt8",   fn1(Ty::Int,   Ty::Int8));
    def!("int16ToText", fn1(Ty::Int16, Ty::Text));
    def!("int16ToInt",  fn1(Ty::Int16, Ty::Int));
    def!("intToInt16",  fn1(Ty::Int,   Ty::Int16));
    def!("int32ToText", fn1(Ty::Int32, Ty::Text));
    def!("int32ToInt",  fn1(Ty::Int32, Ty::Int));
    def!("intToInt32",  fn1(Ty::Int,   Ty::Int32));
    def!("uintToText",  fn1(Ty::UInt,  Ty::Text));
    def!("uintToInt",   fn1(Ty::UInt,  Ty::Int));
    // `intToUint`, not `intToUInt` — `c_fn_name`'s `camel_to_snake`
    // (`crates/codegen/src/emit_mir.rs`) inserts an underscore before
    // *every* uppercase letter, so a mid-identifier `UInt` would mangle to
    // `_u_int` instead of `_uint`; a single leading capital round-trips
    // correctly, matching how `uintToText`/`uintToInt` above are spelled.
    def!("intToUint",   fn1(Ty::Int,   Ty::UInt));

    def!("parseInt",     fn1(Ty::Text, Ty::Option(Box::new(Ty::Int))));
    def!("parseFloat",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Float))));
    def!("parseDecimal", fn1(Ty::Text, Ty::Option(Box::new(Ty::Decimal(None)))));
    // BACKLOG item 194 — `parseInt`/`parseFloat` already existed; `parseBool`
    // was the one genuine gap (`certo generate api`'s own row-to-JSON codegen
    // needed all three to encode typed values instead of always Json.string).
    def!("parseBool",    fn1(Ty::Text, Ty::Option(Box::new(Ty::Bool))));
    // BACKLOG item 228 — no callable function converted a runtime Text value
    // into a UUID at all (only the compile-time `uuid"..."` literal worked);
    // found reading a UUID column back out of a `dbQueryTyped` row mapper,
    // where the raw value necessarily arrives as Text. Fallible, matching
    // `parseInt`/`parseFloat`/`parseDecimal`/`parseBool`'s own convention.
    def!("parseUuid",    fn1(Ty::Text, Ty::Option(Box::new(Ty::Uuid))));

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
    // Both type variables must be quantified — otherwise the unquantified one is
    // shared across every use site and the Err/Ok side leaks between call sites.
    {
        let a = fresh(); let b = fresh();
        env.define("Ok",  poly2(a, b, fn1(Ty::Var(a), Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))))));
    }
    {
        let a = fresh(); let b = fresh();
        env.define("Err", poly2(a, b, fn1(Ty::Var(b), Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))))));
    }

    // Option/Result tag predicates (BACKLOG item 165) — real, independently
    // useful stdlib functions, not just plumbing for `expect(...).toBeSome()`
    // etc (which do call these). Thin wrappers over representations already
    // established elsewhere (`None` is a raw NULL pointer; `__result_is_ok`
    // is the existing runtime-header primitive every Result combinator in
    // `crates/stdlib/src/result.rs` already uses) — see `crates/stdlib/src/
    // core.rs`/`result.rs` for the one-line C implementations.
    {
        let a = fresh();
        env.define("Option.isSome", poly1(a, fn1(Ty::Option(Box::new(Ty::Var(a))), Ty::Bool)));
    }
    {
        let a = fresh();
        env.define("Option.isNone", poly1(a, fn1(Ty::Option(Box::new(Ty::Var(a))), Ty::Bool)));
    }
    // BACKLOG item 256 — `Option.map`, mirroring `List.map`'s own exact
    // shape just below (found missing entirely while verifying item 203's
    // own leading-dot shorthand against the spec's own `coupon.map(...)`
    // example — confirmed via a direct repro that `coupon.map(f)` on a
    // `Coupon?` failed with "no field `map`", since nothing registered it).
    {
        let a = fresh(); let b = fresh();
        let f_ty = fn1(Ty::Var(a), Ty::Var(b));
        env.define("Option.map", Ty::Forall {
            vars: vec![a, b],
            body: Box::new(fn2(Ty::Option(Box::new(Ty::Var(a))), f_ty,
                              Ty::Option(Box::new(Ty::Var(b))))),
        });
    }
    {
        let a = fresh(); let b = fresh();
        env.define("Result.isOk", poly2(a, b, fn1(Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))), Ty::Bool)));
    }
    {
        let a = fresh(); let b = fresh();
        env.define("Result.isErr", poly2(a, b, fn1(Ty::Result(Box::new(Ty::Var(a)), Box::new(Ty::Var(b))), Ty::Bool)));
    }

    // ---------------------------------------------------------------- //
    // Core function combinators (spec §9.1, BACKLOG item 161)
    // ---------------------------------------------------------------- //
    // identity is an ordinary erased passthrough, no different in kind from
    // any other stdlib ∀T function (implemented in `core.rs`'s `certo_identity`).
    // const/compose/flip all *construct and return a new closure value* —
    // unlike every other higher-order stdlib function (List.map, flatMap,
    // etc), which only ever *consumes* a closure it's given — so a
    // hand-written, type-erased C implementation isn't safe here: the
    // returned closure's own type is fully resolved to concrete types by
    // ordinary unification at each call site (e.g. `compose(intToText,
    // double)` has real type `Int => Text`), so the call site expects a
    // native calling convention, not a uniform erased one. These three are
    // intercepted and lowered directly in `crates/mir/src/lower.rs`
    // (`lower_core_combinator_call`), which synthesizes a real,
    // concretely-typed trampoline function per call site — the same
    // per-call-site-synthesis strategy `wrap_named_fn_as_closure` (item
    // 140) and HKT's own erased-closure wrapping (item 142) already use —
    // rather than being registered as ordinary hand-written C runtime
    // functions here.
    {
        let t = fresh();
        env.define("identity", poly1(t, fn1(Ty::Var(t), Ty::Var(t))));
    }
    // `expect<T>(x: T): T` — real, honest identity (BACKLOG item 165): the
    // spec's `expect(x).toBe(y)`/`.toBeSome()`/etc assertion matchers
    // (`Expr::ExpectAssertion`, `crates/parser/src/parse_expr.rs`) don't
    // require their receiver to literally be a call to `expect` — any
    // expression's `.toBe(...)` works — so `expect` exists purely for
    // spec-matching readability at the call site, not as a syntactic gate.
    // Reuses `identity`'s own C implementation exactly (see the `#define`
    // bridge in `crates/stdlib/src/core.rs`), same erased-passthrough
    // shape, safe for the same reason `identity` is (no closure
    // construction, unlike `const`/`compose`/`flip`).
    {
        let t = fresh();
        env.define("expect", poly1(t, fn1(Ty::Var(t), Ty::Var(t))));
    }
    {
        let a = fresh(); let b = fresh();
        env.define("const", poly2(a, b, fn1(Ty::Var(a), fn1(Ty::Var(b), Ty::Var(a)))));
    }
    {
        let a = fresh(); let b = fresh(); let c = fresh();
        env.define("compose", poly3(a, b, c,
            fn2(fn1(Ty::Var(b), Ty::Var(c)), fn1(Ty::Var(a), Ty::Var(b)), fn1(Ty::Var(a), Ty::Var(c)))));
    }
    {
        let a = fresh(); let b = fresh(); let c = fresh();
        env.define("flip", poly3(a, b, c,
            fn1(fn1(Ty::Var(a), fn1(Ty::Var(b), Ty::Var(c))), fn1(Ty::Var(b), fn1(Ty::Var(a), Ty::Var(c))))));
    }

    // ---------------------------------------------------------------- //
    // Result<T, E> combinators
    // ---------------------------------------------------------------- //

    // flatMap :: ∀T U E. Result<T,E> → (T → Result<U,E>) → Result<U,E>
    //
    // BACKLOG item 199 — also registered as `Result.flatMap` (and its three
    // siblings below), since UFCS dot-call resolution (`Ty::qualifying_name`,
    // `crates/typeck/src/ty.rs`) only ever looks up the qualified
    // `"Result.<method>"` name, never the bare one. Before this, the exact
    // dot-call syntax spec §5.4's own pattern table shows
    // (`result.flatMap(...)`) failed with `E0205: no field 'flatMap'`, even
    // though the bare-call form `flatMap(result, ...)` already worked —
    // confirmed directly against `Result.isOk`, which *is* namespaced and
    // *does* dot-call correctly. The bare names stay registered too (real,
    // working, pre-existing call sites may already use them) — this only
    // adds the missing alias, it doesn't replace anything.
    {
        let t = fresh(); let u = fresh(); let e = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        let result_ue = Ty::Result(Box::new(Ty::Var(u)), Box::new(Ty::Var(e)));
        let f_ty = fn1(Ty::Var(t), result_ue.clone());
        env.define("flatMap", poly3(t, u, e, fn2(result_te.clone(), f_ty.clone(), result_ue.clone())));
        env.define("Result.flatMap", poly3(t, u, e, fn2(result_te, f_ty, result_ue)));
    }
    // mapErr :: ∀T E F. Result<T,E> → (E → F) → Result<T,F>
    {
        let t = fresh(); let e = fresh(); let f = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        let result_tf = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(f)));
        let f_ty = fn1(Ty::Var(e), Ty::Var(f));
        env.define("mapErr", poly3(t, e, f, fn2(result_te.clone(), f_ty.clone(), result_tf.clone())));
        env.define("Result.mapErr", poly3(t, e, f, fn2(result_te, f_ty, result_tf)));
    }
    // getOrElse :: ∀T E. Result<T,E> → T → T
    {
        let t = fresh(); let e = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        env.define("getOrElse", poly2(t, e, fn2(result_te.clone(), Ty::Var(t), Ty::Var(t))));
        env.define("Result.getOrElse", poly2(t, e, fn2(result_te, Ty::Var(t), Ty::Var(t))));
    }
    // recover :: ∀T E. Result<T,E> → (E → T) → T
    {
        let t = fresh(); let e = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        let f_ty = fn1(Ty::Var(e), Ty::Var(t));
        env.define("recover", poly2(t, e, fn2(result_te.clone(), f_ty.clone(), Ty::Var(t))));
        env.define("Result.recover", poly2(t, e, fn2(result_te, f_ty, Ty::Var(t))));
    }
    // Result.all :: ∀T E. List<Result<T,E>> → Result<List<T>, E>
    {
        let t = fresh(); let e = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        let ret = Ty::Result(Box::new(Ty::List(Box::new(Ty::Var(t)))), Box::new(Ty::Var(e)));
        env.define("Result.all", poly2(t, e, fn1(Ty::List(Box::new(result_te)), ret)));
    }
    // Result.allSettled :: ∀T E. List<Result<T,E>> → List<Result<T,E>>
    {
        let t = fresh(); let e = fresh();
        let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
        env.define("Result.allSettled",
            poly2(t, e, fn1(Ty::List(Box::new(result_te.clone())), Ty::List(Box::new(result_te)))));
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
    def!("monotonicMillis", Ty::Fn { params: vec![], ret: Box::new(Ty::Int) });
    def!("sleep", fn1(Ty::Int, Ty::Unit));

    // ---------------------------------------------------------------- //
    // Collections — List<T>
    // ---------------------------------------------------------------- //

    {
        let a = fresh();
        // Declared as a zero-arg fn (`fn List.empty<T>(): List<T>`), so it must be
        // registered as `() => List<T>`, not a bare `List<T>` value — every call site
        // is `List.empty()`, an App with zero args, which always unifies the callee's
        // type against `Fn{params: [], ret}`.
        env.define("List.empty", poly1(a, Ty::Fn { params: vec![], ret: Box::new(Ty::List(Box::new(Ty::Var(a)))) }));
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
    // `List.forEach` (BACKLOG item 266, spec §3.2's own `List.forEach(xs)
    // { x => println(x) }` example) — same shape as `List.map`, but the
    // callback returns `Unit` and the call itself is for side effects only.
    {
        let a = fresh();
        let f_ty = fn1(Ty::Var(a), Ty::Unit);
        env.define("List.forEach", poly1(a,
            fn2(Ty::List(Box::new(Ty::Var(a))), f_ty, Ty::Unit)));
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
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.distinct", poly1(a, fn1(list_a.clone(), list_a)));
    }
    {
        let a = fresh();
        let pred = fn1(Ty::Var(a), Ty::Bool);
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.partition", poly1(a,
            fn2(list_a.clone(), pred, Ty::Tuple(vec![list_a.clone(), list_a]))));
    }
    {
        let a = fresh();
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.chunked", poly1(a,
            fn2(list_a.clone(), Ty::Int, Ty::List(Box::new(list_a)))));
    }
    {
        let a = fresh(); let k = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(k));
        env.define("List.groupBy", Ty::Forall {
            vars: vec![a, k],
            body: Box::new(fn2(
                Ty::List(Box::new(Ty::Var(a))),
                key,
                Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::List(Box::new(Ty::Var(a))))),
            )),
        });
    }
    // `List.upsert` (BACKLOG item 209, spec §8.5) — replaces the element
    // whose key matches `item`'s own key, or appends it if none matches.
    // The spec's own literal example (`self.items.upsert(CartItem(item,
    // qty), on: .productId)`) uses the leading-dot property shorthand
    // (item 203, not built), so `on` here takes a real key-projection
    // *function* instead — the same, already-established idiom
    // `groupBy`/`sortBy`/`sumBy`/`minBy`/`maxBy` all use, not a field-name
    // string (this runtime has no reflection to look a field up by name on
    // an arbitrary type). Callers write `on: (x) => x.productId` — the
    // shorthand, once it exists, would just be sugar for exactly this.
    {
        let a = fresh(); let k = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(k));
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.upsert", Ty::Forall {
            vars: vec![a, k],
            body: Box::new(Ty::Fn {
                params: vec![list_a.clone(), Ty::Var(a), key],
                ret: Box::new(list_a),
            }),
        });
    }
    // `List.flatMap`/`List.reduce` (BACKLOG item 162, bounded half — the
    // other four spec-listed functions, `sortBy`/`sumBy`/`minBy`/`maxBy`,
    // all need real per-call-site MIR synthesis to compare/add a generic
    // key type and were split out as their own item rather than rushed in
    // here). `flatMap` is genuinely new (map then flatten one level, no
    // existing equivalent under any name); `reduce` is spec's own name for
    // exactly `List.fold`'s already-shipped signature — same order (list,
    // init, combiner) — so it reuses `fold`'s real C implementation via a
    // `#define` bridge rather than duplicating it (see `collections.rs`).
    {
        let a = fresh(); let b = fresh();
        let f_ty = fn1(Ty::Var(a), Ty::List(Box::new(Ty::Var(b))));
        env.define("List.flatMap", Ty::Forall {
            vars: vec![a, b],
            body: Box::new(fn2(Ty::List(Box::new(Ty::Var(a))), f_ty,
                              Ty::List(Box::new(Ty::Var(b))))),
        });
    }
    {
        let t = fresh(); let acc = fresh();
        env.define("List.reduce", Ty::Forall {
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
    // `List.sortBy`/`List.minBy`/`List.maxBy`/`List.sumBy` (BACKLOG item
    // 162b, split out of item 162's own aggregate-functions half). The
    // type signatures below accept any key/numeric type `K`/`N` — same as
    // the spec's own `K: Ord`/`N: Numeric` bounds — but `crates/typeck/src
    // /infer_expr.rs`'s post-unification check restricts what's actually
    // *accepted* to the types this codebase's `<`/`>`/`+` operators are
    // genuinely correct for (`Int`/`Int8`/`Int16`/`Int32`/`UInt`/`Float`/
    // `Float32` — real C numeric operators); `Text`/`Decimal`/records are
    // rejected with a real error (E0710) rather than silently miscompiling
    // (`Text`'s `<` is pointer comparison, not lexicographic) or hard-
    // failing at the C compiler (`Decimal`'s `+`/`<` on a struct). The
    // comparator/adder itself is synthesized per call site in
    // `crates/mir/src/lower.rs`, using the caller's own resolved concrete
    // key/numeric type — same technique `compose`/`const`/`flip` (items
    // 140-142) already established for this codebase's other functions
    // whose C implementation can't be generic/type-erased.
    {
        let a = fresh(); let k = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(k));
        let list_a = Ty::List(Box::new(Ty::Var(a)));
        env.define("List.sortBy", Ty::Forall {
            vars: vec![a, k],
            body: Box::new(fn2(list_a.clone(), key, list_a)),
        });
    }
    {
        let a = fresh(); let k = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(k));
        env.define("List.minBy", Ty::Forall {
            vars: vec![a, k],
            body: Box::new(fn2(
                Ty::List(Box::new(Ty::Var(a))), key,
                Ty::Option(Box::new(Ty::Var(a))),
            )),
        });
    }
    {
        let a = fresh(); let k = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(k));
        env.define("List.maxBy", Ty::Forall {
            vars: vec![a, k],
            body: Box::new(fn2(
                Ty::List(Box::new(Ty::Var(a))), key,
                Ty::Option(Box::new(Ty::Var(a))),
            )),
        });
    }
    {
        let a = fresh(); let n = fresh();
        let key = fn1(Ty::Var(a), Ty::Var(n));
        env.define("List.sumBy", Ty::Forall {
            vars: vec![a, n],
            body: Box::new(fn2(
                Ty::List(Box::new(Ty::Var(a))), key,
                Ty::Var(n),
            )),
        });
    }

    // ---------------------------------------------------------------- //
    // Collections — Map<K, V>
    // ---------------------------------------------------------------- //

    {
        let k = fresh(); let v = fresh();
        // Same fix as List.empty above — must be `() => Map<K,V>`, not a bare value.
        env.define("Map.empty", Ty::Forall {
            vars: vec![k, v],
            body: Box::new(Ty::Fn {
                params: vec![],
                ret: Box::new(Ty::Map(Box::new(Ty::Var(k)), Box::new(Ty::Var(v)))),
            }),
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
    // Concurrency — Channel<T>
    // ---------------------------------------------------------------- //

    {
        let t = fresh();
        env.define("Channel.new", Ty::Forall {
            vars: vec![t],
            body: Box::new(Ty::Fn {
                params: vec![Ty::Int],
                ret: Box::new(Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] }),
            }),
        });
    }
    {
        let t = fresh();
        env.define("Channel.send", Ty::Forall {
            vars: vec![t],
            body: Box::new(fn2(
                Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] },
                Ty::Var(t),
                Ty::Unit,
            )),
        });
    }
    {
        let t = fresh();
        env.define("Channel.receive", Ty::Forall {
            vars: vec![t],
            body: Box::new(fn1(
                Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] },
                Ty::Option(Box::new(Ty::Var(t))),
            )),
        });
    }
    {
        let t = fresh();
        env.define("Channel.tryReceive", Ty::Forall {
            vars: vec![t],
            body: Box::new(fn1(
                Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] },
                Ty::Option(Box::new(Ty::Var(t))),
            )),
        });
    }
    {
        let t = fresh();
        env.define("Channel.close", Ty::Forall {
            vars: vec![t],
            body: Box::new(fn1(
                Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] },
                Ty::Unit,
            )),
        });
    }
    {
        let t = fresh();
        env.define("Channel.isClosed", Ty::Forall {
            vars: vec![t],
            body: Box::new(fn1(
                Ty::Named { name: "Channel".into(), args: vec![Ty::Var(t)] },
                Ty::Bool,
            )),
        });
    }

    // ---------------------------------------------------------------- //
    // Text
    // ---------------------------------------------------------------- //

    def!("Text.len",        fn1(Ty::Text, Ty::Int));
    def!("Text.byteLength", fn1(Ty::Text, Ty::Int));
    // BACKLOG item 269 — spec §9.3's own `text.length()` example, and its
    // explicit "(not byte count)" wording, distinguish a real *character*
    // count from `Text.len`/`Text.byteLength`'s own byte count (`Text.len`
    // returned the identical byte value as `Text.byteLength` for any
    // multi-byte string, contradicting that wording). Counts real Unicode
    // codepoints via the same UTF-8 decode loop item 258 built for
    // `Text.charAt` — see `certo_text_length` in `crates/stdlib/src/text.rs`.
    def!("Text.length",     fn1(Ty::Text, Ty::Int));
    def!("Text.concat",     fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Text.eq",         fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.contains",   fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.startsWith", fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.endsWith",   fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("Text.toUpper",    fn1(Ty::Text, Ty::Text));
    def!("Text.toLower",    fn1(Ty::Text, Ty::Text));
    // BACKLOG item 269 — spec §9.3's own `text.toUppercase()` example uses
    // this name, not `Text.toUpper`. A plain alias for the identical
    // runtime function (`certo_text_to_upper` — see `crates/stdlib/src/
    // text.rs`), not a second implementation.
    def!("Text.toUppercase", fn1(Ty::Text, Ty::Text));
    // Locale-aware case conversion — BACKLOG item 117. Real, ICU-backed
    // implementation on Windows; a clear runtime error on POSIX (no
    // OS-bundled Unicode library there, and a dlopen-based path couldn't
    // be verified end-to-end in this dev environment — see
    // crates/stdlib/src/text.rs's certo_text_to_upper_locale).
    def!("Text.toUpperLocale", fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Text.toLowerLocale", fn2(Ty::Text, Ty::Text, Ty::Text));
    def!("Text.trim",       fn1(Ty::Text, Ty::Text));
    def!("Text.trimStart",  fn1(Ty::Text, Ty::Text));
    def!("Text.trimEnd",    fn1(Ty::Text, Ty::Text));
    def!("Text.slice",      Ty::Fn { params: vec![Ty::Text, Ty::Int, Ty::Int], ret: Box::new(Ty::Text) });
    def!("Text.indexOf",    fn2(Ty::Text, Ty::Text, Ty::Option(Box::new(Ty::Int))));
    def!("Text.replace",    Ty::Fn { params: vec![Ty::Text, Ty::Text, Ty::Text], ret: Box::new(Ty::Text) });
    def!("Text.split",      fn2(Ty::Text, Ty::Text, Ty::List(Box::new(Ty::Text))));
    def!("Text.join",       fn2(Ty::List(Box::new(Ty::Text)), Ty::Text, Ty::Text));
    def!("Text.repeat",     fn2(Ty::Text, Ty::Int, Ty::Text));
    def!("Text.charAt",     fn2(Ty::Text, Ty::Int, Ty::Option(Box::new(Ty::Char))));
    // BACKLOG item 269 — spec §9.4's own `text.toInt()` example documents a
    // `Result<Int, ParseError>` return, but the real, already-working
    // parsing function is the bare, un-namespaced `parseInt(s): Option<Int>`
    // (no `ParseError` type exists anywhere in the compiler). User confirmed
    // the smaller fix: a namespaced alias keeping `parseInt`'s own
    // Option-shaped convention (matching `parseFloat`/`parseDecimal`/
    // `parseBool`'s own established shape too), not inventing a new error
    // type and Result-shaped API to match the spec literally.
    def!("Text.toInt",      fn1(Ty::Text, Ty::Option(Box::new(Ty::Int))));

    // ---------------------------------------------------------------- //
    // Char
    // ---------------------------------------------------------------- //

    def!("Char.toText",      fn1(Ty::Char, Ty::Text));
    def!("Char.toInt",       fn1(Ty::Char, Ty::Int));
    def!("Char.fromInt",     fn1(Ty::Int,  Ty::Char));
    def!("Char.isDigit",     fn1(Ty::Char, Ty::Bool));
    def!("Char.isAlpha",     fn1(Ty::Char, Ty::Bool));
    def!("Char.isUpperCase", fn1(Ty::Char, Ty::Bool));
    def!("Char.isLowerCase", fn1(Ty::Char, Ty::Bool));
    def!("Char.isWhitespace", fn1(Ty::Char, Ty::Bool));
    def!("Char.toUpperCase", fn1(Ty::Char, Ty::Char));
    def!("Char.toLowerCase", fn1(Ty::Char, Ty::Char));

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
    // Duration
    // ---------------------------------------------------------------- //

    let dur = || Ty::Named { name: "Duration".into(), args: vec![] };

    def!("Duration.milliseconds", fn1(Ty::Int, dur()));
    def!("Duration.seconds",   fn1(Ty::Int, dur()));
    def!("Duration.minutes",   fn1(Ty::Int, dur()));
    def!("Duration.hours",     fn1(Ty::Int, dur()));
    def!("Duration.days",      fn1(Ty::Int, dur()));
    // BACKLOG item 271 — `Duration.months` (§16.9's own worked example,
    // `Duration.months(12)`), mirroring `Duration.days`'s exact shape.
    def!("Duration.months",    fn1(Ty::Int, dur()));
    def!("Duration.toSeconds", fn1(dur(), Ty::Int));
    def!("Duration.toMinutes", fn1(dur(), Ty::Int));
    def!("Duration.toHours",   fn1(dur(), Ty::Int));
    def!("Duration.toDays",    fn1(dur(), Ty::Int));
    def!("Duration.add",       fn2(dur(), dur(), dur()));
    def!("Duration.sub",       fn2(dur(), dur(), dur()));
    def!("Duration.negate",    fn1(dur(), dur()));
    def!("Duration.eq",        fn2(dur(), dur(), Ty::Bool));
    def!("Duration.lt",        fn2(dur(), dur(), Ty::Bool));
    def!("Duration.gt",        fn2(dur(), dur(), Ty::Bool));

    def!("DateTime.addDuration", fn2(dt(), dur(), dt()));
    def!("DateTime.diff",        fn2(dt(), dt(), dur()));
    def!("Date.addDuration",     fn2(date(), dur(), date()));

    // ---------------------------------------------------------------- //
    // Timezone — real IANA zones (BACKLOG item 118). `Timezone(name)` is a
    // plain, unqualified global function (like `sleep`/`getEnv`), not a sum-
    // type variant constructor — resolved as an ordinary call, same as any
    // other bare-name stdlib function.
    // ---------------------------------------------------------------- //

    let tz = || Ty::Named { name: "Timezone".into(), args: vec![] };

    def!("Timezone",             fn1(Ty::Text, Ty::Option(Box::new(tz()))));
    def!("Timezone.name",        fn1(tz(), Ty::Text));
    def!("DateTime.inTimezone",  fn2(dt(), tz(), Ty::Text));
    def!("DateTime.formatTz",    Ty::Fn { params: vec![dt(), Ty::Text, tz()], ret: Box::new(Ty::Text) });
    def!("Date.todayIn",         fn1(tz(), date()));

    // ---------------------------------------------------------------- //
    // Timestamp (BACKLOG item 164b) — the spec's own name for what this
    // codebase already ships as `DateTime`; both are distinct nominal
    // types (confirmed: a `DateTime` value does not unify with a
    // `Timestamp`-typed parameter) but share the identical `CertoDateTime`
    // representation. `Timestamp.now`/`.parse`/`.inTimezone`/`.formatTz`
    // reuse `DateTime`'s already-working C implementation (see
    // `crates/stdlib/src/datetime.rs`'s `#define` bridge); `.of` and
    // `Date.of` are genuinely new — no component-based constructor existed
    // for either name before this.
    // ---------------------------------------------------------------- //

    let ts = || Ty::Named { name: "Timestamp".into(), args: vec![] };

    def!("Timestamp.now",        Ty::Fn { params: vec![], ret: Box::new(ts()) });
    def!("Timestamp.of",         Ty::Fn {
        params: vec![Ty::Int, Ty::Int, Ty::Int, Ty::Int, Ty::Int, Ty::Int, tz()],
        ret: Box::new(ts()),
    });
    def!("Timestamp.parse",      fn1(Ty::Text, ts()));
    def!("Timestamp.inTimezone", fn2(ts(), tz(), Ty::Text));
    def!("Timestamp.formatTz",   Ty::Fn { params: vec![ts(), Ty::Text, tz()], ret: Box::new(Ty::Text) });
    // BACKLOG item 214 — `Timestamp` had no path to a `Duration` at all
    // (bare `timestamp - timestamp` is now rejected, see
    // `OpaqueTemporalArithmetic`/E0218); mirrors `DateTime.diff` exactly,
    // reusing its already-correct C implementation via the same
    // `certo_timestamp_*` → `certo_datetime_*` bridge every other
    // `Timestamp.*` function above already uses.
    def!("Timestamp.diff",       fn2(ts(), ts(), dur()));
    def!("Date.of",              Ty::Fn { params: vec![Ty::Int, Ty::Int, Ty::Int], ret: Box::new(date()) });

    // ---------------------------------------------------------------- //
    // Money / Decimal
    // ---------------------------------------------------------------- //

    def!("Decimal.add",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.sub",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.mul",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.div",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.eq",         fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Bool));
    def!("Decimal.lt",         fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Bool));
    def!("Decimal.gt",         fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Bool));
    def!("Decimal.lte",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Bool));
    def!("Decimal.gte",        fn2(Ty::Decimal(None), Ty::Decimal(None), Ty::Bool));
    def!("Decimal.abs",        fn1(Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.negate",     fn1(Ty::Decimal(None), Ty::Decimal(None)));
    def!("Decimal.round",      fn2(Ty::Decimal(None), Ty::Int, Ty::Decimal(None)));
    def!("Decimal.toInt",      fn1(Ty::Decimal(None), Ty::Int));
    def!("Decimal.fromInt",    fn1(Ty::Int, Ty::Decimal(None)));
    def!("Decimal.toText",     fn1(Ty::Decimal(None), Ty::Text));
    // BACKLOG item 228 — the inverse of `parseUuid` above: no function
    // serialized a UUID back to Text either, needed e.g. to pass a
    // UUID-typed field as a Text SQL parameter (`dbExec`'s params are
    // always `List<Text>`). Named to match `Decimal.toText`'s own
    // namespaced convention (unlike the bare `parseInt`/`parseUuid` side).
    def!("UUID.toText",        fn1(Ty::Uuid, Ty::Text));
    def!("Money.fromCents",    fn1(Ty::Int, Ty::Decimal(None)));
    def!("Money.toCents",      fn1(Ty::Decimal(None), Ty::Int));
    def!("Money.fromDecimal",  fn1(Ty::Decimal(None), Ty::Decimal(None)));

    // ---------------------------------------------------------------- //
    // Env
    // ---------------------------------------------------------------- //

    def!("getEnv",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
    def!("setEnv",   fn2(Ty::Text, Ty::Text, Ty::Unit));
    def!("unsetEnv", fn1(Ty::Text, Ty::Unit));
    def!("getCurrentDir", Ty::Fn { params: vec![], ret: Box::new(Ty::Text) });

    // ---------------------------------------------------------------- //
    // File
    // ---------------------------------------------------------------- //

    def!("readFile",   fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
    def!("writeFile",  fn2(Ty::Text, Ty::Text, Ty::Bool));

    // File — open-handle API (BACKLOG item 192), for `use file = File.open(path) { ... }`
    // (item 152). Opaque handle, same `void*` convention as `__CertoTask`/`Channel`.
    // `.open` reports failure as `Option` like every sibling file function above,
    // not a new `Result<File, IOError>` — no `IOError` type exists in this codebase.
    {
        let file = || Ty::Named { name: "File".into(), args: vec![] };
        def!("File.open",    fn1(Ty::Text, Ty::Option(Box::new(file()))));
        def!("File.readAll", fn1(file(), Ty::Option(Box::new(Ty::Text))));
        def!("File.write",   fn2(file(), Ty::Text, Ty::Bool));
        def!("File.close",   fn1(file(), Ty::Unit));
    }

    // Bytes — opaque binary buffer (pointer-sized handle).
    {
        let bytes = || Ty::Named { name: "Bytes".into(), args: vec![] };
        def!("Bytes.length",  fn1(bytes(), Ty::Int));
        def!("Bytes.empty",   Ty::Fn { params: vec![], ret: Box::new(bytes()) });
        def!("Bytes.slice",   Ty::Fn { params: vec![bytes(), Ty::Int, Ty::Int], ret: Box::new(bytes()) });
        def!("Bytes.concat",  Ty::Fn { params: vec![bytes(), bytes()], ret: Box::new(bytes()) });
        def!("Bytes.toHex",   fn1(bytes(), Ty::Text));
        def!("Bytes.fromText", fn1(Ty::Text, bytes()));
        def!("readFileBytes",  fn1(Ty::Text, Ty::Option(Box::new(bytes()))));
        def!("writeFileBytes", fn2(Ty::Text, bytes(), Ty::Bool));

        // Credential — Windows Credential Manager (Generic creds).
        def!("Credential.getBytes", fn1(Ty::Text, Ty::Option(Box::new(bytes()))));
        def!("Credential.get",      fn1(Ty::Text, Ty::Option(Box::new(Ty::Text))));
        def!("Credential.set",      fn2(Ty::Text, Ty::Text, Ty::Bool));
        def!("Credential.delete",   fn1(Ty::Text, Ty::Bool));
    }
    def!("appendFile", fn2(Ty::Text, Ty::Text, Ty::Bool));
    def!("fileExists", fn1(Ty::Text, Ty::Bool));
    def!("deleteFile", fn1(Ty::Text, Ty::Bool));
    def!("makeDir",    fn1(Ty::Text, Ty::Bool));
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
        let handler = Ty::Fn { params: vec![Ty::Text], ret: Box::new(Ty::Unit) };
        def!("Process.exec",           fn2(Ty::Text, list_text.clone(), pr.clone()));
        def!("Process.execInherit",    fn2(Ty::Text, list_text.clone(), Ty::Int));
        def!("Process.execWithInput",  Ty::Fn { params: vec![Ty::Text, list_text.clone(), Ty::Text], ret: Box::new(pr.clone()) });
        def!("Process.lines",          Ty::Fn { params: vec![Ty::Text, list_text.clone(), handler], ret: Box::new(Ty::Int) });
        def!("Process.spawnDetached",  Ty::Fn { params: vec![Ty::Text, list_text.clone(), Ty::Text], ret: Box::new(Ty::Int) });
        def!("Process.spawnDetachedHidden",  Ty::Fn { params: vec![Ty::Text, list_text.clone(), Ty::Text], ret: Box::new(Ty::Int) });
        def!("Process.quit",           fn1(Ty::Int, Ty::Unit));
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
        // Nullable cell type — Text? — used in query result rows
        let opt_text     = Ty::Option(Box::new(Ty::Text));
        let list_opt_text = Ty::List(Box::new(opt_text.clone()));
        // List<List<Text?>> — full result set
        let list_row_nullable = Ty::List(Box::new(list_opt_text.clone()));

        // Null sentinel
        def!("dbNull", Ty::Fn { params: vec![], ret: Box::new(Ty::Text) });

        // Ambient connection accessor (BACKLOG item 226) — the lowering
        // target `db.<table>.<method>(...)` sugar rewrites to as its
        // synthesized, auto-inserted leading `conn` argument (see
        // `try_db_accessor_call`/`try_db_accessor_rewrite` in
        // `crates/typeck/src/infer_expr.rs`/`crates/hir/src/lower.rs`).
        // Never written directly by user source, but needs a real seeded
        // type here since the rewritten AST re-enters ordinary `infer`/
        // `lower_expr` as if the user had called it by hand. Its real C
        // implementation (`crates/stdlib/src/db.rs`) lazily auto-connects
        // via `DATABASE_URL` once per OS thread.
        def!("__certo_db_conn", Ty::Fn { params: vec![], ret: Box::new(conn.clone()) });

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

        // Run a raw (multi-statement) SQL script via PQexec.
        def!("dbRunScript", fn2(conn.clone(), Ty::Text, Ty::Int));

        // Script run returning a DbResult (status + last result set).
        {
            let dbres = || Ty::Named { name: "DbResult".into(), args: vec![] };
            let cells = Ty::List(Box::new(Ty::List(Box::new(Ty::Option(Box::new(Ty::Text))))));
            def!("dbRunScriptResult", fn2(conn.clone(), Ty::Text, dbres()));
            def!("DbResult.ok",      fn1(dbres(), Ty::Bool));
            def!("DbResult.error",   fn1(dbres(), Ty::Text));
            def!("DbResult.columns", fn1(dbres(), Ty::List(Box::new(Ty::Text))));
            def!("DbResult.rows",    fn1(dbres(), cells));
        }

        // Query — cells are Text? so SQL NULL is None, not ""
        def!("dbQuery", Ty::Fn {
            params: vec![conn.clone(), Ty::Text, list_text.clone()],
            ret: Box::new(list_row_nullable.clone()),
        });

        // dbQueryTyped :: ∀T. (Int, Text, List<Text>, List<Text?> -> T) -> List<T>
        {
            let t = fresh();
            let mapper = fn1(list_opt_text.clone(), Ty::Var(t));
            env.define("dbQueryTyped", Ty::Forall {
                vars: vec![t],
                body: Box::new(Ty::Fn {
                    params: vec![conn.clone(), Ty::Text, list_text.clone(), mapper],
                    ret: Box::new(Ty::List(Box::new(Ty::Var(t)))),
                }),
            });
        }
        // dbQueryRow → List<Text?>?
        def!("dbQueryRow", Ty::Fn {
            params: vec![conn.clone(), Ty::Text, list_text.clone()],
            ret: Box::new(Ty::Option(Box::new(list_opt_text.clone()))),
        });
        // dbQueryOne → Text?  (None when no rows or cell is SQL NULL)
        def!("dbQueryOne",  fn2(conn.clone(), Ty::Text, opt_text.clone()));
        def!("dbColumns",   fn2(conn.clone(), Ty::Text, list_text.clone()));

        // dbStream :: (Int, Text, List<Text>, List<Text?> -> Unit) -> Int
        {
            let handler = Ty::Fn { params: vec![list_opt_text.clone()], ret: Box::new(Ty::Unit) };
            def!("dbStream", Ty::Fn {
                params: vec![conn.clone(), Ty::Text, list_text.clone(), handler],
                ret: Box::new(Ty::Int),
            });
        }

        // Transactions
        def!("dbBegin",    fn1(conn.clone(), Ty::Int));
        def!("dbCommit",   fn1(conn.clone(), Ty::Int));
        def!("dbRollback", fn1(conn.clone(), Ty::Int));

        // withTransaction :: ∀T E. (Int, () -> Result<T,E>) -> Result<T,E>
        {
            let t = fresh(); let e = fresh();
            let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
            let body_ty   = Ty::Fn { params: vec![], ret: Box::new(result_te.clone()) };
            env.define("withTransaction", Ty::Forall {
                vars: vec![t, e],
                body: Box::new(Ty::Fn {
                    params: vec![conn.clone(), body_ty],
                    ret:    Box::new(result_te),
                }),
            });
        }

        // withConnection :: ∀T E. (Text, Int -> Result<T,E>) -> Result<T,E>
        {
            let t = fresh(); let e = fresh();
            let result_te = Ty::Result(Box::new(Ty::Var(t)), Box::new(Ty::Var(e)));
            let body_ty   = fn1(conn.clone(), result_te.clone());
            env.define("withConnection", Ty::Forall {
                vars: vec![t, e],
                body: Box::new(fn2(Ty::Text, body_ty, result_te)),
            });
        }
    }

    // ---------------------------------------------------------------- //
    // DbQuery — fluent Query builder (see crates/stdlib/src/dbquery.rs)
    // ---------------------------------------------------------------- //

    {
        let conn  = Ty::Int;
        let query = || Ty::Named { name: "Query".into(), args: vec![] };
        let list_opt_text = Ty::List(Box::new(Ty::Option(Box::new(Ty::Text))));

        def!("Query.from",   fn1(Ty::Text, query()));
        def!("Query.fromAs", fn2(Ty::Text, Ty::Text, query()));

        def!("Query.filter", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });

        def!("Query.orderBy", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });

        def!("Query.limit",  fn2(query(), Ty::Int, query()));
        def!("Query.offset", fn2(query(), Ty::Int, query()));
        def!("Query.sql",    fn1(query(), Ty::Text));
        def!("Query.count",  fn2(query(), conn.clone(), Ty::Int));

        // Query.list :: ∀T. (Query, Int, List<Text?> -> T) -> List<T>
        {
            let t = fresh();
            let mapper = fn1(list_opt_text.clone(), Ty::Var(t));
            env.define("Query.list", Ty::Forall {
                vars: vec![t],
                body: Box::new(Ty::Fn {
                    params: vec![query(), conn.clone(), mapper],
                    ret:    Box::new(Ty::List(Box::new(Ty::Var(t)))),
                }),
            });
        }

        // Query.first :: ∀T. (Query, Int, List<Text?> -> T) -> T?
        {
            let t = fresh();
            let mapper = fn1(list_opt_text.clone(), Ty::Var(t));
            env.define("Query.first", Ty::Forall {
                vars: vec![t],
                body: Box::new(Ty::Fn {
                    params: vec![query(), conn.clone(), mapper],
                    ret:    Box::new(Ty::Option(Box::new(Ty::Var(t)))),
                }),
            });
        }

        // Joins
        def!("Query.join", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });
        def!("Query.leftJoin", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });
        // .joinAs/.leftJoinAs take an explicit alias — how to join the same table to
        // itself (a self-join), since the repeated occurrence needs a distinct alias.
        def!("Query.joinAs", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });
        def!("Query.leftJoinAs", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });

        // Grouping / aggregation
        def!("Query.groupBy", fn2(query(), Ty::Text, query()));
        def!("Query.aggregate", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });
        def!("Query.having", Ty::Fn {
            params: vec![query(), Ty::Text, Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(query()),
        });

        // Scalar aggregates — Text? so NULL (no matching rows) round-trips as None,
        // consistent with dbQueryOne. Caller parses with .toDecimal()/.toInt().
        let opt_text = Ty::Option(Box::new(Ty::Text));
        def!("Query.sum", Ty::Fn { params: vec![query(), Ty::Text, conn.clone()], ret: Box::new(opt_text.clone()) });
        def!("Query.avg", Ty::Fn { params: vec![query(), Ty::Text, conn.clone()], ret: Box::new(opt_text.clone()) });
        def!("Query.min", Ty::Fn { params: vec![query(), Ty::Text, conn.clone()], ret: Box::new(opt_text.clone()) });
        def!("Query.max", Ty::Fn { params: vec![query(), Ty::Text, conn.clone()], ret: Box::new(opt_text.clone()) });

        // Query.groupedList :: ∀T. (Query, Int, List<Text?> -> T) -> List<T>  — no DbRow bound.
        {
            let t = fresh();
            let mapper = fn1(list_opt_text.clone(), Ty::Var(t));
            env.define("Query.groupedList", Ty::Forall {
                vars: vec![t],
                body: Box::new(Ty::Fn {
                    params: vec![query(), conn.clone(), mapper],
                    ret:    Box::new(Ty::List(Box::new(Ty::Var(t)))),
                }),
            });
        }
    }

    // ---------------------------------------------------------------- //
    // DbMutation — fluent Mutation builder (see crates/stdlib/src/dbmutation.rs)
    // ---------------------------------------------------------------- //

    {
        let conn      = Ty::Int;
        let mutation  = || Ty::Named { name: "Mutation".into(), args: vec![] };
        let list_text = Ty::List(Box::new(Ty::Text));

        def!("Mutation.insertInto",  fn1(Ty::Text, mutation()));
        def!("Mutation.updateTable", fn1(Ty::Text, mutation()));
        def!("Mutation.deleteFrom",  fn1(Ty::Text, mutation()));
        def!("Mutation.insertMany",  fn2(Ty::Text, list_text.clone(), mutation()));

        def!("Mutation.set", Ty::Fn {
            params: vec![mutation(), Ty::Text, Ty::Text],
            ret:    Box::new(mutation()),
        });
        def!("Mutation.filter", Ty::Fn {
            params: vec![mutation(), Ty::Text, Ty::Text, Ty::Text],
            ret:    Box::new(mutation()),
        });
        def!("Mutation.onConflict", fn2(mutation(), Ty::Text, mutation()));
        def!("Mutation.addRow",     fn2(mutation(), list_text.clone(), mutation()));

        def!("Mutation.run", fn2(mutation(), conn.clone(), Ty::Int));
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
        def!("Http.request", Ty::Fn { params: vec![Ty::Text, Ty::Text, list_hdr.clone(), Ty::Text], ret: Box::new(hr()) });
        def!("Http.requestBytes", Ty::Fn { params: vec![Ty::Text, Ty::Text, list_hdr.clone(), Ty::Named { name: "Bytes".into(), args: vec![] }], ret: Box::new(hr()) });

        // HttpResponse accessors
        def!("HttpResponse.status",      fn1(hr(), Ty::Int));
        def!("HttpResponse.body",        fn1(hr(), Ty::Text));
        def!("HttpResponse.bodyLength",  fn1(hr(), Ty::Int));
        def!("HttpResponse.contentType", fn1(hr(), Ty::Text));
        def!("HttpResponse.ok",          fn1(hr(), Ty::Bool));

        // Server
        let handler_ty = Ty::Fn { params: vec![req()], ret: Box::new(hr()) };
        def!("Http.serve", Ty::Fn {
            params: vec![Ty::Int, handler_ty],
            ret:    Box::new(Ty::Unit),
        });

        // Live-query push channel (BACKLOG item 88, stage 2/3) — broadcasts
        // a refresh signal to every open `/__certo_live` SSE connection.
        // Called by generated write-handler code after a successful DB
        // write; a no-op when nothing is connected.
        def!("Http.liveNotify", Ty::Fn { params: vec![], ret: Box::new(Ty::Unit) });

        // Response constructors
        def!("Http.respond",     Ty::Fn { params: vec![Ty::Int, Ty::Text, Ty::Text], ret: Box::new(hr()) });
        def!("Http.ok",          fn2(Ty::Text, Ty::Text, hr()));
        def!("Http.notFound",    fn1(Ty::Text, hr()));
        def!("Http.badRequest",  fn1(Ty::Text, hr()));
        def!("Http.serverError", fn1(Ty::Text, hr()));
        // BACKLOG item 239 — a real server-side redirect (303 See Other),
        // needed so a `form`'s `onSuccess: navigate(View)` can actually
        // navigate the client somewhere.
        def!("Http.redirect",    fn1(Ty::Text, hr()));

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
    def!("Crypto.sha256Bytes",  fn1(Ty::Named { name: "Bytes".into(), args: vec![] },
                                    Ty::Named { name: "Bytes".into(), args: vec![] }));
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
    pm!("identity",        "x");
    pm!("const",           "a");
    pm!("compose",         "f", "g");
    pm!("flip",            "f");
    pm!("pow",             "base", "exp");
    pm!("minInt",          "a", "b");
    pm!("maxInt",          "a", "b");
    pm!("minFloat",        "a", "b");
    pm!("maxFloat",        "a", "b");
    pm!("range",           "from", "to");
    pm!("rangeInclusive",  "from", "to");

    // Result
    pm!("flatMap",             "r", "f");
    pm!("mapErr",              "r", "f");
    pm!("getOrElse",           "r", "default");
    pm!("recover",             "r", "f");
    // BACKLOG item 227 — the `Result.`-qualified dot-call spelling (`r.
    // flatMap(f)`/`Result.flatMap(r, f)`, item 199's own UFCS rewrite
    // target) had no `pm!` entry of its own, unlike the bare name above —
    // `crates/hir/src/lower.rs`'s `stdlib_names` lookup keys strictly on
    // `fn_full_path` with no bare-name fallback, so a dot-called `flatMap`/
    // `recover`/`mapErr` silently got NONE of the lambda-callback
    // param-hint treatment the bare form already had, even though
    // `generic_container_ret`'s *return*-type recovery already explicitly
    // handles both spellings side by side. Confirmed via a direct HIR
    // probe: `r.flatMap((x) => Ok(x))`'s own bare, un-hinted `x` stayed
    // `Ty::Error` — only ever "accidentally" recovered when something else
    // in the body (arithmetic) reconstructed a type structurally.
    pm!("Result.flatMap",      "r", "f");
    pm!("Result.mapErr",       "r", "f");
    pm!("Result.recover",      "r", "f");
    pm!("Result.all",          "results");
    pm!("Result.allSettled",   "results");

    // List
    pm!("List.get",        "list", "index");
    pm!("List.getOrPanic", "list", "index");
    pm!("List.push",       "list", "item");
    pm!("List.concat",     "a", "b");
    pm!("List.slice",      "list", "from", "to");
    pm!("List.contains",   "list", "item");
    pm!("List.map",        "list", "f");
    pm!("List.forEach",    "list", "f");
    pm!("Option.map",      "opt",  "f");
    pm!("List.filter",     "list", "pred");
    pm!("List.fold",       "list", "init", "f");
    pm!("List.find",       "list", "pred");
    pm!("List.any",        "list", "pred");
    pm!("List.all",        "list", "pred");
    pm!("List.sort",       "list", "cmp");
    pm!("List.zip",        "a", "b");
    pm!("List.distinct",   "list");
    pm!("List.partition",  "list", "pred");
    pm!("List.chunked",    "list", "size");
    pm!("List.groupBy",    "list", "key");
    pm!("List.upsert",     "list", "item", "on");
    pm!("List.flatMap",    "list", "f");
    pm!("List.reduce",     "list", "init", "f");
    pm!("List.sortBy",     "list", "key");
    pm!("List.minBy",      "list", "key");
    pm!("List.maxBy",      "list", "key");
    pm!("List.sumBy",      "list", "key");

    // Map
    pm!("Map.insert",      "map", "key", "value");
    pm!("Map.get",         "map", "key");
    pm!("Map.contains",    "map", "key");
    pm!("Map.remove",      "map", "key");

    // Channel
    pm!("Channel.new",         "capacity");
    pm!("Channel.send",        "channel", "item");
    pm!("Channel.receive",     "channel");
    pm!("Channel.tryReceive",  "channel");
    pm!("Channel.close",       "channel");
    pm!("Channel.isClosed",    "channel");

    // Text
    pm!("Text.toUpperLocale", "text", "locale");
    pm!("Text.toLowerLocale", "text", "locale");
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
    pm!("Text.charAt",     "text", "index");

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

    // Duration
    pm!("Duration.add",          "a", "b");
    pm!("Duration.sub",          "a", "b");
    pm!("Duration.eq",           "a", "b");
    pm!("Duration.lt",           "a", "b");
    pm!("Duration.gt",           "a", "b");
    pm!("DateTime.addDuration",  "dt", "duration");
    pm!("DateTime.diff",         "a", "b");
    pm!("Date.addDuration",      "date", "duration");

    // Timezone
    pm!("DateTime.inTimezone",   "dt", "tz");
    pm!("DateTime.formatTz",     "dt", "fmt", "tz");

    // Decimal
    pm!("Decimal.add",    "a", "b");
    pm!("Decimal.sub",    "a", "b");
    pm!("Decimal.mul",    "a", "b");
    pm!("Decimal.div",    "a", "b");
    pm!("Decimal.round",  "d", "places");

    // File / Path
    pm!("writeFile",   "path", "content");
    pm!("appendFile",  "path", "content");
    pm!("File.write",  "file", "content");
    pm!("Path.join",   "base", "part");

    // Process
    pm!("Process.exec",          "cmd", "args");
    pm!("Process.execInherit",   "cmd", "args");
    pm!("Process.execWithInput", "cmd", "args", "input");
    pm!("Process.lines",         "cmd", "args", "handler");
    pm!("Process.spawnDetached", "cmd", "args", "workingDir");
    pm!("Process.spawnDetachedHidden", "cmd", "args", "workingDir");
    pm!("Process.quit",          "code");

    // Json
    pm!("JsonValue.at",   "value", "index");
    pm!("JsonValue.get",  "value", "key");
    pm!("JsonValue.push", "array", "item");
    pm!("JsonValue.set",  "obj", "key", "value");

    // Http
    pm!("Credential.set", "target", "secret");
    pm!("Http.request", "method", "url", "headers", "body");
    pm!("Http.requestBytes", "method", "url", "headers", "body");
    // Real order is (url, body, content_type) — matches both the actual C
    // implementation (crates/stdlib/src/http.rs's certo_http_post/put take
    // (url, body, content_type)) and docs/STDLIB-QUICKREF.md's documented
    // signature. This previously listed "content_type" before "body",
    // silently swapping the two for any *named*-argument call
    // (Http.post(url: ..., content_type: ..., body: ...)) — a real,
    // confirmed bug found while verifying BACKLOG item 93's REST client
    // generator (a positional call, unaffected by this metadata, still
    // sent the content-type string as the request body).
    pm!("Http.post",    "url", "body", "content_type");
    pm!("Http.put",     "url", "body", "content_type");
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
    pm!("dbConnect",         "url");
    pm!("dbExec",            "conn", "sql", "params");
    pm!("dbQuery",           "conn", "sql", "params");
    pm!("dbQueryRow",        "conn", "sql", "params");
    pm!("withTransaction",   "conn", "body");
    pm!("withConnection",    "url",  "body");
    pm!("dbStream",          "conn", "sql", "params", "handler");
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

fn poly2(v1: u32, v2: u32, body: Ty) -> Ty {
    Ty::Forall { vars: vec![v1, v2], body: Box::new(body) }
}

fn poly3(v1: u32, v2: u32, v3: u32, body: Ty) -> Ty {
    Ty::Forall { vars: vec![v1, v2, v3], body: Box::new(body) }
}
