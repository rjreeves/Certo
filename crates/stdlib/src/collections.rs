/// C implementations for `Stdlib.Collections` (List and Map).
pub const COLLECTIONS_C: &str = r#"
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

/* range(start, end_excl): List<Int> / rangeInclusive(start, end_incl): List<Int>.
   Lives here (not core.rs, where these are registered at the Certo level)
   because it needs CertoList/list_alloc, defined just above — moved from a
   previous, genuinely broken implementation that allocated a completely
   different, incompatible layout ([int64_t len][int64_t data[len]] packed
   inline, not the real {len,cap,void**data} struct every other List
   function expects). That mismatch was silent for List.len(range(...))
   (arr[0] happens to be `len` in both layouts by coincidence) but corrupted
   everything else: List.get/`for`-loop iteration read `cap`/`data` from
   whatever raw range values happened to occupy those byte offsets,
   producing garbage element values or segfaulting outright when a bogus
   `data` pointer got dereferenced. Confirmed as a real, reproducible crash
   (not theorized) while verifying BACKLOG item 93's REST client generator,
   whose List<T> JSON-decode loop uses `range(0, JsonValue.length(...))`.
   Elements are boxed via the same `(void*)(intptr_t)(value)` convention
   crates/codegen/src/emit_mir.rs's box_value uses for every other Int list
   element, so a real List.get/for-loop/List.push on the result works
   exactly like a range would if you'd built it with repeated List.push. */
CertoList* certo_range(int64_t start, int64_t end_excl) {
    int64_t len = end_excl > start ? end_excl - start : 0;
    CertoList* l = list_alloc(len);
    for (int64_t i = 0; i < len; i++) l->data[i] = (void*)(intptr_t)(start + i);
    l->len = len;
    return l;
}

CertoList* certo_range_inclusive(int64_t start, int64_t end_incl) {
    return certo_range(start, end_incl + 1);
}

CertoList* certo_list_new_empty(void) {
    return list_alloc(8);
}
/* Alias used by stdlib modules (csv, regex, json). */
CertoList* certo_list_new(void) {
    return list_alloc(8);
}
/* `List.empty()` — the real function call site; see RUNTIME_HEADER's comment
 * (this used to be a `#define certo_list_empty certo_list_new_empty()` macro
 * relying on `List.empty` being referenced without call parens, which broke once
 * typeck correctly registered it as a zero-arg function instead of a bare value). */
CertoList* certo_list_empty(void) {
    return certo_list_new_empty();
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

/* List.get → Option<T> (heap-boxed): None on out-of-bounds, else Some(elem). */
void* certo_list_get(CertoList* l, int64_t i) {
    if (!l || i < 0 || i >= l->len) return NULL;   /* None */
    return __certo_opt_box((int64_t)l->data[i]);   /* Some(elem) */
}

/* List.getOrPanic → T (also used by for-loop codegen): the raw element. */
void* certo_list_get_or_panic(CertoList* l, int64_t i) {
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
    return (l && l->len > 0) ? __certo_opt_box((int64_t)l->data[0]) : NULL;
}

void* certo_list_last(CertoList* l) {
    return (l && l->len > 0) ? __certo_opt_box((int64_t)l->data[l->len - 1]) : NULL;
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

/* map / filter / fold take function *values* — a `certo_fn_t { fn, env }`
 * closure (BACKLOG item 140), not a bare pointer, so a lambda literal can
 * capture a variable from its enclosing scope. Every generated lambda
 * function's own real native signature always takes its closure
 * environment as an explicit leading `void*` parameter (whether or not it
 * actually captures anything), matching these typedefs. */
typedef void* (*CertoFn1)(void*, void*);
typedef void* (*CertoFn2)(void*, void*, void*);
typedef bool  (*CertoPred)(void*, void*);

CertoList* certo_list_map(CertoList* l, certo_fn_t f) {
    if (!l) return list_alloc(0);
    CertoFn1 fn = (CertoFn1)f.fn;
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) n->data[i] = fn(f.env, l->data[i]);
    n->len = l->len;
    return n;
}

CertoList* certo_list_filter(CertoList* l, certo_fn_t f) {
    if (!l) return list_alloc(0);
    CertoPred pred = (CertoPred)f.fn;
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) {
        if (pred(f.env, l->data[i])) n->data[n->len++] = l->data[i];
    }
    return n;
}

void* certo_list_fold(CertoList* l, void* init, certo_fn_t f) {
    CertoFn2 fn = (CertoFn2)f.fn;
    void* acc = init;
    if (!l) return acc;
    for (int64_t i = 0; i < l->len; i++) acc = fn(f.env, acc, l->data[i]);
    return acc;
}

/* List.reduce (BACKLOG item 162) is spec's own name for exactly List.fold's
 * already-shipped (list, init, combiner) signature — same real
 * implementation, no new logic. */
#define certo_list_reduce certo_list_fold

/* List.flatMap(list, f): map each element to a sub-list via f, then
 * concatenate all the sub-lists in order (BACKLOG item 162). Grows via
 * certo_list_push's own functional-update copy, same unoptimized-but-
 * correct style certo_list_distinct/certo_list_group_by already use. */
CertoList* certo_list_flat_map(CertoList* l, certo_fn_t f) {
    CertoList* out = list_alloc(l ? l->len : 0);
    if (!l) return out;
    CertoFn1 fn = (CertoFn1)f.fn;
    for (int64_t i = 0; i < l->len; i++) {
        CertoList* sub = (CertoList*)fn(f.env, l->data[i]);
        int64_t sn = sub ? sub->len : 0;
        for (int64_t j = 0; j < sn; j++) out = certo_list_push(out, sub->data[j]);
    }
    return out;
}

/* `List.contains` (BACKLOG item 181) — this was originally written as
   `certo_list_contains_ptr`, one letter off from the name `crates/codegen`'s
   own `List.contains` → `certo_list_contains` naming convention actually
   calls, and never referenced under either name from anywhere else in the
   codebase — so every real program calling `List.contains` failed to link
   with `undefined symbol: certo_list_contains`, confirmed to have never
   worked at all rather than having regressed. Renamed to the name the call
   site really uses; pointer-equality semantics are unchanged (correct for
   boxed `Int`/`Bool`/`Float` elements, same documented limitation
   `List.distinct` below already carries for boxed `Text`/records). */
bool certo_list_contains(CertoList* l, void* item) {
    if (!l) return false;
    for (int64_t i = 0; i < l->len; i++) if (l->data[i] == item) return true;
    return false;
}

void* certo_list_find(CertoList* l, certo_fn_t f) {
    if (!l) return NULL;   /* find heap-boxes its Some result just below */
    CertoPred pred = (CertoPred)f.fn;
    for (int64_t i = 0; i < l->len; i++)
        if (pred(f.env, l->data[i])) return __certo_opt_box((int64_t)l->data[i]);
    return NULL;
}

bool certo_list_any(CertoList* l, certo_fn_t f) {
    if (!l) return false;
    CertoPred pred = (CertoPred)f.fn;
    for (int64_t i = 0; i < l->len; i++)
        if (pred(f.env, l->data[i])) return true;
    return false;
}

bool certo_list_all(CertoList* l, certo_fn_t f) {
    if (!l) return true;
    CertoPred pred = (CertoPred)f.fn;
    for (int64_t i = 0; i < l->len; i++)
        if (!pred(f.env, l->data[i])) return false;
    return true;
}

/* ---- sort (merge sort, stable) ---- */

typedef int64_t (*CertoCmp)(void*, void*, void*);

static CertoList* list_merge(CertoList* a, CertoList* b, certo_fn_t f) {
    CertoCmp cmp = (CertoCmp)f.fn;
    CertoList* n = list_alloc(a->len + b->len);
    int64_t i = 0, j = 0;
    while (i < a->len && j < b->len) {
        if (cmp(f.env, a->data[i], b->data[j]) <= 0)
            n->data[n->len++] = a->data[i++];
        else
            n->data[n->len++] = b->data[j++];
    }
    while (i < a->len) n->data[n->len++] = a->data[i++];
    while (j < b->len) n->data[n->len++] = b->data[j++];
    return n;
}

CertoList* certo_list_sort(CertoList* l, certo_fn_t f) {
    if (!l || l->len <= 1) return l ? l : list_alloc(0);
    int64_t mid = l->len / 2;
    CertoList* left  = certo_list_slice(l, 0,   mid);
    CertoList* right = certo_list_slice(l, mid, l->len);
    return list_merge(certo_list_sort(left, f),
                      certo_list_sort(right, f), f);
}

/* ---- distinct / partition / chunked ---- */

/* Pointer-equality dedup, first-occurrence order — same equality convention
 * as certo_list_contains/Map's key equality (see STDLIB-QUICKREF's note
 * on Map: use Text keys carefully, since two equal strings are different
 * pointers unless interned). O(n^2), matching this runtime's existing
 * unoptimized style (certo_list_sort etc.). */
CertoList* certo_list_distinct(CertoList* l) {
    if (!l) return list_alloc(0);
    CertoList* n = list_alloc(l->len);
    for (int64_t i = 0; i < l->len; i++) {
        bool seen = false;
        for (int64_t j = 0; j < n->len; j++) {
            if (n->data[j] == l->data[i]) { seen = true; break; }
        }
        if (!seen) n->data[n->len++] = l->data[i];
    }
    return n;
}

/* Returns a 2-tuple (matches, non-matches) — tuples are CertoList* with
 * boxed elements (see Rvalue::Aggregate(Tuple) in emit_mir.rs); List<T>
 * values are already pointer-sized so no f2i boxing is needed here. */
CertoList* certo_list_partition(CertoList* l, certo_fn_t f) {
    CertoPred pred = (CertoPred)f.fn;
    CertoList* yes = list_alloc(l ? l->len : 0);
    CertoList* no  = list_alloc(l ? l->len : 0);
    if (l) {
        for (int64_t i = 0; i < l->len; i++) {
            if (pred(f.env, l->data[i])) yes->data[yes->len++] = l->data[i];
            else                         no->data[no->len++]   = l->data[i];
        }
    }
    return certo_list_of(2, (void*)yes, (void*)no);
}

CertoList* certo_list_chunked(CertoList* l, int64_t size) {
    if (!l || size <= 0) return list_alloc(0);
    int64_t n_chunks = (l->len + size - 1) / size;
    CertoList* out = list_alloc(n_chunks > 0 ? n_chunks : 0);
    for (int64_t i = 0; i < l->len; i += size) {
        int64_t end = i + size;
        if (end > l->len) end = l->len;
        out->data[out->len++] = (void*)certo_list_slice(l, i, end);
    }
    return out;
}

/* Replaces the first element whose key (via f, the same key-projection-
 * lambda convention certo_list_group_by/sortBy/sumBy already use, rather
 * than a field-name string — this runtime has no reflection to look a
 * field up by name on an arbitrary type) matches the new item's own key,
 * or appends the item if no element matches — BACKLOG item 209. Same
 * pointer-equality key convention certo_map_get/certo_list_group_by already
 * use (see their own note re: Text keys); O(n), one pass, matching this
 * runtime's existing unoptimized style. */
CertoList* certo_list_upsert(CertoList* l, void* item, certo_fn_t f) {
    CertoFn1 key = (CertoFn1)f.fn;
    void* item_key = key(f.env, item);
    int64_t len = l ? l->len : 0;
    CertoList* n = list_alloc(len > 0 ? len : 1);
    bool replaced = false;
    for (int64_t i = 0; i < len; i++) {
        void* k = key(f.env, l->data[i]);
        if (!replaced && k == item_key) {
            n->data[n->len++] = item;
            replaced = true;
        } else {
            n->data[n->len++] = l->data[i];
        }
    }
    if (!replaced) n->data[n->len++] = item;
    return n;
}

/* ---- zip (pairs stored as heap-allocated {fst, snd}) ---- */

typedef struct { void* fst; void* snd; } CertoPair;

CertoList* certo_list_zip(CertoList* a, CertoList* b) {
    if (!a || !b) return list_alloc(0);
    int64_t len = a->len < b->len ? a->len : b->len;
    CertoList* n = list_alloc(len);
    for (int64_t i = 0; i < len; i++) {
        CertoPair* p = (CertoPair*)malloc(sizeof(CertoPair));
        if (!p) certo_panic("out of memory");
        p->fst = a->data[i];
        p->snd = b->data[i];
        n->data[n->len++] = p;
    }
    return n;
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

/* `Map.empty()` — see certo_list_empty's comment above; same fix. */
CertoMap* certo_map_empty(void) {
    return certo_map_new();
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
    if (!m->entries[h].occupied) return NULL;         /* None */
    return __certo_opt_box((int64_t)m->entries[h].value);  /* Some(value) */
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

CertoMap* certo_map_from_list(CertoList* l) {
    if (!l) return map_alloc(16);
    CertoMap* m = map_alloc(l->len > 0 ? l->len * 2 : 16);
    for (int64_t i = 0; i < l->len; i++) {
        CertoPair* p = (CertoPair*)l->data[i];
        int64_t h = map_probe(m, p->fst);
        if (!m->entries[h].occupied) m->len++;
        m->entries[h] = (MapEntry){ .key = p->fst, .value = p->snd, .occupied = true };
    }
    return m;
}

/* Groups list elements by a key function into Map<K, List<T>>. Pre-sized to
 * `len*2` slots (worst case: every element has a distinct key) since, unlike
 * certo_map_insert, this builds the map in place rather than growing it via
 * copy-on-write per insert. */
CertoMap* certo_list_group_by(CertoList* l, certo_fn_t f) {
    CertoFn1 key = (CertoFn1)f.fn;
    CertoMap* m = map_alloc(l && l->len > 0 ? l->len * 2 : 16);
    if (!l) return m;
    for (int64_t i = 0; i < l->len; i++) {
        void* k = key(f.env, l->data[i]);
        int64_t h = map_probe(m, k);
        CertoList* bucket = m->entries[h].occupied
            ? (CertoList*)m->entries[h].value
            : list_alloc(4);
        bucket = certo_list_push(bucket, l->data[i]);
        if (!m->entries[h].occupied) m->len++;
        m->entries[h] = (MapEntry){ .key = k, .value = (void*)bucket, .occupied = true };
    }
    return m;
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
"#;
