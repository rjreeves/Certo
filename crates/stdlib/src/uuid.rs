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
"##;
