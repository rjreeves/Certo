/* Runtime shim for the experimental LLVM backend (`certo-llvm`).
 *
 * This is NOT the C backend's runtime. The LLVM emitter represents `Text` as a
 * plain NUL-terminated `ptr`, while the C backend's runtime passes structs and
 * uses different symbol names, so the two cannot be linked together. This file
 * provides the handful of functions the LLVM emitter knows how to call, under
 * `certo_rt_*` names that cannot clash with a user function or the C runtime.
 *
 * Keep in step with `RT_FNS` in `crates/llvm/src/rt.rs`: every symbol listed
 * there must be defined here, and the output formats match the C runtime's
 * (`%g` for floats, `true`/`false` for Bool, byte length for Text.len).
 */
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Also called by compiled code to allocate a record's block. */
void *certo_rt_alloc(int64_t n) {
    void *p = calloc(1, (size_t)n);  /* zeroed: a match may read unused payload slots */
    if (!p) {
        fputs("certo: out of memory\n", stderr);
        exit(1);
    }
    return p;
}

void certo_rt_print(const char *s)    { fputs(s ? s : "", stdout); }
void certo_rt_println(const char *s)  { fputs(s ? s : "", stdout); fputc('\n', stdout); }
void certo_rt_eprint(const char *s)   { fputs(s ? s : "", stderr); }
void certo_rt_eprintln(const char *s) { fputs(s ? s : "", stderr); fputc('\n', stderr); }

const char *certo_rt_int_to_text(int64_t n) {
    char *buf = (char *)certo_rt_alloc(24);
    snprintf(buf, 24, "%" PRId64, n);
    return buf;
}

const char *certo_rt_float_to_text(double f) {
    char *buf = (char *)certo_rt_alloc(64);
    snprintf(buf, 64, "%g", f);
    return buf;
}

const char *certo_rt_bool_to_text(bool b) { return b ? "true" : "false"; }

bool certo_rt_text_eq(const char *a, const char *b) {
    return strcmp(a ? a : "", b ? b : "") == 0;
}

int64_t certo_rt_text_len(const char *s) { return s ? (int64_t)strlen(s) : 0; }

const char *certo_rt_text_concat(const char *a, const char *b) {
    size_t la = a ? strlen(a) : 0;
    size_t lb = b ? strlen(b) : 0;
    char *buf = (char *)certo_rt_alloc(la + lb + 1);
    if (la) memcpy(buf, a, la);
    if (lb) memcpy(buf + la, b, lb);
    buf[la + lb] = '\0';
    return buf;
}

int64_t certo_rt_pow(int64_t base, int64_t exp) {
    if (exp < 0) return 0;
    int64_t result = 1;
    while (exp > 0) {
        if (exp & 1) result *= base;
        base *= base;
        exp >>= 1;
    }
    return result;
}
