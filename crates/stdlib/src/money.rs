/// C implementations for `Stdlib.Money` and extended `Decimal` operations.
pub const MONEY_C: &str = r#"
/* ================================================================
   Stdlib.Money  (and extended Decimal arithmetic)

   Decimal layout: { int64_t value; int8_t scale; }
   where the true value = value / 10^scale.
   e.g. $19.99 → { value: 1999, scale: 2 }
   ================================================================ */

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

/* Strict decimal parse: optional leading '-'/'+', digits, optional '.' plus
   digits. Rejects empty/garbage input — unlike certo_decimal_parse above
   (used for `d"..."` literals, which trusts the parser and silently
   tolerates junk), this backs a fallible Certo-level function so it must
   actually validate. Returns NULL (None) on failure, heap-allocated
   certo_decimal_t* on success — mirrors certo_parse_int/certo_parse_float's
   nullable-box convention so Option<Decimal> unboxes the same way. */
certo_decimal_t* certo_parse_decimal(certo_text_t s) {
    if (!s || !*s) return NULL;
    const char* p = s;
    if (*p == '-' || *p == '+') p++;
    const char* int_start = p;
    while (*p >= '0' && *p <= '9') p++;
    if (p == int_start) return NULL;
    if (*p == '.') {
        p++;
        const char* frac_start = p;
        while (*p >= '0' && *p <= '9') p++;
        if (p == frac_start) return NULL;
    }
    if (*p != '\0') return NULL;
    certo_decimal_t* box = (certo_decimal_t*)malloc(sizeof(certo_decimal_t));
    if (!box) certo_panic("out of memory");
    *box = certo_decimal_parse(s);
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

certo_decimal_t certo_decimal_div(certo_decimal_t a, certo_decimal_t b) {
    if (b.value == 0) certo_panic("decimal division by zero");
    /* Multiply numerator by 10^scale to keep precision */
    int8_t extra = 6;  /* 6 extra digits of precision */
    int64_t scale_factor = 1;
    for (int i = 0; i < extra; i++) scale_factor *= 10;
    certo_decimal_t r = {
        .value = (a.value * scale_factor) / b.value,
        .scale = (int8_t)(a.scale - b.scale + extra),
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

/* Round to `places` decimal places (half-up) */
certo_decimal_t certo_decimal_round(certo_decimal_t d, int8_t places) {
    if (d.scale <= places) return d;
    int8_t excess = d.scale - places;
    int64_t divisor = 1;
    for (int i = 0; i < excess; i++) divisor *= 10;
    int64_t rounded = (d.value + divisor / 2) / divisor;
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
    return certo_decimal_round(m, 2).value;
}

certo_decimal_t certo_money_from_decimal(certo_decimal_t d) {
    return certo_decimal_round(d, 2);
}
"#;
