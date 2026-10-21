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

/* Float -> text (BACKLOG item 400): the fewest significant digits that read back
 * as exactly the same number; plain decimal for exponents in [-5, 17), %g-style
 * exponents beyond; whole numbers without a fractional part. The same rule as the
 * C backend's certo_float_to_text (crates/stdlib/src/core.rs) and the JS loader
 * certo-wasm generates. */
static void certo_rt_fmt_float(char *buf, size_t cap, double f) {
    if (f != f) { snprintf(buf, cap, "nan"); return; }
    if (f - f != 0) { snprintf(buf, cap, f < 0 ? "-inf" : "inf"); return; }
    if (f == 0) { snprintf(buf, cap, "%g", f); return; }
    char t[64];
    int p;
    for (p = 1; p <= 17; p++) {
        snprintf(t, sizeof t, "%.*e", p - 1, f);
        if (strtod(t, NULL) == f) break;
    }
    if (p > 17) p = 17;
    snprintf(t, sizeof t, "%.*e", p - 1, f);
    int e = atoi(strchr(t, 'e') + 1);
    if (e >= -5 && e < 17) snprintf(buf, cap, "%.*f", (p - 1 - e) > 0 ? (p - 1 - e) : 0, f);
    else snprintf(buf, cap, "%.*g", p, f);
}

const char *certo_rt_float_to_text(double f) {
    char *buf = (char *)certo_rt_alloc(64);
    certo_rt_fmt_float(buf, 64, f);
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


/* ---- Lists ----
 * A list is { len, cap, data } with one 8-byte slot per element; an element is
 * held in its slot as its own value (an Int, a pointer, a Float's bits, ...).
 * An Option<T> is a pointer to an 8-byte payload, so a slot's address is a valid
 * `Some(element)` without allocating. */
typedef struct { int64_t len; int64_t cap; int64_t *data; } certo_rt_list_t;

__attribute__((noreturn)) static void certo_rt_panic(const char *msg) {
    fflush(stdout);
    fprintf(stderr, "certo panic: %s\n", msg);
    fflush(stderr);
    abort();
}

static certo_rt_list_t *certo_rt_list_alloc(int64_t n) {
    certo_rt_list_t *l = (certo_rt_list_t *)certo_rt_alloc((int64_t)sizeof *l);
    l->len = n;
    l->cap = n;
    l->data = (int64_t *)certo_rt_alloc((n > 0 ? n : 1) * 8);
    return l;
}

/* A list of `n` zeroed slots, which compiled code then fills in. */
void *certo_rt_list_new(int64_t n) { return certo_rt_list_alloc(n); }
void *certo_rt_list_empty(void) { return certo_rt_list_alloc(0); }
int64_t certo_rt_list_len(void *l) { return l ? ((certo_rt_list_t *)l)->len : 0; }

/* Address of slot `i`; panics when out of range (List.getOrPanic, for loops). */
void *certo_rt_list_slot(void *l, int64_t i) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    if (!list || i < 0 || i >= list->len) certo_rt_panic("list index out of bounds");
    return &list->data[i];
}

/* Address of slot `i`, or NULL when out of range (List.get -> Option). */
void *certo_rt_list_get(void *l, int64_t i) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    if (!list || i < 0 || i >= list->len) return NULL;
    return &list->data[i];
}

void *certo_rt_list_first(void *l) { return certo_rt_list_get(l, 0); }
void *certo_rt_list_last(void *l)  { return certo_rt_list_get(l, certo_rt_list_len(l) - 1); }

/* List.push returns a new list (functional update), like the C runtime. */
void *certo_rt_list_push(void *l, int64_t item) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n + 1);
    if (n) memcpy(out->data, list->data, (size_t)n * sizeof(int64_t));
    out->data[n] = item;
    return out;
}


/* ---- Process and environment ----
 * Option<Text> is a pointer to an 8-byte box holding the text pointer, or NULL. */
static int32_t certo_rt_argc_v;
static char **certo_rt_argv_v;

/* Called by the program entry point when the program uses `arg`. */
void certo_rt_set_args(int32_t argc, char **argv) {
    certo_rt_argc_v = argc;
    certo_rt_argv_v = argv;
}

static void *certo_rt_some_text(const char *s) {
    const char **box = (const char **)certo_rt_alloc(8);
    *box = s;
    return box;
}

/* arg(i): argv[i] (argv[0] is the program), None when out of range. */
void *certo_rt_arg(int64_t i) {
    if (i < 0 || i >= (int64_t)certo_rt_argc_v) return NULL;
    return certo_rt_some_text(certo_rt_argv_v[i]);
}

/* getEnv(key): None when unset. */
void *certo_rt_get_env(const char *key) {
    const char *v = key ? getenv(key) : NULL;
    return v ? certo_rt_some_text(v) : NULL;
}

/* parseInt(s): None unless the whole text is a base-10 integer (strtoll rules). */
void *certo_rt_parse_int(const char *s) {
    if (!s) return NULL;
    char *end;
    long long v = strtoll(s, &end, 10);
    if (end == s || *end != '\0') return NULL;
    int64_t *box = (int64_t *)certo_rt_alloc(8);
    *box = (int64_t)v;
    return box;
}

void certo_rt_quit(int64_t code) { fflush(stdout); exit((int)code); }

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


/* ---- Closures and higher-order list functions ----
 * A closure is a block with its function in slot 0 and its environment in slot 1 (8-byte
 * slots whatever the pointer width). A callback takes the environment first, then its
 * arguments as erased 64-bit values (an Int, a pointer, a Float's bits), and returns the same;
 * a predicate returns 0 or 1. */
typedef int64_t (*certo_rt_fn1_t)(void *, int64_t);
typedef int64_t (*certo_rt_fn2_t)(void *, int64_t, int64_t);

static void *certo_rt_clo_fn(void *c)  { return *(void **)c; }
static void *certo_rt_clo_env(void *c) { return *(void **)((char *)c + 8); }

void *certo_rt_list_map(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    for (int64_t i = 0; i < n; i++) out->data[i] = fn(env, list->data[i]);
    return out;
}

void *certo_rt_list_filter(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    int64_t k = 0;
    for (int64_t i = 0; i < n; i++) if (fn(env, list->data[i]) & 0xff) out->data[k++] = list->data[i];
    out->len = k;
    return out;
}

/* List.fold(xs, init, f): the accumulator travels as a pointer-sized value, as in the C runtime. */
void *certo_rt_list_fold(void *l, void *init, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    certo_rt_fn2_t fn = (certo_rt_fn2_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    int64_t acc = (int64_t)(__UINTPTR_TYPE__)init;
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) acc = fn(env, acc, list->data[i]);
    return (void *)(__UINTPTR_TYPE__)acc;
}

void certo_rt_list_for_each(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) fn(env, list->data[i]);
}

/* The slot of the first element the predicate accepts, which is a valid Some(element); else NULL. */
void *certo_rt_list_find(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) if (fn(env, list->data[i]) & 0xff) return &list->data[i];
    return 0;
}

bool certo_rt_list_any(void *l, void *c) {
    return certo_rt_list_find(l, c) != 0;
}

bool certo_rt_list_all(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) if (!(fn(env, list->data[i]) & 0xff)) return false;
    return true;
}

/* A shared mutable cell, for a `var` that a lambda assigns (BACKLOG item 354). */
void *certo_rt_cell_new(void *v) {
    void **cell = (void **)certo_rt_alloc(8);
    *cell = v;
    return cell;
}
void *certo_rt_cell_get(void *cell) { return *(void **)cell; }
void certo_rt_cell_set(void *cell, void *v) { *(void **)cell = v; }


/* ---- More list functions (BACKLOG item 442) ----
 * A tuple is a bare block of 8-byte slots here (no header), so partition and zip build those. */
static void *certo_rt_pair(void *a, void *b) {
    int64_t *t = (int64_t *)certo_rt_alloc(16);
    t[0] = (int64_t)(__UINTPTR_TYPE__)a;
    t[1] = (int64_t)(__UINTPTR_TYPE__)b;
    return t;
}

void *certo_rt_list_slice(void *l, int64_t start, int64_t end) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    if (start < 0) start = 0;
    if (end > n) end = n;
    if (start >= end) return certo_rt_list_alloc(0);
    certo_rt_list_t *out = certo_rt_list_alloc(end - start);
    for (int64_t i = start; i < end; i++) out->data[i - start] = list->data[i];
    return out;
}

void *certo_rt_list_reverse(void *l) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    for (int64_t i = 0; i < n; i++) out->data[i] = list->data[n - 1 - i];
    return out;
}

void *certo_rt_list_concat(void *a, void *b) {
    certo_rt_list_t *x = (certo_rt_list_t *)a, *y = (certo_rt_list_t *)b;
    int64_t nx = x ? x->len : 0, ny = y ? y->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(nx + ny);
    for (int64_t i = 0; i < nx; i++) out->data[i] = x->data[i];
    for (int64_t i = 0; i < ny; i++) out->data[nx + i] = y->data[i];
    return out;
}

void *certo_rt_list_chunked(void *l, int64_t size) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    if (n == 0 || size <= 0) return certo_rt_list_alloc(0);
    certo_rt_list_t *out = certo_rt_list_alloc((n + size - 1) / size);
    int64_t k = 0;
    for (int64_t i = 0; i < n; i += size)
        out->data[k++] = (int64_t)(__UINTPTR_TYPE__)certo_rt_list_slice(l, i, i + size);
    return out;
}

_Bool certo_rt_list_contains(void *l, int64_t item) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) if (list->data[i] == item) return 1;
    return 0;
}

_Bool certo_rt_list_contains_text(void *l, const char *item) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++)
        if (certo_rt_text_eq((const char *)(__UINTPTR_TYPE__)list->data[i], item)) return 1;
    return 0;
}

void *certo_rt_list_distinct(void *l) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    int64_t k = 0;
    for (int64_t i = 0; i < n; i++) {
        int seen = 0;
        for (int64_t j = 0; j < k; j++) if (out->data[j] == list->data[i]) { seen = 1; break; }
        if (!seen) out->data[k++] = list->data[i];
    }
    out->len = k;
    return out;
}

void *certo_rt_list_distinct_text(void *l) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    int64_t k = 0;
    for (int64_t i = 0; i < n; i++) {
        int seen = 0;
        for (int64_t j = 0; j < k; j++)
            if (certo_rt_text_eq((const char *)(__UINTPTR_TYPE__)out->data[j], (const char *)(__UINTPTR_TYPE__)list->data[i])) { seen = 1; break; }
        if (!seen) out->data[k++] = list->data[i];
    }
    out->len = k;
    return out;
}

void *certo_rt_list_partition(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    certo_rt_list_t *yes = certo_rt_list_alloc(n), *no = certo_rt_list_alloc(n);
    int64_t ky = 0, kn = 0;
    for (int64_t i = 0; i < n; i++) {
        if (fn(env, list->data[i]) & 0xff) yes->data[ky++] = list->data[i];
        else no->data[kn++] = list->data[i];
    }
    yes->len = ky;
    no->len = kn;
    return certo_rt_pair(yes, no);
}

void *certo_rt_list_flat_map(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    void *out = certo_rt_list_alloc(0);
    for (int64_t i = 0; i < n; i++)
        out = certo_rt_list_concat(out, (void *)(__UINTPTR_TYPE__)fn(env, list->data[i]));
    return out;
}

void *certo_rt_list_zip(void *a, void *b) {
    certo_rt_list_t *x = (certo_rt_list_t *)a, *y = (certo_rt_list_t *)b;
    int64_t n = (x && y) ? (x->len < y->len ? x->len : y->len) : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    for (int64_t i = 0; i < n; i++)
        out->data[i] = (int64_t)(__UINTPTR_TYPE__)certo_rt_pair((void *)(__UINTPTR_TYPE__)x->data[i], (void *)(__UINTPTR_TYPE__)y->data[i]);
    return out;
}

/* Stable merge sort; the comparator returns negative, zero or positive. */
typedef int64_t (*certo_rt_cmp_t)(void *, int64_t, int64_t);

static void certo_rt_sort_range(int64_t *d, int64_t *tmp, int64_t lo, int64_t hi, certo_rt_cmp_t cmp, void *env) {
    if (hi - lo <= 1) return;
    int64_t mid = lo + (hi - lo) / 2;
    certo_rt_sort_range(d, tmp, lo, mid, cmp, env);
    certo_rt_sort_range(d, tmp, mid, hi, cmp, env);
    int64_t i = lo, j = mid, k = lo;
    while (i < mid && j < hi) tmp[k++] = (cmp(env, d[i], d[j]) <= 0) ? d[i++] : d[j++];
    while (i < mid) tmp[k++] = d[i++];
    while (j < hi) tmp[k++] = d[j++];
    for (k = lo; k < hi; k++) d[k] = tmp[k];
}

void *certo_rt_list_sort(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n);
    for (int64_t i = 0; i < n; i++) out->data[i] = list->data[i];
    int64_t *tmp = (int64_t *)certo_rt_alloc((n > 0 ? n : 1) * 8);
    certo_rt_sort_range(out->data, tmp, 0, n, (certo_rt_cmp_t)certo_rt_clo_fn(c), certo_rt_clo_env(c));
    return out;
}
