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
/// automatically; generic producers (`List.get<T>`, registered as `Forall`)
/// are skipped here — their payload type depends on the call's argument
/// types, which `generic_container_ret` below recovers structurally instead.
fn stdlib_ret_types() -> &'static HashMap<String, Ty> {
    static TABLE: OnceLock<HashMap<String, Ty>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut env = certo_typeck::TypeEnv::new();
        let mut counter = 0u32;
        certo_stdlib::seed_stdlib(&mut env, &mut counter);
        env.names()
            .into_iter()
            .filter_map(|name| match env.lookup(&name) {
                Some(Ty::Fn { ret, .. }) if !ret.has_vars() => Some((name, (**ret).clone())),
                _ => None,
            })
            .collect()
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

/// Return types for generic stdlib functions whose result depends on an
/// argument's element type (e.g. `List.get<T>(List<T>, Int): T?`). Recovering
/// the element type lets the caller unbox the payload correctly (Float bits).
/// Deliberately mechanical/structural, not real inference — BACKLOG item 113.
fn generic_container_ret(full: Option<&str>, args: &[HirExpr]) -> Option<Ty> {
    let list_elem = |a: &HirExpr| match &a.ty { Ty::List(inner) => Some((**inner).clone()), _ => None };
    let map_kv = |a: &HirExpr| match &a.ty { Ty::Map(k, v) => Some(((**k).clone(), (**v).clone())), _ => None };

    match full {
        Some("List.get") | Some("List.first") | Some("List.last") | Some("List.find") => {
            args.first().and_then(list_elem).map(|e| Ty::Option(Box::new(e)))
        }
        Some("List.getOrPanic") => args.first().and_then(list_elem),
        Some("List.filter") | Some("List.sort") | Some("List.reverse") | Some("List.distinct")
        | Some("List.slice") | Some("List.concat") | Some("List.push") | Some("List.sortBy") => {
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
        Some("List.sumBy") => {
            match args.get(1).map(|a| &a.kind) {
                Some(HirExprKind::Lambda { body, .. }) if !matches!(body.ty, Ty::Error) => {
                    Some(body.ty.clone())
                }
                _ => None,
            }
        }
        Some("List.partition") => {
            args.first().map(|a| Ty::Tuple(vec![a.ty.clone(), a.ty.clone()]))
        }
        Some("List.chunked") => args.first().map(|a| Ty::List(Box::new(a.ty.clone()))),
        // The callback (args[1]) was lowered via `lower_lambda_with_param_hint`,
        // so its body's type is now recoverable (not guaranteed — only when the
        // hinted param propagated through, e.g. a direct arithmetic/literal
        // body) rather than unconditionally `Ty::Error`.
        Some("List.map") => {
            match args.get(1).map(|a| &a.kind) {
                Some(HirExprKind::Lambda { body, .. }) if !matches!(body.ty, Ty::Error) => {
                    Some(Ty::List(Box::new(body.ty.clone())))
                }
                _ => None,
            }
        }
        Some("List.groupBy") => {
            let elem = args.first().and_then(list_elem)?;
            match args.get(1).map(|a| &a.kind) {
                Some(HirExprKind::Lambda { body, .. }) if !matches!(body.ty, Ty::Error) => {
                    Some(Ty::Map(Box::new(body.ty.clone()), Box::new(Ty::List(Box::new(elem)))))
                }
                _ => None,
            }
        }
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

                let body = f.body.as_ref().map(|b| lower_expr(b, &mut cx));

                cx.pop_scope();

                items.push(HirItem::Fn(HirFn {
                    id,
                    name:   f.name.node.clone(),
                    params,
                    ret_ty: f.ret_ty.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &tp_names)).unwrap_or(Ty::Error),
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
                    let body = Some(lower_expr(body_ast, &mut cx));
                    cx.pop_scope();
                    items.push(HirItem::Fn(HirFn {
                        id,
                        name:   qname,
                        params,
                        ret_ty: m.ret_ty.as_ref().map(|t| ast_ty_to_ty_with_params(&t.node, &tp_names)).unwrap_or(Ty::Error),
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

// ------------------------------------------------------------------ //
// Expression lowering
// ------------------------------------------------------------------ //

fn lower_expr(expr: &S<Expr>, cx: &mut Cx) -> HirExpr {
    let span = expr.span;
    match &expr.node {
        // Desugar f-string to ++ chain before generic lit handling
        Expr::Lit { value: Lit::FString(parts), .. } => {
            let mut segments: Vec<HirExpr> = parts.iter().map(|p| match p {
                FStringPart::Literal(s) => HirExpr {
                    kind: HirExprKind::Str(s.clone()), ty: Ty::Text, span,
                },
                FStringPart::Interpolated(e) => lower_expr(e, cx),
            }).collect();
            if segments.is_empty() {
                return HirExpr { kind: HirExprKind::Str(String::new()), ty: Ty::Text, span };
            }
            let first = segments.remove(0);
            return segments.into_iter().fold(first, |acc, seg| HirExpr {
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
                // Stdlib function: only labeled reordering (no defaults).
                if has_labels {
                    let mut slots: Vec<Option<HirExpr>> = vec![None; snames.len()];
                    let mut pos_cursor = 0usize;
                    for arg in args {
                        let expr = lower_expr(&arg.value, cx);
                        if let Some(label) = &arg.label {
                            if let Some(idx) = snames.iter().position(|&n| n == label.node.as_str()) {
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
                    slots.into_iter().map(|maybe| {
                        maybe.unwrap_or_else(|| HirExpr { kind: HirExprKind::Unit, ty: Ty::Error, span })
                    }).collect()
                } else {
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
                    let needs_lambda_hint = matches!(fn_full_path.as_deref(),
                        Some("List.map") | Some("List.groupBy")
                        | Some("List.sortBy") | Some("List.minBy") | Some("List.maxBy") | Some("List.sumBy"));
                    let mut out: Vec<HirExpr> = Vec::with_capacity(args.len());
                    for (i, arg) in args.iter().enumerate() {
                        if needs_lambda_hint && i == 1 {
                            if let Expr::Lambda { params, body, .. } = &arg.value.node {
                                let hint = out.first().and_then(|a: &HirExpr| match &a.ty {
                                    Ty::List(inner) => Some((**inner).clone()),
                                    _ => None,
                                }).unwrap_or(Ty::Error);
                                let lowered = lower_lambda_with_param_hint(params, body, &hint, cx, arg.value.span);
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
                    resolve_bare_generic_return(arg, expected, cx);
                }
            }
            let call_ty = fn_full_path.as_deref()
                .and_then(|fp| cx.sm_returns.get(fp).cloned())
                .or_else(|| fn_full_path.as_deref().and_then(|fp| cx.fn_ret_types.get(fp).cloned()))
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
                    let fn_name = if *op == AstBinOp::RangeInclusive { "range_inclusive" } else { "range" };
                    let lhs = lower_expr(left, cx);
                    let rhs = lower_expr(right, cx);
                    let func = HirExpr { kind: HirExprKind::Global(fn_name.into()), ty: Ty::Error, span };
                    HirExpr { kind: HirExprKind::Call { func: Box::new(func), args: vec![lhs, rhs] }, ty: Ty::Error, span }
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
            let op = match op { AstUnOp::Neg => UnOp::Neg, AstUnOp::Not => UnOp::Not };
            HirExpr { kind: HirExprKind::UnOp { op, arg: Box::new(arg) }, ty: Ty::Error, span }
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
                let (field_ty, boxed) = resolve_field_ty(&base.ty, &field.node, cx);
                HirExpr { kind: HirExprKind::Field { base: Box::new(base), field: field.node.clone(), boxed }, ty: field_ty, span }
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
            let ty = if !matches!(then_.ty, Ty::Error) { then_.ty.clone() } else { else_.ty.clone() };
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
            let ty = hir_arms.iter().find_map(|a| {
                if !matches!(a.body.ty, Ty::Error) { Some(a.body.ty.clone()) } else { None }
            }).unwrap_or(Ty::Error);
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
            let inner = lower_expr(expr, cx);
            HirExpr { kind: HirExprKind::Await(Box::new(inner)), ty: Ty::Error, span }
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
            let deadline_local = timeout.as_ref().map(|t| {
                let timeout_hir = lower_expr(t, cx);
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
                stmts.push(HirStmt::Let { local, name: "__parallel_deadline".into(), ty: Ty::Int, init: deadline_expr });
                local
            });
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
            cx.push_scope();
            let local = cx.define_local(&binding.node);
            let body_hir = lower_expr(body, cx);
            cx.pop_scope();
            HirExpr {
                kind: HirExprKind::For {
                    binding:      local,
                    binding_name: binding.node.clone(),
                    binding_ty:   Ty::Error,
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
            HirExpr {
                kind: HirExprKind::Call {
                    func: Box::new(HirExpr {
                        kind: HirExprKind::Global("DateTime.diff".into()),
                        ty: Ty::Error, span,
                    }),
                    args: vec![now_call, base],
                },
                ty: Ty::Named { name: "Duration".into(), args: vec![] },
                span,
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
                resolve_bare_generic_return(&mut init, declared.as_ref(), cx);
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
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_rec".into(), ty: Ty::Error, init });
                        for pf in fields {
                            let binding_name = if let Some(sub) = &pf.pattern {
                                if let Pattern::Ident { name, .. } = &sub.node { name.node.clone() } else { continue }
                            } else {
                                pf.name.node.clone()
                            };
                            let local = cx.define_local(&binding_name);
                            let base = HirExpr { kind: HirExprKind::Local(tmp), ty: Ty::Error, span };
                            let field_expr = HirExpr {
                                kind: HirExprKind::Field { base: Box::new(base), field: pf.name.node.clone(), boxed: false },
                                ty: Ty::Error, span,
                            };
                            hir_stmts.push(HirStmt::Let { local, name: binding_name, ty: Ty::Error, init: field_expr });
                        }
                    }
                    _ => {
                        // Complex patterns: lower to match + let
                        let tmp = cx.fresh_local();
                        hir_stmts.push(HirStmt::Let { local: tmp, name: "_pat".into(), ty: Ty::Error, init });
                    }
                }
            }
            Stmt::Var { name, value, .. } => {
                let init = lower_expr(value, cx);
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
        // Record / Guard / As — flatten to wildcard for now (full pattern compilation later)
        _ => HirPat::Wildcard,
    }
}

// ------------------------------------------------------------------ //
// Bare-generic-return resolution (BACKLOG item 135)
// ------------------------------------------------------------------ //

/// Try to resolve a call expression's unresolved bare-generic-return
/// sentinel (`Ty::Var(0)`) to a concrete type using `expected` — the type
/// context the call is being consumed in: a `val`'s declared annotation, or
/// an enclosing call's declared concrete param type at this argument
/// position. Only ever touches a `HirExprKind::Call` node whose type is
/// still the unresolved sentinel; anything else (including a legitimate
/// `Ty::Var(0)`-typed local/parameter reference forwarded through a still-
/// generic context, e.g. `v` inside `fn wrap<T>(v: T) = Secret(v)`) is left
/// untouched — those aren't a call result, so there's nothing to resolve.
/// With no concrete `expected` available, this is exactly the case codegen
/// would otherwise silently mis-cast a raw `void*` as a concrete C type —
/// a hard compile error instead of shipping that.
fn resolve_bare_generic_return(expr: &mut HirExpr, expected: Option<&Ty>, cx: &mut Cx) {
    if !matches!(expr.kind, HirExprKind::Call { .. }) || !matches!(expr.ty, Ty::Var(_)) {
        return;
    }
    match expected {
        Some(ty) if !matches!(ty, Ty::Var(_)) => expr.ty = ty.clone(),
        _ => cx.err(LowerErrorKind::Unsupported(
            "cannot determine the concrete type of this generic function's return value here — \
             add an explicit type annotation (e.g. `val x: SomeType = ...`)".into()
        ), expr.span),
    }
}

// ------------------------------------------------------------------ //
// AST type expression → Ty (lightweight conversion for HIR param types)
// ------------------------------------------------------------------ //

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
        HirExprKind::AwaitTimed { task, deadline } => {
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
