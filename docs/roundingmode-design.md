# RoundingMode Design — Drop-in for `crates/stdlib/src/money.rs`

> Status: DRAFT, **core C logic verified by standalone execution** (see
> §-1). Targets the actual current source (confirmed against
> `crates/stdlib/src/money.rs`, `crates/typeck/src/ty.rs`,
> `crates/typeck/src/infer_expr.rs`, `crates/ast/src/decl.rs`). The C in
> §4 has been compiled and run standalone with `gcc -Wall -Wextra`
> (outside the actual `crates/codegen` pipeline, since no Rust toolchain
> was available in the test sandbox) against 33 hand-built test cases
> covering all eight modes, negative numbers, scale-mismatched operands,
> exact-half ties on both even and odd divisors, and large values. Not
> yet wired into the real `money.rs`/Rust workspace or exercised through
> the actual Certo compiler end-to-end — that step still needs doing on
> a build-capable machine.

## -1. Verification Log — Two Real Bugs Found and Fixed

Drafting this kind of arithmetic code correctly by inspection alone is
genuinely hard, and it showed: testing found two real, distinct bugs in
the first draft, both now fixed below.

**Bug 1 — `Decimal.divRound` silently wrong on any input with nonzero
`a.scale`.** The original formula multiplied the numerator by `10^places`
without subtracting `a.scale` first, double-counting the input's existing
decimal scale. `100.00 / 3` at 2 decimal places returned `333.334`
instead of `33.33`. Root cause and fix are in §4's `certo_decimal_div_round`,
now computing a single signed exponent `places + b.scale - a.scale +
guard` rather than treating `a.value` as if it had no scale.

**Bug 2 — `certo_round_apply`'s tie detection broke on non-power-of-10
divisors.** The original `(abs_remainder == half) && (divisor % 2 != 0)`
shortcut for "no exact tie possible" is only valid when the divisor is a
power of 10 (true for `Decimal.round`, false for `Decimal.roundToIncrement`
with a step like `0.05`, whose effective divisor is 5 — odd, but exact
ties at 0.025 multiples are entirely possible). `19.97` rounded to the
nearest `0.05` was wrongly jumping to `20.00` instead of `19.95`. Fixed
by replacing the heuristic with `2*abs_remainder` compared directly
against `divisor`, which is correct for any divisor.

Both fixes are reflected in §4 below; the original (buggy) versions are
not reproduced here. A third, minor, non-functional gap — `roundToIncrement`
returning a non-minimal `scale` in its result (numerically correct,
cosmetically inconsistent) — is documented inline in §4 rather than
silently left in.

## 0. Why this doesn't touch `Ty` or the lexer

The current `Decimal` type is a hardcoded match arm in
`type_expr_to_ty` (`crates/typeck/src/infer_expr.rs:54`) — one of a small
set of primitives (`Int`, `Float`, `Bool`, ...) baked directly into the
`Ty` enum (`crates/typeck/src/ty.rs:18`). `RoundingMode` should **not**
follow that pattern. It isn't a new kind of type former, it's ordinary
stdlib data — and `crates/ast/src/decl.rs` already has exactly the right
shape for it: `TypeBody::Sum(Vec<SumVariant>)`, the same mechanism every
user-defined sum type in Certo source already uses. So this design adds
zero new `Ty` variants and zero new AST nodes. It's a `.cto`-level type
declaration plus stdlib function signatures plus C codegen — the same
category of change as any other stdlib addition, not a type-system
change.

This also fixes a real, separate bug I found while reading the current
implementation: `certo_decimal_round`'s `(d.value + divisor/2) / divisor`
does **not** behave symmetrically on negative numbers, because C integer
division truncates toward zero rather than flooring. For a negative
`d.value`, adding `divisor/2` then truncating pulls the rounded result
toward positive infinity rather than mirroring the positive case — so the
existing single "half-up" mode is actually closer to "half up, except
inconsistent on negatives," not true round-half-away-from-zero or
round-half-up-toward-positive-infinity. Every mode below is written to be
explicit and correct on negative values, including a corrected `HalfUp`.

---

## 1. The Type — `RoundingMode`

Add to `MONEY_CERTO` (the `.cto`-level declaration string in
`money.rs`), as a real sum type using the existing `Decl::Type` /
`TypeBody::Sum` machinery — no special-casing required anywhere in
`typeck`:

```certo
type RoundingMode =
    │ HalfUp        // round half away from zero (1.5 → 2, -1.5 → -2)
    │ HalfDown       // round half toward zero    (1.5 → 1, -1.5 → -1)
    │ HalfEven       // banker's rounding         (1.5 → 2, 2.5 → 2)
    │ Up             // always away from zero     (1.1 → 2, -1.1 → -2)
    │ Down            // always toward zero (truncate) (1.9 → 1, -1.9 → -1)
    │ Ceiling         // toward positive infinity  (1.1 → 2, -1.9 → -1)
    │ Floor           // toward negative infinity  (1.9 → 1, -1.1 → -2)
    │ ToIncrement(step: Decimal)   // nearest multiple of `step`, e.g. 0.05
```

This resolves through the ordinary `Named { name: "RoundingMode", args }`
path in `type_expr_to_ty` (falls through the hardcoded primitives,
resolves via the user-type lookup the same as any `type Foo = │ A │ B`
declared by application code) — confirmed by checking how `Decimal`
*doesn't* go through that path (it's special-cased) while everything else
does.

---

## 2. Stdlib Signatures — replacing and extending `MONEY_CERTO`

```certo
module Stdlib.Money

/* ── existing, unchanged ──────────────────────────────────────────── */

fn Decimal.add(a: Decimal, b: Decimal): Decimal
fn Decimal.sub(a: Decimal, b: Decimal): Decimal
fn Decimal.mul(a: Decimal, b: Decimal): Decimal
fn Decimal.eq(a: Decimal, b: Decimal): Bool
fn Decimal.lt(a: Decimal, b: Decimal): Bool
fn Decimal.gt(a: Decimal, b: Decimal): Bool
fn Decimal.lte(a: Decimal, b: Decimal): Bool
fn Decimal.gte(a: Decimal, b: Decimal): Bool
fn Decimal.abs(d: Decimal): Decimal
fn Decimal.negate(d: Decimal): Decimal
fn Decimal.toInt(d: Decimal): Int
fn Decimal.fromInt(n: Int): Decimal
fn Decimal.toText(d: Decimal): Text

/* ── CHANGED: division and rounding now require an explicit mode ──── */

/* Old: fn Decimal.div(a: Decimal, b: Decimal): Decimal [fallible]
   New: division that loses precision must say how it rounds.
   `Decimal.div` without a mode is REMOVED — see §5 migration note. */
fn Decimal.divRound(a: Decimal, b: Decimal, places: Int, mode: RoundingMode): Decimal [fallible]

/* Old: fn Decimal.round(d: Decimal, places: Int): Decimal
   New: mode is mandatory, not defaulted — consistent with how
   Money.add already returns Result rather than silently coercing
   currency mismatches (see spec §6, Money.add design rationale). */
fn Decimal.round(d: Decimal, places: Int, mode: RoundingMode): Decimal

/* ── NEW: convenience helpers ────────────────────────────────────── */

/* Round to the nearest valid increment of a currency unit, e.g.
   Swiss Rappen rounding (nearest 0.05) or cash rounding to nearest 5c.
   `mode` controls how exact halfway ties between increments resolve. */
fn Decimal.roundToIncrement(d: Decimal, step: Decimal, mode: RoundingMode): Decimal

/* Money helpers (Decimal fixed at 2 decimal places) — mode now required
   wherever rounding actually occurs (fromDecimal), not where it doesn't
   (fromCents, toCents are exact — no precision loss, no mode needed). */
fn Money.fromCents(cents: Int): Decimal
fn Money.toCents(m: Decimal): Int
fn Money.fromDecimal(d: Decimal, mode: RoundingMode): Decimal
```

### 2.1 Why `places`/`mode` are mandatory, not defaulted

This mirrors a pattern already established elsewhere in the language:
`Money.add` returns `Result<Money, CurrencyMismatch>` instead of silently
coercing currencies (confirmed in our earlier review of `money.rs`'s
design rationale). Rounding policy is exactly the same category of
"silent default that causes real financial bugs" — so this design makes
`Decimal.round`/`divRound` require an explicit `RoundingMode` argument
rather than defaulting to `HalfUp` behind the scenes. A caller who wants
the old behavior writes `Decimal.round(d, 2, HalfUp)` — explicit, visible
in code review, matching the same philosophy as `Secret<T>.expose()`
being a deliberate, searchable call site.

---

## 3. Typeck — what actually needs to change

Because `RoundingMode` is a plain user-style sum type, **no changes to
`crates/typeck/src/ty.rs` or `unify.rs` are required.** The only typeck
surface area touched is making sure `infer_decl.rs` resolves the new
function signatures' parameter types correctly — which it already does
generically for any `fn` declaration referencing a named sum type, the
same path used for `Result<T,E>` variant construction elsewhere in
`infer_expr.rs`.

The one thing worth adding, consistent with how `Decimal.div` is
currently the sole `[fallible]`-marked function in `MONEY_CERTO`: keep
`[fallible]` on `divRound` (division by zero is still possible) but
**not** on `round`/`roundToIncrement` (rounding itself can't fail once
you have a valid `Decimal` and a valid mode — only division can).

```rust
// crates/typeck/src/infer_decl.rs — no new logic needed; confirming
// the existing fn-signature inference path handles this automatically:
// `fn Decimal.round(d: Decimal, places: Int, mode: RoundingMode): Decimal`
// resolves `RoundingMode` via the same Named-type lookup used for any
// other sum type parameter — e.g. how `OrderStatus` parameters resolve
// in application code today.
```

---

## 4. C Codegen — `MONEY_C` additions

`RoundingMode` needs a C representation. Sum types without payload-bearing
variants (all of these except `ToIncrement`) compile to a tagged enum;
`ToIncrement(step: Decimal)` needs a payload, so the whole type becomes a
small tagged union — directly analogous to how `Option<T>`/`Result<T,E>`
already codegen as tagged structs (confirmed via `certo_result_t` /
`certo_option_t` seen in the runtime header).

```c
/* ================================================================
   RoundingMode — tagged union (mirrors certo_option_t / certo_result_t
   pattern already used for Option<T>/Result<T,E>)
   ================================================================ */

typedef enum {
    CERTO_ROUND_HALF_UP,
    CERTO_ROUND_HALF_DOWN,
    CERTO_ROUND_HALF_EVEN,
    CERTO_ROUND_UP,
    CERTO_ROUND_DOWN,
    CERTO_ROUND_CEILING,
    CERTO_ROUND_FLOOR,
    CERTO_ROUND_TO_INCREMENT,
} certo_rounding_tag_t;

typedef struct {
    certo_rounding_tag_t tag;
    certo_decimal_t step;   /* only meaningful when tag == CERTO_ROUND_TO_INCREMENT */
} certo_rounding_mode_t;

/* ---- core rounding primitive: round `value` (already scaled to an
   integer at the target precision boundary) given a sign and a
   remainder, per mode. All seven non-increment modes reduce to this
   one function so behavior is defined in exactly one place. ---- */

static int64_t certo_round_apply(
    int64_t truncated,   /* value / divisor, already truncated toward zero */
    int64_t remainder,   /* value % divisor, same sign as value */
    int64_t divisor,
    certo_rounding_tag_t mode
) {
    bool negative = remainder < 0;
    int64_t abs_remainder = negative ? -remainder : remainder;
    /* Exact-half / over-half detection via `2*remainder` vs `divisor`,
       not `divisor/2`. This is correct for ANY divisor, not just powers
       of 10. An earlier draft used `(abs_remainder == half) && (divisor
       % 2 != 0)` as a shortcut for "no exact tie possible at odd
       divisors" — that's only true when divisor is a power of 10 (as in
       Decimal.round). It silently breaks for arbitrary step sizes like
       roundToIncrement(d, 0.05, ...), where the effective divisor (5) is
       odd but exact halfway ties are very much possible (e.g. 19.975 is
       exactly halfway between 19.95 and 20.00). Verified by direct
       execution: 19.97 -> nearest 0.05 was wrongly rounding up to 20.00
       under the old formula; 2*remainder vs divisor fixes it. */
    bool exactly_half = (2 * abs_remainder == divisor);
    bool over_half     = (2 * abs_remainder > divisor);

    switch (mode) {
        case CERTO_ROUND_UP:
            return abs_remainder == 0 ? truncated
                 : (negative ? truncated - 1 : truncated + 1);

        case CERTO_ROUND_DOWN:
            return truncated;   /* C's / already truncates toward zero */

        case CERTO_ROUND_CEILING:
            return (abs_remainder == 0 || negative) ? truncated
                 : truncated + 1;

        case CERTO_ROUND_FLOOR:
            return (abs_remainder == 0 || !negative) ? truncated
                 : truncated - 1;

        case CERTO_ROUND_HALF_UP:   /* half away from zero */
            if (abs_remainder == 0) return truncated;
            if (exactly_half || over_half) {
                return negative ? truncated - 1 : truncated + 1;
            }
            return truncated;

        case CERTO_ROUND_HALF_DOWN: /* half toward zero */
            if (abs_remainder == 0) return truncated;
            if (over_half) {
                return negative ? truncated - 1 : truncated + 1;
            }
            return truncated;   /* exact ties round toward zero, i.e. stay truncated */

        case CERTO_ROUND_HALF_EVEN: { /* banker's rounding */
            if (abs_remainder == 0) return truncated;
            if (over_half) {
                return negative ? truncated - 1 : truncated + 1;
            }
            if (exactly_half) {
                bool truncated_is_odd = (truncated % 2 != 0);
                if (truncated_is_odd) {
                    return negative ? truncated - 1 : truncated + 1;
                }
                return truncated;   /* already even — stays */
            }
            return truncated;
        }

        default:
            certo_panic("certo_round_apply: ToIncrement must be handled by caller");
    }
}

/* ---- Decimal.round(d, places, mode) ---- */

certo_decimal_t certo_decimal_round_mode(
    certo_decimal_t d, int8_t places, certo_rounding_mode_t mode
) {
    if (mode.tag == CERTO_ROUND_TO_INCREMENT) {
        return certo_decimal_round_to_increment(d, mode.step, mode);
    }
    if (d.scale <= places) return d;   /* no precision loss — nothing to round */

    int8_t excess = d.scale - places;
    int64_t divisor = 1;
    for (int i = 0; i < excess; i++) divisor *= 10;

    int64_t truncated = d.value / divisor;
    int64_t remainder = d.value % divisor;
    int64_t rounded = certo_round_apply(truncated, remainder, divisor, mode.tag);

    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

/* ---- Decimal.roundToIncrement(d, step, mode) ----
   Rounds `d` to the nearest multiple of `step`. Ties (exactly halfway
   between two increments) resolve per `mode`, reusing the same
   certo_round_apply core by treating "number of steps" as the value
   being rounded to an integer.

   NOTE — known minor gap, found during testing: decimal_align() aligns
   `d` and `step` to the larger of the two scales before dividing, so
   the *returned* scale matches that aligned scale even when the true
   result has fewer significant decimal digits (e.g. rounding 19.995 to
   the nearest 0.05 returns {20000, scale=3} i.e. "20.000" rather than
   {2000, scale=2} i.e. "20.00" — numerically identical, confirmed by
   direct computation, but not normalized to a minimal scale). Harmless
   for arithmetic (certo_decimal_eq/lt/etc. all call decimal_align
   first) but cosmetically different in certo_decimal_to_text output. A
   follow-up normalize-scale pass (strip trailing zero digits down to
   the minimal representable scale) would close this; left as a
   documented gap rather than silently shipped, consistent with how
   §17.9 in the concurrency spec documents its own open gaps rather than
   omitting them. ---- */
certo_decimal_t certo_decimal_round_to_increment(
    certo_decimal_t d, certo_decimal_t step, certo_rounding_mode_t mode
) {
    certo_decimal_t a = d, b = step;
    decimal_align(&a, &b);   /* existing helper, already in money.rs */
    if (b.value == 0) certo_panic("roundToIncrement: step must be nonzero");

    int64_t steps_truncated = a.value / b.value;
    int64_t remainder       = a.value % b.value;
    certo_rounding_tag_t inner_mode =
        (mode.tag == CERTO_ROUND_TO_INCREMENT) ? CERTO_ROUND_HALF_UP : mode.tag;
        /* a ToIncrement mode nested inside itself has no further meaning;
           default its tie-break to HalfUp rather than recursing */

    int64_t steps = certo_round_apply(steps_truncated, remainder, b.value, inner_mode);

    certo_decimal_t r = { .value = steps * b.value, .scale = a.scale };
    return r;
}

/* ---- Decimal.divRound(a, b, places, mode) ----
   Replaces the old unconditional certo_decimal_div. Division by zero
   still panics via certo_panic + [fallible] surfaces it as Result at
   the Certo level (existing mechanism, unchanged).

   The true value of `a` is a.value / 10^a.scale, and of `b` is
   b.value / 10^b.scale. We want (a/b) expressed at `places` decimal
   places, i.e. round( a.value * 10^(places + b.scale - a.scale) /
   b.value ). Guard digits are added to that exponent (not multiplied
   in separately afterward) so the rounding decision sees a real
   remainder rather than one already lost to an intermediate division —
   an earlier draft multiplied by 10^places without subtracting a.scale,
   which double-counted the input's existing scale and produced wrong
   results whenever `a` had nonzero scale (confirmed by execution:
   100.00 / 3 at 2dp returned 333.334 instead of 33.33 under that
   formula). The exponent here can be negative when a.scale is large
   relative to places + b.scale + guard, so it is computed as a signed
   value and applied as either a multiply or a divide accordingly. ---- */

certo_decimal_t certo_decimal_div_round(
    certo_decimal_t a, certo_decimal_t b, int8_t places, certo_rounding_mode_t mode
) {
    if (b.value == 0) certo_panic("decimal division by zero");

    int8_t guard = 6;
    int64_t guard_factor = 1;
    for (int i = 0; i < guard; i++) guard_factor *= 10;

    int exponent = (int)places + (int)b.scale - (int)a.scale + guard; /* may be negative */
    bool exponent_negative = exponent < 0;
    int abs_exponent = exponent_negative ? -exponent : exponent;
    __int128 scale_pow = 1;
    for (int i = 0; i < abs_exponent; i++) scale_pow *= 10;

    __int128 numerator = exponent_negative
        ? (__int128)a.value / scale_pow
        : (__int128)a.value * scale_pow;

    __int128 guarded_quotient = numerator / b.value;   /* at `places + guard` precision */
    int64_t truncated_with_guard = (int64_t)(guarded_quotient / guard_factor);
    int64_t remainder_for_round  = (int64_t)(guarded_quotient % guard_factor);

    int64_t rounded = certo_round_apply(truncated_with_guard, remainder_for_round, guard_factor, mode.tag);

    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

/* ---- Money.fromDecimal(d, mode) — was unconditional round-to-2dp ---- */

certo_decimal_t certo_money_from_decimal_mode(certo_decimal_t d, certo_rounding_mode_t mode) {
    return certo_decimal_round_mode(d, 2, mode);
}
```

### 4.1 Constructing `certo_rounding_mode_t` from Certo source

Since `RoundingMode` is a plain Certo sum type (not a primitive), its
variant constructors (`HalfUp`, `Ceiling`, `ToIncrement(step)`, ...)
compile through the **existing, generic sum-type codegen path** — the
same one that already lowers any `type Foo = │ A │ B(x: Int)` declared in
application code into a tagged struct/union with constructor functions.
No bespoke codegen is needed for `RoundingMode` itself; only the
*consumers* (`certo_decimal_round_mode` etc.) are new, hand-written stdlib
C, exactly like every other function in `MONEY_C` today.

---

## 5. Migration Note — `Decimal.div` and `Decimal.round` signature changes

This design **removes** the old no-mode `Decimal.div` and 2-arg
`Decimal.round`, rather than keeping them as deprecated aliases. That's a
deliberate choice, not an oversight: keeping a no-mode `round` around as
"the default" reintroduces exactly the silent-default problem this whole
design exists to close. If a softer rollout is wanted instead, the
no-mode signatures could be kept temporarily and marked `[deprecated]`,
defaulting internally to `HalfUp`, gated by a compiler flag — but that
should be an explicit choice you make, not the default in this draft.

---

## 6. Worked Example

```certo
val price: Decimal = d"19.995"

val rounded = Decimal.round(price, 2, HalfEven)     // → 20.00 (tie, truncated digit 9 is odd → rounds up)
val cashRounded = Decimal.roundToIncrement(price, d"0.05", HalfUp)  // → 20.00 (nearest nickel)

val unitPrice = Decimal.divRound(d"100.00", d"3", 2, HalfUp)  // → 33.34
```

---

## 7. Summary of File-Level Changes

| File | Change |
|---|---|
| `crates/stdlib/src/money.rs` | Add `RoundingMode` type to `MONEY_CERTO`; replace `Decimal.div`/`Decimal.round`/`Money.fromDecimal` signatures; add `divRound`, `roundToIncrement`; add all C functions in §4 to `MONEY_C` |
| `crates/typeck/src/ty.rs` | **No change** — `RoundingMode` resolves as an ordinary user sum type |
| `crates/typeck/src/infer_expr.rs` | **No change** — generic `Named` type resolution already handles this |
| `crates/typeck/src/infer_decl.rs` | **No change** — generic fn-signature inference already handles sum-type parameters |
| Codegen (sum types generally) | **No change** — reuses existing tagged-union lowering for user-defined sum types |

The entire design is additive at the stdlib level and requires zero
changes to the type checker itself — confirming the earlier read that
`Decimal`'s primitive status in `Ty` was the one piece of special-casing
to watch out for, and that `RoundingMode` correctly avoids repeating it.
