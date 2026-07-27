pub const PATH_C: &str = r#"
/* ================================================================
   Stdlib.Path — path manipulation (pure string operations)
   Handles both '/' and '\' as separators.
   ================================================================ */

#include <string.h>
#include <stdlib.h>

static bool is_sep(char c) { return c == '/' || c == '\\'; }

/* Find index of last separator, or -1. */
static int64_t last_sep(certo_text_t path, size_t len) {
    for (int64_t i = (int64_t)len - 1; i >= 0; i--)
        if (is_sep(path[i])) return i;
    return -1;
}

/* Join two path segments, inserting a separator if needed. */
certo_text_t certo_path_join(certo_text_t a, certo_text_t b) {
    if (!a || !*a) return b ? b : "";
    if (!b || !*b) return a;
    size_t la = strlen(a);
    size_t lb = strlen(b);
    bool has_sep = is_sep(a[la - 1]) || is_sep(b[0]);
    size_t total = la + lb + (has_sep ? 1 : 2);
    char* out = (char*)malloc(total);
    if (!out) certo_panic("out of memory");
    memcpy(out, a, la);
    size_t pos = la;
    if (!is_sep(a[la - 1]) && !is_sep(b[0]))
        out[pos++] = '/';
    memcpy(out + pos, b, lb + 1);
    return out;
}

/* Last component of a path (after last separator). */
certo_text_t certo_path_basename(certo_text_t path) {
    if (!path || !*path) return "";
    size_t len = strlen(path);
    /* Strip trailing separators */
    while (len > 1 && is_sep(path[len - 1])) len--;
    int64_t sep = last_sep(path, len);
    const char* start = path + (sep >= 0 ? sep + 1 : 0);
    size_t blen = len - (size_t)(sep >= 0 ? sep + 1 : 0);
    char* out = (char*)malloc(blen + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, start, blen);
    out[blen] = '\0';
    return out;
}

/* Everything before the last separator. Returns "." if no separator. */
certo_text_t certo_path_dirname(certo_text_t path) {
    if (!path || !*path) return ".";
    size_t len = strlen(path);
    /* Strip trailing separators (but not a leading one) */
    while (len > 1 && is_sep(path[len - 1])) len--;
    int64_t sep = last_sep(path, len);
    if (sep < 0) return ".";
    if (sep == 0) {
        char* out = (char*)malloc(2);
        if (!out) certo_panic("out of memory");
        out[0] = path[0]; out[1] = '\0';
        return out;
    }
    char* out = (char*)malloc((size_t)sep + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, path, (size_t)sep);
    out[sep] = '\0';
    return out;
}

/* Extension of the last component (after the last dot), or NULL if none. */
void* certo_path_extension(certo_text_t path) {   /* Option<Text> */
    if (!path || !*path) return NULL;
    size_t len = strlen(path);
    int64_t sep = last_sep(path, len);
    const char* base = path + (sep >= 0 ? sep + 1 : 0);
    const char* dot  = strrchr(base, '.');
    if (!dot || dot == base) return NULL;  /* no ext or hidden file */
    size_t elen = strlen(dot + 1);
    if (elen == 0) return NULL;
    char* out = (char*)malloc(elen + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, dot + 1, elen + 1);
    return __certo_opt_box((int64_t)out);   /* Some(ext) */
}

/* Strip the extension from a path. */
certo_text_t certo_path_stem(certo_text_t path) {
    if (!path || !*path) return "";
    size_t len = strlen(path);
    int64_t sep = last_sep(path, len);
    const char* base = path + (sep >= 0 ? sep + 1 : 0);
    const char* dot  = strrchr(base, '.');
    size_t keep;
    if (!dot || dot == base) keep = len;
    else keep = (size_t)(dot - path);
    char* out = (char*)malloc(keep + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, path, keep);
    out[keep] = '\0';
    return out;
}
"#;
