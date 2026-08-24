/// C implementations for `Stdlib.Core`.
pub const CORE_C: &str = r#"
/* ================================================================
   Stdlib.Core — core utilities
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

/* ---- FFI: free a heap buffer handed to a foreign caller ----
   Returned Text (e.g. from a DLL entry point) is a malloc'd buffer the caller
   owns once it has copied the contents. Exported so non-Certo clients can
   release it with the same allocator. Self-contained export attribute because
   CERTO_EXPORT is defined later in the translation unit. */
#ifdef _WIN32
__declspec(dllexport) void certo_free(void* p) { free(p); }
#else
__attribute__((visibility("default"))) void certo_free(void* p) { free(p); }
#endif

/* ---- print / println ---- */
/* Return int64_t so generated code can assign the result to an int64_t temp. */

int64_t certo_print(certo_text_t s) {
    fputs(s ? s : "", stdout);
    return 0;
}

int64_t certo_println(certo_text_t s) {
    puts(s ? s : "");
    return 0;
}

int64_t certo_flush(void) {
    fflush(stdout);
    return 0;
}

int64_t certo_eprint(certo_text_t s) {
    fputs(s ? s : "", stderr);
    return 0;
}

int64_t certo_eprintln(certo_text_t s) {
    fputs(s ? s : "", stderr);
    fputc('\n', stderr);
    return 0;
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

/* ---- Float32 ----
   `float` is a genuinely distinct C type from `double` (Ty::Float32 vs
   Ty::Float do not unify — see crates/typeck/src/unify.rs), so it needs its
   own construction/conversion path rather than reusing the Float ones. */

certo_text_t certo_float32_to_text(float f) {
    char* buf = (char*)malloc(64);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 64, "%g", (double)f);
    return buf;
}

int64_t certo_float32_to_int(float f) {
    return (int64_t)f;
}

float certo_int_to_float32(int64_t n) {
    return (float)n;
}

double certo_float32_to_float(float f) {
    return (double)f;
}

float certo_float_to_float32(double f) {
    return (float)f;
}

/* ---- Int8/Int16/Int32/UInt (BACKLOG item 235) ----
   Each is a genuinely distinct C type from the default int64_t Int (see
   crates/typeck/src/unify.rs), so — mirroring Float32 just above — each
   needs its own construction/conversion path rather than reusing Int's. */

certo_text_t certo_int8_to_text(int8_t n) {
    char* buf = (char*)malloc(8);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 8, "%" PRId8, n);
    return buf;
}
int64_t certo_int8_to_int(int8_t n) { return (int64_t)n; }
int8_t certo_int_to_int8(int64_t n) { return (int8_t)n; }

certo_text_t certo_int16_to_text(int16_t n) {
    char* buf = (char*)malloc(8);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 8, "%" PRId16, n);
    return buf;
}
int64_t certo_int16_to_int(int16_t n) { return (int64_t)n; }
int16_t certo_int_to_int16(int64_t n) { return (int16_t)n; }

certo_text_t certo_int32_to_text(int32_t n) {
    char* buf = (char*)malloc(16);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 16, "%" PRId32, n);
    return buf;
}
int64_t certo_int32_to_int(int32_t n) { return (int64_t)n; }
int32_t certo_int_to_int32(int64_t n) { return (int32_t)n; }

certo_text_t certo_uint_to_text(uint64_t n) {
    char* buf = (char*)malloc(24);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 24, "%" PRIu64, n);
    return buf;
}
int64_t certo_uint_to_int(uint64_t n) { return (int64_t)n; }
uint64_t certo_int_to_uint(int64_t n) { return (uint64_t)n; }

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

int64_t certo_assert(bool cond, certo_text_t msg) {
    if (!cond) certo_panic(msg ? msg : "assertion failed");
    return 0;
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

/* range()/rangeInclusive() live in collections.rs, right after CertoList/
   list_alloc are defined — see the comment there for why. */

/* ---- panic ---- */

__attribute__((noreturn)) void certo_panic(certo_text_t msg) {
    fflush(stdout);
    fprintf(stderr, "certo panic: %s\n", msg ? msg : "(no message)");
    fflush(stderr);
    abort();
}

/* ---- null coalesce / coerce ---- */
void* certo_coalesce(void* opt, void* fallback) {
    return opt ? opt : fallback;
}

/* text_concat lives in text.rs to avoid duplicate definitions */

/* ---- stdin ---- */

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string, or NULL on EOF / error. */
void* certo_read_line(void) {   /* Option<Text> */
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

    if (len == 0 && c == EOF) { free(buf); return NULL; }   /* None (EOF) */
    if (!buf) { buf = (char*)malloc(1); if (!buf) certo_panic("out of memory"); }
    buf[len] = '\0';
    return __certo_opt_box((int64_t)buf);   /* Some(line) */
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
void* certo_arg(int64_t i) {   /* Option<Text> */
    if (i < 0 || i >= (int64_t)__certo_argc) return NULL;   /* None */
    return __certo_opt_box((int64_t)__certo_argv[i]);       /* Some(text) */
}

/* ---- parseInt / parseFloat ---- */
/* Returns NULL (None) on failure, heap-allocated int64_t* on success. */
int64_t* certo_parse_int(certo_text_t s) {
    if (!s) return NULL;
    char *end;
    long long v = strtoll(s, &end, 10);
    if (end == s || *end != '\0') return NULL;
    int64_t *box = (int64_t*)malloc(sizeof(int64_t));
    *box = (int64_t)v;
    return box;
}

double* certo_parse_float(certo_text_t s) {
    if (!s) return NULL;
    char *end;
    double v = strtod(s, &end);
    if (end == s || *end != '\0') return NULL;
    double *box = (double*)malloc(sizeof(double));
    *box = v;
    return box;
}

/* Accepts exactly "true"/"false" (case-sensitive) — the same lowercase
   form `certo_bool_to_text` above already produces, so `parseBool` is a
   real inverse of it, not just an approximation. NULL (None) on anything
   else, same convention as `certo_parse_int`/`certo_parse_float`. */
bool* certo_parse_bool(certo_text_t s) {
    if (!s) return NULL;
    bool v;
    if (strcmp(s, "true") == 0) v = true;
    else if (strcmp(s, "false") == 0) v = false;
    else return NULL;
    bool *box = (bool*)malloc(sizeof(bool));
    *box = v;
    return box;
}

/* Monotonic millisecond counter for measuring elapsed time (not wall-clock).
   Windows headers arrive via the prelude (winsock2.h/windows.h). */
#ifndef _WIN32
#include <time.h>
#endif
int64_t certo_monotonic_millis(void) {
#ifdef _WIN32
    return (int64_t)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
#endif
}

/* Block the current thread for `ms` milliseconds. */
int64_t certo_sleep(int64_t ms) {
    if (ms <= 0) return 0;
#ifdef _WIN32
    Sleep((DWORD)ms);
#else
    struct timespec ts;
    ts.tv_sec  = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
#endif
    return 0;
}

/* identity :: forall T. T -> T (BACKLOG item 161) — an ordinary generic
   passthrough, same erased-value convention as any other stdlib ∀T
   function (e.g. getOrElse's own T-typed argument/return). */
void* certo_identity(void* x) { return x; }

/* expect :: forall T. T -> T (BACKLOG item 165) — the `expect(x).toBe(y)`
   assertion matchers' nominal receiver; identical, real identity, not a
   distinct implementation. */
#define certo_expect certo_identity

/* Option.isSome/isNone (BACKLOG item 165) — NULL means None throughout
   this runtime (see e.g. certo_text_parse_int above); a real, independently
   useful pair of predicates, not just plumbing for expect(...).toBeSome(). */
bool certo_option_is_some(void* o) { return o != NULL; }
bool certo_option_is_none(void* o) { return o == NULL; }
"#;
