pub const BYTES_C: &str = r#"
/* ================================================================
   Stdlib.Bytes — length-carrying, immutable binary buffers.
   Represented as an opaque handle (pointer-sized, like HttpResponse),
   so it rides through Option/List/tuple slots without corruption.
   ================================================================ */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

typedef struct { int64_t len; uint8_t* data; } CertoBytes;

/* Allocate an (uninitialised) buffer of `len` bytes. */
static CertoBytes* certo_bytes_alloc(int64_t len) {
    CertoBytes* b = (CertoBytes*)malloc(sizeof(CertoBytes));
    if (!b) certo_panic("out of memory");
    b->len  = len < 0 ? 0 : len;
    b->data = (uint8_t*)malloc((size_t)b->len + 1); /* +1 keeps data non-NULL for len 0 */
    if (!b->data) certo_panic("out of memory");
    b->data[b->len] = 0;
    return b;
}

int64_t certo_bytes_length(CertoBytes* b) { return b ? b->len : 0; }

CertoBytes* certo_bytes_empty(void) { return certo_bytes_alloc(0); }

/* Copy of bytes [start, end); indices are clamped to [0, len]. */
CertoBytes* certo_bytes_slice(CertoBytes* b, int64_t start, int64_t end) {
    int64_t len = b ? b->len : 0;
    if (start < 0)   start = 0;
    if (end > len)   end   = len;
    if (end < start) end   = start;
    int64_t n = end - start;
    CertoBytes* out = certo_bytes_alloc(n);
    if (n > 0 && b) memcpy(out->data, b->data + start, (size_t)n);
    return out;
}

CertoBytes* certo_bytes_concat(CertoBytes* a, CertoBytes* b) {
    int64_t la = a ? a->len : 0;
    int64_t lb = b ? b->len : 0;
    CertoBytes* out = certo_bytes_alloc(la + lb);
    if (la > 0 && a) memcpy(out->data,      a->data, (size_t)la);
    if (lb > 0 && b) memcpy(out->data + la, b->data, (size_t)lb);
    return out;
}

int64_t certo_bytes_byte_at(CertoBytes* b, int64_t index) {
    if (!b || index < 0 || index >= b->len) return -1;
    return (int64_t)b->data[index];
}

CertoBytes* certo_bytes_from_int64_l_e(int64_t value) {
    CertoBytes* out = certo_bytes_alloc(8);
    uint64_t bits = (uint64_t)value;
    for (int i = 0; i < 8; i++) out->data[i] = (uint8_t)(bits >> (i * 8));
    return out;
}

int64_t certo_bytes_read_int64_l_e(CertoBytes* b, int64_t offset) {
    if (!b || offset < 0 || offset > b->len - 8) return 0;
    uint64_t bits = 0;
    for (int i = 0; i < 8; i++) bits |= ((uint64_t)b->data[offset + i]) << (i * 8);
    return (int64_t)bits;
}

/* Decode a byte slice known by the caller to contain UTF-8 text. */
certo_text_t certo_bytes_to_text(CertoBytes* b) {
    int64_t len = b ? b->len : 0;
    char* out = (char*)malloc((size_t)len + 1);
    if (!out) certo_panic("out of memory");
    if (len > 0) memcpy(out, b->data, (size_t)len);
    out[len] = 0;
    return out;
}

/* Bytes -> lowercase hex text (2 chars per byte). */
certo_text_t certo_bytes_to_hex(CertoBytes* b) {
    int64_t len = b ? b->len : 0;
    char* out = (char*)malloc((size_t)len * 2 + 1);
    if (!out) certo_panic("out of memory");
    static const char* hex = "0123456789abcdef";
    for (int64_t i = 0; i < len; i++) {
        out[i*2]   = hex[(b->data[i] >> 4) & 0xF];
        out[i*2+1] = hex[b->data[i] & 0xF];
    }
    out[len*2] = 0;
    return out;
}

/* Text -> Bytes (copies up to the NUL terminator). */
CertoBytes* certo_bytes_from_text(certo_text_t s) {
    size_t n = s ? strlen(s) : 0;
    CertoBytes* out = certo_bytes_alloc((int64_t)n);
    if (n > 0) memcpy(out->data, s, n);
    return out;
}

/* Read an entire file as raw bytes. Returns Option<Bytes>. */
void* certo_read_file_bytes(certo_text_t path) {
    if (!path) return NULL;
    FILE* f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    if (size < 0) { fclose(f); return NULL; }
    fseek(f, 0, SEEK_SET);
    CertoBytes* b = certo_bytes_alloc((int64_t)size);
    size_t got = fread(b->data, 1, (size_t)size, f);
    fclose(f);
    b->len = (int64_t)got; /* trust bytes actually read */
    return __certo_opt_box((int64_t)b); /* Some(bytes) */
}

/* Read up to `length` bytes starting at `offset` from a file, without
   reading anything before or after that range. Returns Option<Bytes>:
   None only when the path can't be opened or the seek itself fails
   (offset beyond what the stream can seek to) - a file shorter than
   `offset + length` is not an error, it just yields fewer bytes than
   requested (b->len reflects the real count, same "trust bytes
   actually read" convention certo_read_file_bytes already uses).
   Lets a caller check a large file's own fixed-size header (e.g. a
   cache artifact's magic/hash prefix) without paying to read the
   whole file first just to look at the first few bytes of it. */
void* certo_read_file_bytes_range(certo_text_t path, int64_t offset, int64_t length) {
    if (!path || offset < 0 || length < 0) return NULL;
    FILE* f = fopen(path, "rb");
    if (!f) return NULL;
    if (fseek(f, (long)offset, SEEK_SET) != 0) { fclose(f); return NULL; }
    CertoBytes* b = certo_bytes_alloc(length);
    size_t got = fread(b->data, 1, (size_t)length, f);
    fclose(f);
    b->len = (int64_t)got;
    return __certo_opt_box((int64_t)b); /* Some(bytes) */
}

/* Write raw bytes to a file (truncating). Returns true on success. */
bool certo_write_file_bytes(certo_text_t path, CertoBytes* b) {
    if (!path || !b) return false;
    FILE* f = fopen(path, "wb");
    if (!f) return false;
    size_t ok = fwrite(b->data, 1, (size_t)b->len, f);
    fclose(f);
    return ok == (size_t)b->len;
}
"#;
