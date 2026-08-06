/// C implementations for `Stdlib.Text`.
pub const TEXT_C: &str = r#"
/* ================================================================
   Stdlib.Text — Text (certo_text_t = const char*) operations
   ================================================================ */

#include <ctype.h>

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

/* ---- Char — a single ASCII byte, same byte-oriented convention as the rest
   of Text (toUpper/toLower/len are all byte-based, not real Unicode). ---- */

/* Bounds-checked single-character access — Text had no char-level access at
   all before this. Returns Option<Char> (heap-boxed via __certo_opt_box,
   same convention as every other Option-returning stdlib function). */
void* certo_text_char_at(certo_text_t s, int64_t i) {
    if (!s || i < 0) return NULL;
    size_t len = strlen(s);
    if ((size_t)i >= len) return NULL;
    return __certo_opt_box((int64_t)(unsigned char)s[i]);
}

certo_text_t certo_char_to_text(char c) {
    char* out = (char*)malloc(2);
    if (!out) certo_panic("out of memory");
    out[0] = c;
    out[1] = '\0';
    return out;
}

int64_t certo_char_to_int(char c) {
    return (int64_t)(unsigned char)c;
}

/* Truncates to a byte, same convention as certo_float_to_int's silent
   truncation rather than an Option — out-of-range input is a caller bug,
   not a representable failure mode worth threading through every call site. */
char certo_char_from_int(int64_t n) {
    return (char)(n & 0xFF);
}

bool certo_char_is_digit(char c)      { return isdigit((unsigned char)c) != 0; }
bool certo_char_is_alpha(char c)      { return isalpha((unsigned char)c) != 0; }
bool certo_char_is_upper_case(char c) { return isupper((unsigned char)c) != 0; }
bool certo_char_is_lower_case(char c) { return islower((unsigned char)c) != 0; }
bool certo_char_is_whitespace(char c) { return isspace((unsigned char)c) != 0; }
char certo_char_to_upper_case(char c) { return (char)toupper((unsigned char)c); }
char certo_char_to_lower_case(char c) { return (char)tolower((unsigned char)c); }

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
