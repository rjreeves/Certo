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
   if Err, short-circuit with the same error unchanged. `f` is a real
   function *value* (`certo_fn_t { fn, env }`, BACKLOG item 140), not a bare
   pointer, so a lambda literal can capture a variable from its enclosing
   scope. */
void* certo_flat_map(void* r, certo_fn_t f) {
    if (__result_is_ok(r)) return ((CertoFn1)f.fn)(f.env, (void*)__result_unwrap(r));
    return r;
}

/* mapErr(r, f) — transform the error side, leaving Ok untouched. */
void* certo_map_err(void* r, certo_fn_t f) {
    if (__result_is_ok(r)) return r;
    return certo_err((intptr_t)((CertoFn1)f.fn)(f.env, (void*)__result_unwrap(r)));
}

/* getOrElse(r, default) — unwrap Ok, else the given default. */
intptr_t certo_get_or_else(void* r, intptr_t default_value) {
    if (__result_is_ok(r)) return __result_unwrap(r);
    return default_value;
}

/* recover(r, f) — unwrap Ok, else compute a fallback value from the error. */
intptr_t certo_recover(void* r, certo_fn_t f) {
    if (__result_is_ok(r)) return __result_unwrap(r);
    return (intptr_t)((CertoFn1)f.fn)(f.env, (void*)__result_unwrap(r));
}

/* Bridge codegen's `Result.flatMap`/`.mapErr`/`.getOrElse`/`.recover` names
   (BACKLOG item 199 — needed so the dot-call form spec 5.4's own pattern
   table shows actually resolves, see `crates/stdlib/src/seed.rs`'s own
   note) to the identical implementations above, the same `#define`
   bridge pattern `List.reduce`/`Timestamp.*` already use elsewhere in this
   codebase — the qualified names mangle to different C symbols
   (`certo_result_flat_map`, etc.) than the bare ones already implemented. */
#define certo_result_flat_map    certo_flat_map
#define certo_result_map_err     certo_map_err
#define certo_result_get_or_else certo_get_or_else
#define certo_result_recover     certo_recover

/* Result.isOk/isErr (BACKLOG item 165) — thin wrappers over the existing
   __result_is_ok primitive every combinator above already uses. Real,
   independently useful predicates, not just plumbing for
   expect(...).toBeOk()/.toBeErr(). */
bool certo_result_is_ok(void* r) { return __result_is_ok(r); }
bool certo_result_is_err(void* r) { return !__result_is_ok(r); }

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
