#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

/* ---- Certo runtime types ---- */
typedef int64_t      certo_int_t;
typedef double       certo_float_t;
typedef const char*  certo_text_t;
typedef bool         certo_bool_t;

typedef struct { int _unused; } certo_unit_t;
#define CERTO_UNIT ((certo_unit_t){0})

/* Decimal: stored as scaled integer (cents × 10^scale). */
typedef struct { int64_t value; int8_t scale; } certo_decimal_t;
#define CERTO_DECIMAL(s) certo_decimal_parse(s)
certo_decimal_t certo_decimal_parse(const char* s);

/* Text */
#define CERTO_STR(s) ((certo_text_t)(s))
certo_text_t certo_text_concat(certo_text_t a, certo_text_t b);
int64_t      certo_text_len(certo_text_t t);

/* UUID */
typedef struct { uint8_t bytes[16]; } certo_uuid_t;
#define CERTO_UUID(s) certo_uuid_parse(s)
certo_uuid_t certo_uuid_parse(const char* s);
certo_uuid_t certo_uuid_new(void);

/* Generic option (pointer-sized tag + value) */
typedef struct { bool has_value; void* value; } certo_option_t;
typedef struct { void* value; } certo_tuple_t;
typedef void (*certo_fn_t)(void);
typedef void* certo_error_t;

/* List (dynamic array) */
typedef struct { void** data; int64_t len; int64_t cap; } certo_list_base_t;
void* certo_list_new(int64_t len, void** elems);
void* certo_list_get(void* list, int64_t idx);
void  certo_list_push(void* list, void* elem);

/* Arithmetic helpers */
int64_t certo_pow(int64_t base, int64_t exp);
void*   certo_coalesce(void* opt, void* fallback);

/* Panic */
__attribute__((noreturn)) void certo_panic(certo_text_t msg);
#define certo_unreachable() certo_panic("unreachable")
#define certo_todo()        certo_panic("not yet implemented")

/* stdin */
certo_text_t certo_read_line(void);
certo_text_t certo_read_all(void);

/* argv — call certo_main_init(argc, argv) at the top of main() */
void    certo_main_init(int argc, const char** argv);
int64_t certo_arg_count(void);
certo_text_t certo_arg(int64_t i);

/* DB transaction stub */
void* __db_transaction(certo_fn_t thunk);

/* Try/unwrap (Result<T,E> → T, or propagate) */
void* __try_unwrap(void* result);

/* Record update */
void* __record_update(void* base, void* updates);
/* ---- end Certo runtime ---- */


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

/* ---- stdin ---- */

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string, or NULL on EOF / error. */
certo_text_t certo_read_line(void) {
    char*  buf  = NULL;
    size_t cap  = 0;
    size_t len  = 0;
    int    c;

    while ((c = fgetc(stdin)) != EOF) {
        if (len + 2 > cap) {
            cap = cap ? cap * 2 : 128;
            char* nb = (char*)realloc(buf, cap);
            if (!nb) { free(buf); certo_panic("out of memory"); }
            buf = nb;
        }
        if (c == '\n') break;
        buf[len++] = (char)c;
    }

    if (len == 0 && c == EOF) { free(buf); return NULL; }
    if (!buf) { buf = (char*)malloc(1); if (!buf) certo_panic("out of memory"); }
    buf[len] = '\0';
    return buf;
}

/* Read all of stdin into a single heap-allocated string. */
certo_text_t certo_read_all(void) {
    char*  buf = NULL;
    size_t cap = 0;
    size_t len = 0;
    int    c;

    while ((c = fgetc(stdin)) != EOF) {
        if (len + 2 > cap) {
            cap = cap ? cap * 2 : 4096;
            char* nb = (char*)realloc(buf, cap);
            if (!nb) { free(buf); certo_panic("out of memory"); }
            buf = nb;
        }
        buf[len++] = (char)c;
    }

    if (!buf) { buf = (char*)malloc(1); if (!buf) certo_panic("out of memory"); }
    buf[len] = '\0';
    return buf;
}

/* ---- argv ---- */

/* These globals are set by the certo_main_init() bootstrap call that the
   compiled main() must make before calling user code. */
static int          __certo_argc = 0;
static const char** __certo_argv = NULL;

void certo_main_init(int argc, const char** argv) {
    __certo_argc = argc;
    __certo_argv = argv;
}

int64_t certo_arg_count(void) {
    return (int64_t)__certo_argc;
}

/* Returns NULL (None) if index is out of range. */
certo_text_t certo_arg(int64_t i) {
    if (i < 0 || i >= (int64_t)__certo_argc) return NULL;
    return __certo_argv[i];
}

/* ================================================================
   Stdlib.Collections — List<T> and Map<K,V>
   ================================================================
   List is a heap-allocated struct:
     { int64_t len; int64_t cap; void** data; }

   Map is a simple open-addressing hash table:
     { int64_t len; int64_t cap; MapEntry* entries; }
     where MapEntry = { void* key; void* value; bool occupied; }
   ================================================================ */

/* ---- List ---- */

typedef struct {
    int64_t  len;
    int64_t  cap;
    void**   data;
} CertoList;

static CertoList* list_alloc(int64_t cap) {
    if (cap < 8) cap = 8;
    CertoList* l = (CertoList*)malloc(sizeof(CertoList));
    if (!l) certo_panic("out of memory");
    l->len  = 0;
    l->cap  = cap;
    l->data = (void**)malloc((size_t)cap * sizeof(void*));
    if (!l->data) certo_panic("out of memory");
    return l;
}

CertoList* certo_list_new_empty(void) {
    return list_alloc(8);
}

CertoList* certo_list_of(int64_t n, ...) {
    va_list ap;
    CertoList* l = list_alloc(n < 8 ? 8 : n);
    va_start(ap, n);
    for (int64_t i = 0; i < n; i++) l->data[i] = va_arg(ap, void*);
    va_end(ap);
    l->len = n;
    return l;
}

int64_t certo_list_len(CertoList* l) {
    return l ? l->len : 0;
}

void* certo_list_get_opt(CertoList* l, int64_t i) {
    if (!l || i < 0 || i >= l->len) return NULL;
    return l->data[i];
}

void* certo_list_get(CertoList* l, int64_t i) {
    if (!l || i < 0 || i >= l->len) certo_panic("list index out of bounds");
    return l->data[i];
}

CertoList* certo_list_push(CertoList* l, void* item) {
    /* Returns a new list (functional update). */
    CertoList* n = list_alloc(l ? l->len + 1 : 1);
    if (l) {
        memcpy(n->data, l->data, (size_t)l->len * sizeof(void*));
        n->len = l->len;
    }
    n->data[n->len++] = item;
    return n;
}

CertoList* certo_list_concat(CertoList* a, CertoList* b) {
    int64_t al = a ? a->len : 0;
    int64_t bl = b ? b->len : 0;
    CertoList* n = list_alloc(al + bl);
    if (a) memcpy(n->data,      a->data, (size_t)al * sizeof(void*));
    if (b) memcpy(n->data + al, b->data, (size_t)bl * sizeof(void*));
    n->len = al + bl;
    return n;
}

void* certo_list_first(CertoList* l) {
    return (l && l->len > 0) ? l->data[0] : NULL;
}

void* certo_list_last(CertoList* l) {
    return (l && l->len > 0) ? l->data[l->len - 1] : NULL;
}

CertoList* certo_list_slice(CertoList* l, int64_t start, int64_t end) {
    if (!l) return list_alloc(0);
    if (start < 0) start = 0;
    if (end > l->len) end = l->len;
    if (start >= end) return list_alloc(0);
    int64_t len = end - start;
    CertoList* n = list_alloc(len);
    memcpy(n->data, l->data + start, (size_t)len * sizeof(void*));
    n->len = len;
    return n;
}

CertoList* certo_list_reverse(CertoList* l) {
    if (!l) return list_alloc(0);
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) n->data[i] = l->data[l->len - 1 - i];
    n->len = l->len;
    return n;
}

/* map / filter / fold take function pointers — generated code supplies them */
typedef void* (*CertoFn1)(void*);
typedef void* (*CertoFn2)(void*, void*);
typedef bool  (*CertoPred)(void*);

CertoList* certo_list_map(CertoList* l, CertoFn1 f) {
    if (!l) return list_alloc(0);
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) n->data[i] = f(l->data[i]);
    n->len = l->len;
    return n;
}

CertoList* certo_list_filter(CertoList* l, CertoPred pred) {
    if (!l) return list_alloc(0);
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) {
        if (pred(l->data[i])) n->data[n->len++] = l->data[i];
    }
    return n;
}

void* certo_list_fold(CertoList* l, void* init, CertoFn2 f) {
    void* acc = init;
    if (!l) return acc;
    for (int64_t i = 0; i < l->len; i++) acc = f(acc, l->data[i]);
    return acc;
}

bool certo_list_contains_ptr(CertoList* l, void* item) {
    if (!l) return false;
    for (int64_t i = 0; i < l->len; i++) if (l->data[i] == item) return true;
    return false;
}

/* ---- Map (open-addressing, pointer-equality keys) ---- */

typedef struct {
    void* key;
    void* value;
    bool  occupied;
} MapEntry;

typedef struct {
    int64_t   len;
    int64_t   cap;
    MapEntry* entries;
} CertoMap;

static CertoMap* map_alloc(int64_t cap) {
    if (cap < 16) cap = 16;
    CertoMap* m = (CertoMap*)malloc(sizeof(CertoMap));
    if (!m) certo_panic("out of memory");
    m->len     = 0;
    m->cap     = cap;
    m->entries = (MapEntry*)calloc((size_t)cap, sizeof(MapEntry));
    if (!m->entries) certo_panic("out of memory");
    return m;
}

CertoMap* certo_map_new(void) {
    return map_alloc(16);
}

static int64_t map_probe(CertoMap* m, void* key) {
    int64_t h = (int64_t)((uintptr_t)key >> 3) % m->cap;
    while (m->entries[h].occupied && m->entries[h].key != key)
        h = (h + 1) % m->cap;
    return h;
}

CertoMap* certo_map_insert(CertoMap* m, void* key, void* value) {
    /* Copy-on-write: return a new map */
    CertoMap* n = map_alloc(m ? (m->len + 1) * 2 : 16);
    if (m) {
        for (int64_t i = 0; i < m->cap; i++) {
            if (!m->entries[i].occupied) continue;
            int64_t h = map_probe(n, m->entries[i].key);
            n->entries[h] = m->entries[i];
            n->len++;
        }
    }
    int64_t h = map_probe(n, key);
    if (!n->entries[h].occupied) n->len++;
    n->entries[h] = (MapEntry){ .key = key, .value = value, .occupied = true };
    return n;
}

void* certo_map_get(CertoMap* m, void* key) {
    if (!m) return NULL;
    int64_t h = map_probe(m, key);
    if (!m->entries[h].occupied) return NULL;
    return m->entries[h].value;
}

bool certo_map_contains(CertoMap* m, void* key) {
    return certo_map_get(m, key) != NULL;
}

int64_t certo_map_len(CertoMap* m) {
    return m ? m->len : 0;
}

CertoList* certo_map_keys(CertoMap* m) {
    CertoList* out = list_alloc(m ? m->len : 0);
    if (!m) return out;
    for (int64_t i = 0; i < m->cap; i++) {
        if (m->entries[i].occupied) out->data[out->len++] = m->entries[i].key;
    }
    return out;
}

CertoList* certo_map_values(CertoMap* m) {
    CertoList* out = list_alloc(m ? m->len : 0);
    if (!m) return out;
    for (int64_t i = 0; i < m->cap; i++) {
        if (m->entries[i].occupied) out->data[out->len++] = m->entries[i].value;
    }
    return out;
}

CertoMap* certo_map_remove(CertoMap* m, void* key) {
    if (!m) return map_alloc(16);
    CertoMap* n = map_alloc(m->len > 0 ? m->len * 2 : 16);
    for (int64_t i = 0; i < m->cap; i++) {
        if (!m->entries[i].occupied || m->entries[i].key == key) continue;
        int64_t h = map_probe(n, m->entries[i].key);
        n->entries[h] = m->entries[i];
        n->len++;
    }
    return n;
}

/* ================================================================
   Stdlib.Text — Text (certo_text_t = const char*) operations
   ================================================================ */

#include <ctype.h>

int64_t certo_text_len(certo_text_t s) {
    return s ? (int64_t)strlen(s) : 0;
}

certo_text_t certo_text_concat(certo_text_t a, certo_text_t b) {
    size_t la = a ? strlen(a) : 0;
    size_t lb = b ? strlen(b) : 0;
    char* out = (char*)malloc(la + lb + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, a ? a : "", la);
    memcpy(out + la, b ? b : "", lb);
    out[la + lb] = '\0';
    return out;
}

bool certo_text_eq(certo_text_t a, certo_text_t b) {
    if (!a && !b) return true;
    if (!a || !b) return false;
    return strcmp(a, b) == 0;
}

bool certo_text_contains(certo_text_t haystack, certo_text_t needle) {
    if (!haystack || !needle) return false;
    return strstr(haystack, needle) != NULL;
}

bool certo_text_starts_with(certo_text_t s, certo_text_t prefix) {
    if (!s || !prefix) return false;
    size_t pl = strlen(prefix);
    return strncmp(s, prefix, pl) == 0;
}

bool certo_text_ends_with(certo_text_t s, certo_text_t suffix) {
    if (!s || !suffix) return false;
    size_t sl = strlen(s), fl = strlen(suffix);
    if (fl > sl) return false;
    return strcmp(s + sl - fl, suffix) == 0;
}

certo_text_t certo_text_to_upper(certo_text_t s) {
    if (!s) return "";
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    for (size_t i = 0; i <= n; i++) out[i] = (char)toupper((unsigned char)s[i]);
    return out;
}

certo_text_t certo_text_to_lower(certo_text_t s) {
    if (!s) return "";
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    for (size_t i = 0; i <= n; i++) out[i] = (char)tolower((unsigned char)s[i]);
    return out;
}

certo_text_t certo_text_trim(certo_text_t s) {
    if (!s) return "";
    while (*s && isspace((unsigned char)*s)) s++;
    size_t n = strlen(s);
    while (n > 0 && isspace((unsigned char)s[n - 1])) n--;
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n);
    out[n] = '\0';
    return out;
}

certo_text_t certo_text_trim_start(certo_text_t s) {
    if (!s) return "";
    while (*s && isspace((unsigned char)*s)) s++;
    return s;  /* safe — points into caller's string */
}

certo_text_t certo_text_trim_end(certo_text_t s) {
    if (!s) return "";
    size_t n = strlen(s);
    while (n > 0 && isspace((unsigned char)s[n - 1])) n--;
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n);
    out[n] = '\0';
    return out;
}

certo_text_t certo_text_slice(certo_text_t s, int64_t start, int64_t end) {
    if (!s) return "";
    int64_t n = (int64_t)strlen(s);
    if (start < 0) start = 0;
    if (end > n) end = n;
    if (start >= end) return "";
    int64_t len = end - start;
    char* out = (char*)malloc((size_t)len + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s + start, (size_t)len);
    out[len] = '\0';
    return out;
}

/* Returns NULL (None) if not found, else pointer to int64_t index */
void* certo_text_index_of(certo_text_t haystack, certo_text_t needle) {
    if (!haystack || !needle) return NULL;
    const char* p = strstr(haystack, needle);
    if (!p) return NULL;
    int64_t* idx = (int64_t*)malloc(sizeof(int64_t));
    if (!idx) certo_panic("out of memory");
    *idx = (int64_t)(p - haystack);
    return idx;
}

certo_text_t certo_text_replace(certo_text_t s, certo_text_t from, certo_text_t to) {
    if (!s || !from || strlen(from) == 0) return s;
    size_t from_len = strlen(from);
    size_t to_len   = to ? strlen(to) : 0;

    /* Count occurrences */
    size_t count = 0;
    const char* p = s;
    while ((p = strstr(p, from))) { count++; p += from_len; }

    size_t new_len = strlen(s) + count * (to_len - from_len);
    char* out = (char*)malloc(new_len + 1);
    if (!out) certo_panic("out of memory");

    char* dst = out;
    p = s;
    const char* match;
    while ((match = strstr(p, from))) {
        size_t pre = (size_t)(match - p);
        memcpy(dst, p, pre); dst += pre;
        if (to) { memcpy(dst, to, to_len); dst += to_len; }
        p = match + from_len;
    }
    strcpy(dst, p);
    return out;
}

/* Returns a CertoList* of certo_text_t segments */
void* certo_text_split(certo_text_t s, certo_text_t sep) {
    CertoList* out = list_alloc(8);
    if (!s) return out;
    if (!sep || strlen(sep) == 0) {
        /* split into individual characters */
        for (size_t i = 0; s[i]; i++) {
            char* ch = (char*)malloc(2);
            if (!ch) certo_panic("out of memory");
            ch[0] = s[i]; ch[1] = '\0';
            out = (CertoList*)certo_list_push(out, ch);
        }
        return out;
    }
    size_t sep_len = strlen(sep);
    const char* p = s;
    const char* match;
    while ((match = strstr(p, sep))) {
        size_t len = (size_t)(match - p);
        char* seg = (char*)malloc(len + 1);
        if (!seg) certo_panic("out of memory");
        memcpy(seg, p, len); seg[len] = '\0';
        out = (CertoList*)certo_list_push(out, seg);
        p = match + sep_len;
    }
    out = (CertoList*)certo_list_push(out, (void*)p);  /* last segment */
    return out;
}

certo_text_t certo_text_join(CertoList* parts, certo_text_t sep) {
    if (!parts || parts->len == 0) return "";
    size_t sep_len = sep ? strlen(sep) : 0;
    size_t total = 0;
    for (int64_t i = 0; i < parts->len; i++) {
        certo_text_t s = (certo_text_t)parts->data[i];
        if (s) total += strlen(s);
        if (i + 1 < parts->len) total += sep_len;
    }
    char* out = (char*)malloc(total + 1);
    if (!out) certo_panic("out of memory");
    char* dst = out;
    for (int64_t i = 0; i < parts->len; i++) {
        certo_text_t s = (certo_text_t)parts->data[i];
        if (s) { size_t n = strlen(s); memcpy(dst, s, n); dst += n; }
        if (sep && i + 1 < parts->len) { memcpy(dst, sep, sep_len); dst += sep_len; }
    }
    *dst = '\0';
    return out;
}

certo_text_t certo_text_repeat(certo_text_t s, int64_t n) {
    if (!s || n <= 0) return "";
    size_t sl = strlen(s);
    char* out = (char*)malloc(sl * (size_t)n + 1);
    if (!out) certo_panic("out of memory");
    for (int64_t i = 0; i < n; i++) memcpy(out + (size_t)i * sl, s, sl);
    out[sl * (size_t)n] = '\0';
    return out;
}

/* ================================================================
   Stdlib.DateTime
   Dates are stored as int64_t Unix timestamps (seconds since epoch).
   Date-only values use midnight UTC.
   ================================================================ */

#include <time.h>

typedef int64_t CertoDateTime;   /* Unix seconds */
typedef int64_t CertoDate;       /* Unix seconds at midnight UTC */

/* ---- constructors ---- */

CertoDateTime certo_datetime_now(void) {
    return (CertoDateTime)time(NULL);
}

CertoDate certo_date_today(void) {
    time_t now = time(NULL);
    struct tm* t = gmtime(&now);
    t->tm_hour = 0; t->tm_min = 0; t->tm_sec = 0;
    return (CertoDate)timegm(t);
}

CertoDateTime certo_datetime_from_unix(int64_t secs) {
    return (CertoDateTime)secs;
}

int64_t certo_datetime_to_unix(CertoDateTime dt) {
    return (int64_t)dt;
}

/* ---- formatting ---- */

certo_text_t certo_datetime_format(CertoDateTime dt, certo_text_t fmt) {
    time_t t = (time_t)dt;
    struct tm* tm_info = gmtime(&t);
    char* buf = (char*)malloc(256);
    if (!buf) certo_panic("out of memory");
    strftime(buf, 256, fmt ? fmt : "%Y-%m-%dT%H:%M:%SZ", tm_info);
    return buf;
}

certo_text_t certo_date_format(CertoDate d, certo_text_t fmt) {
    return certo_datetime_format((CertoDateTime)d, fmt ? fmt : "%Y-%m-%d");
}

certo_text_t certo_datetime_to_iso(CertoDateTime dt) {
    return certo_datetime_format(dt, "%Y-%m-%dT%H:%M:%SZ");
}

/* ---- arithmetic ---- */

CertoDateTime certo_datetime_add_seconds(CertoDateTime dt, int64_t s) { return dt + s; }
CertoDateTime certo_datetime_add_minutes(CertoDateTime dt, int64_t m) { return dt + m * 60; }
CertoDateTime certo_datetime_add_hours  (CertoDateTime dt, int64_t h) { return dt + h * 3600; }
CertoDateTime certo_datetime_add_days   (CertoDateTime dt, int64_t d) { return dt + d * 86400; }

int64_t certo_datetime_diff_seconds(CertoDateTime a, CertoDateTime b) { return a - b; }
int64_t certo_datetime_diff_days   (CertoDateTime a, CertoDateTime b) { return (a - b) / 86400; }

/* ---- comparison ---- */

bool certo_datetime_before(CertoDateTime a, CertoDateTime b) { return a < b; }
bool certo_datetime_after (CertoDateTime a, CertoDateTime b) { return a > b; }
bool certo_datetime_eq    (CertoDateTime a, CertoDateTime b) { return a == b; }

/* ---- components ---- */

int64_t certo_datetime_year  (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_year + 1900; }
int64_t certo_datetime_month (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_mon + 1; }
int64_t certo_datetime_day   (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_mday; }
int64_t certo_datetime_hour  (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_hour; }
int64_t certo_datetime_minute(CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_min; }
int64_t certo_datetime_second(CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_sec; }

/* ---- parse ISO 8601 ---- */
CertoDateTime certo_datetime_parse_iso(certo_text_t s) {
    if (!s) certo_panic("datetime_parse_iso: null input");
    struct tm t = {0};
    /* Minimal parser: YYYY-MM-DDTHH:MM:SSZ */
    if (sscanf(s, "%d-%d-%dT%d:%d:%d",
               &t.tm_year, &t.tm_mon, &t.tm_mday,
               &t.tm_hour, &t.tm_min, &t.tm_sec) < 3)
        certo_panic("datetime_parse_iso: invalid format");
    t.tm_year -= 1900;
    t.tm_mon  -= 1;
    return (CertoDateTime)timegm(&t);
}

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

/* Generated by Certo compiler — do not edit */

certo_error_t certo_main(void);
certo_error_t certo_dispatch(certo_error_t _l0);
certo_error_t certo_runCommand(void);
certo_error_t certo_listTasks(void);
certo_error_t certo_printHelp(void);

certo_error_t certo_main(void) {
    certo_error_t _l0;
    certo_error_t _l1;
    certo_error_t _l2;
  bb0:
    _l1 = certo_argCount();
    goto bb1;
  bb1:
    _l2 = certo_dispatch(_l1);
    goto bb2;
  bb2:
    _l0 = _l2;
    return _l0;
}

certo_error_t certo_dispatch(certo_error_t _l1) {
    certo_error_t _l0;
    certo_error_t _l2;
    certo_error_t _l3;
    certo_error_t _l4;
    certo_error_t _l5;
  bb0:
    _l2 = (_l1 < 2);
    if (_l2) goto bb1; else goto bb2;
  bb1:
    _l4 = certo_printHelp();
    goto bb4;
  bb2:
    _l5 = certo_runCommand();
    goto bb5;
  bb3:
    _l0 = _l3;
    return _l0;
  bb4:
    _l3 = _l4;
    goto bb3;
  bb5:
    _l3 = _l5;
    goto bb3;
}

certo_error_t certo_runCommand(void) {
    certo_error_t _l0;
    certo_error_t _l1;
    certo_error_t _l2;
    certo_error_t _l3;
    certo_error_t _l4;
    certo_error_t _l5;
    certo_error_t _l6;
    certo_error_t _l7;
  bb0:
    _l1 = certo_print(CERTO_STR("Command received. Total args (including program name): "));
    goto bb1;
  bb1:
    _l2 = certo_argCount();
    goto bb2;
  bb2:
    _l3 = certo_intToText(_l2);
    goto bb3;
  bb3:
    _l4 = certo_println(_l3);
    goto bb4;
  bb4:
    _l5 = certo_print(CERTO_STR("Running tasks list:"));
    goto bb5;
  bb5:
    _l6 = certo_println(CERTO_STR(""));
    goto bb6;
  bb6:
    _l7 = certo_listTasks();
    goto bb7;
  bb7:
    _l0 = _l7;
    return _l0;
}

certo_error_t certo_listTasks(void) {
    certo_error_t _l0;
    certo_error_t _l1;
    certo_error_t _l2;
    certo_error_t _l3;
  bb0:
    _l1 = certo_println(CERTO_STR("  1. Write Certo compiler  [done]"));
    goto bb1;
  bb1:
    _l2 = certo_println(CERTO_STR("  2. Add stdlib I/O        [done]"));
    goto bb2;
  bb2:
    _l3 = certo_println(CERTO_STR("  3. Ship it               [pending]"));
    goto bb3;
  bb3:
    _l0 = _l3;
    return _l0;
}

certo_error_t certo_printHelp(void) {
    certo_error_t _l0;
    certo_error_t _l1;
    certo_error_t _l2;
    certo_error_t _l3;
    certo_error_t _l4;
    certo_error_t _l5;
    certo_error_t _l6;
    certo_error_t _l7;
    certo_error_t _l8;
  bb0:
    _l1 = certo_println(CERTO_STR("Tasks CLI — Certo demo"));
    goto bb1;
  bb1:
    _l2 = certo_println(CERTO_STR(""));
    goto bb2;
  bb2:
    _l3 = certo_println(CERTO_STR("Usage:  tasks <command>"));
    goto bb3;
  bb3:
    _l4 = certo_println(CERTO_STR(""));
    goto bb4;
  bb4:
    _l5 = certo_println(CERTO_STR("Commands:"));
    goto bb5;
  bb5:
    _l6 = certo_println(CERTO_STR("  list   Show the task list"));
    goto bb6;
  bb6:
    _l7 = certo_println(CERTO_STR("  add    Add a task (stub)"));
    goto bb7;
  bb7:
    _l8 = certo_println(CERTO_STR("  help   Show this message"));
    goto bb8;
  bb8:
    _l0 = _l8;
    return _l0;
}
