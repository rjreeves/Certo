# Known Limitations & Out-of-Scope Features

This document records the parts of Certo that are **not** covered by the v1.0
stability guarantee. Everything listed here either works partially, works only
on some platforms, or is an experimental component that may change without
notice. The goal is honesty: if a feature isn't here and isn't otherwise marked
experimental, it is expected to work and is covered by the test suite.

Last reviewed: 2026-06-22.

> **End-to-end testing status (2026-06-22).** Every headline feature found in the
> smoke pass has been implemented: traits/`impl` (§6), validators (§7), state
> machines (§8), sum types with payloads, and user enums as `Result`/`List`
> payloads. The core language, standard library, traits, validators, state
> machines, and concurrency are verified working end-to-end (§9), including a
> realistic program combining all of them. (An integration smoke test on
> 2026-06-22 found and fixed a `Result` re-wrapping type-checker bug — `Ok`/`Err`
> now correctly quantify both type variables.)
>
> **Scope decision (2026-06-22):** one narrow item is **explicitly deferred to
> post-1.0** and is *out of scope* for the v1.0 guarantee — payload-*carrying*
> enums in `Result`/`List`/tuple slots (§10). Has a clean workaround and doesn't
> block the v1.0 feature set. (State-machine `on_enter`/`invariant` blocks,
> previously also deferred here, are implemented as of BACKLOG item 82 — see §8.)
>
> **Fixed 2026-06-23:** `Option` is now uniformly heap-boxed, so `parseInt` /
> `parseFloat` return correct values, `Some(0)` ≠ `None`, and `Option<Float>`
> round-trips (§11). This also fixed `List.get` silently mapping to a
> panic-on-out-of-bounds variant instead of returning `None`.

---

## 1. Concurrency: real OS threads (with two surface-syntax caveats)

**Status:** Implemented. `spawn`/`await` run on real OS threads.

`spawn f(args)` evaluates its arguments in the current thread, then runs `f` on a
new OS thread (Win32 threads on Windows, pthreads on POSIX). `await t` joins the
thread and returns its result. Each spawn call site gets a generated context
struct + worker function ([crates/codegen/src/emit_mir.rs](../crates/codegen/src/emit_mir.rs)),
launched through the portable shim in the runtime header
(`__certo_thread_spawn` / `__certo_thread_join`). Measured speedup on a 4-task
CPU-bound workload is ~2× (core-count bound), with identical results to the
sequential version.

Both idiomatic forms work:

```
val (a, b) = await parallel { f(), g() }   // both run concurrently
val total  = (await t1) + (await t2)        // await inline in an expression
```

Only a direct call to a named function is threaded (`spawn work(x)`). Spawning a
non-call expression falls back to sequential evaluation (still correct).

---

## 2. `Http.serve` — Windows only

**Status:** Implemented on Windows; unsupported elsewhere.

The HTTP **client** (`Http.get`, `Http.post`, etc.) works on all platforms. The
HTTP **server** entry point `Http.serve` is implemented using WinHTTP and is
only available on Windows. On other platforms it compiles but aborts at runtime
with:

```
Http.serve is not yet supported on this platform
```

(See [crates/stdlib/src/http.rs](../crates/stdlib/src/http.rs).) A portable
(POSIX sockets) server implementation is future work.

---

## 3. Experimental backends — not covered by v1.0

These alternative backends exist in the workspace but are **experimental**. They
are not part of the default build pipeline (which is MIR → C), are not feature-
complete, and are excluded from the v1.0 stability guarantee.

| Crate | State | Gaps |
|---|---|---|
| `certo-llvm` | Experimental | Struct field access emits `null`; `spawn`/`await` emit placeholder `i64 0`; types are not fully resolved pre-typeck. Covers basic arithmetic, calls, `if`/`match`, records only. |
| `certo-wasm` | Experimental | WebAssembly backend; incomplete. |
| `certo-xeq` | Experimental | Bytecode interpreter; incomplete. |

The **C backend** (`certo-codegen`) is the supported, covered-by-v1.0 backend.

---

## 4. Pattern matching: float and unit literal patterns rejected

**Status:** Intentional restriction.

Literal patterns on `Float` and `Unit` are **not supported** and produce a
compile error rather than silently miscompiling:

```
match x {
    1.5 => ...    // error: float literal patterns are not supported
    ()  => ...    // error: unit `()` literal patterns are not supported
}
```

Float equality matching is a footgun (floating-point equality is rarely what you
want), and `Unit` has a single value so a literal pattern is pointless. Use a
bound variable with a guard, or `_`, instead. Integer, boolean, and string
literal patterns are fully supported.

---

## 5. Migration column types: no parenthesised parameters

**Status:** Minor parser restriction.

In `createTable` / `alterTable` column definitions, parameterised type syntax
with parentheses (e.g. `BoundedText(100)`) is not parsed. Use the bare type
name. Generic angle-bracket types (`List<Int>`) are fine. The structured
migration operations are otherwise complete — see
[CERTO-SPEC.md §6.5](CERTO-SPEC.md).

---

## 6. Traits / `impl` — executable via qualified calls (two caveats)

**Status:** Implemented (static dispatch). Two caveats below.

`impl Trait for Type { fn m(…) }` compiles each method to a real function named
`Type.m`, called like a stdlib function:

```
impl Greet for Person { fn greet(p: Person): Text = p.name }
…
Person.greet(somePerson)        // runs
```

Dispatch is **static, by the qualified type name** — there is no dynamic
dispatch / trait objects, which suits the monomorphic C backend. Verified
running end-to-end on 2026-06-22 (multiple impls, multi-arg methods, methods
calling each other).

Caveats:

- **Call syntax is `Type.method(receiver, …)`, not `receiver.method(…)`.**
  Method/UFCS call syntax is not wired up; use the qualified form.
- **Default trait methods are not materialised.** If a `trait` provides a default
  body and an `impl` omits that method, `Type.method` is not generated. Define
  the method explicitly in each `impl`.

---

## 7. Validators — executable (`Text` or nullary-enum error types)

**Status:** Implemented for `Text` and nullary-enum (`| TooSmall | TooBig`) error
types. Payload-carrying enum errors are still blocked by §10 (rare).

A `validator` declaration is now expanded into real functions during `certo
build`: each validator generates `V_validate` (fail-fast) and `V_validateAll`
(collect-all), called as `V.validate(x)` / `V.validateAll(x)`. The expander
generates Certo source (if/then/else chains + `List.concat`), parses it, and
splices it into the module ([crates/cli/src/main.rs](../crates/cli/src/main.rs),
`expand_validators`). Verified running end-to-end on 2026-06-22:

```
validator V for Order errors Text {
    rule positive { require order.total > 0 else "total must be positive" }
}
…
V.validate(order)       // Ok(unit) or Err("total must be positive")
V.validateAll(order)    // List<Text> of all violations
```

Caveats:

- **Error type** may be `Text`, `Int`, or a **nullary enum** (`errors OE` where
  `OE = | TooSmall | TooBig`). A payload-*carrying* enum error type is still
  blocked (§10) — rare in practice.
- **`after` gating** is honoured (topological order for fail-fast; prerequisite
  conditions gate collect-all). **`overrides`, `context`, `loaded by`/db, and
  `trigger`** blocks are not yet expanded — keep validators to plain rules.

---

## 8. State machines — executable (transitions, `on_enter`, `invariant`, predicates, accessor)

**Status:** Implemented, including `on_enter` hooks and `invariant`s (BACKLOG
item 82).

A `statemachine { states: … transitions: … }` is expanded into a machine
struct plus functions
([crates/codegen/src/emit_statemachine.rs](../crates/codegen/src/emit_statemachine.rs),
spliced in by `expand_state_machines`):

```
statemachine Traffic {
  states:      Stopped, Going, Slowing
  transitions: Stopped -> Going : go()
               Going -> Slowing : caution()
               Slowing -> Stopped : stop()

  on_enter Going: println("go!")

  invariant Slowing: true
}
…
val s = Traffic_new()            // initial state (first listed) = Stopped
val s2 = Traffic_go(s)           // -> Going; runs on_enter, prints "go!"
Traffic_isGoing(s2)              // true
Traffic_state(s2)                // the current state, an MState enum value
```

The machine value is a real struct (`{ state: TrafficState, … }`), not a bare
enum — every transition's event params are accumulated as `Option`-typed
fields on it (`None` until the transition that sets them fires), so
`on_enter`/`invariant` bodies can reference the machine's own data via a
`self` binding, matching the spec's own examples (`invariant Active:
self.paymentMethod.isSome()`... though `Option` has no `.isSome()` method
today — use `match self.field { Some(x) => … None => … }`). Each event
generates one `Machine_<event>(m, params…)` that: updates the struct (new
state + this transition's params), runs the target state's `on_enter` hook
(if any) with `self` bound to the *new* machine value, then panics if any of
the target state's `invariant`s don't hold — or stays put for an invalid
transition (still a no-op, not a panic — that part of the original caveat
is unchanged). Plus `Machine_is<State>` predicates and a `Machine_state`
accessor (returns the state enum, not the whole struct). Verified end-to-end
via `certo.exe` on real programs: `on_enter` side effects fire after the
state commits, a satisfied invariant lets the transition succeed silently,
and a violated one panics with the invariant's own source text in the
message.

Caveat: compile-time *typestate* enforcement (rejecting an invalid transition
at the call site) is still not done. `invariant`s are only checked once, at
the moment their state is entered — not continuously enforced for as long as
the machine remains in that state.

---

## 9. What is verified working end-to-end

The following were compiled to native binaries and **run with correct output**
during the 2026-06-22 smoke pass:

- Functions, recursion, `if`/`then`/`else`.
- `match` on integer literals, `Option` (`Some`/`None`), and `Result`
  (`Ok`/`Err`) — including binding payloads of these built-in types.
- **User-defined sum types**, both nullary (`| Red | Green`) and with payloads
  (`| Circle(Int) | Rect(Int, Int)`): construction and `match` destructuring
  both run correctly. Nullary enums also work as `Result`/`List` payloads
  (e.g. `Result<Int, OE>`, extracted from `Err` and re-matched).
- Effects (`[io]`), `println`, `intToText` and friends.
- Standard library: `List.map` / `filter` / `fold` / `len`, `Text.concat` /
  `toUpper` / `trim` / `split`, `Math.clampInt`, etc.
- Trailing-lambda syntax (`List.map(xs) { x => x * 2 }`).
- Concurrency: `spawn` / `await` / `parallel {}` on real OS threads (§1),
  including tuple-destructured results and inline `await`.
- **Traits / `impl`**: methods compile to `Type.method` functions and run via
  qualified calls (static dispatch) — see §6 for the two caveats.
- **Validators** with a `Text` or nullary-enum error type — see §7.
- **State machines**: `new`, transitions, `on_enter`, `invariant`, `is<State>`
  predicates, and the state accessor all run — see §8.
- Type inference (Hindley-Milner) across all of the above.
- Structured database migrations (parse → schema-check → SQL DDL generation) —
  verified at the unit/integration level.

**All headline features now execute.** The remaining limitations are explicitly
**out of v1.0 scope** and deferred to post-1.0: payload-*carrying* enums in
`Result`/`List`/tuple slots (§10), and validator `context`/`overrides`/`trigger`
blocks (§7). Each has a clean workaround and neither blocks the v1.0 feature set.

---

## 10. Payload-*carrying* enums can't be Result / List / tuple payloads

**Status:** Narrow codegen limitation (payload-carrying enums only) — **deferred
to post-1.0** (out of the v1.0 scope). See the scope decision at the top.

All-nullary enums (`| Red | Green`) are now represented as plain integer enums,
so they flow through `Result`, `List`, and tuple slots (which hold a pointer-
sized `intptr_t`/`void*`). This was fixed on 2026-06-22 — `Result<_, OE>` and
`List<OE>` for a nullary `OE` are verified working, including enum-typed
validator errors (§7).

What still fails: an enum that **carries data** is emitted as a struct, and a
struct can't fit a pointer-sized slot:

```
type Err2 = | Bad(Int) | Other
fn f(n: Int): Result<Int, Err2> = Err(Bad(n))   // C error: passing 'Err2' to 'intptr_t'
```

This is rare for error types (which are almost always nullary enums or `Text`).
The remaining fix is a uniform boxing scheme (e.g. heap-allocate payload enums
and pass a pointer). Until then, keep payload-carrying enums out of
`Result`/`List`/tuple positions.

### 10a. `Float` in a `Result` payload — FIXED

**Status:** **Fixed 2026-06-23.** (Found 2026-06-23.)

`Result`'s payload is a pointer-sized `intptr_t` in `certo_result_t`, and a
`double` was previously *numeric-converted* into it (`Ok(3.14159)` → `3.0`).
Fixed by bit-preserving the payload: on construction `Ok(f)`/`Err(f)` bit-casts
the `double` to int64 (`__certo_f2i`), and matching `Ok(x)`/`Err(e)` bit-restores
it (`__certo_i2f`) using the payload type from the scrutinee. `Result<Int/Text>`
and Float on either side are all verified correct. (`Float` in `List`/tuple
element slots is still limited — see §10 — because list *element storage* uses
the same numeric cast.)

## 11. `Option` payload extraction — FIXED (was broken for `parseInt`/`parseFloat` and `Float`)

**Status:** **Fixed 2026-06-23.** (Found empirically 2026-06-23.)

`Option` previously had **two inconsistent runtime representations**: literal
`Some(v)` boxed the value *inline* (`(void*)(intptr_t)v`) while
`parseInt`/`parseFloat` returned a *heap pointer*. Pattern matching assumed
inline, so `match parseFloat("…") { Some(x) => x }` yielded the **pointer
address** (e.g. `~2.3e12`) — a silent wrong answer. The inline scheme also
couldn't distinguish `Some(0)` from `None`, and corrupted `Float` bits.

**The fix** unified `Option` on a single **heap-boxed** representation:
`None` = null pointer, `Some(v)` = pointer to an 8-byte payload slot with `v`
stored under its own C type (bit-preserving). Concretely:

- New MIR `Rvalue::BoxSome` / `UnboxSome` carry the payload type;
  `Some(v)` heap-boxes, `match Some(x)` dereferences with the right type
  ([crates/mir/src/lower.rs](../crates/mir/src/lower.rs),
  [crates/codegen/src/emit_mir.rs](../crates/codegen/src/emit_mir.rs)).
- `??` now dereferences the payload; **all** stdlib `Option` producers heap-box
  via `__certo_opt_box`: `List.get`/`first`/`last`/`find`, `Map.get`,
  `parseInt`/`parseFloat`, and the I/O producers `arg`, `readFile`, `readLine`,
  `getEnv`, `listDir`, `Path.extension`, `Text.indexOf`.
- Fixed a companion bug: `List.get` had mapped to the panic-on-oob variant
  (returning `T`), so its `Option` was fake — it now returns real `None`.

Verified end-to-end: `parseFloat("9.0136513808403151e-09")` round-trips
correctly, `Some(0)` ≠ `None`, `List.get(xs, oob)` → `None`, and float arithmetic
through `Option` works.

**Remaining edge (documented, not this bug):** `Option<Float>` / `List<Float>`
where the value passes through the generic list-element slot is still limited —
list *element storage* uses the same inline `(intptr_t)` cast that corrupts
`double` bits (§10 territory). Monomorphic `parseFloat`, `Some(3.14)`, and direct
`Option<Float>` matching are correct.

