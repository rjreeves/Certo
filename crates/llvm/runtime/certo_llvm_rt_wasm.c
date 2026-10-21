/* Runtime shim for the experimental LLVM backend, browser WebAssembly target
 * (`wasm32-unknown-unknown`, built with -nostdlib: there is no libc).
 *
 * Same functions and `certo_rt_*` names as `certo_llvm_rt.c`, but with no libc:
 *   - output goes through the host import `env.certo_write(fd, ptr, len)`;
 *   - Float formatting goes through `env.certo_float_to_text(x, buf, cap)`,
 *     because %g is not something to reimplement here — the JS loader that
 *     `certo-wasm --bindings` writes supplies both;
 *   - memory comes from a bump allocator over the module's linear memory and is
 *     never freed (a program run is short-lived).
 *
 * Keep in step with `RT_FNS` in `crates/llvm/src/rt.rs`.
 */
typedef signed long long int64_t;
typedef unsigned long size_t;

#define IMPORT(name) __attribute__((import_module("env"), import_name(name)))

IMPORT("certo_write") void certo_host_write(int fd, const char *p, int n);
IMPORT("certo_float_to_text") int certo_host_float_to_text(double x, char *buf, int cap);

extern unsigned char __heap_base;
static unsigned char *heap_ptr;
static unsigned char *heap_end;

/* Also called by compiled code to allocate a record's block. */
void *certo_rt_alloc(int64_t size) {
    size_t n = (size_t)size;
    if (!heap_ptr) {
        heap_ptr = &__heap_base;
        heap_end = (unsigned char *)(__builtin_wasm_memory_size(0) * 65536UL);
    }
    n = (n + 7) & ~(size_t)7;
    if (heap_ptr + n > heap_end) {
        size_t need = (size_t)(heap_ptr + n - heap_end + 65535) / 65536;
        if (__builtin_wasm_memory_grow(0, need) == (size_t)-1) __builtin_trap();
        heap_end += need * 65536UL;
    }
    void *p = heap_ptr;
    heap_ptr += n;
    return p;
}

static size_t certo_rt_strlen(const char *s) {
    size_t n = 0;
    if (s) while (s[n]) n++;
    return n;
}

static void certo_rt_out(int fd, const char *s, int newline) {
    int n = (int)certo_rt_strlen(s);
    if (n) certo_host_write(fd, s, n);
    if (newline) certo_host_write(fd, "\n", 1);
}

void certo_rt_print(const char *s)    { certo_rt_out(1, s, 0); }
void certo_rt_println(const char *s)  { certo_rt_out(1, s, 1); }
void certo_rt_eprint(const char *s)   { certo_rt_out(2, s, 0); }
void certo_rt_eprintln(const char *s) { certo_rt_out(2, s, 1); }

const char *certo_rt_int_to_text(int64_t n) {
    char tmp[24];
    int i = 24;
    int neg = n < 0;
    /* Work in unsigned so INT64_MIN does not overflow on negation. */
    unsigned long long u = neg ? 0ULL - (unsigned long long)n : (unsigned long long)n;
    do {
        tmp[--i] = (char)('0' + (u % 10));
        u /= 10;
    } while (u);
    if (neg) tmp[--i] = '-';
    int len = 24 - i;
    char *buf = (char *)certo_rt_alloc((size_t)len + 1);
    for (int k = 0; k < len; k++) buf[k] = tmp[i + k];
    buf[len] = '\0';
    return buf;
}

const char *certo_rt_float_to_text(double f) {
    char *buf = (char *)certo_rt_alloc(64);
    int n = certo_host_float_to_text(f, buf, 63);
    if (n < 0) n = 0;
    if (n > 63) n = 63;
    buf[n] = '\0';
    return buf;
}

const char *certo_rt_bool_to_text(_Bool b) { return b ? "true" : "false"; }

_Bool certo_rt_text_eq(const char *a, const char *b) {
    if (!a) a = "";
    if (!b) b = "";
    while (*a && *a == *b) { a++; b++; }
    return *a == *b;
}

int64_t certo_rt_text_len(const char *s) { return (int64_t)certo_rt_strlen(s); }

const char *certo_rt_text_concat(const char *a, const char *b) {
    size_t la = certo_rt_strlen(a);
    size_t lb = certo_rt_strlen(b);
    char *buf = (char *)certo_rt_alloc(la + lb + 1);
    for (size_t k = 0; k < la; k++) buf[k] = a[k];
    for (size_t k = 0; k < lb; k++) buf[la + k] = b[k];
    buf[la + lb] = '\0';
    return buf;
}


/* ---- Lists ---- (same layout and semantics as certo_llvm_rt.c) */
typedef struct { int64_t len; int64_t cap; int64_t *data; } certo_rt_list_t;

static certo_rt_list_t *certo_rt_list_alloc(int64_t n) {
    certo_rt_list_t *l = (certo_rt_list_t *)certo_rt_alloc((int64_t)sizeof *l);
    l->len = n;
    l->cap = n;
    l->data = (int64_t *)certo_rt_alloc((n > 0 ? n : 1) * 8);
    return l;
}

void *certo_rt_list_new(int64_t n) { return certo_rt_list_alloc(n); }
void *certo_rt_list_empty(void) { return certo_rt_list_alloc(0); }
int64_t certo_rt_list_len(void *l) { return l ? ((certo_rt_list_t *)l)->len : 0; }

void *certo_rt_list_slot(void *l, int64_t i) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    if (!list || i < 0 || i >= list->len) __builtin_trap();  /* list index out of bounds */
    return &list->data[i];
}

void *certo_rt_list_get(void *l, int64_t i) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    if (!list || i < 0 || i >= list->len) return 0;
    return &list->data[i];
}

void *certo_rt_list_first(void *l) { return certo_rt_list_get(l, 0); }
void *certo_rt_list_last(void *l)  { return certo_rt_list_get(l, certo_rt_list_len(l) - 1); }

void *certo_rt_list_push(void *l, int64_t item) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    int64_t n = list ? list->len : 0;
    certo_rt_list_t *out = certo_rt_list_alloc(n + 1);
    for (int64_t k = 0; k < n; k++) out->data[k] = list->data[k];
    out->data[n] = item;
    return out;
}


/* ---- Process and environment ----
 * A browser has no argv and no environment, so `arg` and `getEnv` are always
 * None; `Process.quit` stops the module by trapping. */
void *certo_rt_arg(int64_t i) { (void)i; return 0; }
void *certo_rt_get_env(const char *key) { (void)key; return 0; }

/* parseInt(s): None unless the whole text is a base-10 integer, with the same
 * rules as strtoll: leading whitespace, an optional sign, then digits. An
 * out-of-range value clamps, as strtoll does. */
void *certo_rt_parse_int(const char *s) {
    if (!s) return 0;
    const char *p = s;
    while (*p == ' ' || (*p >= '\t' && *p <= '\r')) p++;
    int neg = 0;
    if (*p == '-') { neg = 1; p++; } else if (*p == '+') { p++; }
    if (*p < '0' || *p > '9') return 0;
    unsigned long long limit = neg ? 9223372036854775808ULL : 9223372036854775807ULL;
    unsigned long long v = 0;
    int clamped = 0;
    while (*p >= '0' && *p <= '9') {
        unsigned d = (unsigned)(*p - '0');
        if (v > (limit - d) / 10) clamped = 1; else v = v * 10 + d;
        p++;
    }
    if (*p) return 0;
    if (clamped) v = limit;
    int64_t *box = (int64_t *)certo_rt_alloc(8);
    *box = neg ? (int64_t)(0ULL - v) : (int64_t)v;
    return box;
}

void certo_rt_quit(int64_t code) { (void)code; __builtin_trap(); }

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

_Bool certo_rt_list_any(void *l, void *c) {
    return certo_rt_list_find(l, c) != 0;
}

_Bool certo_rt_list_all(void *l, void *c) {
    certo_rt_list_t *list = (certo_rt_list_t *)l;
    certo_rt_fn1_t fn = (certo_rt_fn1_t)certo_rt_clo_fn(c);
    void *env = certo_rt_clo_env(c);
    int64_t n = list ? list->len : 0;
    for (int64_t i = 0; i < n; i++) if (!(fn(env, list->data[i]) & 0xff)) return 0;
    return 1;
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
