pub const UUID_C: &str = r##"
/* ===== Stdlib.Uuid ===== */
/* BACKLOG item 197 — the uuid"..." literal (spec 2.4) typechecked and
   generated a call to certo_uuid_parse, but the function itself (and its
   sibling certo_uuid_new, forward-declared alongside it in RUNTIME_HEADER)
   had no body anywhere in the codebase — a real, confirmed link failure
   (`undefined symbol: certo_uuid_parse`), not just a missing feature. */
#include <stdlib.h>

static uint8_t certo_uuid_hex_nibble(char c) {
    if (c >= '0' && c <= '9') return (uint8_t)(c - '0');
    if (c >= 'a' && c <= 'f') return (uint8_t)(c - 'a' + 10);
    if (c >= 'A' && c <= 'F') return (uint8_t)(c - 'A' + 10);
    certo_panic("invalid UUID literal: expected a hex digit");
    return 0;
}

/* Parses the canonical 8-4-4-4-12 hex form (dashes optional/ignored) into
   16 raw bytes — the exact literal shape spec 2.4's own table shows
   (`uuid"550e8400-e29b-41d4-a716-446655440000"`). */
certo_uuid_t certo_uuid_parse(const char* s) {
    certo_uuid_t u;
    int byte_i = 0;
    const char* p = s;
    while (*p && byte_i < 16) {
        if (*p == '-') { p++; continue; }
        uint8_t hi = certo_uuid_hex_nibble(*p++);
        if (!*p) certo_panic("invalid UUID literal: odd number of hex digits");
        uint8_t lo = certo_uuid_hex_nibble(*p++);
        u.bytes[byte_i++] = (uint8_t)((hi << 4) | lo);
    }
    if (byte_i != 16 || *p) certo_panic("invalid UUID literal: expected 32 hex digits");
    return u;
}

/* Random v4 UUID (RFC 4122 section 4.4) — forward-declared in
   RUNTIME_HEADER alongside certo_uuid_parse but not itself reachable from
   any Certo surface syntax yet (no spec-documented "generate a new UUID"
   function exists to wire it to); implemented anyway so the declaration
   isn't a second dangling link failure waiting to happen the moment one is
   added. */
certo_uuid_t certo_uuid_new(void) {
    certo_uuid_t u;
    for (int i = 0; i < 16; i++) {
        u.bytes[i] = (uint8_t)(rand() & 0xFF);
    }
    u.bytes[6] = (uint8_t)((u.bytes[6] & 0x0F) | 0x40); /* version 4 */
    u.bytes[8] = (uint8_t)((u.bytes[8] & 0x3F) | 0x80); /* variant 10xx */
    return u;
}

/* BACKLOG item 201 — certo_uuid_t is a 16-byte plain-value struct with no
   `==` wiring at all before this: `a == b` on two UUIDs failed to compile
   in C (a struct isn't a scalar), the same gap record/enum equality had —
   confirmed while scoping that item. A real byte-wise comparison, not a
   raw memcmp of the whole struct (which would happen to be correct here
   since certo_uuid_t has no padding, but this is more obviously correct
   and doesn't depend on that). */
bool certo_uuid_eq(certo_uuid_t a, certo_uuid_t b) {
    for (int i = 0; i < 16; i++) {
        if (a.bytes[i] != b.bytes[i]) return false;
    }
    return true;
}

/* BACKLOG item 228 — no callable Certo-level function converted a runtime
   Text value into a UUID at all; only the compile-time uuid"..." literal
   (certo_uuid_parse above) worked, and that one panics on bad input —
   fine for a literal the parser already validated, wrong for a fallible
   function reading untrusted runtime Text (e.g. a UUID column read back
   out of a database row as Text). Strict canonical-form check: exactly
   36 characters, dashes at exactly the 4 standard 8-4-4-4-12 positions,
   every other character a hex digit — rejects anything else (missing/
   misplaced dashes, wrong length, non-hex characters) rather than the
   literal parser's dashes-optional/ignored leniency. Returns NULL (None)
   on failure, heap-allocated certo_uuid_t* on success — mirrors
   certo_parse_decimal's nullable-box convention. */
static bool certo_uuid_hex_nibble_checked(char c, uint8_t* out) {
    if (c >= '0' && c <= '9') { *out = (uint8_t)(c - '0'); return true; }
    if (c >= 'a' && c <= 'f') { *out = (uint8_t)(c - 'a' + 10); return true; }
    if (c >= 'A' && c <= 'F') { *out = (uint8_t)(c - 'A' + 10); return true; }
    return false;
}

certo_uuid_t* certo_parse_uuid(certo_text_t s) {
    if (!s) return NULL;
    if (strlen(s) != 36) return NULL;
    if (s[8] != '-' || s[13] != '-' || s[18] != '-' || s[23] != '-') return NULL;
    char hex[32];
    int hi = 0;
    for (int i = 0; i < 36; i++) {
        if (i == 8 || i == 13 || i == 18 || i == 23) continue;
        hex[hi++] = s[i];
    }
    certo_uuid_t u;
    for (int i = 0; i < 16; i++) {
        uint8_t nib_hi, nib_lo;
        if (!certo_uuid_hex_nibble_checked(hex[i * 2], &nib_hi)) return NULL;
        if (!certo_uuid_hex_nibble_checked(hex[i * 2 + 1], &nib_lo)) return NULL;
        u.bytes[i] = (uint8_t)((nib_hi << 4) | nib_lo);
    }
    certo_uuid_t* box = (certo_uuid_t*)malloc(sizeof(certo_uuid_t));
    if (!box) certo_panic("out of memory");
    *box = u;
    return box;
}

/* BACKLOG item 228 — the other direction: no function serialized a UUID
   back to Text either (needed e.g. to pass a UUID-typed field as a Text
   SQL parameter). Canonical lowercase 8-4-4-4-12 dashed form — the same
   shape Postgres's own `uuid` column renders as and certo_parse_uuid
   above accepts, so this is a real inverse of it. */
certo_text_t certo_uuid_to_text(certo_uuid_t u) {
    char* buf = (char*)malloc(37);
    if (!buf) certo_panic("out of memory");
    snprintf(buf, 37,
        "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        u.bytes[0], u.bytes[1], u.bytes[2], u.bytes[3],
        u.bytes[4], u.bytes[5],
        u.bytes[6], u.bytes[7],
        u.bytes[8], u.bytes[9],
        u.bytes[10], u.bytes[11], u.bytes[12], u.bytes[13], u.bytes[14], u.bytes[15]);
    return buf;
}

/* Bridge codegen's UUID.toText name (certo_u_u_i_d_to_text — c_fn_name's
   camel_to_snake inserts an underscore before every uppercase letter, so
   the 4 consecutive capitals in "UUID" each get their own, same gap
   DateTime's own aliases above this file's sibling bridge) to the real
   implementation. `parseUuid` needs no such alias — its single leading
   capital ('U' in "Uuid") already derives to the intended certo_parse_uuid. */
#define certo_u_u_i_d_to_text certo_uuid_to_text
"##;
