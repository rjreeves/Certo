use std::collections::HashMap;
use std::sync::OnceLock;
use certo_ast::module::Module;
use certo_ast::decl::{Decl, FnParam};
use certo_ast::expr::{Expr, ExpectMatcher, Stmt, Lit, BinOp as AstBinOp, UnOp as AstUnOp, FStringPart};
use certo_ast::pattern::Pattern;
use certo_ast::span::{S, Span};
use certo_typeck::Ty;
use crate::hir::*;
use crate::error::{LowerError, LowerErrorKind};

// ------------------------------------------------------------------ //
// Lowering context
// ------------------------------------------------------------------ //

struct Cx {
    /// Next LocalId to assign.
    next_local: LocalId,
    /// Next FnId to assign.
    next_fn:    FnId,
    /// Name → LocalId for the current scope stack.
    locals:     Vec<HashMap<String, LocalId>>,
    /// Name → FnId for top-level functions.
    globals:    HashMap<String, FnId>,
    /// Name → param list for user-defined functions (for labeled/default arg normalization).
    fn_params:  HashMap<String, Vec<FnParam>>,
    /// Full-qualified name → param names for stdlib functions (keyed as "Module.fn").
    stdlib_params: HashMap<&'static str, &'static [&'static str]>,
    /// Statemachine-generated function full names → return type.
    sm_returns:    HashMap<String, Ty>,
    /// User-defined function names → return type (from AST annotation).
    fn_ret_types:  HashMap<String, Ty>,
    /// User-defined function names → declared param types, as written (a
    /// bare type-param param is `Ty::Var(0)`) — BACKLOG item 120.
    fn_param_tys:  HashMap<String, Vec<Ty>>,
    /// Global value types — for sum variant constants like `Red`, `Green`.
    global_types:  HashMap<String, Ty>,
    /// Sum variant name → parent type name (e.g. "Red" → "Color").
    variant_to_type: HashMap<String, String>,
    /// Sum variant name → ordered C field names for its payload (the real
    /// field name where the variant declared one, else the positional
    /// fallback `f{i}` — must match `emit_module.rs`'s struct emission).
    variant_field_names: HashMap<String, Vec<String>>,
    /// Sum variant name → ordered payload field types (needed so a
    /// match-bound field local gets its real type instead of `Ty::Error`,
    /// which codegen maps to `int64_t` and silently truncates e.g. `Float`).
    variant_field_types: HashMap<String, Vec<Ty>>,
    /// Record type name → ordered field names (for spread desugar).
    record_field_names: HashMap<String, Vec<String>>,
    /// Record type name → ordered declared field types, mirroring
    /// `variant_field_types` — a bare type-param field (e.g. `value: T`)
    /// resolves to the opaque `Ty::Var(0)` sentinel via
    /// `ast_ty_to_ty_with_params`. Used for BACKLOG item 119's generic
    /// type-erasure boxing: a `Ty::Var(0)` field is heap-boxed on
    /// construction and unboxed on read, since its real C storage is `void*`
    /// regardless of what concrete type it's instantiated to.
    record_field_types: HashMap<String, Vec<Ty>>,
    /// `computed` property names per record type (BACKLOG item 143) — a
    /// name in here is never a real struct field; `Expr::Field` lowering
    /// checks this before falling through to `resolve_field_ty`'s ordinary
    /// struct-read path, and instead emits a call to the synthesized
    /// accessor method (`crates/parser/src/parse_decl.rs`'s
    /// `synthesize_computed_method`, registered under `Type.name` — its
    /// return type is already in `fn_ret_types` via the exact same
    /// impl-method hoisting every other method uses, so no separate
    /// type table is needed here).
    computed_field_names: HashMap<String, Vec<String>>,
    /// LocalId → type, for local variables whose type is known (val bindings,
    /// function params). Lets a variable *reference* carry its type — needed so
    /// `match q { Some(x) => … }` knows `q`'s Option payload type.
    local_types: HashMap<LocalId, Ty>,
    errors:        Vec<LowerError>,
}

impl Cx {
    fn new() -> Self {
        Cx {
            next_local:    0,
            next_fn:       0,
            locals:        vec![HashMap::new()],
            globals:       HashMap::new(),
            fn_params:     HashMap::new(),
            stdlib_params: stdlib_param_names(),
            sm_returns:    HashMap::new(),
            fn_ret_types:  HashMap::new(),
            fn_param_tys:  HashMap::new(),
            global_types:        HashMap::new(),
            variant_to_type:     HashMap::new(),
            variant_field_names: HashMap::new(),
            variant_field_types: HashMap::new(),
            record_field_names:  HashMap::new(),
            record_field_types:  HashMap::new(),
            computed_field_names: HashMap::new(),
            local_types:         HashMap::new(),
            errors:              Vec::new(),
        }
    }

    fn fresh_local(&mut self) -> LocalId {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    fn fresh_fn(&mut self) -> FnId {
        let id = self.next_fn;
        self.next_fn += 1;
        id
    }

    fn define_local(&mut self, name: &str) -> LocalId {
        let id = self.fresh_local();
        self.locals.last_mut().unwrap().insert(name.to_string(), id);
        id
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        for frame in self.locals.iter().rev() {
            if let Some(&id) = frame.get(name) { return Some(id); }
        }
        None
    }

    fn push_scope(&mut self) { self.locals.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.locals.pop(); }

    fn err(&mut self, kind: LowerErrorKind, span: Span) {
        self.errors.push(LowerError { kind, span });
    }
}

/// Return types for monomorphic stdlib functions whose result type the HIR
/// needs (e.g. to know an `Option`'s payload type for heap-box/unbox, or to
/// give a `val` binding's local its real C type instead of the `Ty::Error`
/// → `int64_t` fallback, which silently truncates non-int64-layout results
/// like `Float`/`Float32`/`Text`).
///
/// Derived mechanically from `certo_stdlib::seed_stdlib`'s registered
/// `TypeEnv` — the same table typeck itself checks calls against — instead
/// of hand-listing individual function names. Two hand-maintained copies of
/// this table previously existed here (one for bare top-level names, one for
/// qualified `Type.method` names) and both missed functions from time to
/// time (confirmed: `intToFloat`/`floatToInt` were never added to either,
/// so `val f = intToFloat(3); val g = f / 2.0` silently computed truncating
/// integer division instead of `Float` division — BACKLOG item 129).
/// Every concrete (non-generic) stdlib function is now picked up
/// automatically; genuine generic producers (`List.get<T>`, whose *return*
/// type itself mentions the bound type var) are skipped here — their
/// payload type depends on the call's argument types, which
/// `generic_container_ret` below recovers structurally instead.
///
/// A `Forall`-wrapped signature is unwrapped to its inner `Ty::Fn` before
/// this check, rather than skipped outright — BACKLOG item 179's own
/// investigation found the previous direct-`Ty::Fn`-only match silently
/// excluded *every* generic stdlib function, even ones like
/// `List.len<T>(list: List<T>): Int` whose return type is fully concrete
/// regardless of `T` (only the *parameter* is generic). That left `List.len`
/// invisible here, so `val n = List.len(xs)`'s local kept the `Ty::Error` →
/// `int64_t` fallback from this doc comment's own warning above — normally
/// silent, but fatal the moment anything downstream needed to know it was
/// really an `Int` (a `certo_int_to_text` conversion for an f-string
/// interpolating `n`, say): the raw `int64_t` bits got read as a `Text`
/// pointer and dereferenced, segfaulting. Confirmed via direct testing
/// (`val xs: List<Int> = [1,2,3]; val n = List.len(xs); println(f"{n}")`)
/// before writing this fix.
fn stdlib_ret_types() -> &'static HashMap<String, Ty> {
    static TABLE: OnceLock<HashMap<String, Ty>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut env = certo_typeck::TypeEnv::new();
        let mut counter = 0u32;
        certo_stdlib::seed_stdlib(&mut env, &mut counter);
        env.names()
            .into_iter()
            .filter_map(|name| {
                let ty = env.lookup(&name)?;
                let unwrapped = match ty {
                    Ty::Forall { body, .. } => body.as_ref(),
                    other => other,
                };
                match unwrapped {
                    Ty::Fn { ret, .. } if !ret.has_vars() => Some((name, (**ret).clone())),
                    _ => None,
                }
            })
            .collect()
    })
}

/// Every registered stdlib function name, generic or not (unlike
/// `stdlib_ret_types()` above, which only covers the narrower subset whose
/// *return type* happens to have no leftover type vars — `Option.isSome<T>`,
/// `Result.isOk<T,E>`, etc. are `Ty::Forall`-wrapped and never match that
/// table's `Ty::Fn` filter even though their concrete `Bool` return has no
/// vars at all once instantiated). Used only to answer "does a stdlib
/// function with this exact qualified name exist" (the dot-call UFCS
/// rewrite, BACKLOG item 162) — a yes/no membership check, not a source of
/// type information, so `Ty::Forall` bodies don't need to be unwrapped here.
fn stdlib_fn_names() -> &'static std::collections::HashSet<String> {
    static TABLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut env = certo_typeck::TypeEnv::new();
        let mut counter = 0u32;
        certo_stdlib::seed_stdlib(&mut env, &mut counter);
        env.names().into_iter().collect()
    })
}

/// Declared param types for stdlib functions, keyed the same way
/// `stdlib_ret_types()` is (built from the same real `TypeEnv` seeding, not
/// hand-maintained) — lets bare-generic-return argument-position resolution
/// (`resolve_bare_generic_return`, BACKLOG item 135) see a *stdlib* callee's
/// declared param type, not just a user-defined one via `cx.fn_param_tys`.
/// Without this, `println(Secret.expose(s))` neither resolved nor errored —
/// it silently stayed as the raw unboxed pointer and printed garbage,
/// confirmed by direct testing; `cx.fn_param_tys` only ever covers
/// user-defined `fn`/`impl` declarations, never stdlib signatures.
fn stdlib_param_types() -> &'static HashMap<String, Vec<Ty>> {
    static TABLE: OnceLock<HashMap<String, Vec<Ty>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut env = certo_typeck::TypeEnv::new();
        let mut counter = 0u32;
        certo_stdlib::seed_stdlib(&mut env, &mut counter);
        env.names()
            .into_iter()
            .filter_map(|name| match env.lookup(&name) {
                Some(Ty::Fn { params, .. }) => Some((name, params.clone())),
                // Generic stdlib functions (`List.map`, `List.sortBy`, etc)
                // register as `Ty::Forall { body: Box<Ty::Fn>, .. }`, not a
                // bare `Ty::Fn` — the match above silently excluded every
                // one of them from this table (BACKLOG item 162b). Harmless
                // for functions dispatched through `BOXED_ABI_CALLEES`
                // (`List.map`'s own lambda-arg param types come from a
                // completely separate mechanism, the call site's own list
                // element type — see `crates/mir/src/lower.rs`'s
                // `elem_ty_hint`), but `List.sortBy`/`minBy`/`maxBy`/
                // `sumBy` are the first Forall-wrapped functions that both
                // take a lambda argument *and* aren't in that list, so
                // nothing had ever exercised this gap before: a bare
                // unannotated `(x) => ...` key lambda's own param stayed
                // `Ty::Error`, silently miscompiling to `int64_t` instead
                // of the real element type — confirmed directly (`_l4(_l5)`:
                // called object type 'int64_t' is not a function).
                Some(Ty::Forall { body, .. }) => match body.as_ref() {
                    Ty::Fn { params, .. } => Some((name, params.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    })
}

/// Merges two independently-recovered branch types from the same `if`/
/// `match` (BACKLOG item 227) — e.g. `if cond then Err(e) else Ok(x)`,
/// where each branch's own call-type recovery (the new `Ok`/`Err` arms in
/// `generic_container_ret` below) can only ever know ONE slot of the
/// shared `Result` shape, leaving the other `Ty::Error`. The previous,
/// shallower check here (`if !matches!(then_.ty, Error) { then_ } else
/// { else_ }`) picked one branch's type wholesale the moment it wasn't
/// *literally* `Ty::Error` at the top level — so `Err("too big")`'s own
/// `Result<Error, Text>` was accepted as-is and `Ok(x)`'s `Result<Int,
/// Error>` was never even consulted, leaving the success slot broken
/// (confirmed via a real segfault reading it). Recurses into same-shape
/// `Result`/`Option` containers so each slot is resolved independently,
/// preferring whichever side isn't `Ty::Error`; a genuine disagreement
/// between two *already concrete* types (which shouldn't occur in
/// type-checked code) arbitrarily keeps the first, matching this
/// function's own predecessor's tie-breaking, not a new risk it introduces.
fn merge_partial(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        (Ty::Error, b) => b,
        (a, Ty::Error) => a,
        (Ty::Result(a_ok, a_err), Ty::Result(b_ok, b_err)) =>
            Ty::Result(Box::new(merge_partial(*a_ok, *b_ok)), Box::new(merge_partial(*a_err, *b_err))),
        (Ty::Option(a_inner), Ty::Option(b_inner)) =>
            Ty::Option(Box::new(merge_partial(*a_inner, *b_inner))),
        (a, _) => a,
    }
}

/// Return types for generic stdlib functions whose result depends on an
/// argument's element type (e.g. `List.get<T>(List<T>, Int): T?`). Recovering
/// the element type lets the caller unbox the payload correctly (Float bits).
/// Deliberately mechanical/structural, not real inference — BACKLOG item 113.
fn generic_container_ret(full: Option<&str>, args: &[HirExpr]) -> Option<Ty> {
    let list_elem = |a: &HirExpr| match &a.ty { Ty::List(inner) => Some((**inner).clone()), _ => None };
    let map_kv = |a: &HirExpr| match &a.ty { Ty::Map(k, v) => Some(((**k).clone(), (**v).clone())), _ => None };
    let result_ok_err = |a: &HirExpr| match &a.ty { Ty::Result(ok, err) => Some(((**ok).clone(), (**err).clone())), _ => None };
    // A callback argument's own return type, recovered two ways: an inline
    // lambda whose param-hinted body actually resolved (the original path,
    // e.g. `(x) => x * 2`), or — BACKLOG item 180 — a bare reference to a
    // named function (`xs.map(double)`), whose real `Ty::Fn{params, ret}` is
    // already populated by `Expr::Path`'s own lowering above (see that arm's
    // comment: reconstructed from `cx.fn_params`/`cx.fn_ret_types` for
    // exactly this "callback passed by name" case, following the identical
    // precedent `dbQueryTyped`/`Query.list`/`Query.first` already use just
    // below). Without the second branch, a named-function callback silently
    // left the caller's `val` binding `Ty::Error`-typed — harmless until
    // something downstream (an f-string interpolation, another chained call)
    // actually needed the real type, at which point it read raw bits through
    // the wrong C type and crashed — the same failure shape item 179 found
    // for `List.len`, confirmed here by direct testing with
    // `xs.map(double)` where `double` is a plain top-level `fn`.
    let callback_ret_ty = |a: &HirExpr| match &a.kind {
        HirExprKind::Lambda { body, .. } if !matches!(body.ty, Ty::Error) => Some(body.ty.clone()),
        _ => match &a.ty {
            Ty::Fn { ret, .. } => Some((**ret).clone()),
            _ => None,
        },
    };

    match full {
        Some("List.get") | Some("List.first") | Some("List.last") | Some("List.find") => {
            args.first().and_then(list_elem).map(|e| Ty::Option(Box::new(e)))
        }
        Some("List.getOrPanic") => args.first().and_then(list_elem),
        Some("List.filter") | Some("List.sort") | Some("List.reverse") | Some("List.distinct")
        | Some("List.slice") | Some("List.concat") | Some("List.push") | Some("List.sortBy")
        | Some("List.upsert") => {
            args.first().map(|a| a.ty.clone())
        }
        // `List.minBy`/`maxBy` (BACKLOG item 162b) — same shape as
        // `List.first`/`.last`/`.find` above: `Option<T>` from the list's
        // own element type, unrelated to the key projection's type.
        Some("List.minBy") | Some("List.maxBy") => {
            args.first().and_then(list_elem).map(|e| Ty::Option(Box::new(e)))
        }
        // `List.sumBy` (BACKLOG item 162b) — the key lambda's own resolved
        // body type *is* the return type here (unlike `sortBy`/`minBy`/
        // `maxBy`, which return the list's element type). Same recovery
        // pattern as `List.map` just below: only available when the
        // lambda's param hint let its body actually resolve.
        Some("List.sumBy") => args.get(1).and_then(callback_ret_ty),
        Some("List.partition") => {
            args.first().map(|a| Ty::Tuple(vec![a.ty.clone(), a.ty.clone()]))
        }
        Some("List.chunked") => args.first().map(|a| Ty::List(Box::new(a.ty.clone()))),
        // The callback (args[1]) was lowered via `lower_lambda_with_param_hint`,
        // so its body's type is now recoverable (not guaranteed — only when the
        // hinted param propagated through, e.g. a direct arithmetic/literal
        // body) rather than unconditionally `Ty::Error`.
        Some("List.map") => args.get(1).and_then(callback_ret_ty).map(|t| Ty::List(Box::new(t))),
        Some("List.groupBy") => {
            let elem = args.first().and_then(list_elem)?;
            args.get(1).and_then(callback_ret_ty)
                .map(|key_ty| Ty::Map(Box::new(key_ty), Box::new(Ty::List(Box::new(elem)))))
        }
        // `List.fold`/`List.reduce` (BACKLOG item 211) — the accumulator's
        // own type (args[1], the `init` value) *is* the return type here,
        // by definition (`fold(list, init, f): acc` — `f`'s own signature
        // is `(acc, T) -> acc`, so there's no need to inspect the combiner
        // lambda at all, unlike `sumBy`'s callback-return-type recovery
        // above). Without this arm, `xs.reduce(0, (a,b)=>a+b)`'s call type
        // fell through to `Ty::Error`; harmless until something downstream
        // needed to know it was really an `Int` (an f-string interpolation,
        // say) — the same "known type thrown away" crash items 179/180/182/
        // 189/191 already fixed elsewhere, missed for `fold`/`reduce`.
        Some("List.fold") | Some("List.reduce") => args.get(1).map(|a| a.ty.clone()),
        // `Ok`/`Err` (BACKLOG item 227) — registered (`crates/stdlib/src/
        // seed.rs`) as `∀A,B. A -> Result<A,B>` / `∀A,B. B -> Result<A,B>`:
        // the *other* type parameter (`B` for `Ok`, `A` for `Err`) is never
        // constrained by the constructor's own single argument, so both
        // fall through `stdlib_ret_types()` (`ret.has_vars()`) same as
        // every other arm here — but unlike every other arm, NOTHING
        // recovered anything for them at all before this: a direct HIR
        // probe confirmed `Ok(x + 1)`'s own call type was plain `Ty::Error`
        // in its entirety, not "a `Result` with one bad slot" as it first
        // appeared from the outside — since `Ok`/`Err` are stdlib
        // constructors, not user-defined sum-type variants, they also
        // never matched `recover_generic_variant_call_ty` below. Used
        // directly as an unannotated inline lambda's own tail (`(x) =>
        // Ok(x + 1)`, passed to `flatMap`), that `Ty::Error` propagated
        // into the enclosing call's own type, then into whatever consumed
        // it (a `match` arm's binding, then an f-string interpolation) —
        // the same "known type thrown away" segfault class items 179/180/
        // 182/189/191/199/211 already fixed elsewhere, one level deeper.
        // Recovers the one half that *is* knowable from the argument,
        // leaving the unconstrained half `Ty::Error` — mirrors
        // `recover_generic_variant_call_ty`/`recover_generic_record_ty`
        // below's own established "recover what's known, `Error` for the
        // rest" convention for user-defined generics. `flatMap`'s own arm
        // just below then backfills that remaining `Error` half from the
        // receiver's own already-known error type (real, not a guess:
        // `flatMap`'s signature guarantees the error type never changes).
        Some("Ok")  => args.first().map(|a| Ty::Result(Box::new(a.ty.clone()), Box::new(Ty::Error))),
        Some("Err") => args.first().map(|a| Ty::Result(Box::new(Ty::Error), Box::new(a.ty.clone()))),
        // `Some(x)` (BACKLOG item 250) — the identical gap as `Ok`/`Err`
        // just above, for `Option` instead of `Result`: recovers `Ty::Option`
        // from the argument's own already-known type. Went unnoticed
        // longer than `Ok`/`Err` since a bare `val x = Some(5)` with no
        // further consumption needing the real type limps along fine on
        // `Ty::Error` alone — only surfaced chasing item 246's `.age` fix,
        // where `val t: Timestamp? = Some(Timestamp.now())` stayed
        // `Ty::Error` even after that fix.
        Some("Some") => args.first().map(|a| Ty::Option(Box::new(a.ty.clone()))),
        // `flatMap` and its `Result.`-qualified alias (BACKLOG item 199,
        // extended by item 227 above) — registered as `Result<T,E> -> (T ->
        // Result<U,E>) -> Result<U,E>`: the error type `E` is always
        // identical between the receiver and the result, by the
        // signature itself, so it's taken directly from the receiver
        // (already known, e.g. from a `val r: Result<Int,Text> = ...`
        // annotation) rather than from the callback's own recovered
        // return type — correct even when the callback's body is a bare
        // `Ok(...)` (whose own `Err`-side slot the arm above can only ever
        // leave as `Ty::Error`, never actually knowing it). Confirmed by
        // direct testing: even the long-working *bare*-call form
        // (`flatMap(r, f)`) segfaulted the moment its result was actually
        // consumed (e.g. `match result { ... }`) — the call's own type
        // silently stayed `Ty::Error`, the exact "known type thrown away"
        // crash items 179/180/182/189/191/211 already fixed elsewhere,
        // just never caught here since dot-call reachability (item 199)
        // was the only thing previously exercising these functions at all.
        Some("flatMap") | Some("Result.flatMap") => {
            let succ_ty = args.get(1).and_then(callback_ret_ty).map(|t| match t {
                Ty::Result(succ, _) => *succ,
                other => other,
            })?;
            let err_ty = args.first().and_then(result_ok_err).map(|(_, e)| e).unwrap_or(Ty::Error);
            Some(Ty::Result(Box::new(succ_ty), Box::new(err_ty)))
        }
        // `recover`/`Result.recover`'s callback returns the raw success
        // value `T` directly, never a `Result` (`Result<T,E> -> (E -> T)
        // -> T` — `crates/stdlib/src/seed.rs`), so a bare `Ok(...)`/
        // `Err(...)` tail could never legitimately appear here in the
        // first place; no backfill needed, unlike `flatMap` above.
        Some("recover") | Some("Result.recover") => args.get(1).and_then(callback_ret_ty),
        Some("mapErr") | Some("Result.mapErr") => {
            let ok_ty = args.first().and_then(result_ok_err).map(|(ok, _)| ok)?;
            let err_ty = args.get(1).and_then(callback_ret_ty)?;
            Some(Ty::Result(Box::new(ok_ty), Box::new(err_ty)))
        }
        Some("getOrElse") | Some("Result.getOrElse") => args.get(1).map(|a| a.ty.clone()),
        Some("Map.get") => args.first().and_then(map_kv).map(|(_, v)| Ty::Option(Box::new(v))),
        Some("Map.remove") | Some("Map.insert") => args.first().map(|a| a.ty.clone()),
        Some("Map.keys") => args.first().and_then(map_kv).map(|(k, _)| Ty::List(Box::new(k))),
        Some("Map.values") => args.first().and_then(map_kv).map(|(_, v)| Ty::List(Box::new(v))),
        // `dbQueryTyped`/`Query.list`/`Query.first`/`Query.groupedList` are
        // registered in `crates/stdlib/src/seed.rs` as `Forall` generics
        // (`(..., List<Text?> -> T) -> List<T>` / `-> T?`), so they're
        // filtered out of `stdlib_ret_types()` (`ret.has_vars()`) and land
        // here instead. `T` is recovered from the mapper argument's own
        // type — always the last argument, and (per BACKLOG item 134) always
        // a bare reference to a user-defined `fn` in practice, whose real
        // `Ty::Fn{params, ret}` is now populated in `Expr::Path` lowering
        // above. Without this, a `val rows = dbQueryTyped(...)` binding's
        // type stays `Ty::Error`, which cascades into `List.first(rows)`
        // also failing to resolve its own `Option<T>` result type — the
        // exact gap that let a struct-element `Option` skip the
        // `UnwrapOptStructBox` fixup and return one level of indirection
        // too deep.
        Some("dbQueryTyped") | Some("Query.list") | Some("Query.groupedList") => {
            args.last().and_then(|a| match &a.ty { Ty::Fn { ret, .. } => Some((**ret).clone()), _ => None })
                .map(|t| Ty::List(Box::new(t)))
        }
        Some("Query.first") => {
            args.last().and_then(|a| match &a.ty { Ty::Fn { ret, .. } => Some((**ret).clone()), _ => None })
                .map(|t| Ty::Option(Box::new(t)))
        }
        // `compose`/`const`/`flip` (BACKLOG item 161) — registered as
        // `Forall` generics whose return type is itself a `Ty::Fn`, so
        // they're filtered out of `stdlib_ret_types()` (`ret.has_vars()`)
        // and land here instead. Without this, the call's own `call_ty`
        // falls all the way through to the final fallback (the *callee's*
        // own uninstantiated `Ty::Fn { params: [Var], ret: Var }`, straight
        // from `compose`'s raw stdlib signature) — `Ty::Fn` itself always
        // maps to `certo_fn_t` regardless of its inner param/ret types
        // (`ty_to_c.rs`), so the returned closure's *local* still gets
        // declared correctly, but `Ty::Var` itself maps to `void*` — so a
        // later call through it (`emit_callee`) would cast using the
        // erased `void*`-in/`void*`-out convention instead of the real
        // concrete types (e.g. `Int => Text`) `lower_compose_call`'s own
        // synthesized trampoline (`crates/mir/src/lower.rs`) actually uses.
        // For most types this "happens to" still produce the right bits
        // (pointer/int-sized values pass through either convention
        // identically) — but a `Float` param/return uses a genuinely
        // different register class (XMM vs general-purpose), where the
        // mismatch would silently corrupt the value, the same class of bug
        // item 112 already fixed once for lambda callbacks. Recovering the
        // real concrete types here (from `f`/`g`'s own already-known types,
        // no unification needed) closes that gap before it can bite.
        Some("compose") => {
            let a_ty = args.get(1).and_then(|g| match &g.ty {
                Ty::Fn { params, .. } if params.len() == 1 => Some(params[0].clone()),
                _ => None,
            })?;
            let c_ty = args.first().and_then(|f| match &f.ty {
                Ty::Fn { ret, .. } => Some((**ret).clone()),
                _ => None,
            })?;
            Some(Ty::Fn { params: vec![a_ty], ret: Box::new(c_ty) })
        }
        // `const(a): B => A` — `B` is genuinely unconstrained by any
        // argument to `const` itself, so it's left erased (`Ty::Var(0)` →
        // `void*`) rather than guessed; must match `lower_const_call`'s own
        // identical choice (`crates/mir/src/lower.rs`) exactly, or the call
        // site and the synthesized trampoline would disagree on the
        // parameter's calling convention.
        Some("const") => {
            let a_ty = args.first()?.ty.clone();
            Some(Ty::Fn { params: vec![Ty::Var(0)], ret: Box::new(a_ty) })
        }
        // `flip(f: A => B => C): B => A => C` — A, B, C all come from `f`'s
        // own already-known (curried) type, no unification needed. Must
        // match `lower_flip_call`'s identical nested-closure shape exactly.
        Some("flip") => {
            let (a_ty, bc_ty) = match &args.first()?.ty {
                Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
                _ => return None,
            };
            let (b_ty, c_ty) = match &bc_ty {
                Ty::Fn { params, ret } if params.len() == 1 => (params[0].clone(), (**ret).clone()),
                _ => return None,
            };
            Some(Ty::Fn { params: vec![b_ty], ret: Box::new(Ty::Fn { params: vec![a_ty], ret: Box::new(c_ty) }) })
        }
        _ => None,
    }
}

/// Mirrors `mir::lower::binop_result_ty` at the HIR level, operating directly
/// on already-known operand types instead of MIR operands. Lets e.g.
/// `x * 2.0` (inside a lambda whose param type was hinted — see
/// `lower_lambda_with_param_hint`) resolve to `Float` instead of `Ty::Error`,
/// which is what let a `List.map` call's return type stay unrecoverable
/// even after this fix — BACKLOG item 113 (the compounding gap found while
/// verifying item 112).
fn binop_result_ty(op: &crate::hir::BinOp, lhs: &Ty, rhs: &Ty) -> Ty {
    use crate::hir::BinOp::*;
    match op {
        Eq | NotEq | Lt | LtEq | Gt | GtEq | And | Or => Ty::Bool,
        Concat => Ty::Text,
        NullCoalesce => match lhs {
            Ty::Option(inner) => (**inner).clone(),
            _ => rhs.clone(),
        },
        Add | Sub | Mul | Div | Rem | Pow => match lhs {
            Ty::Error => rhs.clone(),
            t => t.clone(),
        },
    }
}

/// Lower a lambda literal used directly as the callback argument to a stdlib
/// call whose param type is known from context (currently `List.map`'s and
/// `List.groupBy`'s first argument — the scrutinee list's element type).
/// Lambda params are almost always unannotated, so without this hint the
/// param — and anything the body computes from it — stays `Ty::Error`
/// through HIR, and downstream code (e.g. a `val` binding to the call's
/// result, or a `for` loop over it) can't tell it's ever handling a `Float`
/// and skips unboxing it. See BACKLOG item 113.
fn lower_lambda_with_param_hint(params: &[certo_ast::expr::LambdaParam], body: &S<Expr>, hint: &Ty, cx: &mut Cx, span: Span) -> HirExpr {
    lower_lambda_with_param_hints(params, body, std::slice::from_ref(hint), &Ty::Error, cx, span)
}

/// Same as `lower_lambda_with_param_hint`, but one hint per param instead of
/// only ever hinting param 0, plus a return-type hint — needed when a
/// lambda literal is passed directly as an argument whose *declared* type
/// is itself a multi-param `Ty::Fn` (e.g. a user-defined higher-kinded
/// function's own `unwrap: F<A> => A` / `wrap: B => F<B>` parameters,
/// BACKLOG item 76): each of the lambda's own params must pick up the
/// matching declared param type from that `Ty::Fn` (typically `Ty::Var(0)`,
/// the same erased-generic sentinel a bare `T` already uses), or a still-
/// erased value flowing into a concrete call inside the lambda body
/// (`Box.unwrap(x)`) has no declared type at all to trigger unboxing from;
/// `ret_hint` is threaded onto the resulting `HirExprKind::Lambda` node
/// itself (see its own doc comment) so MIR can box the lambda's real
/// concrete result (`Box.wrap(x)`'s real `Box`) if the declared return
/// position expects an erased value instead.
fn lower_lambda_with_param_hints(params: &[certo_ast::expr::LambdaParam], body: &S<Expr>, hints: &[Ty], ret_hint: &Ty, cx: &mut Cx, span: Span) -> HirExpr {
    let capture_threshold = cx.next_local;
    cx.push_scope();
    let hir_params: Vec<HirParam> = params.iter().enumerate().map(|(i, p)| {
        let local = cx.define_local(&p.name.node);
        let ty = p.ty.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &[]))
            .unwrap_or_else(|| hints.get(i).cloned().unwrap_or(Ty::Error));
        if !matches!(ty, Ty::Error) { cx.local_types.insert(local, ty.clone()); }
        HirParam { local, name: p.name.node.clone(), ty, span: p.span }
    }).collect();
    let body = lower_expr(body, cx);
    cx.pop_scope();
    let captures = collect_lambda_captures(&body, capture_threshold);
    HirExpr { kind: HirExprKind::Lambda { params: hir_params, body: Box::new(body), captures, ret_hint: ret_hint.clone() }, ty: Ty::Error, span }
}

/// `List.sortBy`/`minBy`/`maxBy`/`sumBy`'s key/numeric projection type
/// restriction (BACKLOG item 162b) — mirrors `crates/typeck/src/
/// infer_expr.rs`'s `is_supported_key_type`, kept in sync by hand since
/// the two run at different pipeline stages against differently-resolved
/// types (see the call site's own doc comment for why both exist).
fn is_supported_key_ty(ty: &Ty) -> bool {
    matches!(ty,
        Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt | Ty::Float | Ty::Float32)
}

fn stdlib_param_names() -> HashMap<&'static str, &'static [&'static str]> {
    let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();

    // Core
    m.insert("assert",          &["cond", "msg"]);
    m.insert("pow",             &["base", "exp"]);
    m.insert("minInt",          &["a", "b"]);
    m.insert("maxInt",          &["a", "b"]);
    m.insert("minFloat",        &["a", "b"]);
    m.insert("maxFloat",        &["a", "b"]);
    m.insert("range",           &["from", "to"]);
    m.insert("rangeInclusive",  &["from", "to"]);

    // List
    m.insert("List.get",        &["list", "index"]);
    m.insert("List.getOrPanic", &["list", "index"]);
    m.insert("List.push",       &["list", "item"]);
    m.insert("List.concat",     &["a", "b"]);
    m.insert("List.slice",      &["list", "from", "to"]);
    m.insert("List.contains",   &["list", "item"]);
    m.insert("List.map",        &["list", "f"]);
    m.insert("List.filter",     &["list", "pred"]);
    m.insert("List.fold",       &["list", "init", "f"]);
    m.insert("List.find",       &["list", "pred"]);
    m.insert("List.any",        &["list", "pred"]);
    m.insert("List.all",        &["list", "pred"]);
    m.insert("List.sort",       &["list", "cmp"]);
    m.insert("List.zip",        &["a", "b"]);
    m.insert("List.sortBy",     &["list", "key"]);
    m.insert("List.minBy",      &["list", "key"]);
    m.insert("List.maxBy",      &["list", "key"]);
    m.insert("List.sumBy",      &["list", "key"]);
    // `List.groupBy` needed this table for a different reason than
    // sortBy/minBy/maxBy/sumBy did (BACKLOG item 170, corrected after
    // investigation): its callback's own param type comes from a wholly
    // separate mechanism (`lower_lambda_boxed`'s `elem_ty_hint`, since
    // `List.groupBy` is a `BOXED_ABI_CALLEES` member — struct-keyed
    // `List.groupBy` already worked fine without this entry). What
    // actually broke, only reachable after BACKLOG item 171 fixed
    // labeled-arg reordering for module-qualified typeck calls generally:
    // `List.groupBy(key: ..., list: ...)` then type-checked correctly but
    // still *lowered* its arguments in written (unreordered) source order
    // here, since this table — not item 171's typeck-side one — is what
    // gates HIR's own labeled-arg reordering, producing a real, confirmed
    // C-level argument-order mismatch (`certo_list_group_by` called with
    // the key closure and list swapped).
    m.insert("List.groupBy",    &["list", "key"]);
    m.insert("List.upsert",     &["list", "item", "on"]);

    // Result (BACKLOG item 227) — needed for the exact same reason
    // `List.map` etc. above are: without a `stdlib_params` entry here,
    // `stdlib_names` (`crates/hir/src/lower.rs`'s own `Expr::App` arm)
    // never recognizes `flatMap`/`mapErr`/`recover` as a function whose
    // lambda argument needs a param-type hint at all — this table is a
    // separate, HIR-local structure from `crates/stdlib/src/seed.rs`'s own
    // `pm!`-registered param names (which back typeck's default-argument
    // handling, not this). Both the bare and `Result.`-qualified spellings
    // need their own entry — this table's `fn_full_path` lookup has no
    // bare-name fallback, so a dot-called `r.flatMap(f)` (rewritten to
    // `Result.flatMap` by item 199's UFCS rewrite) silently got neither
    // this table's bare entry (name mismatch) nor a qualified one (never
    // existed) — confirmed via a direct HIR probe: `r.flatMap((x) => Ok(x))`
    // left `x` as `Ty::Error` even after this exact list's own hint-source
    // selection (`result_hint_side`, this arm's own sibling logic below)
    // was added, until this entry existed too.
    m.insert("flatMap",         &["r", "f"]);
    m.insert("Result.flatMap",  &["r", "f"]);
    m.insert("mapErr",          &["r", "f"]);
    m.insert("Result.mapErr",   &["r", "f"]);
    m.insert("recover",         &["r", "f"]);
    m.insert("Result.recover",  &["r", "f"]);

    // Map
    m.insert("Map.insert",      &["map", "key", "value"]);
    m.insert("Map.get",         &["map", "key"]);
    m.insert("Map.contains",    &["map", "key"]);
    m.insert("Map.remove",      &["map", "key"]);

    // Text
    m.insert("Text.concat",     &["a", "b"]);
    m.insert("Text.contains",   &["text", "sub"]);
    m.insert("Text.startsWith", &["text", "prefix"]);
    m.insert("Text.endsWith",   &["text", "suffix"]);
    m.insert("Text.slice",      &["text", "from", "to"]);
    m.insert("Text.indexOf",    &["text", "sub"]);
    m.insert("Text.replace",    &["text", "from", "to"]);
    m.insert("Text.split",      &["text", "sep"]);
    m.insert("Text.join",       &["parts", "sep"]);
    m.insert("Text.repeat",     &["text", "n"]);

    // DateTime
    m.insert("DateTime.format",      &["dt", "fmt"]);
    m.insert("DateTime.addSeconds",  &["dt", "secs"]);
    m.insert("DateTime.addMinutes",  &["dt", "mins"]);
    m.insert("DateTime.addHours",    &["dt", "hours"]);
    m.insert("DateTime.addDays",     &["dt", "days"]);
    m.insert("DateTime.diffSeconds", &["a", "b"]);
    m.insert("DateTime.diffDays",    &["a", "b"]);
    m.insert("DateTime.before",      &["a", "b"]);
    m.insert("DateTime.after",       &["a", "b"]);
    m.insert("Date.format",          &["date", "fmt"]);

    // Decimal / Money
    m.insert("Decimal.add",    &["a", "b"]);
    m.insert("Decimal.sub",    &["a", "b"]);
    m.insert("Decimal.mul",    &["a", "b"]);
    m.insert("Decimal.div",    &["a", "b"]);
    m.insert("Decimal.round",  &["d", "places"]);

    // File / Path / IO
    m.insert("writeFile",      &["path", "content"]);
    m.insert("appendFile",     &["path", "content"]);
    m.insert("Path.join",      &["base", "part"]);

    // Process
    m.insert("Process.exec",   &["cmd", "args"]);

    // Json
    m.insert("JsonValue.at",   &["value", "index"]);
    m.insert("JsonValue.get",  &["value", "key"]);
    m.insert("JsonValue.push", &["array", "item"]);
    m.insert("JsonValue.set",  &["obj", "key", "value"]);

    m
}

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

pub fn lower_module(module: &Module) -> Result<HirModule, Vec<LowerError>> {
    let mut cx = Cx::new();

    // Register all top-level fn names first (for mutual recursion).
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            let id = cx.fresh_fn();
            cx.globals.insert(f.name.node.clone(), id);
            cx.fn_params.insert(f.name.node.clone(), f.params.clone());
            let tp_names: Vec<&str> = f.type_params.iter().map(|tp| tp.name.node.as_str()).collect();
            // Declared param types (BACKLOG item 120) — needed regardless of
            // whether a return-type annotation is present, unlike
            // `fn_ret_types` below, so this isn't nested inside `if let Some(ret)`.
            cx.fn_param_tys.insert(f.name.node.clone(),
                f.params.iter().map(|p| ast_ty_to_ty_with_params(&p.ty.node, &tp_names)).collect());
            if let Some(ret) = &f.ret_ty {
                cx.fn_ret_types.insert(f.name.node.clone(), ast_ty_to_ty_with_params(&ret.node, &tp_names));
            }
        }
        // Impl methods register as qualified globals `Type.method`.
        if let Decl::Impl(i) = &sdecl.node {
            let type_name = i.type_path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            for m in &i.methods {
                if m.body.is_none() { continue; } // no body → nothing to lower
                let qname = format!("{}.{}", type_name, m.name.node);
                let id = cx.fresh_fn();
                cx.globals.insert(qname.clone(), id);
                cx.fn_params.insert(qname.clone(), m.params.clone());
                // Same `i.type_params.chain(m.type_params)` fix as the actual
                // lowering pass below — this pre-registration loop is what
                // populates `fn_ret_types`/`fn_param_tys` for *callers* (e.g.
                // `Secret.expose`'s signature as seen from `main`), so it
                // needs the impl block's own `<T>` in scope too, or a
                // caller's bound local gets the literal (and undeclared)
                // C type `T` instead of `Ty::Var(0)`/`void*` — BACKLOG item 119.
                let tp_names: Vec<&str> = i.type_params.iter().chain(m.type_params.iter())
                    .map(|tp| tp.name.node.as_str()).collect();
                cx.fn_param_tys.insert(qname.clone(),
                    m.params.iter().map(|p| ast_ty_to_ty_with_params(&p.ty.node, &tp_names)).collect());
                if let Some(ret) = &m.ret_ty {
                    cx.fn_ret_types.insert(qname, ast_ty_to_ty_with_params(&ret.node, &tp_names));
                }
            }
        }
        // Register record field names for spread desugar, and declared field
        // types (item 119's generic-erasure boxing — see `record_field_types` doc).
        if let Decl::Type(t) = &sdecl.node {
            if let certo_ast::decl::TypeBody::Record(rec) = &t.body {
                let names: Vec<String> = rec.fields.iter().map(|f| f.name.node.clone()).collect();
                cx.record_field_names.insert(t.name.node.clone(), names);
                let tp_names: Vec<&str> = t.type_params.iter().map(|tp| tp.name.node.as_str()).collect();
                let field_types: Vec<Ty> = rec.fields.iter()
                    .map(|f| ast_ty_to_ty_with_params(&f.ty.node, &tp_names))
                    .collect();
                cx.record_field_types.insert(t.name.node.clone(), field_types);
                if !rec.computed.is_empty() {
                    let computed_names: Vec<String> = rec.computed.iter().map(|c| c.name.node.clone()).collect();
                    cx.computed_field_names.insert(t.name.node.clone(), computed_names);
                }
            }
        }
        // Register sum variant constructors and unit values.
        if let Decl::Type(t) = &sdecl.node {
            if let certo_ast::decl::TypeBody::Sum(variants) = &t.body {
                let parent_ty = Ty::Named { name: t.name.node.clone(), args: vec![] };
                for v in variants {
                    cx.variant_to_type.insert(v.name.node.clone(), t.name.node.clone());
                    if v.fields.is_empty() {
                        cx.global_types.insert(v.name.node.clone(), parent_ty.clone());
                    } else {
                        cx.fn_ret_types.insert(v.name.node.clone(), parent_ty.clone());
                        let field_names: Vec<String> = v.fields.iter().enumerate()
                            .map(|(i, f)| f.name.as_ref().map(|n| n.node.clone()).unwrap_or_else(|| format!("f{i}")))
                            .collect();
                        cx.variant_field_names.insert(v.name.node.clone(), field_names);
                        let tp_names: Vec<&str> = t.type_params.iter().map(|tp| tp.name.node.as_str()).collect();
                        let field_types: Vec<Ty> = v.fields.iter()
                            .map(|f| ast_ty_to_ty_with_params(&f.ty.node, &tp_names))
                            .collect();
                        cx.variant_field_types.insert(v.name.node.clone(), field_types);
                    }
                }
            }
        }
    }

    // Pre-register statemachine-generated function return types so HIR call
    // expressions get the correct type (used by MIR for C codegen).
    for sdecl in &module.decls {
        if let Decl::StateMachine(sm) = &sdecl.node {
            let machine_ty = Ty::Named { name: sm.name.node.clone(), args: vec![] };
            let state_ty   = Ty::Named { name: format!("{}State", sm.name.node), args: vec![] };
            cx.sm_returns.insert(format!("{}_new",   sm.name.node), machine_ty.clone());
            cx.sm_returns.insert(format!("{}_state", sm.name.node), state_ty);
            for t in &sm.transitions {
                cx.sm_returns.insert(format!("{}_{}", sm.name.node, t.event.node), machine_ty.clone());
            }
            for state in &sm.states {
                cx.sm_returns.insert(format!("{}_is{}", sm.name.node, state.node), Ty::Bool);
            }
        }
    }

    let mut items = Vec::new();
    for sdecl in &module.decls {
        match &sdecl.node {
            Decl::Fn(f) => {
                // `extern "C"` declarations have no Certo body — codegen emits a
                // prototype and the definition is linked from a native library.
                if f.is_extern { continue; }
                let id = cx.globals[&f.name.node];
                cx.push_scope();

                let tp_names: Vec<&str> = f.type_params.iter()
                    .map(|tp| tp.name.node.as_str())
                    .collect();

                let params: Vec<HirParam> = f.params.iter().map(|p| {
                    let local = cx.define_local(&p.name.node);
                    let ty = ast_ty_to_ty_with_params(&p.ty.node, &tp_names);
                    if !matches!(ty, Ty::Error) { cx.local_types.insert(local, ty.clone()); }
                    HirParam { local, name: p.name.node.clone(), ty, span: p.span }
                }).collect();

                let mut body = f.body.as_ref().map(|b| lower_expr(b, &mut cx));
                let ret_ty = f.ret_ty.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &tp_names)).unwrap_or(Ty::Error);
                // BACKLOG item 235 — `fn f(): Int8 = 100`'s bare-literal
                // body: MIR derives a function's *real* C return type from
                // the body's own computed operand type (`infer_operand_ty`,
                // `crates/mir/src/lower.rs`), not `HirFn.ret_ty` below, so
                // the literal's own `Ty::Int` default must be corrected here
                // or the C function is still emitted returning `int64_t`.
                if let (Some(b), Some(f_body)) = (&mut body, &f.body) {
                    if literal_matches_fixed_width(&f_body.node, &ret_ty) { b.ty = ret_ty.clone(); }
                }

                cx.pop_scope();

                items.push(HirItem::Fn(HirFn {
                    id,
                    name:   f.name.node.clone(),
                    params,
                    ret_ty,
                    body,
                    span:   f.span,
                }));
            }

            Decl::Val(v) => {
                let value = lower_expr(&v.value, &mut cx);
                let name = match &v.pattern.node {
                    Pattern::Ident { name, .. } => name.node.clone(),
                    _ => "<pattern>".to_string(),
                };
                items.push(HirItem::Const(HirConst {
                    name,
                    ty:    Ty::Error,
                    value,
                    span:  v.span,
                }));
            }

            // Impl methods lower to top-level functions named `Type.method`.
            Decl::Impl(i) => {
                let type_name = i.type_path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
                for m in &i.methods {
                    let Some(body_ast) = &m.body else { continue };
                    let qname = format!("{}.{}", type_name, m.name.node);
                    let Some(&id) = cx.globals.get(&qname) else { continue };
                    cx.push_scope();
                    // Both the impl block's own `<T>` and the method's `<U>`
                    // are in scope — mirrors typeck's identical fix (item 115)
                    // for the same gap; HIR has its own separate lowering pass
                    // that never got it. Without `i.type_params` here, `T`
                    // inside `impl<T> Secret { fn wrap(v: T): Secret<T> = ... }`
                    // doesn't resolve to `Ty::Var(0)` and falls through to a
                    // literal (and undeclared) C type name `T` — BACKLOG item 119.
                    let tp_names: Vec<&str> = i.type_params.iter().chain(m.type_params.iter())
                        .map(|tp| tp.name.node.as_str()).collect();
                    // Unlike the top-level `Decl::Fn` case above, this path
                    // previously never inserted its own params into
                    // `cx.local_types` — so a *reference* to an impl method's
                    // own param inside its own body (e.g. `v` in
                    // `impl<T> Box { fn wrap(v: T): Box<T> = Wrap(v) }`)
                    // always fell back to `Ty::Error`, not the real `Ty::Var(0)`
                    // this method's own `HirParam.ty` (below) already
                    // correctly computes — found while implementing BACKLOG
                    // item 120 (needed to distinguish "already-opaque
                    // argument, don't re-box" from "concrete argument, box
                    // it", which requires this type to be right).
                    let params: Vec<HirParam> = m.params.iter().map(|p| {
                        let local = cx.define_local(&p.name.node);
                        let ty = ast_ty_to_ty_with_params(&p.ty.node, &tp_names);
                        if !matches!(ty, Ty::Error) { cx.local_types.insert(local, ty.clone()); }
                        HirParam { local, name: p.name.node.clone(), ty, span: p.span }
                    }).collect();
                    let mut body = Some(lower_expr(body_ast, &mut cx));
                    let ret_ty = m.ret_ty.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &tp_names)).unwrap_or(Ty::Error);
                    // BACKLOG item 235 — same fixed-width literal-body
                    // correction as the top-level `Decl::Fn` case above.
                    if let Some(b) = &mut body {
                        if literal_matches_fixed_width(&body_ast.node, &ret_ty) { b.ty = ret_ty.clone(); }
                    }
                    cx.pop_scope();
                    items.push(HirItem::Fn(HirFn {
                        id,
                        name:   qname,
                        params,
                        ret_ty,
                        body,
                        span:   m.span,
                    }));
                }
            }

            _ => {} // type decls, migrations, on_enter hooks, etc. don't lower to HIR items
        }
    }

    let name = module.path.segments.last()
        .map(|s| s.node.clone())
        .unwrap_or_default();

    if cx.errors.is_empty() {
        Ok(HirModule {
            name, items,
            record_field_types:  cx.record_field_types,
            variant_field_types: cx.variant_field_types,
            fn_param_tys:        cx.fn_param_tys,
            fn_ret_tys:          cx.fn_ret_types,
        })
    } else {
        Err(cx.errors)
    }
}

/// `db.<table>.<method>(args)` (BACKLOG item 226) — see the matching
/// doc comment on typeck's own `try_db_accessor_call`
/// (`crates/typeck/src/infer_expr.rs`) for the full design rationale;
/// this is the HIR-side mirror, needed because HIR never reuses typeck's
/// own resolution (the same "independent lowering pass" tradeoff the
/// adjacent UFCS-rewrite comment already documents). Builds a brand-new,
/// ordinary `Expr::App` calling the real generated function name directly
/// (`{table}FindById`/`{table}FindAll`/`{table}DeleteById`), with a call to
/// the new zero-arg builtin `__certo_db_conn()` spliced in as the leading
/// argument — the caller re-lowers this synthesized node with `lower_expr`,
/// reusing all of the existing call-lowering machinery unchanged.
fn try_db_accessor_rewrite(func: &S<Expr>, args: &[certo_ast::expr::Arg], cx: &Cx) -> Option<S<Expr>> {
    let Expr::Field { expr: mid, field: method, .. } = &func.node else { return None };
    let Expr::Field { expr: inner, field: table, .. } = &mid.node else { return None };
    let Expr::Path { path, .. } = &inner.node else { return None };
    if path.segments.len() != 1 || path.segments[0].node != "db" { return None; }
    if cx.lookup_local("db").is_some() { return None; } // shadowed by a real local

    let suffix = match method.node.as_str() {
        "find"   => "FindById",
        "all"    => "FindAll",
        "delete" => "DeleteById",
        _ => return None, // typeck already rejected this with E0711
    };
    let fn_name = format!("{}{}", table.node, suffix);
    let span = func.span;
    let path_expr = |name: &str| S::new(
        Expr::Path {
            path: certo_ast::types::ModulePath { segments: vec![S::new(name.to_string(), span)], span },
            span,
        },
        span,
    );
    let conn_call = certo_ast::expr::Arg {
        label: None,
        value: S::new(Expr::App { func: Box::new(path_expr("__certo_db_conn")), args: vec![], span }, span),
        span,
    };
    let mut new_args = vec![conn_call];
    new_args.extend(args.iter().cloned());
    Some(S::new(Expr::App { func: Box::new(path_expr(&fn_name)), args: new_args, span }, span))
}

// ------------------------------------------------------------------ //
// Expression lowering
// ------------------------------------------------------------------ //

fn lower_expr(expr: &S<Expr>, cx: &mut Cx) -> HirExpr {
    let span = expr.span;
    match &expr.node {
        // Desugar f-string to ++ chain before generic lit handling
        Expr::Lit { value: Lit::FString(parts), .. } => {
            let segments: Vec<HirExpr> = parts.iter().map(|p| match p {
                FStringPart::Literal(s) => HirExpr {
                    kind: HirExprKind::Str(s.clone()), ty: Ty::Text, span,
                },
                FStringPart::Interpolated(e) => lower_expr(e, cx),
            }).collect();
            if segments.is_empty() {
                return HirExpr { kind: HirExprKind::Str(String::new()), ty: Ty::Text, span };
            }
            // Always seed with an empty Text literal and fold *every* segment
            // (not just segments after the first) through a Concat node, even
            // when there's only one — `f"{n}"` (bare interpolation, no
            // surrounding literal text) parses to a single-element `parts`
            // list. Returning that one segment directly would skip Concat
            // entirely, so a non-Text interpolated value (e.g. `n: Int`)
            // would keep its original type and reach codegen's
            // `certo_println`/`certo_text_concat` as a raw int, read as a
            // pointer and segfault — `coerce_to_text` (`crates/codegen/src/
            // emit_mir.rs`) only ever runs on a Concat operand.
            let seed = HirExpr { kind: HirExprKind::Str(String::new()), ty: Ty::Text, span };
            return segments.into_iter().fold(seed, |acc, seg| HirExpr {
                kind: HirExprKind::BinOp {
                    op:  BinOp::Concat,
                    lhs: Box::new(acc),
                    rhs: Box::new(seg),
                },
                ty: Ty::Text, span,
            });
        }

        Expr::Lit { value, .. } => lower_lit(value, span),

        Expr::Path { path, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            if let Some(local) = cx.lookup_local(name) {
                let ty = cx.local_types.get(&local).cloned().unwrap_or(Ty::Error);
                HirExpr { kind: HirExprKind::Local(local), ty, span }
            } else {
                // `global_types` itself is only ever populated for one narrow
                // val-destructuring case, so it misses a *bare* reference to
                // an ordinary top-level `fn` (as opposed to a call) — e.g.
                // passed as a named-function callback argument
                // (`dbQueryTyped(..., widgetsFromRow)`, BACKLOG item 134).
                // Reconstruct the real `Ty::Fn` from `fn_params`/
                // `fn_ret_types` instead, which — unlike `global_types` — ARE
                // populated up front for every top-level `fn` (see
                // `lower_module`). Doesn't handle generic functions (no
                // `type_params` are threaded through here), but a bare
                // function reference used as a callback is never generic in
                // practice.
                let ty = cx.global_types.get(name).cloned().unwrap_or_else(|| {
                    match (cx.fn_params.get(name), cx.fn_ret_types.get(name)) {
                        (Some(params), Some(ret)) => Ty::Fn {
                            params: params.iter().map(|p| ast_ty_to_ty_with_params(&p.ty.node, &[])).collect(),
                            ret: Box::new(ret.clone()),
                        },
                        _ => Ty::Error,
                    }
                });
                HirExpr { kind: HirExprKind::Global(name.to_string()), ty, span }
            }
        }

        Expr::Pipe { left, right, .. } => {
            let lhs = lower_expr(left, cx);
            match &right.node {
                // `a |> f(b, c)` → `f(a, b, c)`
                Expr::App { func, args, .. } => {
                    let func = lower_expr(func, cx);
                    let mut call_args = vec![lhs];
                    call_args.extend(args.iter().map(|a| lower_expr(&a.value, cx)));
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: call_args }, ty: Ty::Error, span }
                }
                // `a |> f` → `f(a)`
                _ => {
                    let func = lower_expr(right, cx);
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs] }, ty: Ty::Error, span }
                }
            }
        }

        Expr::App { func, args, .. } => {
            // `db.<table>.<method>(...)` (BACKLOG item 226) — mirrors
            // typeck's own `try_db_accessor_call`
            // (`crates/typeck/src/infer_expr.rs`) exactly, checked first for
            // the identical reason: it must never fall through to the
            // ordinary UFCS rewrite just below, whose own receiver-lowering
            // would otherwise try to lower `db.customers` as an ordinary
            // field access on an unbound `db`. By the time HIR lowering
            // runs, typeck has already validated the table/method
            // combination resolves to a real generated function (or
            // aborted with `E0711`), so this version needs no error path of
            // its own — only the same table/method → function-name mapping.
            if let Some(rewritten) = try_db_accessor_rewrite(func, args, cx) {
                return lower_expr(&rewritten, cx);
            }

            // Dot-call UFCS (BACKLOG item 162) — HIR does its own, entirely
            // independent lowering pass over the original AST (it never
            // reuses typeck's substitution), so it needs the identical
            // detect-and-rewrite this arm's typeck counterpart already does
            // (`crates/typeck/src/infer_expr.rs`'s `Expr::App` arm). When
            // `func` is `Expr::Field{expr, field}` and `expr` isn't an
            // uppercase module/type `Path` (the already-handled
            // `List.map(...)` form, detected the same way the plain
            // `Expr::Field` arm below does via `is_module_path`), lower the
            // receiver once just to read its type, and if a real function
            // named `"<TypeName>.<field>"` is registered anywhere HIR looks
            // up callees (stdlib param names, a user `impl` method, or the
            // full stdlib name table — `stdlib_fn_names()`, which unlike
            // `stdlib_ret_types()` also covers generic stdlib functions),
            // rewrite this call *locally* into a
            // synthetic `Expr::Field{Path(TypeName), field}` callee with the
            // receiver spliced in as a cloned, leading `Arg` — the exact
            // same AST shape a real `List.map(xs, f)` call already produces
            // — before any of the rest of this arm's logic (labeled-arg
            // reordering, default-param insertion, lambda-hinting) runs.
            // That logic is then unchanged and unaware anything special
            // happened. The receiver's first, type-probing lowering is
            // discarded, not reused, exactly like typeck's own version:
            // `lower_expr` only builds a tree (it never executes anything),
            // so the discarded pass can't double-evaluate a side effect at
            // runtime — the sole cost is a wasted, unused local id and a
            // possible duplicate diagnostic if the receiver itself contains
            // a nested lowering error, an accepted narrow blemish matching
            // typeck's own documented tradeoff for the identical rewrite.
            let ufcs_rewrite: Option<(S<Expr>, Vec<certo_ast::expr::Arg>)> =
                if let Expr::Field { expr, field, .. } = &func.node {
                    let is_module_path = match &expr.node {
                        Expr::Path { path, .. } => {
                            let first = path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
                            let is_upper = first.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                            let last = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                            is_upper && cx.lookup_local(last).is_none()
                        }
                        _ => false,
                    };
                    if is_module_path {
                        None
                    } else {
                        let base = lower_expr(expr, cx);
                        base.ty.qualifying_name().and_then(|type_name| {
                            let qualified = format!("{type_name}.{}", field.node);
                            let exists = cx.stdlib_params.contains_key(qualified.as_str())
                                || cx.fn_ret_types.contains_key(&qualified)
                                || stdlib_fn_names().contains(&qualified);
                            if exists {
                                let type_path = certo_ast::types::ModulePath {
                                    segments: vec![S::new(type_name, field.span)],
                                    span: field.span,
                                };
                                let type_expr = Box::new(S::new(Expr::Path { path: type_path, span: field.span }, field.span));
                                let synthetic_func = S::new(
                                    Expr::Field { expr: type_expr, field: field.clone(), span: field.span },
                                    field.span,
                                );
                                let receiver_arg = certo_ast::expr::Arg { label: None, value: (**expr).clone(), span: expr.span };
                                let mut new_args = vec![receiver_arg];
                                new_args.extend(args.iter().cloned());
                                Some((synthetic_func, new_args))
                            } else {
                                None
                            }
                        })
                    }
                } else {
                    None
                };
            let args_owned;
            let func_owned;
            let (func, args): (&S<Expr>, &[certo_ast::expr::Arg]) = match ufcs_rewrite {
                Some((f, a)) => { func_owned = f; args_owned = a; (&func_owned, &args_owned) }
                None => (func, args.as_slice()),
            };

            let func_hir = lower_expr(func, cx);

            // Extract the call target name(s) for param-reordering and return-type
            // lookups. `Module.fn(...)` lowers the callee to a `Global` with a
            // dotted name, so fall back to that when the AST func isn't a Path.
            let (fn_full_path, fn_short_name) = match &func.node {
                Expr::Path { path, .. } => {
                    let full = path.segments.iter()
                        .map(|s| s.node.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let short = path.segments.last().map(|s| s.node.clone());
                    (Some(full), short)
                }
                _ => match &func_hir.kind {
                    HirExprKind::Global(name) => {
                        let short = name.rsplit('.').next().map(|s| s.to_string());
                        (Some(name.clone()), short)
                    }
                    _ => (None, None),
                },
            };
            let has_labels = args.iter().any(|a| a.label.is_some());

            // Resolve param names: stdlib (by full path) takes priority, then user-defined (by short name).
            let stdlib_names: Option<&[&str]> = fn_full_path.as_deref()
                .and_then(|fp| cx.stdlib_params.get(fp).copied());
            let user_params: Option<Vec<FnParam>> = fn_short_name.as_ref()
                .and_then(|s| cx.fn_params.get(s).cloned());

            let mut lowered_args: Vec<HirExpr> = if let Some(snames) = stdlib_names {
                // List.map/groupBy's callback param is almost always
                // unannotated; hint its type from the already-lowered
                // scrutinee list's element type (BACKLOG item 113).
                // `sortBy`/`minBy`/`maxBy`/`sumBy`'s own key-projection
                // param needs the identical hint for the identical
                // reason (BACKLOG item 162b) — confirmed directly: an
                // unhinted `(x) => x` compiled to a bare `int64_t`
                // local instead of a real `certo_fn_t` closure inside
                // the per-call-site comparator `crates/mir/src/
                // lower.rs`'s `lower_sort_by_call` synthesizes,
                // producing "called object type 'int64_t' is not a
                // function" — this same-shaped `(List<T>, T=>...)`
                // call just never happened to have a lambda argument
                // *and* live outside `BOXED_ABI_CALLEES` before now.
                // `flatMap`/`recover`/`mapErr`'s callback param needs the
                // identical hint treatment (BACKLOG item 227), sourced from
                // the receiver's `Result<A,B>` instead of a list's element
                // type: `flatMap: A -> Result<C,B>` takes the success side
                // (`A`); `recover: B -> A` and `mapErr: B -> F` both take
                // the error side (`B`). Without this, `(x) => Ok(x)`/`(x) =>
                // if cond then Err(e) else Ok(x)`'s own bare `x` stayed
                // `Ty::Error` (the same hardcoded default `Expr::Lambda`'s
                // own generic arm gives every param with no hint) — the new
                // `Ok`/`Err` call-type recovery above could only ever
                // recover a real type from `x` when something *else*
                // (arithmetic, a field access) happened to reconstruct one
                // structurally; a bare, unmodified `x` had nothing to
                // recover from at all. Confirmed via a real segfault: `r.
                // flatMap((x) => if x > 3 then Err("too big") else Ok(x))`
                // crashed reading `x` back out of the `Ok` branch, even
                // with the `Ok`/`Err`-recovery and `merge_partial` fixes
                // above both already in place.
                let result_hint_side: Option<bool> = match fn_full_path.as_deref() {
                    Some("flatMap") | Some("Result.flatMap") => Some(true),
                    Some("recover") | Some("Result.recover")
                    | Some("mapErr") | Some("Result.mapErr") => Some(false),
                    _ => None,
                };
                let needs_lambda_hint = matches!(fn_full_path.as_deref(),
                    Some("List.map") | Some("List.groupBy")
                    | Some("List.sortBy") | Some("List.minBy") | Some("List.maxBy") | Some("List.sumBy")
                    | Some("List.upsert")) || result_hint_side.is_some();
                // Every one of the above takes its lambda as the 2nd
                // positional argument (index 1) — except `List.upsert`
                // (BACKLOG item 209), whose signature is `(list, item, on)`,
                // putting the key-projection lambda at index 2 instead.
                let lambda_pos: usize = if fn_full_path.as_deref() == Some("List.upsert") { 2 } else { 1 };
                // Stdlib function: only labeled reordering (no defaults).
                if has_labels {
                    // Determine each arg's target slot without lowering yet
                    // (mirrors the plain positional/labeled reordering below
                    // exactly, just deferring `lower_expr`), so that for a
                    // needs_lambda_hint function the "list" slot's arg can be
                    // lowered first regardless of written order — the hint
                    // below only ever worked when the lambda happened to be
                    // written in positional order (BACKLOG item 172: the
                    // labeled branch always lowered every arg via plain
                    // `lower_expr`, so a labeled call's inline lambda
                    // param/body silently stayed `Ty::Error`, same as before
                    // items 162b/170 fixed the positional-only case).
                    let mut slot_for_arg: Vec<Option<usize>> = vec![None; args.len()];
                    let mut taken = vec![false; snames.len()];
                    let mut pos_cursor = 0usize;
                    for (ai, arg) in args.iter().enumerate() {
                        let label_idx = arg.label.as_ref()
                            .and_then(|label| snames.iter().position(|&n| n == label.node.as_str()));
                        let idx = if let Some(idx) = label_idx {
                            Some(idx)
                        } else {
                            while pos_cursor < taken.len() && taken[pos_cursor] { pos_cursor += 1; }
                            if pos_cursor < taken.len() { let i = pos_cursor; pos_cursor += 1; Some(i) } else { None }
                        };
                        if let Some(i) = idx { taken[i] = true; }
                        slot_for_arg[ai] = idx;
                    }

                    let list_slot_idx = if needs_lambda_hint { snames.iter().position(|&n| n == "list") } else { None };
                    let lambda_slot_idx = if needs_lambda_hint {
                        snames.iter().position(|&n| n == "f" || n == "key" || n == "on")
                    } else { None };

                    let mut slots: Vec<Option<HirExpr>> = vec![None; snames.len()];
                    if let Some(list_idx) = list_slot_idx {
                        if let Some(ai) = slot_for_arg.iter().position(|s| *s == Some(list_idx)) {
                            slots[list_idx] = Some(lower_expr(&args[ai].value, cx));
                        }
                    }
                    let list_ty_hint: Ty = list_slot_idx
                        .and_then(|i| slots[i].as_ref())
                        .and_then(|e| match &e.ty { Ty::List(inner) => Some((**inner).clone()), _ => None })
                        .unwrap_or(Ty::Error);

                    for (ai, arg) in args.iter().enumerate() {
                        let Some(i) = slot_for_arg[ai] else { continue };
                        if slots[i].is_some() { continue; }
                        if Some(i) == lambda_slot_idx {
                            if let Expr::Lambda { params, body, .. } = &arg.value.node {
                                let lowered = lower_lambda_with_param_hint(params, body, &list_ty_hint, cx, arg.value.span);
                                // `sortBy`/`minBy`/`maxBy`/`sumBy`'s
                                // key/numeric projection type restriction
                                // (BACKLOG item 162b) is *mostly* enforced
                                // in typeck (E0710) — but typeck can't see
                                // through a field access on a struct
                                // element (`(p) => p.price`), since its own
                                // `resolve_field_ty` returns a permanently
                                // disconnected fresh var for field access
                                // on a still-unbound type, before this
                                // hint even exists. This lowered lambda's
                                // body, by contrast, *is* correctly typed
                                // here (the hint was just applied), so this
                                // is the first point a field-access key can
                                // actually be checked — confirmed directly:
                                // without this, `List.sortBy` on a struct
                                // list keyed by a Text field silently
                                // miscompiled instead of erroring.
                                let is_key_fn = matches!(fn_full_path.as_deref(),
                                    Some("List.sortBy") | Some("List.minBy") | Some("List.maxBy") | Some("List.sumBy"));
                                if is_key_fn {
                                    if let HirExprKind::Lambda { body, .. } = &lowered.kind {
                                        if !matches!(body.ty, Ty::Error) && !is_supported_key_ty(&body.ty) {
                                            cx.err(LowerErrorKind::Unsupported(format!(
                                                "`{}`'s key/numeric projection resolved to `{}`, which isn't \
                                                 supported — only Int/Int8/Int16/Int32/UInt/Float/Float32 are \
                                                 (Text's ordering isn't lexicographic here and Decimal has no \
                                                 generic comparison/addition yet)",
                                                fn_full_path.as_deref().unwrap_or(""), body.ty.display(),
                                            )), arg.value.span);
                                        }
                                    }
                                }
                                slots[i] = Some(lowered);
                                continue;
                            }
                        }
                        slots[i] = Some(lower_expr(&arg.value, cx));
                    }
                    slots.into_iter().map(|maybe| {
                        maybe.unwrap_or_else(|| HirExpr { kind: HirExprKind::Unit, ty: Ty::Error, span })
                    }).collect()
                } else {
                    let mut out: Vec<HirExpr> = Vec::with_capacity(args.len());
                    for (i, arg) in args.iter().enumerate() {
                        if needs_lambda_hint && i == lambda_pos {
                            if let Expr::Lambda { params, body, .. } = &arg.value.node {
                                let hint = if let Some(want_ok) = result_hint_side {
                                    out.first().and_then(|a: &HirExpr| match &a.ty {
                                        Ty::Result(ok, err) =>
                                            Some(if want_ok { (**ok).clone() } else { (**err).clone() }),
                                        _ => None,
                                    }).unwrap_or(Ty::Error)
                                } else {
                                    out.first().and_then(|a: &HirExpr| match &a.ty {
                                        Ty::List(inner) => Some((**inner).clone()),
                                        _ => None,
                                    }).unwrap_or(Ty::Error)
                                };
                                let lowered = lower_lambda_with_param_hint(params, body, &hint, cx, arg.value.span);
                                let is_key_fn = matches!(fn_full_path.as_deref(),
                                    Some("List.sortBy") | Some("List.minBy") | Some("List.maxBy") | Some("List.sumBy"));
                                if is_key_fn {
                                    if let HirExprKind::Lambda { body, .. } = &lowered.kind {
                                        if !matches!(body.ty, Ty::Error) && !is_supported_key_ty(&body.ty) {
                                            cx.err(LowerErrorKind::Unsupported(format!(
                                                "`{}`'s key/numeric projection resolved to `{}`, which isn't \
                                                 supported — only Int/Int8/Int16/Int32/UInt/Float/Float32 are \
                                                 (Text's ordering isn't lexicographic here and Decimal has no \
                                                 generic comparison/addition yet)",
                                                fn_full_path.as_deref().unwrap_or(""), body.ty.display(),
                                            )), arg.value.span);
                                        }
                                    }
                                }
                                out.push(lowered);
                                continue;
                            }
                        }
                        out.push(lower_expr(&arg.value, cx));
                    }
                    out
                }
            } else if let Some(params) = user_params {
                if has_labels || args.len() < params.len() {
                    // Normalize: reorder labeled args, insert defaults for missing
                    let mut slots: Vec<Option<HirExpr>> = vec![None; params.len()];
                    let mut pos_cursor = 0usize;
                    for arg in args {
                        let expr = lower_expr(&arg.value, cx);
                        if let Some(label) = &arg.label {
                            if let Some(idx) = params.iter().position(|p| p.name.node == label.node) {
                                slots[idx] = Some(expr);
                            } else {
                                while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                                if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                            }
                        } else {
                            while pos_cursor < slots.len() && slots[pos_cursor].is_some() { pos_cursor += 1; }
                            if pos_cursor < slots.len() { slots[pos_cursor] = Some(expr); pos_cursor += 1; }
                        }
                    }
                    slots.into_iter().enumerate().map(|(i, maybe)| {
                        maybe.unwrap_or_else(|| {
                            if let Some(default_expr) = &params[i].default {
                                lower_expr(default_expr, cx)
                            } else {
                                cx.err(LowerErrorKind::Unsupported(
                                    format!("missing required argument `{}`", params[i].name.node)
                                ), span);
                                HirExpr { kind: HirExprKind::Unit, ty: Ty::Error, span }
                            }
                        })
                    }).collect()
                } else {
                    // A lambda literal passed directly to a user-defined
                    // function's parameter must pick up that parameter's own
                    // declared type per-lambda-param (BACKLOG item 76's HKT
                    // consumer needs this: `unwrap: F<A> => A`'s own erased
                    // `Ty::Var(0)` param/return types are the only way a
                    // still-erased value flowing into a concrete call inside
                    // the lambda body — `Box.unwrap(x)` — has a declared
                    // type to trigger unboxing from at all). Falls back to
                    // ordinary unhinted lowering (`Ty::Error` params) exactly
                    // as before when the callee isn't a known function or
                    // its declared type at this position isn't a `Ty::Fn`.
                    let declared_params: Option<Vec<Ty>> = fn_full_path.as_deref()
                        .and_then(|fp| cx.fn_param_tys.get(fp)).cloned()
                        .or_else(|| fn_short_name.as_deref().and_then(|s| cx.fn_param_tys.get(s)).cloned());
                    args.iter().enumerate().map(|(i, a)| {
                        if let Expr::Lambda { params, body, .. } = &a.value.node {
                            if let Some(Ty::Fn { params: hints, ret }) = declared_params.as_ref().and_then(|dp| dp.get(i)) {
                                return lower_lambda_with_param_hints(params, body, hints, ret, cx, a.value.span);
                            }
                        }
                        lower_expr(&a.value, cx)
                    }).collect()
                }
            } else {
                args.iter().map(|a| lower_expr(&a.value, cx)).collect()
            };

            // Look up the return type: statemachine fns first, then user-defined fns.
            // `impl X { fn m(...) }` methods are registered in cx.fn_ret_types under
            // their qualified name ("X.m", set during hoisting below) — try that
            // full path before falling back to the short name (which is what a
            // plain top-level function's own unqualified name equals), otherwise
            // a `Type.method(...)` call's return type is silently never found and
            // defaults to Ty::Error.
            let short = fn_short_name.as_deref().unwrap_or("");
            // Same bare-`T`-return resolution as `val`'s (BACKLOG item
            // 135), applied at argument position: if an argument is itself
            // an unresolved generic call (`Ty::Var(0)`) and the callee's
            // declared param type at that position is concrete, resolve
            // (and mark for unboxing) using it; otherwise it's the same
            // hard error `val` gives, since nothing else here can tell what
            // concrete type it should be.
            let declared_params: Option<&Vec<Ty>> = fn_full_path.as_deref()
                .and_then(|fp| cx.fn_param_tys.get(fp))
                .or_else(|| cx.fn_param_tys.get(short))
                .or_else(|| fn_full_path.as_deref().and_then(|fp| stdlib_param_types().get(fp)))
                .or_else(|| stdlib_param_types().get(short));
            if let Some(declared) = declared_params {
                let declared = declared.clone();
                for (i, arg) in lowered_args.iter_mut().enumerate() {
                    let expected = declared.get(i).filter(|t| !matches!(t, Ty::Var(_)));
                    resolve_bare_generic_return(arg, expected, false, cx);
                }
            }
            let call_ty = fn_full_path.as_deref()
                .and_then(|fp| cx.sm_returns.get(fp).cloned())
                .or_else(|| fn_full_path.as_deref().and_then(|fp| cx.fn_ret_types.get(fp).cloned()))
                // BACKLOG item 224 — a validator's real generated function
                // is named with an underscore (`V_validate`, so the
                // generated Certo source parses as an ordinary function —
                // `crates/codegen/src/emit_validator.rs`'s own
                // `build_fn_sig`), but a call site's dot-qualified
                // reference (`V.validate(...)`) lowers to the *dotted*
                // global name (`"V.validate"`, `Expr::Field`'s own
                // `Module.fn` handling just above) — this table is a plain
                // `HashMap` with no dot/underscore normalization, so the
                // lookup above always misses even though `c_fn_name`
                // (`crates/codegen/src/emit_mir.rs`) already normalizes
                // both spellings to the identical C symbol, so the call
                // itself links and runs correctly — only the *return
                // type* silently stayed `Ty::Error`, confirmed directly: a
                // bare `V.validate(entity).isOk()` UFCS dot-call failed to
                // compile (`member reference base type 'int64_t'`) even
                // though `match V.validate(entity) { Ok(_) => ..., ... }`
                // on the identical value already worked (a `match`
                // scrutinee's type comes from a different path — see
                // `Expr::Match`'s own lowering — that never needed this
                // table at all).
                .or_else(|| fn_full_path.as_deref()
                    .and_then(|fp| cx.fn_ret_types.get(fp.replace('.', "_").as_str()).cloned()))
                .or_else(|| cx.fn_ret_types.get(short).cloned())
                .or_else(|| fn_full_path.as_deref().and_then(|fp| stdlib_ret_types().get(fp).cloned()))
                .or_else(|| stdlib_ret_types().get(short).cloned())
                .or_else(|| generic_container_ret(fn_full_path.as_deref(), &lowered_args))
                // Calling a *local* function value (a parameter/`val` typed
                // `Ty::Fn`, not a named global — e.g. a higher-kinded
                // function's own `wrap: B => F<B>` parameter called inside
                // its own body, BACKLOG item 76) has no name for any of the
                // lookups above to key on at all; fall back to the callee
                // expression's own already-known `Ty::Fn.ret` instead of
                // giving up to `Ty::Error`, which silently discarded the
                // call's real declared result type.
                .or_else(|| match &func_hir.ty {
                    Ty::Fn { ret, .. } => Some((**ret).clone()),
                    _ => None,
                })
                .unwrap_or(Ty::Error);
            // Recover a generic sum-type variant constructor's concrete
            // instantiation argument from the call's args (e.g. `Secret(42)`
            // ⇒ `Ty::Named{"Secret", args:[Ty::Int]}` instead of the plain,
            // always-empty-args lookup above) — BACKLOG item 119. Lets later
            // field access/pattern-matching substitute the type param back.
            let call_ty = recover_generic_variant_call_ty(short, &lowered_args, cx).unwrap_or(call_ty);

            HirExpr { kind: HirExprKind::Call { func: Box::new(func_hir), args: lowered_args }, ty: call_ty, span }
        }

        Expr::BinOp { op, left, right, .. } => {
            // Desugar range ops to stdlib calls; keep primitives as BinOp.
            match op {
                AstBinOp::RangeInclusive | AstBinOp::RangeExclusive => {
                    // BACKLOG item 231 — `range`/`rangeInclusive` (both
                    // registered in `crates/stdlib/src/seed.rs` as
                    // `(Int, Int) -> List<Int>`, never generic) always
                    // hardcoded `Ty::Error` here instead of the real,
                    // always-correct `List<Int>`. This call bypasses the
                    // general `fn_full_path`/`stdlib_ret_types()` lookup
                    // chain `Expr::App` uses (it's a separate, direct
                    // desugar, not a real call expression in the source),
                    // so nothing else ever recovered the real type either —
                    // confirmed via a real segfault: `for i in 1..5 {
                    // f"{i}" }` interpolated the loop variable's raw,
                    // untyped bits as if already `Text`.
                    let fn_name = if *op == AstBinOp::RangeInclusive { "range_inclusive" } else { "range" };
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    let func = HirExpr { kind: HirExprKind::Global(fn_name.into()), ty: Ty::Error, span };
                    let list_int = Ty::List(Box::new(Ty::Int));
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs, rhs] }, ty: list_int, span }
                }
                // `a in xs` — BACKLOG item 245. Desugars directly to the
                // already-real, already-working `List.contains(xs, a)`
                // (note the argument order: `List.contains`'s own signature
                // is `(list, item)`, the reverse of `in`'s own `item in
                // list` source order) — same "direct desugar, not a real
                // call expression, so hardcode the known result type"
                // treatment the range desugar just above uses, since this
                // bypasses `Expr::App`'s own call-type-recovery chain too.
                // `not in` needs no separate handling here — the parser
                // itself desugars it to `UnOp::Not` wrapping this same
                // `BinOp::In` node (`crates/parser/src/parse_expr.rs`).
                AstBinOp::In => {
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    let func = HirExpr { kind: HirExprKind::Global("List.contains".into()), ty: Ty::Error, span };
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![rhs, lhs] }, ty: Ty::Bool, span }
                }
                _ => {
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    let ty = binop_result_ty(&lower_binop(op), &lhs.ty, &rhs.ty);
                    HirExpr {
                        kind: HirExprKind::BinOp { op: lower_binop(op), lhs: Box::new(lhs), rhs: Box::new(rhs) },
                        ty,
                        span,
                    }
                }
            }
        }

        Expr::UnOp { op, expr, .. } => {
            let arg = lower_expr(expr, cx);
            // BACKLOG item 248 — this unconditionally hardcoded Ty::Error
            // regardless of the operand's own type, masked for the common
            // `-Int` case only because Ty::Error's own C codegen fallback
            // happens to coincide with Ty::Int's (int64_t either way); any
            // other operand type (Float, Decimal, a fixed-width int) got
            // silently miscompiled once typeck itself started accepting
            // them. Negation preserves the operand's type; `not` always
            // produces Bool.
            let (op, ty) = match op {
                AstUnOp::Neg => (UnOp::Neg, arg.ty.clone()),
                AstUnOp::Not => (UnOp::Not, Ty::Bool),
            };
            HirExpr { kind: HirExprKind::UnOp { op, arg: Box::new(arg) }, ty, span }
        }

        Expr::Field { expr, field, .. } => {
            // If the base is a module/type path (starts with an uppercase letter and
            // resolves to no local), treat `Module.fn` as a global function reference
            // rather than a struct field access.
            let is_module_path = match &expr.node {
                Expr::Path { path, .. } => {
                    let first = path.segments.first().map(|s| s.node.as_str()).unwrap_or("");
                    let is_upper = first.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                    let last = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
                    is_upper && cx.lookup_local(last).is_none()
                }
                _ => false,
            };
            if is_module_path {
                // Build a dotted name: "Text.indexOf"
                let base_name = match &expr.node {
                    Expr::Path { path, .. } => path.segments.iter()
                        .map(|s| s.node.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                    _ => unreachable!(),
                };
                let global_name = format!("{}.{}", base_name, field.node);
                HirExpr { kind: HirExprKind::Global(global_name), ty: Ty::Error, span }
            } else {
                let base = lower_expr(expr, cx);
                // `computed` properties (BACKLOG item 143) desugar to a
                // call to the synthesized accessor method
                // (`crates/parser/src/parse_decl.rs`'s
                // `synthesize_computed_method`, registered under
                // `Type.name` exactly like any other in-body `fn` method
                // — BACKLOG item 150) rather than a struct-field read.
                // Reuses the same `HirExprKind::Call`/`fn_ret_types`
                // machinery an ordinary `Type.method(x)` call already
                // uses — confirmed directly to already work correctly
                // end-to-end via that call form before this field-access
                // sugar was added, so no new return-type-unboxing logic
                // is needed here either (MIR's own `Expr::App` lowering
                // already handles a generic method's raw return the same
                // way for every other call).
                let is_computed = match &base.ty {
                    Ty::Named { name, .. } => cx.computed_field_names.get(name)
                        .map(|ns| ns.iter().any(|n| n == &field.node))
                        .unwrap_or(false),
                    _ => false,
                };
                if is_computed {
                    let type_name = match &base.ty { Ty::Named { name, .. } => name.clone(), _ => unreachable!() };
                    let qname = format!("{type_name}.{}", field.node);
                    let call_ty = cx.fn_ret_types.get(&qname).cloned().unwrap_or(Ty::Error);
                    let func = HirExpr { kind: HirExprKind::Global(qname), ty: Ty::Error, span };
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![base] }, ty: call_ty, span }
                } else {
                    let (field_ty, boxed) = resolve_field_ty(&base.ty, &field.node, cx);
                    HirExpr { kind: HirExprKind::Field { base: Box::new(base), field: field.node.clone(), boxed }, ty: field_ty, span }
                }
            }
        }

        // SafeField `e?.f` → `match e { Some(v) => Some(v.f), None => None }`.
        // The `Some` arm's pattern must be a real `Constructor` pattern, not
        // a bare `Bind` — a bare `Bind` is irrefutable (matches unconditionally
        // and binds the *whole* Option, per `Pattern::Ident`'s identical
        // lowering above), which made the `None` arm dead code and bound
        // `tmp` to the still-wrapped Option instead of its payload. Found
        // while fixing BACKLOG item 146's typeck half (the base's type was
        // never unwrapped before field resolution there either) — `?.`
        // could never previously reach this code at all on a real Optional,
        // since the typeck bug rejected it first, so this second, deeper
        // bug in the desugaring itself had never been exercised end-to-end.
        Expr::SafeField { expr, field, .. } => {
            let base = lower_expr(expr, cx);
            let inner_ty = match &base.ty {
                Ty::Option(inner) => (**inner).clone(),
                _ => Ty::Error,
            };
            let tmp = cx.fresh_local();
            if !matches!(inner_ty, Ty::Error) {
                cx.local_types.insert(tmp, inner_ty.clone());
            }
            let (field_ty, boxed) = resolve_field_ty(&inner_ty, &field.node, cx);
            let field_access = HirExpr {
                kind: HirExprKind::Field {
                    base:  Box::new(HirExpr { kind: HirExprKind::Local(tmp), ty: inner_ty.clone(), span }),
                    field: field.node.clone(),
                    boxed,
                },
                ty: field_ty.clone(), span,
            };
            let some_arm = HirArm {
                pat: HirPat::Constructor {
                    name: "Some".into(),
                    fields: vec![HirPat::Bind { local: tmp, name: "_safe_tmp".into() }],
                    field_names: vec!["f0".into()],
                    field_types: vec![inner_ty],
                },
                guard: None,
                body: HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr { kind: HirExprKind::Global("Some".into()), ty: Ty::Error, span }),
                        args: vec![field_access],
                    },
                    ty: Ty::Option(Box::new(field_ty.clone())), span,
                },
            };
            let none_arm = HirArm {
                pat:   HirPat::Constructor { name: "None".into(), fields: vec![], field_names: vec![], field_types: vec![] },
                guard: None,
                body:  HirExpr { kind: HirExprKind::Global("None".into()), ty: Ty::Option(Box::new(field_ty.clone())), span },
            };
            HirExpr {
                kind: HirExprKind::Match { scrutinee: Box::new(base), arms: vec![some_arm, none_arm] },
                ty: Ty::Option(Box::new(field_ty)), span,
            }
        }

        Expr::If { cond, then_expr, else_expr, .. } => {
            let cond = lower_expr(cond, cx);
            let then_ = lower_expr(then_expr, cx);
            let else_ = lower_expr(else_expr, cx);
            let ty = merge_partial(then_.ty.clone(), else_.ty.clone());
            HirExpr { kind: HirExprKind::If { cond: Box::new(cond), then_expr: Box::new(then_), else_expr: Box::new(else_) }, ty, span }
        }

        Expr::Match { scrutinee, arms, .. } => {
            let scrut = lower_expr(scrutinee, cx);
            let hir_arms: Vec<HirArm> = arms.iter().map(|arm| {
                cx.push_scope();
                let pat   = lower_pat(&arm.pattern, &scrut.ty, cx);
                let guard = arm.guard.as_ref().map(|g| lower_expr(g, cx));
                let body  = lower_expr(&arm.body, cx);
                cx.pop_scope();
                HirArm { pat, guard, body }
            }).collect();
            let ty = hir_arms.iter()
                .map(|a| a.body.ty.clone())
                .fold(Ty::Error, merge_partial);
            HirExpr { kind: HirExprKind::Match { scrutinee: Box::new(scrut), arms: hir_arms }, ty, span }
        }

        Expr::Block { stmts, .. } => lower_block(stmts, span, cx),

        Expr::Lambda { params, body, .. } => {
            let capture_threshold = cx.next_local;
            cx.push_scope();
            let hir_params: Vec<HirParam> = params.iter().map(|p| {
                let local = cx.define_local(&p.name.node);
                HirParam { local, name: p.name.node.clone(), ty: Ty::Error, span: p.span }
            }).collect();
            let body = lower_expr(body, cx);
            cx.pop_scope();
            let captures = collect_lambda_captures(&body, capture_threshold);
            HirExpr { kind: HirExprKind::Lambda { params: hir_params, body: Box::new(body), captures, ret_hint: Ty::Error }, ty: Ty::Error, span }
        }

        Expr::List { elements, .. } => {
            let elems: Vec<HirExpr> = elements.iter().map(|e| lower_expr(e, cx)).collect();
            // Element type from the first element (homogeneous), so iteration/reads
            // can unbox correctly (e.g. keep a Float's bits).
            let elem_ty = elems.first().map(|e| e.ty.clone()).unwrap_or(Ty::Error);
            HirExpr { kind: HirExprKind::List(elems), ty: Ty::List(Box::new(elem_ty)), span }
        }

        Expr::Tuple { elements, .. } => {
            let elems: Vec<HirExpr> = elements.iter().map(|e| lower_expr(e, cx)).collect();
            // Carry element types so destructuring bindings are typed (needed to
            // unbox e.g. a Float element with its bits intact).
            let ty = Ty::Tuple(elems.iter().map(|e| e.ty.clone()).collect());
            HirExpr { kind: HirExprKind::Tuple(elems), ty, span }
        }

        Expr::Record { ty_name, base, fields, .. } => {
            let record_ty = ty_name.as_deref()
                .map(|n| Ty::Named { name: n.to_string(), args: vec![] })
                .unwrap_or(Ty::Error);
            let explicit: Vec<(String, HirExpr)> = fields.iter()
                .map(|f| (f.name.node.clone(), lower_expr(&f.value, cx)))
                .collect();
            let hir_fields = if let Some(b) = base {
                // Spread: `TypeName { ..base, field: val }`, or `.with(...)`'s
                // desugar (BACKLOG item 151), which has no syntactic
                // `ty_name` at all — `base_hir.ty` (already resolved by
                // typeck before HIR lowering ever runs) is the fallback
                // source of the record's real name in that case.
                let base_hir = lower_expr(b, cx);
                // Stash the base expression in a fresh local so it's evaluated once.
                let base_local = cx.fresh_local();
                // We'll reference the base via HirExprKind::Local for each field access.
                let base_ty = if matches!(record_ty, Ty::Error) { base_hir.ty.clone() } else { record_ty.clone() };
                let base_ty_name: Option<&str> = ty_name.as_deref().or_else(|| match &base_ty {
                    Ty::Named { name, .. } => Some(name.as_str()),
                    _ => None,
                });
                let all_fields: Vec<String> = base_ty_name
                    .and_then(|n| cx.record_field_names.get(n).cloned())
                    .unwrap_or_else(|| explicit.iter().map(|(n, _)| n.clone()).collect());

                // Build a block: let _base = base_expr; TypeName { f1: _base.f1, ..overrides }
                let base_let = HirStmt::Let {
                    local: base_local,
                    name:  "_spread_base".into(),
                    ty:    base_ty.clone(),
                    init:  base_hir,
                };
                let merged: Vec<(String, HirExpr)> = all_fields.iter().map(|field_name| {
                    // Use explicit override if present, else read from base.
                    if let Some(pos) = explicit.iter().position(|(n, _)| n == field_name) {
                        (field_name.clone(), explicit[pos].1.clone())
                    } else {
                        let base_ref = HirExpr { kind: HirExprKind::Local(base_local), ty: base_ty.clone(), span };
                        // Base's own args aren't recovered here (a spread base's
                        // instantiation isn't tracked through the temp local) —
                        // best-effort: falls back to Ty::Error/unboxed for a
                        // spread-copied generic field. See BACKLOG item 119.
                        let (field_ty, boxed) = resolve_field_ty(&base_ty, field_name, cx);
                        let field_access = HirExpr {
                            kind: HirExprKind::Field { base: Box::new(base_ref), field: field_name.clone(), boxed },
                            ty: field_ty,
                            span,
                        };
                        (field_name.clone(), field_access)
                    }
                }).collect();
                let (record_ty, field_types) = recover_generic_record_ty(base_ty_name, &merged, cx);
                let record_expr = HirExpr { kind: HirExprKind::Record { fields: merged, field_types }, ty: record_ty.clone(), span };
                return HirExpr {
                    kind: HirExprKind::Block { stmts: vec![base_let], tail: Box::new(record_expr) },
                    ty: record_ty,
                    span,
                };
            } else {
                explicit
            };
            let (record_ty, field_types) = recover_generic_record_ty(ty_name.as_deref(), &hir_fields, cx);
            HirExpr { kind: HirExprKind::Record { fields: hir_fields, field_types }, ty: record_ty, span }
        }

        Expr::Try { expr, .. } => {
            let inner = lower_expr(expr, cx);
            // `e?`'s type is `e`'s Result Ok-payload type — needed so MIR's
            // unwrap knows whether to bit-restore a Float or dereference a
            // heap-boxed struct payload (see BACKLOG item 114); previously
            // always Ty::Error, silently truncating a Float here too.
            let ty = match &inner.ty {
                Ty::Result(t, _) => (**t).clone(),
                _ => Ty::Error,
            };
            HirExpr { kind: HirExprKind::Try(Box::new(inner)), ty, span }
        }

        Expr::Unsafe { body, .. } => {
            let inner = lower_expr(body, cx);
            HirExpr { kind: HirExprKind::Unsafe(Box::new(inner)), ty: Ty::Error, span }
        }

        // `await task` — join a spawned task.
        Expr::Await { expr, .. } => {
            // BACKLOG item 191 — this used to hardcode `Ty::Error` regardless
            // of `inner`'s own already-resolved type, discarding it exactly
            // like items 179/180/182 each found in a different spot. `await
            // <plain call>` (no preceding `spawn`) never changes the value's
            // type at all — MIR's own `Await` lowering confirms this,
            // falling back to the operand's own inferred type whenever it
            // isn't a real `__CertoTask` handle — so propagating `inner.ty`
            // directly is correct, not just a fallback. `await <a genuinely
            // spawned task>` is a separate, pre-existing, still-open gap:
            // `spawn`'s own HIR node is *also* unconditionally `Ty::Error`
            // (its own comment cites the same reason `parallel`'s tuple
            // elements are, since only the *task's own* type is knowable
            // this early, not its unwrapped-by-await result) and — per
            // `lower_block`'s `Stmt::Val` handling just below — a `Ty::Error`
            // initializer never even registers a local type, so `inner.ty`
            // here is already `Ty::Error` for that case regardless; this
            // fix can only ever improve on that, never regress it.
            let inner = lower_expr(expr, cx);
            let ty = inner.ty.clone();
            HirExpr { kind: HirExprKind::Await(Box::new(inner)), ty, span }
        }

        // `spawn expr` — run expr in a new task.
        // Lower `spawn f(a, b)` as a Call node wrapped in Spawn so MIR can emit the call,
        // with Spawn being transparent at HIR (the actual threading is done by codegen/runtime).
        Expr::Spawn { expr, .. } => {
            let capture_threshold = cx.next_local;
            let inner = lower_expr(expr, cx);
            let captures = collect_lambda_captures(&inner, capture_threshold);
            HirExpr { kind: HirExprKind::Spawn { fn_name: String::new(), args: vec![inner], captures }, ty: Ty::Error, span }
        }

        // `guard cond else e` → `if !cond { e }; unit`
        Expr::Guard { cond, else_expr, .. } => {
            let cond = lower_expr(cond, cx);
            let else_ = lower_expr(else_expr, cx);
            let not_cond = HirExpr { kind: HirExprKind::UnOp { op: UnOp::Not, arg: Box::new(cond) }, ty: Ty::Bool, span };
            let unit = HirExpr { kind: HirExprKind::Unit, ty: Ty::Unit, span };
            let if_expr = HirExpr {
                kind: HirExprKind::If { cond: Box::new(not_cond), then_expr: Box::new(else_), else_expr: Box::new(unit.clone()) },
                ty: Ty::Unit, span,
            };
            HirExpr { kind: HirExprKind::Block { stmts: vec![HirStmt::Expr(if_expr)], tail: Box::new(unit) }, ty: Ty::Unit, span }
        }

        // `require e (Err(..))` → `match e { Ok(v) => v, _ => return Err(..) }`
        // At HIR level we lower this to a Try on the expression.
        Expr::Require { expr, .. } => {
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Try(Box::new(inner)), ty: Ty::Error, span }
        }

        Expr::Parallel { tasks, timeout, .. } => {
            // parallel { a, b, c } — spawn each task then await all, yielding a tuple.
            // Lower as: { val t0 = spawn a; val t1 = spawn b; ...; (await t0, await t1, ...) }
            //
            // parallel(timeout: d) { ... } additionally computes a single
            // shared deadline (an absolute monotonic-clock millisecond value)
            // *before* spawning, and every task's join is a timed join
            // against that same deadline instead of an unbounded wait —
            // BACKLOG item 81. A prior sequential timed-join still enforces
            // the *overall* block deadline correctly: the remaining budget
            // for task N is `deadline - now()`, computed fresh at codegen
            // time for that join, so time spent waiting on earlier tasks is
            // correctly deducted rather than each task getting its own full
            // `timeout`.
            let mut stmts: Vec<HirStmt> = Vec::new();
            let deadline_local = timeout.as_ref()
                .map(|t| lower_deadline(t, "__parallel_deadline", &mut stmts, cx, span));
            let mut task_locals: Vec<LocalId> = Vec::new();
            for (i, task) in tasks.iter().enumerate() {
                let capture_threshold = cx.next_local;
                let spawn_inner = lower_expr(task, cx);
                let captures = collect_lambda_captures(&spawn_inner, capture_threshold);
                let spawn_expr = HirExpr {
                    kind: HirExprKind::Spawn { fn_name: format!("__parallel_task_{i}"), args: vec![spawn_inner], captures },
                    ty: Ty::Error, span,
                };
                let local = cx.fresh_local();
                stmts.push(HirStmt::Let { local, name: format!("__task_{i}"), ty: Ty::Error, init: spawn_expr });
                task_locals.push(local);
            }
            let awaited: Vec<HirExpr> = task_locals.iter().map(|&l| {
                let local_expr = HirExpr { kind: HirExprKind::Local(l), ty: Ty::Error, span };
                let kind = match deadline_local {
                    Some(d) => HirExprKind::AwaitTimed { task: Box::new(local_expr), deadline: d },
                    None    => HirExprKind::Await(Box::new(local_expr)),
                };
                HirExpr { kind, ty: Ty::Error, span }
            }).collect();
            let tail = HirExpr { kind: HirExprKind::Tuple(awaited), ty: Ty::Error, span };
            HirExpr { kind: HirExprKind::Block { stmts, tail: Box::new(tail) }, ty: Ty::Error, span }
        }

        Expr::WithTimeout { duration, body, .. } => {
            // withTimeout(d) { body } (BACKLOG item 122) — spawn `body` as a
            // background task (the identical machinery a single-task
            // `parallel` already uses), then join it with a cancel-capable
            // timed join instead of `parallel`'s own panic-on-timeout
            // `AwaitTimed`: `JoinTimedCancel` yields `Some(v)` if the task
            // finishes in time, `None` (task safely abandoned, never
            // panicking, never blocking the caller past the deadline) if
            // the deadline passes first.
            let mut stmts: Vec<HirStmt> = Vec::new();
            let deadline_local = lower_deadline(duration, "__with_timeout_deadline", &mut stmts, cx, span);

            let capture_threshold = cx.next_local;
            let spawn_inner = lower_expr(body, cx);
            // Captured *before* `spawn_inner` moves into `Spawn`'s args below —
            // `withTimeout`'s own result type is knowable right now (`body`'s
            // own type, Option-wrapped), unlike `parallel`'s `Ty::Error`
            // placeholder above (whose element types only ever needed to
            // survive as far as the immediately-following `Tuple`). Setting
            // this for real (not `Ty::Error`) matters here specifically
            // because `withTimeout`'s result commonly gets bound to a `val`
            // and pattern-matched afterward (`match result { Some(v) => ... }`)
            // — `Ty::Error` would skip `cx.local_types` registration entirely
            // (`lower_block`'s `Stmt::Val` handling only registers a non-Error
            // type), leaving `v`'s bound type unresolved and downstream code
            // (e.g. an f-string interpolating `v`) unable to tell it's an
            // `Int` that needs `certo_int_to_text`, not raw text — confirmed
            // by direct testing: it read the boxed `int64_t` as a `Text`
            // pointer and segfaulted.
            let body_ty = spawn_inner.ty.clone();
            let captures = collect_lambda_captures(&spawn_inner, capture_threshold);
            let spawn_expr = HirExpr {
                kind: HirExprKind::Spawn { fn_name: "__with_timeout_task".into(), args: vec![spawn_inner], captures },
                ty: Ty::Error, span,
            };
            let task_local = cx.fresh_local();
            stmts.push(HirStmt::Let { local: task_local, name: "__with_timeout_task".into(), ty: Ty::Error, init: spawn_expr });

            let task_ref = HirExpr { kind: HirExprKind::Local(task_local), ty: Ty::Error, span };
            let opt_ty = Ty::Option(Box::new(body_ty));
            let tail = HirExpr {
                kind: HirExprKind::JoinTimedCancel { task: Box::new(task_ref), deadline: deadline_local },
                ty: opt_ty.clone(), span,
            };
            HirExpr { kind: HirExprKind::Block { stmts, tail: Box::new(tail) }, ty: opt_ty, span }
        }

        Expr::Transaction { body, .. } => {
            // db.transaction { body } — lower as a call to __db_transaction(|| body).
            // No new scope is pushed here (the thunk has zero params of its own),
            // so *any* Local reference in `body` predates it — capture_threshold
            // is just "whatever next_local already is" going in.
            let capture_threshold = cx.next_local;
            let inner = lower_expr(body, cx);
            let captures = collect_lambda_captures(&inner, capture_threshold);
            let thunk = HirExpr { kind: HirExprKind::Lambda { params: vec![], body: Box::new(inner), captures, ret_hint: Ty::Error }, ty: Ty::Error, span };
            let func  = HirExpr { kind: HirExprKind::Global("__db_transaction".into()), ty: Ty::Error, span };
            HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![thunk] }, ty: Ty::Error, span }
        }

        Expr::Ascribe { expr, .. } => lower_expr(expr, cx),

        Expr::For { binding, iter, body, .. } => {
            let iter_hir = lower_expr(iter, cx);
            // BACKLOG item 231 — `binding_ty` was hardcoded `Ty::Error`
            // *and* the loop variable's own local type was never recorded
            // in `cx.local_types` before lowering the body — so every
            // reference to the loop variable *inside* the loop (`Expr::Path`
            // looks the local up in `cx.local_types`, defaulting to
            // `Ty::Error` when absent) saw the wrong type regardless of
            // what `iter`'s own type was. Recover the real element type
            // from `iter_hir.ty` (already correct once the range-desugar
            // arm above sets it properly) so both the loop variable's
            // in-body references and `binding_ty` (MIR's own fallback,
            // `crates/mir/src/lower.rs`) agree.
            let elem_ty = match &iter_hir.ty {
                Ty::List(inner) => (**inner).clone(),
                _ => Ty::Error,
            };
            cx.push_scope();
            let local = cx.define_local(&binding.node);
            if !matches!(elem_ty, Ty::Error) { cx.local_types.insert(local, elem_ty.clone()); }
            let body_hir = lower_expr(body, cx);
            cx.pop_scope();
            HirExpr {
                kind: HirExprKind::For {
                    binding:      local,
                    binding_name: binding.node.clone(),
                    binding_ty:   elem_ty,
                    iter:         Box::new(iter_hir),
                    body:         Box::new(body_hir),
                },
                ty: Ty::Unit,
                span,
            }
        }

        Expr::While { cond, body, .. } => {
            let cond_hir = lower_expr(cond, cx);
            let body_hir = lower_expr(body, cx);
            HirExpr {
                kind: HirExprKind::While {
                    cond: Box::new(cond_hir),
                    body: Box::new(body_hir),
                },
                ty: Ty::Unit,
                span,
            }
        }
        // `e.age` → `DateTime.diff(DateTime.now(), e)` — real elapsed-time
        // computation (BACKLOG item 164). The previous stub just lowered to
        // the inner expression unchanged — `t.age` compiled to literally
        // `t`, silently returning the raw Timestamp value mistyped as a
        // Duration, never caught before because `Timestamp`'s own codegen
        // mapping was separately broken and blocked anything using `.age`
        // from compiling at all. `Timestamp` has no separate runtime
        // representation from `DateTime` (identical C type, see
        // `crates/codegen/src/ty_to_c.rs`), so the base expression is
        // passed into `DateTime.diff` with no conversion needed.
        Expr::Age { expr, .. } => {
            let base = lower_expr(expr, cx);
            let duration_ty = Ty::Named { name: "Duration".into(), args: vec![] };
            let now_call = HirExpr {
                kind: HirExprKind::Call {
                    func: Box::new(HirExpr {
                        kind: HirExprKind::Global("DateTime.now".into()),
                        ty: Ty::Error, span,
                    }),
                    args: vec![],
                },
                ty: Ty::Named { name: "DateTime".into(), args: vec![] },
                span,
            };
            // BACKLOG item 246 — `e.age` for `e: Timestamp?` previously
            // desugared straight-line, passing the still-boxed/optional
            // value where `DateTime.diff` expects a raw `CertoDateTime`
            // struct — no `None` → `Duration.max` branch (the spec's own
            // documented behaviour) and no `Some` unwrap, producing garbage
            // for both cases at runtime. Mirrors `Expr::SafeField`'s own
            // `Some`/`None` match-desugar just above for the identical
            // "optional base, real Some-arm unwrap" shape.
            if let Ty::Option(inner) = &base.ty {
                let inner_ty = (**inner).clone();
                let tmp = cx.fresh_local();
                if !matches!(inner_ty, Ty::Error) {
                    cx.local_types.insert(tmp, inner_ty.clone());
                }
                let diff_call = HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr {
                            kind: HirExprKind::Global("DateTime.diff".into()),
                            ty: Ty::Error, span,
                        }),
                        args: vec![now_call, HirExpr { kind: HirExprKind::Local(tmp), ty: inner_ty.clone(), span }],
                    },
                    ty: duration_ty.clone(), span,
                };
                let some_arm = HirArm {
                    pat: HirPat::Constructor {
                        name: "Some".into(),
                        fields: vec![HirPat::Bind { local: tmp, name: "_age_tmp".into() }],
                        field_names: vec!["f0".into()],
                        field_types: vec![inner_ty],
                    },
                    guard: None,
                    body: diff_call,
                };
                let none_arm = HirArm {
                    pat: HirPat::Constructor { name: "None".into(), fields: vec![], field_names: vec![], field_types: vec![] },
                    guard: None,
                    // Duration's own C representation is a plain int64
                    // millisecond count (`CertoDuration`, `crates/stdlib/
                    // src/datetime.rs`) — the largest representable span,
                    // matching `Duration.max`'s own "max" semantics.
                    body: HirExpr { kind: HirExprKind::Int(i64::MAX), ty: duration_ty.clone(), span },
                };
                HirExpr {
                    kind: HirExprKind::Match { scrutinee: Box::new(base), arms: vec![some_arm, none_arm] },
                    ty: duration_ty, span,
                }
            } else {
                HirExpr {
                    kind: HirExprKind::Call {
                        func: Box::new(HirExpr {
                            kind: HirExprKind::Global("DateTime.diff".into()),
                            ty: Ty::Error, span,
                        }),
                        args: vec![now_call, base],
                    },
                    ty: duration_ty, span,
                }
            }
        }
        // BACKLOG item 165 — desugars to the same real `assert(cond: Bool,
        // msg: Text): Unit` primitive every hand-written test in this
        // codebase already uses, exactly the way `.age` above desugars to
        // real `DateTime.*` calls rather than inventing new runtime
        // machinery. `.toBeSome`/`.toBeNone`/`.toBeOk`/`.toBeErr` call the
        // small new `Option.isSome`/`Option.isNone`/`Result.isOk`/
        // `Result.isErr` predicates (BACKLOG item 165) rather than
        // synthesizing a `match` here, since those are real, independently
        // useful stdlib functions, not lowering-only plumbing.
        Expr::ExpectAssertion { actual, matcher, .. } => {
            let actual_hir = lower_expr(actual, cx);
            let (cond, msg): (HirExpr, &str) = match matcher {
                ExpectMatcher::ToBe(y) => {
                    let y_hir = lower_expr(y, cx);
                    (HirExpr {
                        kind: HirExprKind::BinOp { op: BinOp::Eq, lhs: Box::new(actual_hir), rhs: Box::new(y_hir) },
                        ty: Ty::Bool, span,
                    }, "expected values to be equal")
                }
                ExpectMatcher::ToBeTrue => (actual_hir, "expected true"),
                ExpectMatcher::ToBeFalse => (
                    HirExpr { kind: HirExprKind::UnOp { op: UnOp::Not, arg: Box::new(actual_hir) }, ty: Ty::Bool, span },
                    "expected false",
                ),
                ExpectMatcher::ToBeSome => (call_global1("Option.isSome", actual_hir, span), "expected Some"),
                ExpectMatcher::ToBeNone => (call_global1("Option.isNone", actual_hir, span), "expected None"),
                ExpectMatcher::ToBeOk   => (call_global1("Result.isOk",   actual_hir, span), "expected Ok"),
                ExpectMatcher::ToBeErr  => (call_global1("Result.isErr",  actual_hir, span), "expected Err"),
            };
            let msg_hir = HirExpr { kind: HirExprKind::Str(msg.into()), ty: Ty::Text, span };
            HirExpr {
                kind: HirExprKind::Call {
                    func: Box::new(HirExpr { kind: HirExprKind::Global("assert".into()), ty: Ty::Error, span }),
                    args: vec![cond, msg_hir],
                },
                ty: Ty::Unit,
                span,
            }
        }
    }
}

/// `name(arg)` as a `Bool`-returning HIR call — helper for the tag-check
/// `expect(...)` matchers (BACKLOG item 165), which all share this shape.
fn call_global1(name: &str, arg: HirExpr, span: Span) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(HirExpr { kind: HirExprKind::Global(name.into()), ty: Ty::Error, span }),
            args: vec![arg],
        },
        ty: Ty::Bool,
        span,
    }
}

fn lower_lit(lit: &Lit, span: Span) -> HirExpr {
    use certo_ast::expr::FStringPart;
    let (kind, ty) = match lit {
        Lit::Int(n)     => (HirExprKind::Int(*n),           Ty::Int),
        Lit::Float(f)   => (HirExprKind::Float(*f),         Ty::Float),
        Lit::Decimal(s) => (HirExprKind::Decimal(s.clone()), Ty::Decimal(None)),
        Lit::Bool(b)    => (HirExprKind::Bool(*b),          Ty::Bool),
        Lit::String(s)  => (HirExprKind::Str(s.clone()),    Ty::Text),
        Lit::FString(parts) => {
            // Parts with interpolations are desugared in lower_expr before reaching here.
            // If we still have interpolated parts, join literal segments as a fallback.
            let joined = parts.iter().filter_map(|p| {
                if let FStringPart::Literal(s) = p { Some(s.clone()) } else { None }
            }).collect::<Vec<_>>().join("");
            (HirExprKind::Str(joined), Ty::Text)
        }
        Lit::Uuid(u) => (HirExprKind::Uuid(u.clone()), Ty::Uuid),
        Lit::Unit    => (HirExprKind::Unit, Ty::Unit),
    };
    HirExpr { kind, ty, span }
}

fn lower_binop(op: &AstBinOp) -> BinOp {
    match op {
        AstBinOp::Add => BinOp::Add,
        AstBinOp::Sub => BinOp::Sub,
        AstBinOp::Mul => BinOp::Mul,
        AstBinOp::Div => BinOp::Div,
        AstBinOp::Rem => BinOp::Rem,
        AstBinOp::Pow => BinOp::Pow,
        AstBinOp::Eq  => BinOp::Eq,
        AstBinOp::NotEq => BinOp::NotEq,
        AstBinOp::Lt  => BinOp::Lt,
        AstBinOp::LtEq => BinOp::LtEq,
        AstBinOp::Gt  => BinOp::Gt,
        AstBinOp::GtEq => BinOp::GtEq,
        AstBinOp::And => BinOp::And,
        AstBinOp::Or  => BinOp::Or,
        AstBinOp::NullCoalesce => BinOp::NullCoalesce,
        AstBinOp::Concat       => BinOp::Concat,
        AstBinOp::RangeInclusive | AstBinOp::RangeExclusive => unreachable!("handled above"),
        AstBinOp::In => unreachable!("handled above"),
    }
}

fn lower_block(stmts: &[Stmt], span: Span, cx: &mut Cx) -> HirExpr {
    cx.push_scope();
    let mut hir_stmts = Vec::new();
    let mut tail: Option<HirExpr> = None;

    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i == stmts.len() - 1;
        match stmt {
            Stmt::Val { pattern, ty: val_ty_ann, value, .. } => {
                let mut init = lower_expr(value, cx);
                // A generic function's bare-`T` return collapses to the
                // unresolved `Ty::Var(0)` sentinel (see `ast_ty_to_ty_with_params`)
                // — if the `val` declares a concrete type, use it to resolve
                // (and mark for unboxing) the call's real return type;
                // otherwise this is exactly the case codegen would silently
                // mis-cast a raw `void*` as a concrete C type, so it's a
                // hard error instead — BACKLOG item 135.
                let declared = val_ty_ann.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &[]));
                resolve_bare_generic_return(&mut init, declared.as_ref(), true, cx);
                if let Some(d) = &declared {
                    if literal_matches_fixed_width(&value.node, d) { init.ty = d.clone(); }
                }
                match &pattern.node {
                    Pattern::Ident { name, .. } => {
                        let local = cx.define_local(&name.node);
                        let ty = init.ty.clone(); // propagate init type (e.g. statemachine return type)
                        // Remember the type so later references to this variable
                        // carry it (needed for `match q { Some(x) => … }`).
                        if !matches!(ty, Ty::Error) {
                            cx.local_types.insert(local, ty.clone());
                        }
                        hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty, init });
                    }
                    Pattern::Wildcard { .. } => {
                        hir_stmts.push(HirStmt::Expr(init));
                    }
                    Pattern::Tuple { elements, .. } => {
                        // Recover element types from the tuple's type so each
                        // binding (and its unbox) knows its real type.
                        let elem_tys: Vec<Ty> = match &init.ty {
                            Ty::Tuple(ts) => ts.clone(),
                            _ => vec![Ty::Error; elements.len()],
                        };
                        let tup_ty = init.ty.clone();
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_tup".into(), ty: tup_ty.clone(), init });
                        for (i, elem) in elements.iter().enumerate() {
                            if let Pattern::Ident { name, .. } = &elem.node {
                                let elem_ty = elem_tys.get(i).cloned().unwrap_or(Ty::Error);
                                let local = cx.define_local(&name.node);
                                let base = HirExpr { kind: HirExprKind::Local(tmp), ty: tup_ty.clone(), span };
                                let field_expr = HirExpr {
                                    kind: HirExprKind::Field { base: Box::new(base), field: i.to_string(), boxed: false },
                                    ty: elem_ty.clone(), span,
                                };
                                if !matches!(elem_ty, Ty::Error) {
                                    cx.local_types.insert(local, elem_ty.clone());
                                }
                                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: elem_ty, init: field_expr });
                            }
                        }
                    }
                    Pattern::Record { fields, .. } => {
                        // Field types come from the record's own declared
                        // shape via `resolve_field_ty` — the same lookup an
                        // ordinary `expr.field` read already uses — rather
                        // than the `Ty::Error` this used to hardcode, which
                        // silently truncated any boxed field on read (BACKLOG
                        // item 145: confirmed via a real repro that a `Float`
                        // field destructured this way came back as its raw
                        // bit pattern reinterpreted as `Int` — `19.99`
                        // printed as `19`).
                        let rec_ty = init.ty.clone();
                        let tmp = cx.fresh_local();
                        if !matches!(rec_ty, Ty::Error) { cx.local_types.insert(tmp, rec_ty.clone()); }
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_rec".into(), ty: rec_ty.clone(), init });
                        for pf in fields {
                            let binding_name = if let Some(sub) = &pf.pattern {
                                if let Pattern::Ident { name, .. } = &sub.node { name.node.clone() } else { continue }
                            } else {
                                pf.name.node.clone()
                            };
                            let (field_ty, boxed) = resolve_field_ty(&rec_ty, &pf.name.node, cx);
                            let local = cx.define_local(&binding_name);
                            if !matches!(field_ty, Ty::Error) { cx.local_types.insert(local, field_ty.clone()); }
                            let base = HirExpr { kind: HirExprKind::Local(tmp), ty: rec_ty.clone(), span };
                            let field_expr = HirExpr {
                                kind: HirExprKind::Field { base: Box::new(base), field: pf.name.node.clone(), boxed },
                                ty: field_ty.clone(), span,
                            };
                            hir_stmts.push(HirStmt::Let { local, name: binding_name, ty: field_ty, init: field_expr });
                        }
                    }
                    Pattern::List { head, tail, .. } => {
                        // BACKLOG item 195 — `val [head, ...tail] = items`.
                        // Mirrors the `Tuple` arm above: bind a temp to the
                        // whole list, then each `head` element via
                        // `List.getOrPanic` (a real runtime panic if the list
                        // is shorter than `head.len()` — `val` destructuring
                        // is irrefutable, so there's no fallback arm to fall
                        // through to the way a `match` arm has one), and
                        // `tail` (if bound) via `List.slice(list, head.len(),
                        // List.len(list))` — the same `List<T>` type as the
                        // whole value, not a smaller one.
                        let elem_ty = match &init.ty {
                            Ty::List(t) => (**t).clone(),
                            _ => Ty::Error,
                        };
                        let list_ty = init.ty.clone();
                        let tmp = cx.fresh_local();
                        if !matches!(list_ty, Ty::Error) { cx.local_types.insert(tmp, list_ty.clone()); }
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_lst".into(), ty: list_ty.clone(), init });

                        for (i, h) in head.iter().enumerate() {
                            if let Pattern::Ident { name, .. } = &h.node {
                                let local = cx.define_local(&name.node);
                                if !matches!(elem_ty, Ty::Error) { cx.local_types.insert(local, elem_ty.clone()); }
                                let list_ref = HirExpr { kind: HirExprKind::Local(tmp), ty: list_ty.clone(), span };
                                let idx = HirExpr { kind: HirExprKind::Int(i as i64), ty: Ty::Int, span };
                                let get_expr = HirExpr {
                                    kind: HirExprKind::Call {
                                        func: Box::new(HirExpr { kind: HirExprKind::Global("List.getOrPanic".into()), ty: Ty::Error, span }),
                                        args: vec![list_ref, idx],
                                    },
                                    ty: elem_ty.clone(), span,
                                };
                                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: elem_ty.clone(), init: get_expr });
                            }
                        }

                        if let Some(t) = tail {
                            if let Pattern::Ident { name, .. } = &t.node {
                                let local = cx.define_local(&name.node);
                                if !matches!(list_ty, Ty::Error) { cx.local_types.insert(local, list_ty.clone()); }
                                let list_ref_for_len = HirExpr { kind: HirExprKind::Local(tmp), ty: list_ty.clone(), span };
                                let len_call = HirExpr {
                                    kind: HirExprKind::Call {
                                        func: Box::new(HirExpr { kind: HirExprKind::Global("List.len".into()), ty: Ty::Error, span }),
                                        args: vec![list_ref_for_len],
                                    },
                                    ty: Ty::Int, span,
                                };
                                let list_ref = HirExpr { kind: HirExprKind::Local(tmp), ty: list_ty.clone(), span };
                                let from = HirExpr { kind: HirExprKind::Int(head.len() as i64), ty: Ty::Int, span };
                                let slice_expr = HirExpr {
                                    kind: HirExprKind::Call {
                                        func: Box::new(HirExpr { kind: HirExprKind::Global("List.slice".into()), ty: Ty::Error, span }),
                                        args: vec![list_ref, from, len_call],
                                    },
                                    ty: list_ty.clone(), span,
                                };
                                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty: list_ty.clone(), init: slice_expr });
                            }
                        }
                    }
                    _ => {
                        // Complex patterns: lower to match + let
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_pat".into(), ty: Ty::Error, init });
                    }
                }
            }
            Stmt::Var { name, ty: var_ty_ann, value, .. } => {
                let mut init = lower_expr(value, cx);
                // BACKLOG item 235 — mirrors `Stmt::Val`'s own fixed-width
                // literal override just above (`var`'s declared annotation
                // was previously discarded entirely here, unlike `Stmt::Val`).
                if let Some(t) = var_ty_ann {
                    let declared = ast_ty_to_ty_with_params(&t.node, &[]);
                    if literal_matches_fixed_width(&value.node, &declared) { init.ty = declared; }
                }
                let ty = init.ty.clone();
                let local = cx.define_local(&name.node);
                hir_stmts.push(HirStmt::Let { local, name: name.node.clone(), ty, init });
            }
            Stmt::Assign { target, value, .. } => {
                let v = lower_expr(value, cx);
                if let Some(local) = cx.lookup_local(&target.node) {
                    hir_stmts.push(HirStmt::Assign { local, value: v });
                } else {
                    cx.err(LowerErrorKind::UnresolvedName(target.node.clone()), target.span);
                }
            }
            Stmt::Defer { body, .. } => {
                let e = lower_expr(body, cx);
                hir_stmts.push(HirStmt::Defer { body: e });
            }
            Stmt::Expr { expr, .. } => {
                let e = lower_expr(expr, cx);
                if is_last {
                    tail = Some(e);
                } else {
                    hir_stmts.push(HirStmt::Expr(e));
                }
            }
        }
    }

    cx.pop_scope();

    let tail = tail.unwrap_or(HirExpr { kind: HirExprKind::Unit, ty: Ty::Unit, span });
    if hir_stmts.is_empty() {
        tail
    } else {
        HirExpr { kind: HirExprKind::Block { stmts: hir_stmts, tail: Box::new(tail) }, ty: Ty::Error, span }
    }
}

/// `scrut_ty` is the type of the value this pattern matches against — needed
/// so a bound variable (`Circle(r) => r`, `Some(x) => x`, ...) gets its real
/// type registered in `cx.local_types` instead of `Ty::Error`. Without it,
/// `Expr::Match`'s own inferred type falls back to whatever arm happens to
/// have a literal body (or `Ty::Error` if none does), which silently
/// truncates e.g. a `Float` payload once codegen maps `Ty::Error` to
/// `int64_t` (see BACKLOG item 111).
fn lower_pat(pat: &S<certo_ast::pattern::Pattern>, scrut_ty: &Ty, cx: &mut Cx) -> HirPat {
    use certo_ast::pattern::{Pattern, LitPat};
    match &pat.node {
        Pattern::Wildcard { .. } => HirPat::Wildcard,
        Pattern::Ident { name, .. } => {
            let local = cx.define_local(&name.node);
            if !matches!(scrut_ty, Ty::Error) {
                cx.local_types.insert(local, scrut_ty.clone());
            }
            HirPat::Bind { local, name: name.node.clone() }
        }
        Pattern::Literal { value, .. } => match value {
            LitPat::Int(n)    => HirPat::Lit(HirLitPat::Int(*n)),
            LitPat::Bool(b)   => HirPat::Lit(HirLitPat::Bool(*b)),
            LitPat::String(s) => HirPat::Lit(HirLitPat::Str(s.clone())),
            // Float equality matching is a footgun and Unit has a single value;
            // neither is supported as a literal pattern. Reject rather than
            // silently miscompile (use a binding + guard, or `_`, instead).
            LitPat::Float(_) => {
                cx.err(LowerErrorKind::Unsupported(
                    "float literal patterns are not supported — match on a bound variable with a guard instead".into()),
                    pat.span);
                HirPat::Wildcard
            }
            LitPat::Unit => {
                cx.err(LowerErrorKind::Unsupported(
                    "unit `()` literal patterns are not supported — use `_` to match unit".into()),
                    pat.span);
                HirPat::Wildcard
            }
        },
        Pattern::List { head, tail, .. } => {
            // BACKLOG item 195 — `elem_ty` for each `head` binding is the
            // list's own element type; `tail` (if bound) gets the *same*
            // `List<T>` type as the scrutinee itself, matching typeck's own
            // `check_pattern` (`crates/typeck/src/infer_expr.rs`).
            let elem_ty = match scrut_ty {
                Ty::List(t) => (**t).clone(),
                _ => Ty::Error,
            };
            let head_pats = head.iter().map(|h| lower_pat(h, &elem_ty, cx)).collect();
            let tail_pat = tail.as_ref().map(|t| Box::new(lower_pat(t, scrut_ty, cx)));
            HirPat::List { head: head_pats, tail: tail_pat, elem_ty }
        }
        Pattern::Tuple { elements, .. } => {
            let elem_tys: Vec<Ty> = match scrut_ty {
                Ty::Tuple(ts) => ts.clone(),
                _ => vec![Ty::Error; elements.len()],
            };
            HirPat::Tuple(elements.iter().zip(elem_tys.iter()).map(|(e, ety)| lower_pat(e, ety, cx)).collect())
        }
        Pattern::Constructor { path, fields, .. } => {
            let variant = path.segments.last().map(|s| s.node.clone()).unwrap_or_default();
            // Build a fully qualified tag name so MIR can emit `TypeName_VariantName`.
            let name = if let Some(parent) = cx.variant_to_type.get(&variant) {
                format!("{}__{}", parent, variant)
            } else {
                variant.clone()
            };
            let field_names = cx.variant_field_names.get(&variant).cloned()
                .unwrap_or_else(|| (0..fields.len()).map(|i| format!("f{i}")).collect());
            // Built-in Option/Result constructors' payload type comes from
            // the scrutinee's own type argument, not `variant_field_types`
            // (which only knows about user-declared `Decl::Type` sum types).
            let field_types: Vec<Ty> = match variant.as_str() {
                "Some" => match scrut_ty { Ty::Option(inner) => vec![(**inner).clone()], _ => vec![Ty::Error; fields.len()] },
                "None" => vec![],
                "Ok"   => match scrut_ty { Ty::Result(t, _) => vec![(**t).clone()], _ => vec![Ty::Error; fields.len()] },
                "Err"  => match scrut_ty { Ty::Result(_, e) => vec![(**e).clone()], _ => vec![Ty::Error; fields.len()] },
                _ => cx.variant_field_types.get(&variant).cloned()
                    .unwrap_or_else(|| vec![Ty::Error; fields.len()]),
            };
            // For binding the field pattern's *local*, substitute a bare
            // type-param field (`Ty::Var(_)`) with the scrutinee's own
            // concrete instantiation argument — mirrors `resolve_field_ty`'s
            // identical substitution for plain `.field` reads (BACKLOG item
            // 119), just missing here previously. Without it, `v`'s HIR type
            // in `Wrap(v) => v` was *always* `Ty::Var(0)` regardless of
            // whether the scrutinee was concrete or still abstract, which
            // fed into the whole `match` expression's own inferred type and
            // silently made a perfectly ordinary, non-generic function like
            // `fn unwrapAsInt(b: Box<Int>): Int = match b { Wrap(v) => v }`
            // return `void*` instead of `Int` — found while implementing
            // BACKLOG item 120. Deliberately keeps `field_types` itself
            // (returned in `HirPat::Constructor` below) as the *raw*,
            // unsubstituted declared types — MIR's own match-arm lowering
            // needs to know a field is *declared* `Ty::Var(_)` regardless of
            // substitution, since that's what decides whether its C storage
            // is a heap-boxed `void*` needing an unbox at all.
            let scrut_args: &[Ty] = match scrut_ty { Ty::Named { args, .. } => args, _ => &[] };
            let binding_field_types: Vec<Ty> = field_types.iter().map(|fty| match fty {
                Ty::Var(_) => scrut_args.first().cloned().unwrap_or_else(|| fty.clone()),
                other => other.clone(),
            }).collect();
            let lowered_fields: Vec<HirPat> = fields.iter().enumerate()
                .map(|(i, f)| lower_pat(f, binding_field_types.get(i).unwrap_or(&Ty::Error), cx))
                .collect();
            HirPat::Constructor { name, fields: lowered_fields, field_names, field_types }
        }
        Pattern::Or { left, right, .. } => HirPat::Or(
            Box::new(lower_pat(left, scrut_ty, cx)),
            Box::new(lower_pat(right, scrut_ty, cx)),
        ),
        Pattern::Record { path, fields, .. } => {
            // Bare (path-less) record patterns resolve structurally against
            // the scrutinee's own already-known type (BACKLOG item 145) —
            // mirrors typeck's own identical fallback in `check_pattern`.
            // Prefer `scrut_ty` itself whenever it's already a concrete
            // `Ty::Named` (carrying real instantiation args for a generic
            // record), falling back to a bare, arg-less `Ty::Named` built
            // from the pattern's own written type name only when `scrut_ty`
            // isn't concrete yet (defensive — typeck's own unification
            // already guarantees the two agree by the time HIR runs).
            let named_ty: Ty = match scrut_ty {
                Ty::Named { .. } => scrut_ty.clone(),
                _ => match path.as_ref().and_then(|p| p.segments.last()) {
                    Some(s) => Ty::Named { name: s.node.clone(), args: vec![] },
                    None => Ty::Error,
                },
            };
            let field_types: Vec<Ty> = fields.iter()
                .map(|pf| record_field_declared_ty(&named_ty, &pf.name.node, cx))
                .collect();
            // For naming a bound local's own type (and hinting a nested
            // sub-pattern), substitute a bare type-param field (`Ty::Var`)
            // with the scrutinee's own recovered instantiation argument —
            // mirrors `Constructor`'s identical `binding_field_types`
            // substitution just above (BACKLOG item 119/120), which this
            // arm originally missed: without it, a bound field's *local*
            // carried the record's raw, unsubstituted declared type
            // (`Ty::Var(0)`) instead of the real concrete type, so a later
            // reference to it (e.g. the match arm's own body) inferred the
            // wrong HIR type and codegen mixed up a boxed `void*` with a
            // real unboxed `double` — confirmed via a real repro (`match b
            // { Box { value: v } => v, ... }` for `b: Box<Float>` failed to
            // compile at the C stage) while verifying BACKLOG item 177.
            // `field_types` above stays raw — `HirPat::Record` still needs
            // it unsubstituted for the same reason `Constructor` does.
            let named_args: &[Ty] = match &named_ty { Ty::Named { args, .. } => args, _ => &[] };
            let binding_field_types: Vec<Ty> = field_types.iter().map(|fty| match fty {
                Ty::Var(_) => named_args.first().cloned().unwrap_or_else(|| fty.clone()),
                other => other.clone(),
            }).collect();
            let lowered_fields: Vec<HirPat> = fields.iter().zip(&binding_field_types).map(|(pf, bty)| {
                match &pf.pattern {
                    Some(sub) => lower_pat(sub, bty, cx),
                    None => {
                        // shorthand `{ x }` = `{ x: x }`
                        let local = cx.define_local(&pf.name.node);
                        if !matches!(bty, Ty::Error) { cx.local_types.insert(local, bty.clone()); }
                        HirPat::Bind { local, name: pf.name.node.clone() }
                    }
                }
            }).collect();
            let field_names: Vec<String> = fields.iter().map(|pf| pf.name.node.clone()).collect();
            HirPat::Record { fields: lowered_fields, field_names, field_types }
        }
        // Guard / As — flatten to wildcard for now (full pattern compilation later)
        _ => HirPat::Wildcard,
    }
}

/// A record pattern field's *raw*, unsubstituted declared type (BACKLOG
/// item 145) — the first half of `resolve_field_ty` without its own
/// `Ty::Var` substitution, since `HirPat::Record`'s own `field_types`
/// deliberately stays raw for MIR to substitute later against the real
/// scrutinee (see `HirPat::Record`'s own doc comment).
fn record_field_declared_ty(named_ty: &Ty, field: &str, cx: &Cx) -> Ty {
    let Ty::Named { name, .. } = named_ty else { return Ty::Error };
    let Some(names) = cx.record_field_names.get(name) else { return Ty::Error };
    let Some(pos) = names.iter().position(|n| n == field) else { return Ty::Error };
    cx.record_field_types.get(name).and_then(|tys| tys.get(pos)).cloned().unwrap_or(Ty::Error)
}

// ------------------------------------------------------------------ //
// Bare-generic-return resolution (BACKLOG item 135)
// ------------------------------------------------------------------ //

/// Whether `ty` contains an unresolved `Ty::Var` *anywhere* inside it,
/// recursively — not just as the whole type itself. BACKLOG item 247: every
/// declared type param collapses to the identical sentinel `Ty::Var(0)`
/// (`ast_ty_to_ty_with_params`'s own type-param handling, just below), so
/// `fn wrapOk<T, E>(v: T): Result<T, E>`'s declared return type is
/// `Ty::Result(Var(0), Var(0))` — both slots the same sentinel, at
/// different tree positions. `resolve_bare_generic_return` originally only
/// recognized a *bare* `Ty::Var(_)` as its whole type, missing every
/// compound shape like this one entirely — the payload's real concrete
/// type never reached the eventual `match Ok(v) => ...` arm that bound it,
/// which then passed a raw heap pointer straight into text concatenation
/// (or worse) as if it were already the right type.
fn ty_contains_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Option(inner) | Ty::List(inner) => ty_contains_var(inner),
        Ty::Result(a, b) | Ty::Map(a, b) => ty_contains_var(a) || ty_contains_var(b),
        Ty::Tuple(elems) => elems.iter().any(ty_contains_var),
        Ty::Named { args, .. } => args.iter().any(ty_contains_var),
        Ty::Record(fields) => fields.iter().any(|(_, t)| ty_contains_var(t)),
        // Deliberately excludes `Ty::Fn` — a function *value*'s own erased
        // parameter types are a normal, harmless situation (its eventual
        // caller supplies and boxes concrete arguments at the call site;
        // the function value itself is never directly read/matched the way
        // a container's payload is), not the same "value flows through an
        // erased slot and needs boxing/unboxing at *this* point" concern
        // Result/Option/etc. have. Including `Fn` here regressed a real,
        // working case: `val h = const("hi")` (`h: Fn { params: [Var(0)],
        // ret: Text }`, the unused/ignored param staying deliberately
        // erased) started forcing a hard "cannot determine concrete type"
        // error with no annotation available, even though nothing about
        // that program was actually broken.
        _ => false,
    }
}

/// Recursively substitute every `Ty::Var` slot in `actual` with whatever
/// sits at the *same tree position* in `expected` — position-based, not
/// identity-based, since (per `ty_contains_var`'s own doc comment above)
/// every type param already collapsed to the same indistinguishable
/// `Ty::Var(0)` sentinel long before this runs; there is no real variable
/// identity left to match on, only structural position. Already-concrete
/// parts of `actual` are kept as-is rather than overwritten wholesale by
/// `expected` — e.g. `Ty::Result(Ty::Int, Ty::Var(0))` (an error type still
/// unresolved) against `expected = Ty::Result(Ty::Int, Ty::Text)` keeps its
/// own already-known `Ty::Int` success slot and fills in `Text` only for
/// the genuinely unresolved one. A shape mismatch between `actual` and
/// `expected` (or a leaf that isn't a `Var`) falls back to `actual`
/// unchanged — this only ever narrows a still-erased slot, never discards
/// something already known.
fn merge_var_slots(actual: &Ty, expected: &Ty) -> Ty {
    match actual {
        Ty::Var(_) => expected.clone(),
        Ty::Option(a) => match expected {
            Ty::Option(e) => Ty::Option(Box::new(merge_var_slots(a, e))),
            _ => actual.clone(),
        },
        Ty::List(a) => match expected {
            Ty::List(e) => Ty::List(Box::new(merge_var_slots(a, e))),
            _ => actual.clone(),
        },
        Ty::Result(a1, a2) => match expected {
            Ty::Result(e1, e2) => Ty::Result(Box::new(merge_var_slots(a1, e1)), Box::new(merge_var_slots(a2, e2))),
            _ => actual.clone(),
        },
        Ty::Map(a1, a2) => match expected {
            Ty::Map(e1, e2) => Ty::Map(Box::new(merge_var_slots(a1, e1)), Box::new(merge_var_slots(a2, e2))),
            _ => actual.clone(),
        },
        Ty::Tuple(a_elems) => match expected {
            Ty::Tuple(e_elems) if e_elems.len() == a_elems.len() => Ty::Tuple(
                a_elems.iter().zip(e_elems.iter()).map(|(a, e)| merge_var_slots(a, e)).collect(),
            ),
            _ => actual.clone(),
        },
        Ty::Named { name, args: a_args } => match expected {
            Ty::Named { name: e_name, args: e_args } if e_name == name && e_args.len() == a_args.len() => Ty::Named {
                name: name.clone(),
                args: a_args.iter().zip(e_args.iter()).map(|(a, e)| merge_var_slots(a, e)).collect(),
            },
            _ => actual.clone(),
        },
        _ => actual.clone(),
    }
}

/// Try to resolve a call expression's unresolved bare-generic-return
/// sentinel (`Ty::Var(0)`, or — BACKLOG item 247 — one nested inside a
/// compound type like `Result<T, E>`/`Option<T>`) to a concrete type using
/// `expected` — the type context the call is being consumed in: a `val`'s
/// declared annotation, or an enclosing call's declared concrete param type
/// at this argument position. Only ever touches a `HirExprKind::Call` node
/// whose type still contains an unresolved var somewhere; anything else
/// (including a legitimate `Ty::Var(0)`-typed local/parameter reference
/// forwarded through a still-generic context, e.g. `v` inside `fn
/// wrap<T>(v: T) = Secret(v)`) is left untouched — those aren't a call
/// result, so there's nothing to resolve. With no concrete `expected`
/// available, this is exactly the case codegen would otherwise silently
/// mis-cast a raw `void*` as a concrete C type — a hard compile error
/// instead of shipping that.
fn resolve_bare_generic_return(expr: &mut HirExpr, expected: Option<&Ty>, hard_error_if_unresolvable: bool, cx: &mut Cx) {
    // A bare `None` reference (BACKLOG item 250) — unlike `Some(x)` (fixed
    // directly in `generic_container_ret` above, since its own argument
    // already carries a real type to derive `Option<T>` from), `None` has
    // no argument at all to recover anything from — it can only ever be
    // resolved from *external* context, exactly like a generic call's
    // unresolved return just below. Left as `Ty::Error` (not a hard error)
    // when no `expected` is available here, matching the pre-fix silent
    // behavior for callers who never needed the real type — unlike the
    // Call case below, this isn't a *guaranteed*-unresolvable situation
    // (an unannotated `None` might still be fine downstream), so it
    // doesn't force the same hard-error treatment.
    if matches!(&expr.kind, HirExprKind::Global(name) if name == "None") && matches!(expr.ty, Ty::Error) {
        if let Some(ty @ Ty::Option(_)) = expected {
            expr.ty = ty.clone();
        }
        return;
    }
    // BACKLOG item 251 — `await withAudit(...)`'s own bound value is
    // `HirExprKind::Await(inner)`, not a bare `Call`, so the check just
    // below (requiring `Call` directly) never even looked at it — `await
    // <plain call>`'s own type already mirrors its inner call's type
    // exactly (`Expr::Await`'s own lowering, item 191), so resolve through
    // to the inner call and sync the outer `Await` node's type back
    // afterward, rather than duplicating the whole resolution logic here.
    if let HirExprKind::Await(inner) = &mut expr.kind {
        resolve_bare_generic_return(inner, expected, hard_error_if_unresolvable, cx);
        expr.ty = inner.ty.clone();
        return;
    }
    if !matches!(expr.kind, HirExprKind::Call { .. }) || !ty_contains_var(&expr.ty) {
        return;
    }
    // BACKLOG item 247 — combines two independent signals for whether an
    // unresolvable case should be a hard error:
    //  - `hard_error_if_unresolvable` (per call site): true at a `val`'s
    //    own binding, where the value is consumed directly from here on,
    //    so an unresolved generic return (bare *or* compound) is always a
    //    real problem; false at the call-argument backfill site, where
    //    something else downstream may already have its own correct,
    //    separate erasure handling for a still-unresolved *compound* type
    //    (e.g. `Box.wrap`'s own `Box<T>` return, passed straight into
    //    `Box.unwrap`, whose own pattern-match-based unboxing — item 119's
    //    established convention — needs no help from this function).
    //  - `is_bare_var`: a bare `Ty::Var(0)` *as the whole type* (e.g.
    //    `identity<T>(v: T): T`) is a dead end regardless of call site —
    //    nothing about a compound wrapper's own established handling
    //    applies to it, so it still hard-errors even as a call argument
    //    (confirmed necessary: `identity(Box.unwrap(Box.wrap(42)))`,
    //    `identity`'s own param is itself generic so there's no expected
    //    type to resolve against, and this must still fail rather than
    //    silently let a raw `Ty::Var(0)` flow through unchecked).
    let is_bare_var = matches!(expr.ty, Ty::Var(_));
    match expected {
        Some(ty) if !ty_contains_var(ty) => expr.ty = merge_var_slots(&expr.ty, ty),
        _ if hard_error_if_unresolvable || is_bare_var => cx.err(LowerErrorKind::Unsupported(
            "cannot determine the concrete type of this generic function's return value here — \
             add an explicit type annotation (e.g. `val x: SomeType = ...`)".into()
        ), expr.span),
        _ => {}
    }
}

// ------------------------------------------------------------------ //
// AST type expression → Ty (lightweight conversion for HIR param types)
// ------------------------------------------------------------------ //

/// Whether a bare literal `expr` can be typed directly as the fixed-width
/// numeric type `declared` instead of its rigid default (`Ty::Int`/
/// `Ty::Float`, set unconditionally by `lower_lit`) — BACKLOG item 235's own
/// HIR-side half of the same narrow, annotated-position-only literal
/// inference `crates/typeck/src/infer_expr.rs`'s own `literal_matches_fixed_width`
/// implements (kept as an independent, syntax-level check here rather than
/// threaded across crates — MIR derives a function's real C return type from
/// its *body's own computed operand type*, not `HirFn.ret_ty`, so leaving
/// `lower_lit`'s `Ty::Int`/`Ty::Float` default uncorrected here would still
/// emit `int64_t`/`double` C code even once typeck itself stopped rejecting
/// the annotation). An int literal matches `Int8`/`Int16`/`Int32`/`UInt`; a
/// float literal matches `Float32` only.
fn literal_matches_fixed_width(expr: &Expr, declared: &Ty) -> bool {
    // `-5000: Int32` parses as `UnOp::Neg` wrapping a bare `Lit::Int` — the
    // signed fixed-width types must see through it (`UInt` must not; it has
    // no negative values for a negated literal to widen into).
    let (inner, is_negated) = match expr {
        Expr::UnOp { op: AstUnOp::Neg, expr, .. } => (&expr.node, true),
        other => (other, false),
    };
    match (inner, declared) {
        (Expr::Lit { value: Lit::Int(_), .. }, Ty::Int8 | Ty::Int16 | Ty::Int32) => true,
        (Expr::Lit { value: Lit::Int(_), .. }, Ty::UInt) => !is_negated,
        (Expr::Lit { value: Lit::Float(_), .. }, Ty::Float32) => true,
        _ => false,
    }
}

fn ast_ty_to_ty_with_params(te: &certo_ast::types::TypeExpr, type_params: &[&str]) -> Ty {
    use certo_ast::types::TypeExpr;
    match te {
        TypeExpr::Named { path, args, .. } => {
            let name = path.segments.last().map(|s| s.node.as_str()).unwrap_or("");
            // Single-segment name that matches a known type param → void* (opaque generic).
            if args.is_empty() && path.segments.len() == 1 && type_params.contains(&name) {
                return Ty::Var(0);
            }
            // `F<A>` where `F` is itself a declared (constructor-kind)
            // type param — higher-kinded application (BACKLOG item 76).
            // HIR doesn't track kinds at all (unlike `crates/typeck`'s real
            // `Ty::App`/`Ty::Ctor` — see `type_expr_to_ty`); it only ever
            // needs to know whether the whole thing is still generic and
            // therefore opaque, which is exactly as true for `F<A>` as for
            // a bare `T` — same `Ty::Var(0)` erasure, not a bogus
            // `Ty::Named { name: "F", .. }` naming an undeclared C type.
            if args.len() == 1 && path.segments.len() == 1 && type_params.contains(&name) {
                return Ty::Var(0);
            }
            let targs: Vec<Ty> = args.iter().map(|a| ast_ty_to_ty_with_params(&a.node, type_params)).collect();
            match name {
                "Int"     => Ty::Int,
                "Int8"    => Ty::Int8,
                "Int16"   => Ty::Int16,
                "Int32"   => Ty::Int32,
                "UInt"    => Ty::UInt,
                "Float"   => Ty::Float,
                "Float32" => Ty::Float32,
                "Decimal" => Ty::Decimal(None),
                "Bool"    => Ty::Bool,
                "Char"    => Ty::Char,
                "Text"    => Ty::Text,
                "BoundedText" => Ty::BoundedText(None),
                "Unit"    => Ty::Unit,
                "UUID"    => Ty::Uuid,
                "List"    => Ty::List(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Option"  => Ty::Option(Box::new(targs.into_iter().next().unwrap_or(Ty::Error))),
                "Result"  => {
                    let mut it = targs.into_iter();
                    Ty::Result(Box::new(it.next().unwrap_or(Ty::Error)), Box::new(it.next().unwrap_or(Ty::Error)))
                }
                "Map"     => {
                    let mut it = targs.into_iter();
                    Ty::Map(Box::new(it.next().unwrap_or(Ty::Error)), Box::new(it.next().unwrap_or(Ty::Error)))
                }
                other     => Ty::Named { name: other.to_string(), args: targs },
            }
        }
        TypeExpr::Option { inner, .. } => Ty::Option(Box::new(ast_ty_to_ty_with_params(&inner.node, type_params))),
        TypeExpr::Tuple { elements, .. } => Ty::Tuple(elements.iter().map(|e| ast_ty_to_ty_with_params(&e.node, type_params)).collect()),
        TypeExpr::Fn { params, ret, .. } => Ty::Fn {
            params: params.iter().map(|p| ast_ty_to_ty_with_params(&p.node, type_params)).collect(),
            ret:    Box::new(ast_ty_to_ty_with_params(&ret.node, type_params)),
        },
        // Type parameters (e.g. T in fn foo<T>) are opaque at the HIR level.
        // Ty::Var(0) round-trips to void* in the C backend.
        TypeExpr::Param { .. } => Ty::Var(0),
        TypeExpr::DecimalParam { precision, scale, .. } => Ty::Decimal(Some((*precision, *scale))),
        TypeExpr::BoundedTextParam { max_len, .. } => Ty::BoundedText(Some(*max_len)),
        _ => Ty::Error,
    }
}

/// Collect the locals a lambda body references from an enclosing scope —
/// real closure capture (BACKLOG item 140, superseding item 136's interim
/// "reject captures" compile error). Certo's lambda lowering (both the
/// plain `HirExprKind::Lambda` path and `lower_lambda_boxed` in
/// `crates/mir`) builds the lambda body in a *completely fresh*
/// `Builder`/local-numbering space with no bridge back to the enclosing
/// function's own locals — MIR uses this list to box each captured value
/// into the lambda's own heap environment at the construction site, and to
/// unbox it back into a same-named local inside the lambda's own lowered
/// function, instead of `Builder::get_local`'s old silent-corruption
/// fallback (reinterpreting the *outer* function's `LocalId` as an index
/// into the *lambda's own*, much smaller `locals` vec). Uses a `LocalId`
/// threshold rather than a full scope walk: `Cx::next_local` only ever
/// increments, so any `Local(id)` referenced inside the lambda body with
/// `id < threshold` (the counter's value right before the lambda's own
/// scope was pushed) must have been bound in an enclosing scope.
/// Compute an absolute monotonic-clock millisecond deadline from a
/// `Duration` expression (`now() + duration.toSeconds() * 1000`) and push
/// its `let` binding into `stmts`, returning the bound local — shared by
/// `parallel(timeout: ...) { ... }` (BACKLOG item 81) and
/// `withTimeout(...) { ... }` (BACKLOG item 122), which both need the
/// identical deadline math.
fn lower_deadline(duration: &S<Expr>, name: &str, stmts: &mut Vec<HirStmt>, cx: &mut Cx, span: Span) -> LocalId {
    let timeout_hir = lower_expr(duration, cx);
    let to_seconds = HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(HirExpr { kind: HirExprKind::Global("Duration.toSeconds".into()), ty: Ty::Error, span }),
            args: vec![timeout_hir],
        },
        ty: Ty::Int, span,
    };
    let to_ms = HirExpr {
        kind: HirExprKind::BinOp {
            op: BinOp::Mul,
            lhs: Box::new(to_seconds),
            rhs: Box::new(HirExpr { kind: HirExprKind::Int(1000), ty: Ty::Int, span }),
        },
        ty: Ty::Int, span,
    };
    let now_ms = HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(HirExpr { kind: HirExprKind::Global("monotonicMillis".into()), ty: Ty::Error, span }),
            args: vec![],
        },
        ty: Ty::Int, span,
    };
    let deadline_expr = HirExpr {
        kind: HirExprKind::BinOp { op: BinOp::Add, lhs: Box::new(now_ms), rhs: Box::new(to_ms) },
        ty: Ty::Int, span,
    };
    let local = cx.fresh_local();
    stmts.push(HirStmt::Let { local, name: name.into(), ty: Ty::Int, init: deadline_expr });
    local
}

fn collect_lambda_captures(body: &HirExpr, threshold: LocalId) -> Vec<LocalId> {
    let mut free = Vec::new();
    collect_free_locals(body, threshold, &mut free);
    free.sort_unstable();
    free.dedup();
    free
}

/// Recursively collect `HirExprKind::Local(id)` references with `id < threshold`
/// — see `check_no_lambda_capture`. Does not descend into a nested `Lambda`'s
/// own body: a nested lambda gets its own independent capture check at its
/// own construction site, with its own (later, so still `>= threshold`)
/// starting point — recursing here would just duplicate that error.
fn collect_free_locals(expr: &HirExpr, threshold: LocalId, out: &mut Vec<LocalId>) {
    let visit_stmt = |s: &HirStmt, out: &mut Vec<LocalId>| match s {
        HirStmt::Let { init, .. } => collect_free_locals(init, threshold, out),
        HirStmt::Assign { value, .. } => collect_free_locals(value, threshold, out),
        HirStmt::Expr(e) => collect_free_locals(e, threshold, out),
        HirStmt::Defer { body } => collect_free_locals(body, threshold, out),
    };
    match &expr.kind {
        HirExprKind::Int(_) | HirExprKind::Float(_) | HirExprKind::Decimal(_)
        | HirExprKind::Bool(_) | HirExprKind::Str(_) | HirExprKind::Uuid(_)
        | HirExprKind::Unit | HirExprKind::Global(_) => {}
        HirExprKind::Local(id) => { if *id < threshold { out.push(*id); } }
        HirExprKind::Call { func, args } => {
            collect_free_locals(func, threshold, out);
            for a in args { collect_free_locals(a, threshold, out); }
        }
        HirExprKind::BinOp { lhs, rhs, .. } => {
            collect_free_locals(lhs, threshold, out);
            collect_free_locals(rhs, threshold, out);
        }
        HirExprKind::UnOp { arg, .. } => collect_free_locals(arg, threshold, out),
        HirExprKind::Field { base, .. } => collect_free_locals(base, threshold, out),
        HirExprKind::Record { fields, .. } => {
            for (_, v) in fields { collect_free_locals(v, threshold, out); }
        }
        HirExprKind::Tuple(elems) | HirExprKind::List(elems) => {
            for e in elems { collect_free_locals(e, threshold, out); }
        }
        HirExprKind::If { cond, then_expr, else_expr } => {
            collect_free_locals(cond, threshold, out);
            collect_free_locals(then_expr, threshold, out);
            collect_free_locals(else_expr, threshold, out);
        }
        HirExprKind::Block { stmts, tail } => {
            for s in stmts { visit_stmt(s, out); }
            collect_free_locals(tail, threshold, out);
        }
        HirExprKind::Lambda { .. } => {} // own independent check — see doc comment
        HirExprKind::Match { scrutinee, arms } => {
            collect_free_locals(scrutinee, threshold, out);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_free_locals(g, threshold, out); }
                collect_free_locals(&arm.body, threshold, out);
            }
        }
        HirExprKind::Try(inner) | HirExprKind::Unsafe(inner) | HirExprKind::Await(inner) => {
            collect_free_locals(inner, threshold, out);
        }
        HirExprKind::For { iter, body, .. } => {
            collect_free_locals(iter, threshold, out);
            collect_free_locals(body, threshold, out);
        }
        HirExprKind::While { cond, body } => {
            collect_free_locals(cond, threshold, out);
            collect_free_locals(body, threshold, out);
        }
        HirExprKind::Spawn { args, .. } => {
            for a in args { collect_free_locals(a, threshold, out); }
        }
        HirExprKind::AwaitTimed { task, deadline }
        | HirExprKind::JoinTimedCancel { task, deadline } => {
            collect_free_locals(task, threshold, out);
            if *deadline < threshold { out.push(*deadline); }
        }
    }
}

/// Resolve a record field access's declared type and whether it needs
/// unboxing — BACKLOG item 119. Only succeeds when `base_ty` is a known
/// `Ty::Named` record type; otherwise conservatively falls back to
/// `(Ty::Error, false)`, matching this access's pre-existing behavior.
/// A `Ty::Var(_)` declared field type is substituted using the base type's
/// own recovered instantiation argument (single-type-param scope only —
/// `args.first()`), and `boxed` is set so MIR knows to unbox the read.
fn resolve_field_ty(base_ty: &Ty, field: &str, cx: &Cx) -> (Ty, bool) {
    let Ty::Named { name, args } = base_ty else { return (Ty::Error, false) };
    let Some(names) = cx.record_field_names.get(name) else { return (Ty::Error, false) };
    let Some(pos) = names.iter().position(|n| n == field) else { return (Ty::Error, false) };
    let Some(declared) = cx.record_field_types.get(name).and_then(|tys| tys.get(pos)) else { return (Ty::Error, false) };
    match declared {
        // Only unbox when the recovered instantiation argument is itself a
        // *known concrete* type — if `args.first()` is itself `Ty::Var(_)`
        // (the base's own instantiation is still abstract, e.g. reading a
        // field of `b: Box<T>` inside a generic function), the field was
        // never (re-)boxed on construction either, so unboxing here would
        // strip a level of pointer indirection that was never added —
        // BACKLOG item 120 (mirrors the same guard added to MIR's Record/
        // Call-arg boxing and variant match-arm unboxing).
        Ty::Var(_) => {
            let concrete = args.first().cloned().unwrap_or(Ty::Error);
            let needs_unbox = !matches!(concrete, Ty::Var(_));
            (concrete, needs_unbox)
        }
        other => (other.clone(), false),
    }
}

/// Recover a generic sum-type variant constructor call's concrete
/// instantiation argument — the constructor-call analog of
/// `recover_generic_record_ty` below, BACKLOG item 119. `None` when
/// `variant_name` isn't a known sum-type variant at all (the ordinary,
/// non-generic call path), so the caller falls back to its own `call_ty`.
fn recover_generic_variant_call_ty(variant_name: &str, args: &[HirExpr], cx: &Cx) -> Option<Ty> {
    let parent = cx.variant_to_type.get(variant_name)?;
    let field_types = cx.variant_field_types.get(variant_name)?;
    let recovered = field_types.iter().zip(args.iter())
        .find(|(fty, _)| matches!(fty, Ty::Var(_)))
        .map(|(_, arg)| arg.ty.clone());
    let ty_args = match recovered {
        Some(t) if !matches!(t, Ty::Error) => vec![t],
        _ => vec![],
    };
    Some(Ty::Named { name: parent.clone(), args: ty_args })
}

/// Recover a generic record literal's concrete instantiation argument from
/// its field values — BACKLOG item 119. For each field whose *declared*
/// type is a bare type parameter (`Ty::Var(0)`), the field's own (already
/// lowered) value type is taken as the recovered argument (single-type-param
/// scope: the first such field found wins, and any additional type params
/// beyond the first aren't recovered). Returns the record's `Ty::Named`
/// tagged with that argument (empty args if the type isn't generic, or
/// nothing could be recovered) plus the declared field types in `fields`'
/// order, so MIR knows which fields to heap-box on construction.
fn recover_generic_record_ty(ty_name: Option<&str>, fields: &[(String, HirExpr)], cx: &Cx) -> (Ty, Vec<Ty>) {
    let Some(name) = ty_name else { return (Ty::Error, vec![Ty::Error; fields.len()]) };
    let Some(names) = cx.record_field_names.get(name) else {
        return (Ty::Named { name: name.to_string(), args: vec![] }, vec![Ty::Error; fields.len()]);
    };
    let declared: Vec<Ty> = fields.iter().map(|(fname, _)| {
        names.iter().position(|n| n == fname)
            .and_then(|pos| cx.record_field_types.get(name).and_then(|tys| tys.get(pos)))
            .cloned()
            .unwrap_or(Ty::Error)
    }).collect();
    let recovered = fields.iter().zip(declared.iter())
        .find(|(_, ty)| matches!(ty, Ty::Var(_)))
        .map(|((_, value), _)| value.ty.clone());
    let args = match recovered {
        Some(t) if !matches!(t, Ty::Error) => vec![t],
        _ => vec![],
    };
    (Ty::Named { name: name.to_string(), args }, declared)
}
