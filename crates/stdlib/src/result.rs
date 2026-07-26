/// C implementations for `Stdlib.Result` combinators.
///
/// `certo_result_t`/`certo_ok`/`certo_err`/`__result_is_ok`/`__result_unwrap`
/// are already defined in the core runtime preamble (`RUNTIME_HEADER` in
/// `crates/codegen/src/emit_module.rs`), which is always emitted before this
/// module's code. `CertoList`/`CertoFn1`/`list_alloc` come from
/// `Stdlib.Collections`, which `full_c_runtime()` places before this module.
pub const RESULT_C: &str = r#"
/* ------------------------------------------------------------------ */
/* Stdlib.Result — combinators over certo_result_t                     */
/* ------------------------------------------------------------------ */

/* flatMap(r, f) — if r is Ok(v), run f(v) (itself Result-returning);
   if Err, short-circuit with the same error unchanged. */
void* certo_flat_map(void* r, CertoFn1 f) {
    if (__result_is_ok(r)) return f((void*)__result_unwrap(r));
    return r;
}

/* mapErr(r, f) — transform the error side, leaving Ok untouched. */
void* certo_map_err(void* r, CertoFn1 f) {
    if (__result_is_ok(r)) return r;
    return certo_err((intptr_t)f((void*)__result_unwrap(r)));
}

/* getOrElse(r, default) — unwrap Ok, else the given default. */
intptr_t certo_get_or_else(void* r, intptr_t default_value) {
    if (__result_is_ok(r)) return __result_unwrap(r);
    return default_value;
}

/* recover(r, f) — unwrap Ok, else compute a fallback value from the error. */
intptr_t certo_recover(void* r, CertoFn1 f) {
    if (__result_is_ok(r)) return __result_unwrap(r);
    return (intptr_t)f((void*)__result_unwrap(r));
}

/* Result.all(results) — Ok(list of every payload) if all succeeded,
   else the first Err encountered (short-circuits). */
void* certo_result_all(CertoList* results) {
    int64_t n = results ? results->len : 0;
    CertoList* out = list_alloc(n);
    for (int64_t i = 0; i < n; i++) {
        void* r = results->data[i];
        if (!__result_is_ok(r)) return r;
        out->data[i] = (void*)__result_unwrap(r);
    }
    out->len = n;
    return certo_ok((intptr_t)out);
}

/* Result.allSettled(results) — every individual outcome, unchanged; unlike
   .all it never short-circuits. Results are already fully computed by the
   time you have a List<Result<T,E>> (Certo has no separate future/promise
   to await), so this is a deliberate identity — it exists for symmetry
   with .all and to name the "keep every outcome" intent at the call site. */
CertoList* certo_result_all_settled(CertoList* results) {
    return results;
}
"#;

/// Certo source declarations for `Stdlib.Result` (documentation text — see
/// BACKLOG item 107 for why this isn't actually parsed).
pub const RESULT_CERTO: &str = r#"
module Stdlib.Result

/// Chain a fallible operation onto a Result: if `r` is `Ok(v)`, run `f(v)`
/// (which may itself fail); if `r` is `Err(e)`, short-circuit with that
/// same error unchanged.
///
///   dbConnect(url) |> flatMap(runQuery)
fn flatMap<T, U, E>(r: Result<T, E>, f: fn(T): Result<U, E>): Result<U, E>

/// Transform the error side of a Result, leaving `Ok` untouched.
///
///   parseConfig(text) |> mapErr(ConfigError.from)
fn mapErr<T, E, F>(r: Result<T, E>, f: fn(E): F): Result<T, F>

/// Unwrap a Result, substituting `default` for `Err`.
///
///   val port = parsePort(text) |> getOrElse(8080)
fn getOrElse<T, E>(r: Result<T, E>, default: T): T

/// Unwrap a Result, computing a fallback value from the error for `Err`.
///
///   val total = computeTotal(order) |> recover(e => 0)
fn recover<T, E>(r: Result<T, E>, f: fn(E): T): T

/// Require every Result in the list to succeed: `Ok` of every payload if
/// they all did, else the first `Err` encountered.
///
///   Result.all([validateName(n), validateAge(a), validateEmail(e)])
fn Result.all<T, E>(results: List<Result<T, E>>): Result<List<T>, E>

/// Every individual outcome, unchanged — unlike `.all` this never
/// short-circuits, so you see every success and failure.
///
///   Result.allSettled([r1, r2, r3])
fn Result.allSettled<T, E>(results: List<Result<T, E>>): List<Result<T, E>>
"#;
