pub const JSON_C: &str = r#"
/* ================================================================
   Stdlib.Json — pure-C JSON parser and serializer
   No external dependencies.
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>
#include <inttypes.h>

/* ---- Value type ------------------------------------------------- */

typedef enum {
    CERTO_JSON_NULL   = 0,
    CERTO_JSON_BOOL   = 1,
    CERTO_JSON_INT    = 2,
    CERTO_JSON_FLOAT  = 3,
    CERTO_JSON_STRING = 4,
    CERTO_JSON_ARRAY  = 5,
    CERTO_JSON_OBJECT = 6
} CertoJsonType;

typedef struct CertoJsonValue {
    CertoJsonType type;
    union {
        bool    b;
        int64_t i;
        double  f;
        char*   s;
        struct {
            struct CertoJsonValue** items;
            int64_t count;
            int64_t cap;
        } arr;
        struct {
            char**                  keys;
            struct CertoJsonValue** vals;
            int64_t                 count;
            int64_t                 cap;
        } obj;
    };
} CertoJsonValue;

/* ---- Allocator -------------------------------------------------- */

static CertoJsonValue* json_new(CertoJsonType t) {
    CertoJsonValue* v = (CertoJsonValue*)calloc(1, sizeof(CertoJsonValue));
    if (!v) certo_panic("out of memory");
    v->type = t;
    return v;
}

/* ---- Growing string buffer (for stringify) --------------------- */

typedef struct { char* buf; size_t pos; size_t cap; } JBuf;

static void jbuf_ensure(JBuf* b, size_t need) {
    if (b->pos + need > b->cap) {
        b->cap = (b->pos + need) * 2 + 64;
        b->buf = (char*)realloc(b->buf, b->cap);
        if (!b->buf) certo_panic("out of memory");
    }
}
static void jbuf_ch(JBuf* b, char c) { jbuf_ensure(b, 1); b->buf[b->pos++] = c; }
static void jbuf_raw(JBuf* b, const char* s, size_t n) { jbuf_ensure(b, n); memcpy(b->buf + b->pos, s, n); b->pos += n; }
static void jbuf_cstr(JBuf* b, const char* s) { jbuf_raw(b, s, strlen(s)); }

/* ---- Parser context -------------------------------------------- */

typedef struct { const char* src; size_t pos; size_t len; } JCtx;

static void jctx_ws(JCtx* c) {
    while (c->pos < c->len) {
        char ch = c->src[c->pos];
        if (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r') c->pos++;
        else break;
    }
}

static CertoJsonValue* jctx_value(JCtx* c);

static CertoJsonValue* jctx_string(JCtx* c) {
    /* expects '"' at c->pos */
    c->pos++;
    size_t cap = 64;
    char*  out = (char*)malloc(cap);
    if (!out) certo_panic("out of memory");
    size_t j = 0;
    while (c->pos < c->len && c->src[c->pos] != '"') {
        if (j + 8 >= cap) { cap *= 2; out = (char*)realloc(out, cap); if (!out) certo_panic("out of memory"); }
        if (c->src[c->pos] == '\\') {
            c->pos++;
            if (c->pos >= c->len) break;
            char e = c->src[c->pos++];
            switch (e) {
                case '"': out[j++] = '"';  break;
                case '\\':out[j++] = '\\'; break;
                case '/': out[j++] = '/';  break;
                case 'b': out[j++] = '\b'; break;
                case 'f': out[j++] = '\f'; break;
                case 'n': out[j++] = '\n'; break;
                case 'r': out[j++] = '\r'; break;
                case 't': out[j++] = '\t'; break;
                case 'u': {
                    if (c->pos + 4 <= c->len) {
                        char hex[5] = {0};
                        memcpy(hex, c->src + c->pos, 4);
                        c->pos += 4;
                        unsigned cp = (unsigned)strtoul(hex, NULL, 16);
                        if      (cp < 0x80)  { out[j++] = (char)cp; }
                        else if (cp < 0x800) { out[j++] = (char)(0xC0|(cp>>6)); out[j++] = (char)(0x80|(cp&0x3F)); }
                        else                 { out[j++] = (char)(0xE0|(cp>>12)); out[j++] = (char)(0x80|((cp>>6)&0x3F)); out[j++] = (char)(0x80|(cp&0x3F)); }
                    }
                    break;
                }
                default: out[j++] = e; break;
            }
        } else {
            out[j++] = c->src[c->pos++];
        }
    }
    if (c->pos < c->len && c->src[c->pos] == '"') c->pos++;
    out[j] = '\0';
    CertoJsonValue* v = json_new(CERTO_JSON_STRING);
    v->s = out;
    return v;
}

static CertoJsonValue* jctx_value(JCtx* c) {
    jctx_ws(c);
    if (c->pos >= c->len) return json_new(CERTO_JSON_NULL);
    char ch = c->src[c->pos];

    if (ch == 'n' && c->pos + 4 <= c->len && strncmp(c->src + c->pos, "null",  4) == 0) { c->pos += 4; return json_new(CERTO_JSON_NULL); }
    if (ch == 't' && c->pos + 4 <= c->len && strncmp(c->src + c->pos, "true",  4) == 0) { c->pos += 4; CertoJsonValue* v = json_new(CERTO_JSON_BOOL); v->b = true;  return v; }
    if (ch == 'f' && c->pos + 5 <= c->len && strncmp(c->src + c->pos, "false", 5) == 0) { c->pos += 5; CertoJsonValue* v = json_new(CERTO_JSON_BOOL); v->b = false; return v; }
    if (ch == '"') return jctx_string(c);

    if (ch == '[') {
        c->pos++;
        CertoJsonValue* v = json_new(CERTO_JSON_ARRAY);
        v->arr.cap   = 4;
        v->arr.items = (CertoJsonValue**)malloc((size_t)v->arr.cap * sizeof(CertoJsonValue*));
        if (!v->arr.items) certo_panic("out of memory");
        jctx_ws(c);
        if (c->pos < c->len && c->src[c->pos] == ']') { c->pos++; return v; }
        while (c->pos < c->len) {
            CertoJsonValue* item = jctx_value(c);
            if (v->arr.count >= v->arr.cap) {
                v->arr.cap *= 2;
                v->arr.items = (CertoJsonValue**)realloc(v->arr.items, (size_t)v->arr.cap * sizeof(CertoJsonValue*));
                if (!v->arr.items) certo_panic("out of memory");
            }
            v->arr.items[v->arr.count++] = item;
            jctx_ws(c);
            if (c->pos >= c->len) break;
            if (c->src[c->pos] == ']') { c->pos++; break; }
            if (c->src[c->pos] == ',') c->pos++;
        }
        return v;
    }

    if (ch == '{') {
        c->pos++;
        CertoJsonValue* v = json_new(CERTO_JSON_OBJECT);
        v->obj.cap  = 4;
        v->obj.keys = (char**)malloc((size_t)v->obj.cap * sizeof(char*));
        v->obj.vals = (CertoJsonValue**)malloc((size_t)v->obj.cap * sizeof(CertoJsonValue*));
        if (!v->obj.keys || !v->obj.vals) certo_panic("out of memory");
        jctx_ws(c);
        if (c->pos < c->len && c->src[c->pos] == '}') { c->pos++; return v; }
        while (c->pos < c->len) {
            jctx_ws(c);
            if (c->pos >= c->len || c->src[c->pos] != '"') break;
            CertoJsonValue* kv = jctx_string(c);
            char* key = kv ? kv->s : NULL;
            if (kv) { kv->s = NULL; free(kv); }
            jctx_ws(c);
            if (c->pos < c->len && c->src[c->pos] == ':') c->pos++;
            CertoJsonValue* val = jctx_value(c);
            if (v->obj.count >= v->obj.cap) {
                v->obj.cap *= 2;
                v->obj.keys = (char**)realloc(v->obj.keys, (size_t)v->obj.cap * sizeof(char*));
                v->obj.vals = (CertoJsonValue**)realloc(v->obj.vals, (size_t)v->obj.cap * sizeof(CertoJsonValue*));
                if (!v->obj.keys || !v->obj.vals) certo_panic("out of memory");
            }
            v->obj.keys[v->obj.count] = key;
            v->obj.vals[v->obj.count] = val;
            v->obj.count++;
            jctx_ws(c);
            if (c->pos >= c->len) break;
            if (c->src[c->pos] == '}') { c->pos++; break; }
            if (c->src[c->pos] == ',') c->pos++;
        }
        return v;
    }

    if (ch == '-' || (ch >= '0' && ch <= '9')) {
        size_t start = c->pos;
        bool is_float = false;
        if (c->src[c->pos] == '-') c->pos++;
        while (c->pos < c->len && c->src[c->pos] >= '0' && c->src[c->pos] <= '9') c->pos++;
        if (c->pos < c->len && c->src[c->pos] == '.') {
            is_float = true; c->pos++;
            while (c->pos < c->len && c->src[c->pos] >= '0' && c->src[c->pos] <= '9') c->pos++;
        }
        if (c->pos < c->len && (c->src[c->pos] == 'e' || c->src[c->pos] == 'E')) {
            is_float = true; c->pos++;
            if (c->pos < c->len && (c->src[c->pos] == '+' || c->src[c->pos] == '-')) c->pos++;
            while (c->pos < c->len && c->src[c->pos] >= '0' && c->src[c->pos] <= '9') c->pos++;
        }
        char numstr[64] = {0};
        size_t nlen = c->pos - start;
        if (nlen >= sizeof(numstr)) nlen = sizeof(numstr) - 1;
        memcpy(numstr, c->src + start, nlen);
        if (is_float) {
            CertoJsonValue* v = json_new(CERTO_JSON_FLOAT);
            v->f = strtod(numstr, NULL);
            return v;
        } else {
            CertoJsonValue* v = json_new(CERTO_JSON_INT);
            v->i = (int64_t)strtoll(numstr, NULL, 10);
            return v;
        }
    }

    return json_new(CERTO_JSON_NULL);
}

/* ---- Stringify helper ------------------------------------------ */

static void json_emit(JBuf* b, CertoJsonValue* v) {
    if (!v) { jbuf_cstr(b, "null"); return; }
    switch (v->type) {
        case CERTO_JSON_NULL:  jbuf_cstr(b, "null"); break;
        case CERTO_JSON_BOOL:  jbuf_cstr(b, v->b ? "true" : "false"); break;
        case CERTO_JSON_INT: {
            char tmp[32];
            int n = snprintf(tmp, sizeof(tmp), "%" PRId64, v->i);
            jbuf_raw(b, tmp, (size_t)n);
            break;
        }
        case CERTO_JSON_FLOAT: {
            char tmp[64];
            int n = snprintf(tmp, sizeof(tmp), "%.17g", v->f);
            jbuf_raw(b, tmp, (size_t)n);
            break;
        }
        case CERTO_JSON_STRING: {
            jbuf_ch(b, '"');
            if (v->s) {
                for (const char* p = v->s; *p; p++) {
                    unsigned char uc = (unsigned char)*p;
                    if      (uc == '"')  jbuf_raw(b, "\\\"", 2);
                    else if (uc == '\\') jbuf_raw(b, "\\\\", 2);
                    else if (uc == '\n') jbuf_raw(b, "\\n",  2);
                    else if (uc == '\r') jbuf_raw(b, "\\r",  2);
                    else if (uc == '\t') jbuf_raw(b, "\\t",  2);
                    else if (uc < 0x20) {
                        char esc[7];
                        snprintf(esc, sizeof(esc), "\\u%04x", uc);
                        jbuf_raw(b, esc, 6);
                    } else {
                        jbuf_ch(b, *p);
                    }
                }
            }
            jbuf_ch(b, '"');
            break;
        }
        case CERTO_JSON_ARRAY: {
            jbuf_ch(b, '[');
            for (int64_t i = 0; i < v->arr.count; i++) {
                if (i > 0) jbuf_ch(b, ',');
                json_emit(b, v->arr.items[i]);
            }
            jbuf_ch(b, ']');
            break;
        }
        case CERTO_JSON_OBJECT: {
            jbuf_ch(b, '{');
            for (int64_t i = 0; i < v->obj.count; i++) {
                if (i > 0) jbuf_ch(b, ',');
                CertoJsonValue ks = { CERTO_JSON_STRING };
                ks.s = v->obj.keys[i];
                json_emit(b, &ks);
                jbuf_ch(b, ':');
                json_emit(b, v->obj.vals[i]);
            }
            jbuf_ch(b, '}');
            break;
        }
    }
}

/* ================================================================
   Public API
   ================================================================ */

CertoJsonValue* certo_json_parse(certo_text_t text) {
    if (!text) return json_new(CERTO_JSON_NULL);
    JCtx ctx = { text, 0, strlen(text) };
    return jctx_value(&ctx);
}

certo_text_t certo_json_stringify(CertoJsonValue* v) {
    JBuf b = { NULL, 0, 256 };
    b.buf = (char*)malloc(b.cap);
    if (!b.buf) certo_panic("out of memory");
    json_emit(&b, v);
    jbuf_ch(&b, '\0');
    return b.buf;
}

/* Type predicates */
bool certo_json_is_null  (CertoJsonValue* v) { return !v || v->type == CERTO_JSON_NULL;   }
bool certo_json_is_bool  (CertoJsonValue* v) { return v && v->type == CERTO_JSON_BOOL;    }
bool certo_json_is_int   (CertoJsonValue* v) { return v && v->type == CERTO_JSON_INT;     }
bool certo_json_is_float (CertoJsonValue* v) { return v && (v->type == CERTO_JSON_FLOAT || v->type == CERTO_JSON_INT); }
bool certo_json_is_string(CertoJsonValue* v) { return v && v->type == CERTO_JSON_STRING;  }
bool certo_json_is_array (CertoJsonValue* v) { return v && v->type == CERTO_JSON_ARRAY;   }
bool certo_json_is_object(CertoJsonValue* v) { return v && v->type == CERTO_JSON_OBJECT;  }

/* Value extractors */
bool         certo_json_as_bool  (CertoJsonValue* v) { return v && v->type == CERTO_JSON_BOOL && v->b; }
int64_t      certo_json_as_int   (CertoJsonValue* v) { if (!v) return 0; if (v->type==CERTO_JSON_INT) return v->i; if (v->type==CERTO_JSON_FLOAT) return (int64_t)v->f; return 0; }
double       certo_json_as_float (CertoJsonValue* v) { if (!v) return 0.0; if (v->type==CERTO_JSON_FLOAT) return v->f; if (v->type==CERTO_JSON_INT) return (double)v->i; return 0.0; }
certo_text_t certo_json_as_string(CertoJsonValue* v) { return (v && v->type==CERTO_JSON_STRING && v->s) ? v->s : ""; }

/* Structural access */
int64_t certo_json_length(CertoJsonValue* v) {
    if (!v) return 0;
    if (v->type == CERTO_JSON_ARRAY)  return v->arr.count;
    if (v->type == CERTO_JSON_OBJECT) return v->obj.count;
    return 0;
}

CertoJsonValue* certo_json_at(CertoJsonValue* v, int64_t i) {
    if (!v || v->type != CERTO_JSON_ARRAY) return json_new(CERTO_JSON_NULL);
    if (i < 0 || i >= v->arr.count)       return json_new(CERTO_JSON_NULL);
    return v->arr.items[i];
}

/* Serialized array access for consumers that do not need to retain the
   JsonValue handle. This also avoids an unnecessary parse/stringify bridge in
   generated tooling code.

   BACKLOG item 336 — this returns the element's whole re-encoded JSON text,
   not an unwrapped value: for an object element that's the same text a
   nested Json.parse(...) call expects, but for a plain string element it
   still has its surrounding quotes ("foo", not foo). Unwrap with
   JsonValue.asText(Json.parse(JsonValue.atText(arr, i))) when the element is
   itself a string. */
certo_text_t certo_json_value_at_text(CertoJsonValue* v, int64_t i) {
    return certo_json_stringify(certo_json_at(v, i));
}

CertoJsonValue* certo_json_get(CertoJsonValue* v, certo_text_t key) {
    if (!v || v->type != CERTO_JSON_OBJECT || !key) return json_new(CERTO_JSON_NULL);
    for (int64_t i = 0; i < v->obj.count; i++)
        if (v->obj.keys[i] && strcmp(v->obj.keys[i], key) == 0)
            return v->obj.vals[i];
    return json_new(CERTO_JSON_NULL);
}

CertoList* certo_json_keys(CertoJsonValue* v) {
    CertoList* list = certo_list_new();
    if (!v || v->type != CERTO_JSON_OBJECT) return list;
    for (int64_t i = 0; i < v->obj.count; i++)
        certo_list_push(list, (void*)v->obj.keys[i]);
    return list;
}

/* Constructors */
CertoJsonValue* certo_json_null_val  ()              { return json_new(CERTO_JSON_NULL); }
CertoJsonValue* certo_json_bool_val  (bool b)        { CertoJsonValue* v = json_new(CERTO_JSON_BOOL);   v->b = b;       return v; }
CertoJsonValue* certo_json_int_val   (int64_t i)     { CertoJsonValue* v = json_new(CERTO_JSON_INT);    v->i = i;       return v; }
CertoJsonValue* certo_json_float_val (double f)      { CertoJsonValue* v = json_new(CERTO_JSON_FLOAT);  v->f = f;       return v; }
CertoJsonValue* certo_json_string_val(certo_text_t s){ CertoJsonValue* v = json_new(CERTO_JSON_STRING); v->s = s ? strdup(s) : NULL; return v; }
CertoJsonValue* certo_json_array_val ()              { CertoJsonValue* v = json_new(CERTO_JSON_ARRAY);  v->arr.cap = 4; v->arr.items = (CertoJsonValue**)malloc(4*sizeof(CertoJsonValue*)); return v; }
CertoJsonValue* certo_json_object_val()              { CertoJsonValue* v = json_new(CERTO_JSON_OBJECT); v->obj.cap = 4; v->obj.keys = (char**)malloc(4*sizeof(char*)); v->obj.vals = (CertoJsonValue**)malloc(4*sizeof(CertoJsonValue*)); return v; }

void certo_json_array_push(CertoJsonValue* arr, CertoJsonValue* item) {
    if (!arr || arr->type != CERTO_JSON_ARRAY) return;
    if (arr->arr.count >= arr->arr.cap) {
        arr->arr.cap = arr->arr.cap ? arr->arr.cap * 2 : 4;
        arr->arr.items = (CertoJsonValue**)realloc(arr->arr.items, (size_t)arr->arr.cap * sizeof(CertoJsonValue*));
        if (!arr->arr.items) certo_panic("out of memory");
    }
    arr->arr.items[arr->arr.count++] = item;
}

void certo_json_object_set(CertoJsonValue* obj, certo_text_t key, CertoJsonValue* val) {
    if (!obj || obj->type != CERTO_JSON_OBJECT || !key) return;
    for (int64_t i = 0; i < obj->obj.count; i++) {
        if (obj->obj.keys[i] && strcmp(obj->obj.keys[i], key) == 0) { obj->obj.vals[i] = val; return; }
    }
    if (obj->obj.count >= obj->obj.cap) {
        obj->obj.cap = obj->obj.cap ? obj->obj.cap * 2 : 4;
        obj->obj.keys = (char**)realloc(obj->obj.keys, (size_t)obj->obj.cap * sizeof(char*));
        obj->obj.vals = (CertoJsonValue**)realloc(obj->obj.vals, (size_t)obj->obj.cap * sizeof(CertoJsonValue*));
        if (!obj->obj.keys || !obj->obj.vals) certo_panic("out of memory");
    }
    obj->obj.keys[obj->obj.count] = strdup(key);
    obj->obj.vals[obj->obj.count] = val;

    obj->obj.count++;
}

/* Aliases so camelCase Certo names (Json.object → certo_json_object) resolve */
static inline CertoJsonValue* certo_json_object(void)                           { return certo_json_object_val(); }
static inline CertoJsonValue* certo_json_array(void)                            { return certo_json_array_val(); }
static inline CertoJsonValue* certo_json_string(certo_text_t s)                 { return certo_json_string_val(s); }
static inline CertoJsonValue* certo_json_int(int64_t i)                         { return certo_json_int_val(i); }
static inline CertoJsonValue* certo_json_float(double f)                        { return certo_json_float_val(f); }
static inline CertoJsonValue* certo_json_bool(bool b)                           { return certo_json_bool_val(b); }
static inline CertoJsonValue* certo_json_null(void)                             { return certo_json_null_val(); }
static inline int64_t          certo_json_value_set(CertoJsonValue* o, certo_text_t k, CertoJsonValue* v) { certo_json_object_set(o, k, v); return 0; }
static inline int64_t          certo_json_array_append(CertoJsonValue* a, CertoJsonValue* v)              { certo_json_array_push(a, v); return 0; }

/* Bridge codegen's JsonValue.* names (certo_json_value_*) to the implementations
   above. Placed after all definitions so only call sites are rewritten. */
#define certo_json_value_get        certo_json_get
#define certo_json_value_at         certo_json_at
#define certo_json_value_as_text    certo_json_as_string
#define certo_json_value_as_int     certo_json_as_int
#define certo_json_value_as_bool    certo_json_as_bool
#define certo_json_value_as_float   certo_json_as_float
#define certo_json_value_is_null    certo_json_is_null
#define certo_json_value_is_bool    certo_json_is_bool
#define certo_json_value_is_int     certo_json_is_int
#define certo_json_value_is_float   certo_json_is_float
#define certo_json_value_is_string  certo_json_is_string
#define certo_json_value_is_array   certo_json_is_array
#define certo_json_value_is_object  certo_json_is_object
#define certo_json_value_length     certo_json_length
#define certo_json_value_keys       certo_json_keys
/* JsonValue.push is registered (seed.rs) as returning Unit — every
   Unit-returning stdlib function compiles to int64_t 0 (see
   ret_ty_to_c's doc comment in crates/codegen/src/ty_to_c.rs), so this
   must alias to the already-defined int64_t-returning wrapper
   (certo_json_array_append, right above), not directly to the raw
   void-returning certo_json_array_push — that mismatch was a real,
   confirmed compile error ("assigning to 'int64_t' from incompatible
   type 'void'"), caught while building item 93's REST client generator. */
#define certo_json_value_push       certo_json_array_append
#define certo_json_value_null       certo_json_null
"#;
