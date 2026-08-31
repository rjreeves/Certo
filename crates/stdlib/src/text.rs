/// C implementations for `Stdlib.Text`.
pub const TEXT_C: &str = r#"
/* ================================================================
   Stdlib.Text — Text (certo_text_t = const char*) operations
   ================================================================ */

#include <ctype.h>
#ifdef _WIN32
#include <windows.h>
#endif

int64_t certo_text_len(certo_text_t s) {
    return s ? (int64_t)strlen(s) : 0;
}

/* UTF-8 byte count. Certo Text values are UTF-8 encoded C strings, so this
   is identical to certo_text_len — a distinct name exists because callers
   who need character count (not yet implemented, see BACKLOG) must not
   accidentally reach for `len` and get bytes instead. */
int64_t certo_text_byte_length(certo_text_t s) {
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

/* ASCII-only case conversion — a byte-at-a-time toupper()/tolower() loop.
   Correct only for pure-ASCII input: any other byte is passed through
   unchanged (never *wrong*, since it's a genuine no-op on non-ASCII bytes,
   but not real Unicode case conversion either — e.g. "straße" stays
   "STRAßE", not the real "STRASSE"). Used as the real, unchanged POSIX
   implementation (kept exactly as it always was — this item's own POSIX
   gap is documented below, not silently papered over) and as a Windows
   fallback for the vanishingly unlikely case ICU genuinely isn't present. */
static certo_text_t certo_text_to_upper_ascii(certo_text_t s) {
    if (!s) return "";
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    for (size_t i = 0; i <= n; i++) out[i] = (char)toupper((unsigned char)s[i]);
    return out;
}

static certo_text_t certo_text_to_lower_ascii(certo_text_t s) {
    if (!s) return "";
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    for (size_t i = 0; i <= n; i++) out[i] = (char)tolower((unsigned char)s[i]);
    return out;
}

#ifdef _WIN32
/* Real Unicode-aware (and, with a non-NULL locale, locale-aware) case
   conversion via the OS-bundled ICU (`icuuc.dll`, present since Windows 10
   1903 alongside `icuin.dll` — item 118's own Timezone work already
   confirmed this bundling and its UNVERSIONED exported symbols, unlike
   classic ICU4C's documented versioned convention). Loaded the same way:
   `LoadLibraryA`+`GetProcAddress`, no ICU SDK/headers needed. Handles
   multi-character expansions (German "ß" -> "SS") a byte-at-a-time
   toupper()/tolower() cannot express at all — confirmed via Unicode's own
   SpecialCasing data that this is the *unconditional default* uppercase
   mapping, not a locale-specific exception, so even the no-locale
   `certo_text_to_upper` needs it for real correctness — BACKLOG item 117.
   A POSIX equivalent was investigated and deliberately not attempted: real
   Linux ICU4C packages export *versioned* symbols (`u_strToUpper_74`, not
   `u_strToUpper`), unlike Windows' bundle, and this couldn't be verified
   end-to-end in this dev environment — see `certo_text_to_upper_locale`'s
   POSIX branch below. */
typedef uint16_t CertoUCharT;
typedef int32_t CertoUErrorCodeT;
typedef int32_t (*certo_u_strToUpper_fn)(CertoUCharT*, int32_t, const CertoUCharT*, int32_t, const char*, CertoUErrorCodeT*);
typedef int32_t (*certo_u_strToLower_fn)(CertoUCharT*, int32_t, const CertoUCharT*, int32_t, const char*, CertoUErrorCodeT*);

#define CERTO_U_BUFFER_OVERFLOW_ERROR 15

static certo_u_strToUpper_fn __certo_u_strToUpper;
static certo_u_strToLower_fn __certo_u_strToLower;
static INIT_ONCE __certo_icuuc_init_once = INIT_ONCE_STATIC_INIT;

static BOOL CALLBACK __certo_icuuc_init(PINIT_ONCE once, PVOID param, PVOID* ctx) {
    (void)once; (void)param; (void)ctx;
    HMODULE h = LoadLibraryA("icuuc.dll");
    if (!h) return TRUE; /* function pointers stay NULL; caller falls back to ASCII */
    __certo_u_strToUpper = (certo_u_strToUpper_fn)GetProcAddress(h, "u_strToUpper");
    __certo_u_strToLower = (certo_u_strToLower_fn)GetProcAddress(h, "u_strToLower");
    return TRUE;
}

static void __certo_text_ensure_icuuc(void) {
    InitOnceExecuteOnce(&__certo_icuuc_init_once, __certo_icuuc_init, NULL, NULL);
}

/* `*out_len` excludes the null terminator (matches ICU's own explicit-
   length string convention — the output buffer isn't guaranteed to be
   null-terminated by u_strToUpper/u_strToLower). Returns NULL on failure. */
static CertoUCharT* __certo_text_utf8_to_utf16_alloc(const char* s, int* out_len) {
    int wlen = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (wlen <= 0) { *out_len = 0; return NULL; }
    CertoUCharT* buf = (CertoUCharT*)malloc((size_t)wlen * sizeof(CertoUCharT));
    if (!buf) certo_panic("out of memory");
    MultiByteToWideChar(CP_UTF8, 0, s, -1, (wchar_t*)buf, wlen);
    *out_len = wlen - 1;
    return buf;
}

/* `locale` may be NULL for ICU's locale-independent default mapping
   (still real Unicode case conversion, e.g. ß -> SS — just not the
   locale-*conditional* cases like Turkish dotless-i). */
static certo_text_t __certo_text_case_convert(certo_text_t s, const char* locale, int to_upper) {
    if (!s) return "";
    __certo_text_ensure_icuuc();
    if (to_upper ? !__certo_u_strToUpper : !__certo_u_strToLower) {
        return to_upper ? certo_text_to_upper_ascii(s) : certo_text_to_lower_ascii(s);
    }

    int src_len = 0;
    CertoUCharT* src = __certo_text_utf8_to_utf16_alloc(s, &src_len);
    if (!src) return to_upper ? certo_text_to_upper_ascii(s) : certo_text_to_lower_ascii(s);

    CertoUErrorCodeT status = 0;
    int32_t need = to_upper
        ? __certo_u_strToUpper(NULL, 0, src, src_len, locale, &status)
        : __certo_u_strToLower(NULL, 0, src, src_len, locale, &status);
    if (need <= 0) { free(src); return ""; }

    CertoUCharT* dst = (CertoUCharT*)malloc((size_t)need * sizeof(CertoUCharT));
    if (!dst) certo_panic("out of memory");
    status = 0;
    int32_t written = to_upper
        ? __certo_u_strToUpper(dst, need, src, src_len, locale, &status)
        : __certo_u_strToLower(dst, need, src, src_len, locale, &status);
    free(src);
    if (status > 0 && status != CERTO_U_BUFFER_OVERFLOW_ERROR) {
        /* Genuine ICU error (e.g. an unrecognized locale tag) — the
           safest fallback is the original text unchanged, not a crash. */
        free(dst);
        return s;
    }

    int out_len = WideCharToMultiByte(CP_UTF8, 0, (wchar_t*)dst, written, NULL, 0, NULL, NULL);
    char* out = (char*)malloc((size_t)out_len + 1);
    if (!out) certo_panic("out of memory");
    WideCharToMultiByte(CP_UTF8, 0, (wchar_t*)dst, written, out, out_len, NULL, NULL);
    out[out_len] = '\0';
    free(dst);
    return out;
}
#endif

certo_text_t certo_text_to_upper(certo_text_t s) {
#ifdef _WIN32
    return __certo_text_case_convert(s, NULL, 1);
#else
    return certo_text_to_upper_ascii(s);
#endif
}

certo_text_t certo_text_to_lower(certo_text_t s) {
#ifdef _WIN32
    return __certo_text_case_convert(s, NULL, 0);
#else
    return certo_text_to_lower_ascii(s);
#endif
}

/* Locale-aware case conversion (BACKLOG item 117) — e.g. certo_text_to_upper_locale(s, "tr")
   correctly maps "i" to dotless "I" for Turkish, which the locale-independent
   certo_text_to_upper above does not (that's the whole point of a locale
   being *conditional*, not part of Unicode's default mapping). Real,
   ICU-backed implementation on Windows; POSIX has no OS-bundled Unicode
   library to load and a `dlopen`-based path couldn't be verified
   end-to-end in this dev environment (see the ICU block above) — a clear
   runtime error, not a silent no-op or ASCII fallback that would look
   like it worked while quietly ignoring the locale. */
certo_text_t certo_text_to_upper_locale(certo_text_t s, certo_text_t locale) {
#ifdef _WIN32
    return __certo_text_case_convert(s, locale, 1);
#else
    (void)s; (void)locale;
    certo_panic("locale-aware text case conversion is not available on this platform yet");
    return "";
#endif
}

certo_text_t certo_text_to_lower_locale(certo_text_t s, certo_text_t locale) {
#ifdef _WIN32
    return __certo_text_case_convert(s, locale, 0);
#else
    (void)s; (void)locale;
    certo_panic("locale-aware text case conversion is not available on this platform yet");
    return "";
#endif
}

/* ---- Char — a real Unicode scalar value (spec §4.1: "4 bytes, a Unicode
   scalar value, not a byte"), BACKLOG item 258. Certo Text is UTF-8, so
   accessing "the i-th character" must decode UTF-8 to codepoints rather than
   index raw bytes — the previous byte-indexed implementation corrupted any
   multi-byte character it touched (`Text.charAt("héllo", 1)` returned a lone
   continuation byte of 'é', not 'é' itself). `Char`'s own C representation
   changed accordingly, from a 1-byte `char` to `int32_t` (`ty_to_c.rs`),
   matching the spec's explicit size and holding any legal codepoint up to
   0x10FFFF. */

/* Decode one UTF-8 codepoint starting at byte offset `*pos` (advanced past
   the whole sequence, even on failure, so a caller's scanning loop always
   makes progress). Returns -1 for invalid/truncated UTF-8 at this position —
   `*pos` is still advanced by (at least) one byte so callers can skip over
   it and keep counting real characters in the rest of the string. */
static int32_t __certo_utf8_decode_at(const unsigned char* s, size_t len, size_t* pos) {
    size_t i = *pos;
    unsigned char b0 = s[i];
    int32_t cp; size_t seq_len;
    if ((b0 & 0x80) == 0x00)      { cp = b0;          seq_len = 1; }
    else if ((b0 & 0xE0) == 0xC0) { cp = b0 & 0x1F;    seq_len = 2; }
    else if ((b0 & 0xF0) == 0xE0) { cp = b0 & 0x0F;    seq_len = 3; }
    else if ((b0 & 0xF8) == 0xF0) { cp = b0 & 0x07;    seq_len = 4; }
    else { *pos = i + 1; return -1; } /* not a valid UTF-8 leading byte */
    if (i + seq_len > len) { *pos = len; return -1; } /* truncated sequence */
    for (size_t k = 1; k < seq_len; k++) {
        unsigned char bk = s[i + k];
        if ((bk & 0xC0) != 0x80) { *pos = i + 1; return -1; } /* bad continuation byte */
        cp = (cp << 6) | (bk & 0x3F);
    }
    *pos = i + seq_len;
    return cp;
}

/* Bounds-checked single-character access, counting real Unicode characters
   (codepoints), not bytes — `i` is a character index. Returns Option<Char>
   (heap-boxed via __certo_opt_box, same convention as every other
   Option-returning stdlib function; `ty_to_c(Ty::Char)` is now `int32_t`, so
   the generic Some/None unboxing machinery reads back the right width). */
void* certo_text_char_at(certo_text_t s, int64_t i) {
    if (!s || i < 0) return NULL;
    size_t len = strlen(s);
    size_t pos = 0;
    int64_t idx = 0;
    while (pos < len) {
        int32_t cp = __certo_utf8_decode_at((const unsigned char*)s, len, &pos);
        if (cp < 0) continue; /* invalid byte at this position — already skipped, keep scanning */
        if (idx == i) return __certo_opt_box((int64_t)cp);
        idx++;
    }
    return NULL;
}

/* Encodes a Unicode scalar value back to its UTF-8 byte sequence (1-4 bytes). */
certo_text_t certo_char_to_text(int32_t c) {
    char* out = (char*)malloc(5);
    if (!out) certo_panic("out of memory");
    size_t n = 0;
    if (c < 0) c = 0xFFFD; /* U+FFFD REPLACEMENT CHARACTER, for a bogus negative value */
    if (c <= 0x7F) {
        out[n++] = (char)c;
    } else if (c <= 0x7FF) {
        out[n++] = (char)(0xC0 | (c >> 6));
        out[n++] = (char)(0x80 | (c & 0x3F));
    } else if (c <= 0xFFFF) {
        out[n++] = (char)(0xE0 | (c >> 12));
        out[n++] = (char)(0x80 | ((c >> 6) & 0x3F));
        out[n++] = (char)(0x80 | (c & 0x3F));
    } else {
        out[n++] = (char)(0xF0 | (c >> 18));
        out[n++] = (char)(0x80 | ((c >> 12) & 0x3F));
        out[n++] = (char)(0x80 | ((c >> 6) & 0x3F));
        out[n++] = (char)(0x80 | (c & 0x3F));
    }
    out[n] = '\0';
    return out;
}

int64_t certo_char_to_int(int32_t c) {
    return (int64_t)c;
}

/* Out-of-range input (outside a legal Unicode scalar value) is a caller bug,
   not a representable failure mode worth threading through every call site —
   same convention as certo_float_to_int's own silent truncation. Unlike the
   previous byte-truncating `& 0xFF`, this no longer clips a real character
   down to ASCII. */
int32_t certo_char_from_int(int64_t n) {
    return (int32_t)n;
}

/* isdigit/isalpha/etc. (ctype.h) are only well-defined for a value
   representable as unsigned char or EOF — passing an arbitrary Unicode
   codepoint (e.g. 0x65E5, a CJK character) is undefined behavior, not just
   "wrong". Gated to the ASCII range: `isX` conservatively answers false for
   anything non-ASCII (never *wrong*, since it's a genuine "not a plain ASCII
   digit/letter", but not full Unicode general-category classification
   either — that needs a real Unicode character database, out of scope
   here), and the two case-conversion functions return the codepoint
   unchanged rather than mis-converting it — matching this file's own
   already-established ASCII-only-is-honest-not-wrong convention (see
   certo_text_to_upper_ascii's own doc comment above). */
bool certo_char_is_digit(int32_t c)      { return c >= 0 && c <= 127 && isdigit((unsigned char)c) != 0; }
bool certo_char_is_alpha(int32_t c)      { return c >= 0 && c <= 127 && isalpha((unsigned char)c) != 0; }
bool certo_char_is_upper_case(int32_t c) { return c >= 0 && c <= 127 && isupper((unsigned char)c) != 0; }
bool certo_char_is_lower_case(int32_t c) { return c >= 0 && c <= 127 && islower((unsigned char)c) != 0; }
bool certo_char_is_whitespace(int32_t c) { return c >= 0 && c <= 127 && isspace((unsigned char)c) != 0; }
int32_t certo_char_to_upper_case(int32_t c) { return (c >= 0 && c <= 127) ? (int32_t)toupper((unsigned char)c) : c; }
int32_t certo_char_to_lower_case(int32_t c) { return (c >= 0 && c <= 127) ? (int32_t)tolower((unsigned char)c) : c; }

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
"#;
