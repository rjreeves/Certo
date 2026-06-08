/// C implementations for `Stdlib.Core`.
pub const CORE_C: &str = r#"
/* ================================================================
   Stdlib.Core — core utilities
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

/* ---- print / println ---- */

void certo_print(certo_text_t s) {
    fputs(s ? s : "", stdout);
}

void certo_println(certo_text_t s) {
    puts(s ? s : "");
}

void certo_eprint(certo_text_t s) {
    fputs(s ? s : "", stderr);
}

void certo_eprintln(certo_text_t s) {
    fputs(s ? s : "", stderr);
    fputc('\n', stderr);
}

/* ---- conversions ---- */

certo_text_t certo_int_to_text(int64_t n) {
    /* 21 bytes is enough for any int64 including sign and NUL */
    char* buf = (char*)malloc(24);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 24, "%" PRId64, n);
    return buf;
}

certo_text_t certo_float_to_text(double f) {
    char* buf = (char*)malloc(64);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 64, "%g", f);
    return buf;
}

certo_text_t certo_bool_to_text(bool b) {
    return b ? "true" : "false";
}

int64_t certo_float_to_int(double f) {
    return (int64_t)f;
}

double certo_int_to_float(int64_t n) {
    return (double)n;
}

int64_t certo_text_to_int_unsafe(certo_text_t s) {
    if (!s) certo_panic("text_to_int: null string");
    return (int64_t)strtoll(s, NULL, 10);
}

double certo_text_to_float_unsafe(certo_text_t s) {
    if (!s) certo_panic("text_to_float: null string");
    return strtod(s, NULL);
}

/* Option<Int> — NULL pointer means None */
void* certo_text_parse_int(certo_text_t s) {
    if (!s || !*s) return NULL;
    char* end;
    int64_t v = strtoll(s, &end, 10);
    if (*end != '\0') return NULL;
    int64_t* p = (int64_t*)malloc(sizeof(int64_t));
    if (!p) certo_panic("out of memory");
    *p = v;
    return p;
}

void* certo_text_parse_float(certo_text_t s) {
    if (!s || !*s) return NULL;
    char* end;
    double v = strtod(s, &end);
    if (*end != '\0') return NULL;
    double* p = (double*)malloc(sizeof(double));
    if (!p) certo_panic("out of memory");
    *p = v;
    return p;
}

/* ---- assert ---- */

void certo_assert(bool cond, certo_text_t msg) {
    if (!cond) certo_panic(msg ? msg : "assertion failed");
}

/* ---- arithmetic ---- */

int64_t certo_pow(int64_t base, int64_t exp) {
    if (exp < 0) return 0;
    int64_t result = 1;
    while (exp > 0) {
        if (exp & 1) result *= base;
        base *= base;
        exp >>= 1;
    }
    return result;
}

int64_t certo_abs_int(int64_t n)   { return n < 0 ? -n : n; }
double  certo_abs_float(double f)  { return f < 0.0 ? -f : f; }
int64_t certo_min_int(int64_t a, int64_t b) { return a < b ? a : b; }
int64_t certo_max_int(int64_t a, int64_t b) { return a > b ? a : b; }
double  certo_min_float(double a, double b) { return a < b ? a : b; }
double  certo_max_float(double a, double b) { return a > b ? a : b; }
double  certo_floor(double f) { return floor(f); }
double  certo_ceil(double f)  { return ceil(f); }
double  certo_round(double f) { return round(f); }
double  certo_sqrt(double f)  { return sqrt(f); }

/* ---- range ---- */

/* Returns a heap-allocated array of int64_t preceded by a length header.
   Layout: [int64_t len] [int64_t data[len]]
   Caller owns the allocation. */
void* certo_range(int64_t start, int64_t end_excl) {
    int64_t len = end_excl > start ? end_excl - start : 0;
    int64_t* arr = (int64_t*)malloc((1 + (size_t)len) * sizeof(int64_t));
    if (!arr) certo_panic("out of memory");
    arr[0] = len;
    for (int64_t i = 0; i < len; i++) arr[i + 1] = start + i;
    return arr;
}

void* certo_range_inclusive(int64_t start, int64_t end_incl) {
    return certo_range(start, end_incl + 1);
}

/* ---- panic ---- */

__attribute__((noreturn)) void certo_panic(certo_text_t msg) {
    fprintf(stderr, "certo panic: %s\n", msg ? msg : "(no message)");
    abort();
}

/* ---- null coalesce / coerce ---- */
void* certo_coalesce(void* opt, void* fallback) {
    return opt ? opt : fallback;
}
"#;

/// Certo source declaration of `Stdlib.Core`.
pub const CORE_CERTO: &str = r#"
module Stdlib.Core

/// Print text to stdout (no newline).
fn print(s: Text): Unit [io]

/// Print text to stdout followed by a newline.
fn println(s: Text): Unit [io]

/// Print text to stderr (no newline).
fn eprint(s: Text): Unit [io]

/// Print text to stderr followed by a newline.
fn eprintln(s: Text): Unit [io]

/// Convert an Int to its decimal Text representation.
fn intToText(n: Int): Text

/// Convert a Float to Text.
fn floatToText(f: Float): Text

/// Convert a Bool to "true" or "false".
fn boolToText(b: Bool): Text

/// Convert a Float to Int by truncation.
fn floatToInt(f: Float): Int

/// Convert an Int to Float.
fn intToFloat(n: Int): Float

/// Parse a Text as Int, returning None if it is not a valid integer.
fn parseInt(s: Text): Int?

/// Parse a Text as Float, returning None if it is not a valid float.
fn parseFloat(s: Text): Float?

/// Abort with a message if `cond` is false.
fn assert(cond: Bool, msg: Text): Unit

/// Integer exponentiation: base^exp.
fn pow(base: Int, exp: Int): Int

/// Absolute value.
fn absInt(n: Int): Int
fn absFloat(f: Float): Float

fn minInt(a: Int, b: Int): Int
fn maxInt(a: Int, b: Int): Int
fn minFloat(a: Float, b: Float): Float
fn maxFloat(a: Float, b: Float): Float

fn floor(f: Float): Float
fn ceil(f: Float): Float
fn round(f: Float): Float
fn sqrt(f: Float): Float

/// Exclusive range [start, end).
fn range(start: Int, end: Int): List<Int>

/// Inclusive range [start, end].
fn rangeInclusive(start: Int, end: Int): List<Int>
"#;
