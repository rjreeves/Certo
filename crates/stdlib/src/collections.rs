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

void* certo_list_find(CertoList* l, CertoPred pred) {
    if (!l) return NULL;
    for (int64_t i = 0; i < l->len; i++)
        if (pred(l->data[i])) return l->data[i];
    return NULL;
}

bool certo_list_any(CertoList* l, CertoPred pred) {
    if (!l) return false;
    for (int64_t i = 0; i < l->len; i++)
        if (pred(l->data[i])) return true;
    return false;
}

bool certo_list_all(CertoList* l, CertoPred pred) {
    if (!l) return true;
    for (int64_t i = 0; i < l->len; i++)
        if (!pred(l->data[i])) return false;
    return true;
}

/* ---- sort (merge sort, stable) ---- */

typedef int64_t (*CertoCmp)(void*, void*);

static CertoList* list_merge(CertoList* a, CertoList* b, CertoCmp cmp) {
    CertoList* n = list_alloc(a->len + b->len);
    int64_t i = 0, j = 0;
    while (i < a->len && j < b->len) {
        if (cmp(a->data[i], b->data[j]) <= 0)
            n->data[n->len++] = a->data[i++];
        else
            n->data[n->len++] = b->data[j++];
    }
    while (i < a->len) n->data[n->len++] = a->data[i++];
    while (j < b->len) n->data[n->len++] = b->data[j++];
    return n;
}

CertoList* certo_list_sort(CertoList* l, CertoCmp cmp) {
    if (!l || l->len <= 1) return l ? l : list_alloc(0);
    int64_t mid = l->len / 2;
    CertoList* left  = certo_list_slice(l, 0,   mid);
    CertoList* right = certo_list_slice(l, mid, l->len);
    return list_merge(certo_list_sort(left, cmp),
                      certo_list_sort(right, cmp), cmp);
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

/// Certo source declaration of `Stdlib.Collections`.
pub const COLLECTIONS_CERTO: &str = r#"
module Stdlib.Collections

/* ---- List<T> ---- */

fn List.empty<T>(): List<T>
fn List.len<T>(list: List<T>): Int
fn List.get<T>(list: List<T>, i: Int): T?
fn List.getOrPanic<T>(list: List<T>, i: Int): T
fn List.push<T>(list: List<T>, item: T): List<T>
fn List.concat<T>(a: List<T>, b: List<T>): List<T>
fn List.first<T>(list: List<T>): T?
fn List.last<T>(list: List<T>): T?
fn List.slice<T>(list: List<T>, start: Int, end: Int): List<T>
fn List.reverse<T>(list: List<T>): List<T>
fn List.map<A, B>(list: List<A>, f: A => B): List<B>
fn List.filter<T>(list: List<T>, pred: T => Bool): List<T>
fn List.fold<T, A>(list: List<T>, init: A, f: (A, T) => A): A
fn List.contains<T>(list: List<T>, item: T): Bool
fn List.find<T>(list: List<T>, pred: T => Bool): T?
fn List.any<T>(list: List<T>, pred: T => Bool): Bool
fn List.all<T>(list: List<T>, pred: T => Bool): Bool
fn List.sort<T>(list: List<T>, cmp: (T, T) => Int): List<T>
fn List.zip<A, B>(a: List<A>, b: List<B>): List<(A, B)>

/* ---- Map<K, V> ---- */

fn Map.empty<K, V>(): Map<K, V>
fn Map.insert<K, V>(map: Map<K, V>, key: K, value: V): Map<K, V>
fn Map.get<K, V>(map: Map<K, V>, key: K): V?
fn Map.contains<K, V>(map: Map<K, V>, key: K): Bool
fn Map.remove<K, V>(map: Map<K, V>, key: K): Map<K, V>
fn Map.len<K, V>(map: Map<K, V>): Int
fn Map.keys<K, V>(map: Map<K, V>): List<K>
fn Map.values<K, V>(map: Map<K, V>): List<V>
fn Map.fromList<K, V>(pairs: List<(K, V)>): Map<K, V>
"#;
