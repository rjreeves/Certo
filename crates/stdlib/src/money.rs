/// C implementations for `Stdlib.Money` and extended `Decimal` operations.
pub const MONEY_C: &str = r#"
/* ================================================================
   Stdlib.Money  (and extended Decimal arithmetic)

   Decimal layout: { int64_t value; int8_t scale; }
   where the true value = value / 10^scale.
   e.g. $19.99 → { value: 1999, scale: 2 }
   ================================================================ */

/* ---- Windows compiler-rt shims ----
   lld-link does not ship __divti3 / __modti3 (128-bit signed division).
   Provide them here so the decimal division code links on Windows. */
#if defined(_WIN32) && (defined(__clang__) || defined(__GNUC__))
typedef unsigned __int128 certo__u128;

static certo__u128 certo__udiv128(certo__u128 n, certo__u128 d) {
    if (d == 0) return 0;
    certo__u128 q = 0, r = 0;
    for (int i = 127; i >= 0; i--) {
        r = (r << 1) | ((n >> i) & 1);
        if (r >= d) { r -= d; q |= (certo__u128)1 << i; }
    }
    return q;
}

__int128 __divti3(__int128 a, __int128 b) {
    int s = (a < 0) ^ (b < 0);
    certo__u128 ua = a < 0 ? -(certo__u128)a : (certo__u128)a;
    certo__u128 ub = b < 0 ? -(certo__u128)b : (certo__u128)b;
    certo__u128 q  = certo__udiv128(ua, ub);
    return s ? -(__int128)q : (__int128)q;
}

__int128 __modti3(__int128 a, __int128 b) {
    return a - __divti3(a, b) * b;
}
#endif

/* ---- Decimal parse / format ---- */

certo_decimal_t certo_decimal_parse(const char* s) {
    if (!s) { certo_decimal_t z = {0, 0}; return z; }
    /* Find decimal point */
    const char* dot = strchr(s, '.');
    int8_t scale = 0;
    int64_t value;
    if (dot) {
        /* Count decimal places */
        scale = (int8_t)strlen(dot + 1);
        /* Parse integer part + fractional part as one integer */
        char buf[64];
        size_t int_len = (size_t)(dot - s);
        if (int_len >= sizeof(buf) - 20) { certo_decimal_t z = {0, 0}; return z; }
        memcpy(buf, s, int_len);
        strcpy(buf + int_len, dot + 1);
        value = strtoll(buf, NULL, 10);
    } else {
        value = strtoll(s, NULL, 10);
    }
    certo_decimal_t d = { .value = value, .scale = scale };
    return d;
}

certo_text_t certo_decimal_to_text(certo_decimal_t d) {
    char* buf = (char*)malloc(64);
    if (!buf) certo_panic("out of memory");
    if (d.scale == 0) {
        snprintf(buf, 64, "%" PRId64, d.value);
    } else {
        int64_t divisor = 1;
        for (int i = 0; i < d.scale; i++) divisor *= 10;
        int64_t whole    = d.value / divisor;
        int64_t frac     = d.value % divisor;
        if (frac < 0) frac = -frac;
        char fmt[16];
        snprintf(fmt, sizeof(fmt), "%%" PRId64 ".%%0%d" PRId64, (int)d.scale);
        snprintf(buf, 64, fmt, whole, frac);
    }
    return buf;
}

/* Align two decimals to the same scale */
static void decimal_align(certo_decimal_t* a, certo_decimal_t* b) {
    while (a->scale < b->scale) { a->value *= 10; a->scale++; }
    while (b->scale < a->scale) { b->value *= 10; b->scale++; }
}

certo_decimal_t certo_decimal_add(certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b);
    certo_decimal_t r = { .value = a.value + b.value, .scale = a.scale };
    return r;
}

certo_decimal_t certo_decimal_sub(certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b);
    certo_decimal_t r = { .value = a.value - b.value, .scale = a.scale };
    return r;
}

certo_decimal_t certo_decimal_mul(certo_decimal_t a, certo_decimal_t b) {
    certo_decimal_t r = {
        .value = a.value * b.value,
        .scale = (int8_t)(a.scale + b.scale),
    };
    return r;
}


bool certo_decimal_eq (certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b); return a.value == b.value;
}
bool certo_decimal_lt (certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b); return a.value < b.value;
}
bool certo_decimal_gt (certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b); return a.value > b.value;
}
bool certo_decimal_lte(certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b); return a.value <= b.value;
}
bool certo_decimal_gte(certo_decimal_t a, certo_decimal_t b) {
    decimal_align(&a, &b); return a.value >= b.value;
}

certo_decimal_t certo_decimal_abs(certo_decimal_t d) {
    certo_decimal_t r = { .value = d.value < 0 ? -d.value : d.value, .scale = d.scale };
    return r;
}

certo_decimal_t certo_decimal_negate(certo_decimal_t d) {
    certo_decimal_t r = { .value = -d.value, .scale = d.scale };
    return r;
}

/* ================================================================
   RoundingMode — tagged union (mirrors certo_option_t / certo_result_t
   pattern used for Option<T>/Result<T,E>)
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

/* Core rounding primitive: all seven non-increment modes reduce to this
   one function so the tie-break logic is defined in exactly one place.
   Uses 2*abs_remainder vs divisor (correct for any divisor, not just
   powers of 10 — the power-of-10 shortcut is wrong for roundToIncrement
   with non-power-of-10 steps like 0.05). */
static int64_t certo_round_apply(
    int64_t truncated,
    int64_t remainder,
    int64_t divisor,
    certo_rounding_tag_t mode
) {
    bool negative = remainder < 0;
    int64_t abs_remainder = negative ? -remainder : remainder;
    bool exactly_half = (2 * abs_remainder == divisor);
    bool over_half    = (2 * abs_remainder >  divisor);

    switch (mode) {
        case CERTO_ROUND_UP:
            return abs_remainder == 0 ? truncated
                 : (negative ? truncated - 1 : truncated + 1);

        case CERTO_ROUND_DOWN:
            return truncated;

        case CERTO_ROUND_CEILING:
            return (abs_remainder == 0 || negative) ? truncated
                 : truncated + 1;

        case CERTO_ROUND_FLOOR:
            return (abs_remainder == 0 || !negative) ? truncated
                 : truncated - 1;

        case CERTO_ROUND_HALF_UP:
            if (abs_remainder == 0) return truncated;
            if (exactly_half || over_half)
                return negative ? truncated - 1 : truncated + 1;
            return truncated;

        case CERTO_ROUND_HALF_DOWN:
            if (abs_remainder == 0) return truncated;
            if (over_half)
                return negative ? truncated - 1 : truncated + 1;
            return truncated;

        case CERTO_ROUND_HALF_EVEN: {
            if (abs_remainder == 0) return truncated;
            if (over_half)
                return negative ? truncated - 1 : truncated + 1;
            if (exactly_half) {
                bool truncated_is_odd = (truncated % 2 != 0);
                if (truncated_is_odd)
                    return negative ? truncated - 1 : truncated + 1;
                return truncated;
            }
            return truncated;
        }

        default:
            certo_panic("certo_round_apply: ToIncrement must be handled by caller");
    }
}

/* Decimal.round(d, places, mode) */
certo_decimal_t certo_decimal_round_mode(
    certo_decimal_t d, int8_t places, certo_rounding_mode_t mode
) {
    if (mode.tag == CERTO_ROUND_TO_INCREMENT)
        certo_panic("Decimal.round: use Decimal.roundToIncrement for ToIncrement mode");
    if (d.scale <= places) return d;

    int8_t excess = d.scale - places;
    int64_t divisor = 1;
    for (int i = 0; i < excess; i++) divisor *= 10;

    int64_t truncated = d.value / divisor;
    int64_t remainder = d.value % divisor;
    int64_t rounded   = certo_round_apply(truncated, remainder, divisor, mode.tag);

    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

/* Decimal.roundToIncrement(d, step, mode)
   NOTE: returned scale matches the aligned scale, not the minimal scale
   (e.g. 19.995 rounded to 0.05 returns scale=3 "20.000" not scale=2
   "20.00"). Numerically identical; cosmetically non-minimal. */
certo_decimal_t certo_decimal_round_to_increment(
    certo_decimal_t d, certo_decimal_t step, certo_rounding_mode_t mode
) {
    certo_decimal_t a = d, b = step;
    decimal_align(&a, &b);
    if (b.value == 0) certo_panic("Decimal.roundToIncrement: step must be nonzero");

    int64_t steps_truncated = a.value / b.value;
    int64_t remainder       = a.value % b.value;
    certo_rounding_tag_t inner_mode =
        (mode.tag == CERTO_ROUND_TO_INCREMENT) ? CERTO_ROUND_HALF_UP : mode.tag;

    int64_t steps = certo_round_apply(steps_truncated, remainder, b.value, inner_mode);

    certo_decimal_t r = { .value = steps * b.value, .scale = a.scale };
    return r;
}

/* Decimal.divRound(a, b, places, mode)
   Exponent computed as (places + b.scale - a.scale + guard) — a single
   signed value that correctly accounts for a's existing scale, avoiding
   the double-counting bug present in the original certo_decimal_div. */
certo_decimal_t certo_decimal_div_round(
    certo_decimal_t a, certo_decimal_t b, int8_t places, certo_rounding_mode_t mode
) {
    if (b.value == 0) certo_panic("decimal division by zero");

    int8_t guard = 6;
    int64_t guard_factor = 1;
    for (int i = 0; i < guard; i++) guard_factor *= 10;

    int exponent = (int)places + (int)b.scale - (int)a.scale + guard;
    bool exponent_negative = exponent < 0;
    int abs_exponent = exponent_negative ? -exponent : exponent;
    __int128 scale_pow = 1;
    for (int i = 0; i < abs_exponent; i++) scale_pow *= 10;

    __int128 numerator = exponent_negative
        ? (__int128)a.value / scale_pow
        : (__int128)a.value * scale_pow;

    __int128 guarded_quotient    = numerator / b.value;
    int64_t truncated_with_guard = (int64_t)(guarded_quotient / guard_factor);
    int64_t remainder_for_round  = (int64_t)(guarded_quotient % guard_factor);

    int64_t rounded = certo_round_apply(truncated_with_guard, remainder_for_round, guard_factor, mode.tag);

    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

/* Convert decimal to int64 (truncate) */
int64_t certo_decimal_to_int(certo_decimal_t d) {
    int64_t divisor = 1;
    for (int i = 0; i < d.scale; i++) divisor *= 10;
    return d.value / divisor;
}

/* Convert int64 to decimal with scale 0 */
certo_decimal_t certo_decimal_from_int(int64_t n) {
    certo_decimal_t d = { .value = n, .scale = 0 };
    return d;
}

/* ---- Money (alias for Decimal with scale=2) ---- */

certo_decimal_t certo_money_from_cents(int64_t cents) {
    certo_decimal_t d = { .value = cents, .scale = 2 };
    return d;
}

int64_t certo_money_to_cents(certo_decimal_t m) {
    certo_rounding_mode_t half_up = { .tag = CERTO_ROUND_HALF_UP };
    return certo_decimal_round_mode(m, 2, half_up).value;
}

/* Money.fromDecimal(d, mode) — was unconditional round-to-2dp with HalfUp */
certo_decimal_t certo_money_from_decimal_mode(certo_decimal_t d, certo_rounding_mode_t mode) {
    return certo_decimal_round_mode(d, 2, mode);
}
"#;

/// Certo source declaration of `Stdlib.Money`.
pub const MONEY_CERTO: &str = r#"
module Stdlib.Money

/* Rounding policy — plain sum type, resolves via normal user-type path */

type RoundingMode =
    | HalfUp              // round half away from zero (1.5 → 2, -1.5 → -2)
    | HalfDown            // round half toward zero    (1.5 → 1, -1.5 → -1)
    | HalfEven            // banker's rounding         (1.5 → 2, 2.5 → 2)
    | Up                  // always away from zero     (1.1 → 2, -1.1 → -2)
    | Down                // always toward zero (truncate) (1.9 → 1, -1.9 → -1)
    | Ceiling             // toward positive infinity  (1.1 → 2, -1.9 → -1)
    | Floor               // toward negative infinity  (1.9 → 1, -1.1 → -2)
    | ToIncrement(step: Decimal)   // nearest multiple of step, e.g. 0.05

/* Decimal arithmetic */

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

/* Rounding — mode is mandatory (no silent default) */

fn Decimal.round(d: Decimal, places: Int, mode: RoundingMode): Decimal
fn Decimal.roundToIncrement(d: Decimal, step: Decimal, mode: RoundingMode): Decimal
fn Decimal.divRound(a: Decimal, b: Decimal, places: Int, mode: RoundingMode): Decimal [fallible]

/* Money helpers (Decimal fixed at 2 decimal places) */

fn Money.fromCents(cents: Int): Decimal
fn Money.toCents(m: Decimal): Int
fn Money.fromDecimal(d: Decimal, mode: RoundingMode): Decimal
"#;
