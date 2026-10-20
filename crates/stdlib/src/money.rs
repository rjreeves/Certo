/// C implementations for `Stdlib.Money` and extended `Decimal` operations.
pub const MONEY_C: &str = r#"
/* ================================================================
   Stdlib.Money  (and extended Decimal arithmetic)

   Decimal layout: { int64_t value; int8_t scale; }
   where the true value = value / 10^scale.
   e.g. $19.99 → { value: 1999, scale: 2 }
   ================================================================ */

/* ---- Decimal arithmetic helpers (BACKLOG items 386/387) ----
   A decimal is an int64 value plus a scale, so every operation can overflow. Intermediate
   results are computed in __int128; a result that does not fit an int64 value (or a scale
   above 127) stops the program with "decimal overflow" instead of wrapping silently. */

static int64_t dec_fit(__int128 v) {
    if (v > (__int128)INT64_MAX || v < (__int128)INT64_MIN) certo_panic("decimal overflow");
    return (int64_t)v;
}

static __int128 dec_pow10(int n) {
    __int128 r = 1;
    for (int i = 0; i < n; i++) r *= 10;
    return r;
}

/* d.value rescaled to `scale` (>= d.scale); a rescale that cannot fit an __int128 panics. */
static __int128 dec_scaled(certo_decimal_t d, int scale) {
    int diff = scale - d.scale;
    if (diff <= 0 || d.value == 0) return d.value;
    if (diff > 19) certo_panic("decimal overflow");
    return (__int128)d.value * dec_pow10(diff);
}

/* ---- Decimal parse / format ---- */

/* Parses [-+]digits[.digits]; returns 0 on malformed input or a value that does not fit. */
static int dec_parse_checked(const char* s, certo_decimal_t* out) {
    if (!s) return 0;
    const char* p = s;
    int neg = 0;
    if (*p == '-' || *p == '+') { neg = (*p == '-'); p++; }
    const char* int_start = p;
    __int128 acc = 0;
    int digits = 0;
    while (*p >= '0' && *p <= '9') {
        acc = acc * 10 + (*p - '0');
        if (acc > (__int128)INT64_MAX + 1) return 0;
        digits++; p++;
    }
    int scale = 0;
    if (*p == '.') {
        p++;
        while (*p >= '0' && *p <= '9') {
            acc = acc * 10 + (*p - '0');
            if (acc > (__int128)INT64_MAX + 1) return 0;
            scale++; digits++; p++;
        }
    }
    if (p == int_start || digits == 0 || *p != '\0' || scale > 127) return 0;
    if (neg) acc = -acc;
    if (acc > (__int128)INT64_MAX || acc < (__int128)INT64_MIN) return 0;
    out->value = (int64_t)acc;
    out->scale = (int8_t)scale;
    return 1;
}

/* `d"..."` literals: the parser trusts the text, but a value that does not fit stops the
   program rather than saturating. */
certo_decimal_t certo_decimal_parse(const char* s) {
    certo_decimal_t d = {0, 0};
    if (!s) return d;
    if (!dec_parse_checked(s, &d)) certo_panic("decimal literal out of range or malformed");
    return d;
}

/* Strict decimal parse backing Text.toDecimal: NULL (None) on malformed or out-of-range input,
   otherwise a heap-allocated certo_decimal_t* (the nullable-box convention parseInt uses). */
certo_decimal_t* certo_parse_decimal(certo_text_t s) {
    if (!s || !*s) return NULL;
    certo_decimal_t d;
    if (!dec_parse_checked(s, &d)) return NULL;
    certo_decimal_t* box = (certo_decimal_t*)malloc(sizeof(certo_decimal_t));
    if (!box) certo_panic("out of memory");
    *box = d;
    return box;
}

/* BACKLOG item 288 — spec §9.3's own `text.toDecimal()` example, the
   direct sibling of `Text.toInt` (item 269); a namespaced alias for
   `certo_parse_decimal` just above, keeping its Option<Decimal>-shaped
   convention rather than inventing a new Result<Decimal, ParseError>-shaped
   API to match the spec literally (same narrower scope item 269 already
   established for Text.toInt). Defined here, not in text.rs alongside
   certo_text_to_int, because MONEY_C is concatenated *after* TEXT_C in
   `full_c_runtime` (crates/stdlib/src/lib.rs) — putting it in text.rs would
   reference certo_parse_decimal before it's declared. */
certo_decimal_t* certo_text_to_decimal(certo_text_t s) {
    return certo_parse_decimal(s);
}

/* The sign is written separately from the magnitude, so values between -1 and 0 keep it
   (item 386: -0.05 used to print as 0.05). */
certo_text_t certo_decimal_to_text(certo_decimal_t d) {
    char* buf = (char*)malloc(192);
    if (!buf) certo_panic("out of memory");
    uint64_t mag = d.value < 0 ? (uint64_t)0 - (uint64_t)d.value : (uint64_t)d.value;
    char digits[32];
    snprintf(digits, sizeof(digits), "%" PRIu64, mag);
    int nd = (int)strlen(digits);
    int scale = d.scale > 0 ? d.scale : 0;
    char* o = buf;
    if (d.value < 0) *o++ = '-';
    if (scale == 0) {
        memcpy(o, digits, (size_t)nd + 1);
        return buf;
    }
    int whole_digits = nd - scale;
    if (whole_digits <= 0) {
        *o++ = '0'; *o++ = '.';
        for (int i = 0; i < -whole_digits; i++) *o++ = '0';
        memcpy(o, digits, (size_t)nd + 1);
    } else {
        memcpy(o, digits, (size_t)whole_digits); o += whole_digits;
        *o++ = '.';
        memcpy(o, digits + whole_digits, (size_t)scale + 1);
    }
    return buf;
}

certo_decimal_t certo_decimal_add(certo_decimal_t a, certo_decimal_t b) {
    int s = a.scale > b.scale ? a.scale : b.scale;
    certo_decimal_t r = { .value = dec_fit(dec_scaled(a, s) + dec_scaled(b, s)), .scale = (int8_t)s };
    return r;
}

certo_decimal_t certo_decimal_sub(certo_decimal_t a, certo_decimal_t b) {
    int s = a.scale > b.scale ? a.scale : b.scale;
    certo_decimal_t r = { .value = dec_fit(dec_scaled(a, s) - dec_scaled(b, s)), .scale = (int8_t)s };
    return r;
}

/* Strip trailing zeros from the value while the scale stays >= 0 (exact). */
static void dec_trim(certo_decimal_t* d) {
    while (d->scale > 0 && d->value != 0 && d->value % 10 == 0) { d->value /= 10; d->scale--; }
}

certo_decimal_t certo_decimal_mul(certo_decimal_t a, certo_decimal_t b) {
    int64_t v;
    if (__builtin_mul_overflow(a.value, b.value, &v)) {
        /* Drop trailing zeros (exact) before giving up. */
        dec_trim(&a); dec_trim(&b);
        if (__builtin_mul_overflow(a.value, b.value, &v)) certo_panic("decimal overflow");
    }
    int scale = a.scale + b.scale;
    if (scale > 127) certo_panic("decimal overflow");
    certo_decimal_t r = { .value = v, .scale = (int8_t)scale };
    return r;
}

/* Decimal ** Int (BACKLOG item 422): exact repeated multiplication, so the scale is
   scale * n. A negative exponent, or a result whose scale would not fit, stops the program. */
certo_decimal_t certo_decimal_pow(certo_decimal_t base, int64_t n) {
    if (n < 0) certo_panic("decimal ** with a negative exponent is not supported");
    if (n > 0 && (int64_t)base.scale * n > 127) certo_panic("decimal ** result scale out of range");
    certo_decimal_t r = { .value = 1, .scale = 0 };
    for (int64_t i = 0; i < n; i++) r = certo_decimal_mul(r, base);
    return r;
}

/* n / d for an unsigned 128-bit numerator by long division (the compiler's own 128-bit divide
   needs a runtime library that is not always linked). Returns 0 if the quotient needs more than
   64 bits. */
static int dec_div128(unsigned __int128 n, uint64_t d, uint64_t* q) {
    unsigned __int128 r = 0;
    uint64_t quo = 0;
    for (int i = 127; i >= 0; i--) {
        r = (r << 1) | ((n >> i) & 1);
        if (r >= d) {
            r -= d;
            if (i >= 64) return 0;
            quo |= (uint64_t)1 << i;
        }
    }
    *q = quo;
    return 1;
}

/* Quotient with up to 6 extra digits of precision (truncated). Fewer extra digits are used
   when the quotient would not fit; a result scale is never negative. */
certo_decimal_t certo_decimal_div(certo_decimal_t a, certo_decimal_t b) {
    if (b.value == 0) certo_panic("decimal division by zero");
    int extra = 6;
    int scale = a.scale - b.scale + extra;
    if (scale < 0) { extra -= scale; scale = 0; }
    if (extra > 19) certo_panic("decimal overflow");
    int neg = (a.value < 0) != (b.value < 0);
    uint64_t am = a.value < 0 ? (uint64_t)0 - (uint64_t)a.value : (uint64_t)a.value;
    uint64_t bm = b.value < 0 ? (uint64_t)0 - (uint64_t)b.value : (uint64_t)b.value;
    for (;;) {
        uint64_t q;
        unsigned __int128 n = (unsigned __int128)am * (unsigned __int128)dec_pow10(extra);
        uint64_t limit = neg ? (uint64_t)INT64_MAX + 1 : (uint64_t)INT64_MAX;
        if (dec_div128(n, bm, &q) && q <= limit) {
            certo_decimal_t r = { .value = neg ? (int64_t)((uint64_t)0 - q) : (int64_t)q, .scale = (int8_t)scale };
            return r;
        }
        if (extra == 0 || scale == 0) certo_panic("decimal overflow");
        extra--; scale--;
    }
}

static int dec_cmp(certo_decimal_t a, certo_decimal_t b) {
    int s = a.scale > b.scale ? a.scale : b.scale;
    __int128 x = dec_scaled(a, s), y = dec_scaled(b, s);
    return x < y ? -1 : (x > y ? 1 : 0);
}

bool certo_decimal_eq (certo_decimal_t a, certo_decimal_t b) { return dec_cmp(a, b) == 0; }
bool certo_decimal_lt (certo_decimal_t a, certo_decimal_t b) { return dec_cmp(a, b) <  0; }
bool certo_decimal_gt (certo_decimal_t a, certo_decimal_t b) { return dec_cmp(a, b) >  0; }
bool certo_decimal_lte(certo_decimal_t a, certo_decimal_t b) { return dec_cmp(a, b) <= 0; }
bool certo_decimal_gte(certo_decimal_t a, certo_decimal_t b) { return dec_cmp(a, b) >= 0; }

certo_decimal_t certo_decimal_abs(certo_decimal_t d) {
    certo_decimal_t r = { .value = dec_fit(d.value < 0 ? -(__int128)d.value : (__int128)d.value), .scale = d.scale };
    return r;
}

certo_decimal_t certo_decimal_negate(certo_decimal_t d) {
    certo_decimal_t r = { .value = dec_fit(-(__int128)d.value), .scale = d.scale };
    return r;
}

/* v / 10^n rounded half away from zero, in 64-bit arithmetic (n >= 0). */
static int64_t dec_round_pow10(int64_t v, int n) {
    if (n <= 0) return v;
    if (n > 19) return 0;                       /* |v| < 9.3e18 < half of 10^20 */
    uint64_t div = 1;
    for (int i = 0; i < n; i++) div *= 10;       /* 10^19 still fits a uint64 */
    uint64_t mag = v < 0 ? (uint64_t)0 - (uint64_t)v : (uint64_t)v;
    uint64_t q = mag / div, rem = mag % div;
    if (rem >= div - rem) q++;                   /* rem * 2 >= div, without overflow */
    return dec_fit(v < 0 ? -(__int128)q : (__int128)q);
}

/* Round to `places` decimal places, half away from zero. A negative `places` rounds to tens,
   hundreds, ... and the result has scale 0 (BACKLOG item 387: round(x, -2) used to print junk). */
certo_decimal_t certo_decimal_round(certo_decimal_t d, int8_t places) {
    if (places >= 0) {
        if (d.scale <= places) return d;
        certo_decimal_t r = { .value = dec_round_pow10(d.value, d.scale - places), .scale = places };
        return r;
    }
    int k = -places;
    int64_t whole = d.scale > 0 ? dec_round_pow10(d.value, d.scale) : d.value;
    certo_decimal_t r = { .value = whole, .scale = 0 };
    if (k > 19) { r.value = 0; return r; }
    int64_t q = dec_round_pow10(whole, k);
    r.value = dec_fit((__int128)q * dec_pow10(k));
    return r;
}

/* Convert decimal to int64 (truncate) */
int64_t certo_decimal_to_int(certo_decimal_t d) {
    if (d.scale <= 0) return d.value;
    if (d.scale > 19) return 0;
    uint64_t div = 1;
    for (int i = 0; i < d.scale; i++) div *= 10;
    uint64_t mag = d.value < 0 ? (uint64_t)0 - (uint64_t)d.value : (uint64_t)d.value;
    uint64_t q = mag / div;
    return d.value < 0 ? (int64_t)((uint64_t)0 - q) : (int64_t)q;
}

/* Convert int64 to decimal with scale 0 */
certo_decimal_t certo_decimal_from_int(int64_t n) {
    certo_decimal_t d = { .value = n, .scale = 0 };
    return d;
}

/* BACKLOG item 307 — spec §4.5's own `n.toDecimal()` dot-call example;
   a namespaced alias for the identical real implementation above, not a
   second one (mirrors item 269's own `certo_text_to_uppercase` precedent). */
certo_decimal_t certo_int_to_decimal(int64_t n) {
    return certo_decimal_from_int(n);
}

/* ---- Money (alias for Decimal with scale=2) ---- */

certo_decimal_t certo_money_from_cents(int64_t cents) {
    certo_decimal_t d = { .value = cents, .scale = 2 };
    return d;
}

int64_t certo_money_to_cents(certo_decimal_t m) {
    certo_decimal_t r = certo_decimal_round(m, 2);
    return dec_fit(dec_scaled(r, 2));   /* d"5" is 500 cents, not 5 */
}

certo_decimal_t certo_money_from_decimal(certo_decimal_t d) {
    return certo_decimal_round(d, 2);
}
"#;
