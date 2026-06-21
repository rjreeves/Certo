# Typestate Design — State-Machine Transition Safety at Compile Time

> Status: DRAFT, not yet implemented.
> Target: `crates/typeck/src/ty.rs`, `crates/typeck/src/infer_decl.rs`,
> `crates/codegen/` (C lowering for state-indexed types).
> Prior art consulted: Rust's `TypedBuilder` phantom-state pattern, session
> types in Links/Haskell, Swift's `~Copyable` typestate proposals.

---

## 0. The Problem

Given a Certo state machine:

```certo
statemachine Order {
    states: [Draft, Submitted, Fulfilled, Cancelled]
    transition submit  from Draft      to Submitted
    transition fulfil  from Submitted  to Fulfilled
    transition cancel  from Draft | Submitted to Cancelled
}
```

`infer_decl.rs:166` currently generates **one** function type per transition
event, ignoring the `from` state entirely:

```
Order_submit  : (Order, …) → Order
Order_fulfil  : (Order, …) → Order
Order_cancel  : (Order, …) → Order
```

So `Order_fulfil(draft_order)` typechecks. The error is only caught at runtime
via `certo_panic("illegal transition")` inside the generated C — which means it
surfaces as a crash, not a type error, and only if that branch is exercised by
a test.

The goal: make `Order_fulfil(draft_order)` a **compile-time type error**, with
a message like:

```
error[E0601]: transition `fulfil` requires state `Submitted`, got `Draft`
   --> order.cto:14:5
    |
 14 │     Order_fulfil(my_order)
    |     ^^^^^^^^^^^^^^^^^^^^^^
    |     `my_order` has type `Order<Draft>`, but `fulfil` requires `Order<Submitted>`
```

---

## 1. The Approach: Phantom State Parameter

The cleanest compile-time encoding that requires **no new Ty variants** is to
add a phantom type parameter to the machine type:

```
Order<S>   where S ∈ { OrderDraft, OrderSubmitted, OrderFulfilled, OrderCancelled }
```

The phantom parameter `S` carries state identity in the type system but has
zero runtime representation — exactly like Rust's `PhantomData<S>`. The
existing `Ty::Named { name, args }` variant already supports type arguments, so
the type checker needs no new sum arms.

### 1.1 State marker types

For each state `Foo` in machine `M`, generate a unit type `M_Foo` (or
`M.Foo` — see §1.3 on naming):

```
OrderDraft      : Type       (unit, zero runtime bytes)
OrderSubmitted  : Type
OrderFulfilled  : Type
OrderCancelled  : Type
```

The machine type `Order<S>` has one phantom argument. Constructors, transitions,
and predicates are all typed in terms of it.

### 1.2 Transition signatures (after change)

```
Order_new     : () → Order<OrderDraft>          // initial state is the first listed

Order_submit  : (Order<OrderDraft>, …)     → Order<OrderSubmitted>
Order_fulfil  : (Order<OrderSubmitted>, …) → Order<OrderFulfilled>
Order_cancel  : (Order<OrderDraft>, …)     → Order<OrderCancelled>
             | (Order<OrderSubmitted>, …)  → Order<OrderCancelled>    // multi-from: two overloads
```

`Order_state  : (Order<S>) → OrderState`    // unchanged — erases phantom, returns runtime enum
`Order_isDraft: (Order<S>) → Bool`           // unchanged — predicate does not constrain S

### 1.3 Naming — avoiding user namespace pollution

The state marker types (`OrderDraft`, `OrderSubmitted`, …) are generated names
that must not collide with user-defined types. Two options:

**Option A — Prefix with machine name (recommended):** `OrderDraft`,
`OrderSubmitted`. Simple, readable in error messages, mirrors how `OrderState`
is already generated today. Collision risk exists but is low and detectable at
definition time (same-name user type → E0603 duplicate type).

**Option B — Module-scoped:** `Order.Draft`, `Order.Submitted`. Cleaner
namespacing, but requires the type checker to resolve dotted type names inside
`Named { name }` — currently `name` is a plain `String`, not a path. This
would need a small `ty.rs` change and is probably more work than it's worth for
a generated-only type.

Recommendation: **Option A** for now, with a note that Option B becomes
attractive if Certo adds first-class module paths to `Ty::Named`.

---

## 2. Multi-`from` Transitions

A transition with `from: [Draft, Submitted]` needs to accept either
`Order<OrderDraft>` or `Order<OrderSubmitted>` — two distinct types in
`Option A`. There are two strategies:

**Strategy 1 — Two overloads (recommended for now):**
Register two function types in the environment under the same name, differing
only in the first argument. The call-site inference unifies against each in
turn; the first match wins. This is exactly how overloaded arithmetic operators
work in most HM systems and requires no new inference machinery — unification
already tries Named arms by structure.

**Strategy 2 — A union phantom:**
Introduce `Order<OrderDraftOrSubmitted>` where `OrderDraftOrSubmitted` is a
synthetic marker. Cleaner in principle but requires manufacturing new marker
types per multi-from edge set, which grow combinatorially in complex machines.
Not recommended.

---

## 3. What Actually Changes

### 3.1 `crates/ast/src/decl.rs` — **no change**

`StateMachineDecl` already has `states: Vec<Ident>` and `transitions:
Vec<Transition>` with `from`/`to` fields. The AST is fine as-is.

### 3.2 `crates/typeck/src/ty.rs` — **no change**

`Ty::Named { name, args }` already supports parametric types. `Order<OrderDraft>`
is `Ty::Named { name: "Order", args: [Ty::Named { name: "OrderDraft", args: [] }] }`.
No new variants needed.

### 3.3 `crates/typeck/src/infer_decl.rs` — **the main change**

Replace the current `Decl::StateMachine` arm (lines 150–191) with:

```rust
Decl::StateMachine(sm) => {
    let mname = &sm.name.node;

    // Register each state marker type (unit type — no constructors needed).
    for state in &sm.states {
        let marker_name = format!("{}{}", mname, state.node);
        env.define(marker_name.clone(),
            Ty::Named { name: marker_name, args: vec![] });
    }

    // machine_ty(s) returns Order<OrderS> for a given state name.
    let machine_ty = |state: &str| -> Ty {
        let marker = Ty::Named { name: format!("{}{}", mname, state), args: vec![] };
        Ty::Named { name: mname.clone(), args: vec![marker] }
    };

    // Constructor: Order_new() → Order<OrderDraft>  (first state)
    let initial = sm.states[0].node.as_str();
    env.define(
        format!("{}_new", mname),
        Ty::Fn { params: vec![], ret: Box::new(machine_ty(initial)) },
    );

    // Transition functions — one overload per `from` state.
    for t in &sm.transitions {
        let ret_ty = machine_ty(&t.to.node);
        let mut ctx = Ctx { env, uf, errors, counter };
        let extra_params: Vec<Ty> = t.params.iter()
            .map(|p| type_expr_to_ty(&p.ty.node, &mut ctx))
            .collect();

        // `from` in the AST is a single state (Ident); for multi-from
        // transitions the parser may expand to multiple Transition entries
        // with the same event name — OR we extend Transition.from to
        // Vec<Ident> here (see §4).  Either way, one env.define per from state:
        let param_tys: Vec<Ty> = std::iter::once(machine_ty(&t.from.node))
            .chain(extra_params)
            .collect();
        env.define(
            format!("{}_{}", mname, t.event.node),
            Ty::Fn { params: param_tys, ret: Box::new(ret_ty) },
        );
    }

    // State predicate and accessor — unchanged: accept any Order<S>.
    // Use a fresh type variable for the phantom so inference works on
    // any machine value regardless of current state.
    let s_var = { *counter += 1; Ty::Var(*counter) };
    let any_machine = Ty::Named { name: mname.clone(), args: vec![s_var.clone()] };
    let state_ty    = Ty::Named { name: format!("{}State", mname), args: vec![] };

    env.define(format!("{}State", mname), state_ty.clone());
    env.define(format!("{}_state", mname),
        Ty::Forall {
            vars: s_var.free_vars(),
            body: Box::new(Ty::Fn {
                params: vec![any_machine.clone()],
                ret:    Box::new(state_ty),
            }),
        });

    for state in &sm.states {
        env.define(
            format!("{}_is{}", mname, state.node),
            Ty::Forall {
                vars: s_var.free_vars(),
                body: Box::new(Ty::Fn {
                    params: vec![any_machine.clone()],
                    ret:    Box::new(Ty::Bool),
                }),
            });
    }
}
```

The key change: `machine_ty(from)` instead of `machine_ty` in the first
parameter of each transition. Everything else is plumbing that already exists.

### 3.4 Multi-from: AST decision (§4 dependency)

The current `Transition.from` is a single `Ident`. Multi-from (`from: [Draft,
Submitted]`) needs to either:

**(a)** Expand to multiple `Transition` entries during parsing (parser change,
AST unchanged), or

**(b)** Change `Transition.from` to `Vec<Ident>` and expand in `infer_decl`.

Option (a) keeps `infer_decl` simpler. Option (b) preserves the original
user-written structure for tooling/LSP. Recommendation: **(b)** — change
`Transition.from` from `Ident` to `Vec<Ident>` in `decl.rs`, update the
parser to produce `vec![single_state]` for the common case, and loop in
`infer_decl` to register one overload per from state.

### 3.5 `crates/codegen/` — phantom erasure

At the C level `Order<OrderDraft>` and `Order<OrderSubmitted>` are the **same
struct** — the phantom parameter is erased. The codegen pass already produces
one C struct per state machine (`typedef struct { uint8_t state; ... }
certo_Order_t`); no change to the struct layout is needed.

The *only* codegen change is removing the runtime `from`-state guard:

```c
// BEFORE — runtime panic if wrong state:
certo_Order_t Order_submit(certo_Order_t m, ...) {
    if (m.state != ORDER_STATE_DRAFT) certo_panic("illegal transition: submit");
    m.state = ORDER_STATE_SUBMITTED;
    return m;
}

// AFTER — type checker guarantees the argument is always Draft; guard removed:
certo_Order_t Order_submit(certo_Order_t m, ...) {
    m.state = ORDER_STATE_SUBMITTED;
    return m;
}
```

This is a small codegen change and can be done as a flag (`--typestate` / or
simply whenever the new `infer_decl` path is active). The runtime guard can be
kept temporarily behind a `#[debug_assertions]`-style compile flag while the
new inference is being validated.

---

## 4. What the Error Messages Look Like

The existing `unify` function in `crates/typeck/src/unify.rs` already produces
mismatch errors from failed unification. When
`Order_fulfil(draft_order)` is type-checked:

- Inferred call: `(Order<OrderDraft>, …) → ?`
- Required by `Order_fulfil`: `(Order<OrderSubmitted>, …) → Order<OrderFulfilled>`
- Unify `Order<OrderDraft>` with `Order<OrderSubmitted>` →
  unify `OrderDraft` with `OrderSubmitted` → **fail**: two distinct `Named`
  types whose names differ.

The error fires at the `Named` arm of `unify`:

```rust
(Ty::Named { name: n1, args: a1 }, Ty::Named { name: n2, args: a2 }) => {
    if n1 != n2 { return Err(TyError::Mismatch(got, expected)); }
    ...
}
```

The error message that bubbles up will say `Order<OrderDraft>` vs
`Order<OrderSubmitted>`. A small improvement to the error formatter
(`crates/typeck/src/error.rs`) that detects "both sides are `M<MState>` for the
same machine `M`" could produce the friendlier message shown in §0, but that's
cosmetic and can be done as a follow-up.

---

## 5. What Doesn't Change

| Component | Change needed |
|---|---|
| `crates/ast/src/decl.rs` | Only if `Transition.from` → `Vec<Ident>` (§3.4b) |
| `crates/typeck/src/ty.rs` | **None** |
| `crates/typeck/src/unify.rs` | **None** |
| `crates/typeck/src/infer_expr.rs` | **None** |
| `crates/parser/` | Minor — `from` field parsing for multi-state |
| `crates/codegen/` | Remove runtime `from`-state guard only |
| `crates/hir/`, `crates/mir/` | **None** — HIR/MIR pass-through for machine types |

The type-system surface area is surprisingly narrow: one block in
`infer_decl.rs`, zero new `Ty` variants, and phantom erasure in codegen.

---

## 6. Open Questions Before Implementation

1. **`Transition.from` as `Vec<Ident>` vs parser expansion** — decide before
   touching `decl.rs`. The parser change is small either way.

2. **Overload resolution ambiguity** — if two transitions share the same event
   name and both have `from: Draft` (a user error), `env.define` silently
   shadows the first. Should this be a new error code (E0602 duplicate
   transition)? Probably yes — add it to `check_state_machine` in typeck.

3. **`Order_new` initial state** — currently hardcoded to `sm.states[0]`. Is
   the initial state always the first declared, or should the AST have an
   explicit `initial:` field? The current `StateMachineDecl` has no `initial`
   field; adding one to the AST is the cleaner long-term path.

4. **Error message polish** — the raw `Named` mismatch message is readable but
   not as friendly as §0's example. A targeted improvement in
   `crates/typeck/src/error.rs` can be done after the core works.

---

## 7. Implementation Order

1. `decl.rs` — change `Transition.from: Ident` → `Vec<Ident>` (or decide on
   parser expansion)
2. `infer_decl.rs` — replace `Decl::StateMachine` arm with §3.3
3. `cargo test -p certo-typeck` — existing typeck tests should still pass;
   any state-machine tests that call `Order_fulfil` on a `Draft` value now
   fail with a type error (which is the correct outcome)
4. `codegen` — remove the runtime from-guard
5. Add typeck test: `transition_wrong_state_is_type_error`
6. (Optional) Error message polish in `error.rs`
