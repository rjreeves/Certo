pub const REGEX_C: &str = r##"
/* ===== Stdlib.Regex ===== */
/* A compiled backtracking regular-expression engine (BACKLOG item 401).

   Syntax: literals; `.` (any character); bracket expressions `[abc]`, `[^a-z]`, with
   POSIX classes (`[[:digit:]]`) and the escapes `\d \w \s \D \W \S`; the escapes `\d \D \w \W
   \s \S \b \B \n \t \r \f \v` and `\` before any punctuation to mean it literally; anchors `^`
   and `$`; groups `( )` (capturing) and `(?: )` (not); alternation `|`; and the quantifiers
   `* + ? {n} {n,} {n,m}`, each optionally followed by `?` for a lazy match. A `{` that does
   not begin a valid interval is an ordinary character.

   Matching is leftmost-first: alternatives are tried left to right and the first that leads
   to a match wins, quantifiers are greedy — the rule most languages use, not POSIX's
   leftmost-longest (`cat|category` finds `cat` in `category`). Text is UTF-8: `.`, classes and
   `\W`-style escapes match one whole character, and an invalid byte is one character of its
   own that only `.` and negated classes match. `\d \w \s` and the POSIX classes are ASCII.

   An invalid pattern (unmatched parenthesis or bracket, nothing to repeat, a backreference,
   a repeat count above 1000, ...) stops the program with a message naming the pattern and
   the problem — it used to crash. So does a match that exceeds the step limit. */
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <stdint.h>

#define CERTO_RE_MAX_REPEAT 1000
#define CERTO_RE_MAX_PROG   200000
#define CERTO_RE_STEP_LIMIT 50000000L

typedef struct { uint32_t lo, hi; } certo_re_range;
typedef struct { certo_re_range* r; int n, cap; int neg; } certo_re_class;

enum { RE_CHAR, RE_ANY, RE_CLASS, RE_BOL, RE_EOL, RE_WORDB, RE_NWORDB,
       RE_SPLIT, RE_JMP, RE_SAVE, RE_MARK, RE_CHKPROG, RE_MATCH };
typedef struct { int op; int x, y; uint32_t c; } certo_re_inst;

enum { N_EMPTY, N_CHAR, N_ANY, N_CLASS, N_BOL, N_EOL, N_WB, N_NWB, N_CAT, N_ALT, N_REP, N_GROUP };
typedef struct certo_re_node {
    int type;
    struct certo_re_node *a, *b;
    uint32_t c;
    int cls, min, max, greedy, idx;
} certo_re_node;

typedef struct {
    certo_re_inst* code; int ncode, capcode;
    certo_re_class* cls; int ncls, capcls;
    int ngroups;   /* capturing groups, not counting the whole match */
    int nmarks;
} certo_re_prog;

typedef struct {
    const char* pat; const char* p; const char* err;
    certo_re_node** nodes; int nnodes, capnodes;
    certo_re_prog* prog;
} certo_re_parser;

/* ---- UTF-8 ---- */

/* Decode the character at s[i]; *len gets its byte length. An invalid byte decodes to
   0xDC00 + byte (a value no pattern can name), length 1. */
static uint32_t certo_re_decode(const unsigned char* s, size_t n, size_t i, int* len) {
    unsigned char b = s[i];
    if (b < 0x80) { *len = 1; return b; }
    int k = 0; uint32_t cp = 0;
    if      ((b & 0xE0) == 0xC0) { k = 2; cp = b & 0x1F; }
    else if ((b & 0xF0) == 0xE0) { k = 3; cp = b & 0x0F; }
    else if ((b & 0xF8) == 0xF0) { k = 4; cp = b & 0x07; }
    else { *len = 1; return 0xDC00u + b; }
    if (i + (size_t)k > n) { *len = 1; return 0xDC00u + b; }
    for (int j = 1; j < k; j++) {
        if ((s[i + j] & 0xC0) != 0x80) { *len = 1; return 0xDC00u + b; }
        cp = (cp << 6) | (s[i + j] & 0x3F);
    }
    *len = k;
    return cp;
}

static int certo_re_isword(uint32_t c) {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_';
}

/* ---- parser ---- */

static certo_re_node* certo_re_new(certo_re_parser* ps, int type) {
    certo_re_node* n = (certo_re_node*)calloc(1, sizeof(certo_re_node));
    if (!n) certo_panic("out of memory");
    if (ps->nnodes == ps->capnodes) {
        ps->capnodes = ps->capnodes ? ps->capnodes * 2 : 32;
        ps->nodes = (certo_re_node**)realloc(ps->nodes, (size_t)ps->capnodes * sizeof(*ps->nodes));
        if (!ps->nodes) certo_panic("out of memory");
    }
    ps->nodes[ps->nnodes++] = n;
    n->type = type;
    n->greedy = 1;
    return n;
}

static void certo_re_fail(certo_re_parser* ps, const char* msg) {
    if (!ps->err) ps->err = msg;
}

static int certo_re_class_new(certo_re_parser* ps) {
    certo_re_prog* g = ps->prog;
    if (g->ncls == g->capcls) {
        g->capcls = g->capcls ? g->capcls * 2 : 8;
        g->cls = (certo_re_class*)realloc(g->cls, (size_t)g->capcls * sizeof(*g->cls));
        if (!g->cls) certo_panic("out of memory");
    }
    memset(&g->cls[g->ncls], 0, sizeof(certo_re_class));
    return g->ncls++;
}

static void certo_re_class_add(certo_re_class* c, uint32_t lo, uint32_t hi) {
    if (c->n == c->cap) {
        c->cap = c->cap ? c->cap * 2 : 8;
        c->r = (certo_re_range*)realloc(c->r, (size_t)c->cap * sizeof(*c->r));
        if (!c->r) certo_panic("out of memory");
    }
    c->r[c->n].lo = lo; c->r[c->n].hi = hi; c->n++;
}

/* The ranges of \d \w \s as ASCII sets; `neg` adds the complement over all code points. */
static void certo_re_class_escape(certo_re_class* c, char e) {
    char lower = (char)(e | 0x20);
    int neg = (e >= 'A' && e <= 'Z');
    certo_re_range set[8]; int n = 0;
    if (lower == 'd')      { set[n].lo = '0'; set[n].hi = '9'; n++; }
    else if (lower == 'w') {
        set[n].lo = 'a'; set[n].hi = 'z'; n++; set[n].lo = 'A'; set[n].hi = 'Z'; n++;
        set[n].lo = '0'; set[n].hi = '9'; n++; set[n].lo = '_'; set[n].hi = '_'; n++;
    } else { /* s */
        set[n].lo = ' '; set[n].hi = ' '; n++; set[n].lo = 9; set[n].hi = 13; n++;
    }
    if (!neg) { for (int i = 0; i < n; i++) certo_re_class_add(c, set[i].lo, set[i].hi); return; }
    /* complement of the (sorted-by-construction below) set over [0, 0x10FFFF] */
    for (int i = 0; i < n; i++) for (int j = i + 1; j < n; j++)
        if (set[j].lo < set[i].lo) { certo_re_range t = set[i]; set[i] = set[j]; set[j] = t; }
    uint32_t next = 0;
    for (int i = 0; i < n; i++) {
        if (set[i].lo > next) certo_re_class_add(c, next, set[i].lo - 1);
        next = set[i].hi + 1;
    }
    certo_re_class_add(c, next, 0x10FFFFu);
}

static int certo_re_posix_class(certo_re_class* c, const char* name, size_t len) {
    #define NAME(s) (len == sizeof(s) - 1 && memcmp(name, s, len) == 0)
    if      (NAME("alpha"))  { certo_re_class_add(c, 'a', 'z'); certo_re_class_add(c, 'A', 'Z'); }
    else if (NAME("digit"))  { certo_re_class_add(c, '0', '9'); }
    else if (NAME("alnum"))  { certo_re_class_add(c, 'a', 'z'); certo_re_class_add(c, 'A', 'Z'); certo_re_class_add(c, '0', '9'); }
    else if (NAME("upper"))  { certo_re_class_add(c, 'A', 'Z'); }
    else if (NAME("lower"))  { certo_re_class_add(c, 'a', 'z'); }
    else if (NAME("space"))  { certo_re_class_add(c, ' ', ' '); certo_re_class_add(c, 9, 13); }
    else if (NAME("blank"))  { certo_re_class_add(c, ' ', ' '); certo_re_class_add(c, 9, 9); }
    else if (NAME("punct"))  { certo_re_class_add(c, 33, 47); certo_re_class_add(c, 58, 64); certo_re_class_add(c, 91, 96); certo_re_class_add(c, 123, 126); }
    else if (NAME("print"))  { certo_re_class_add(c, 32, 126); }
    else if (NAME("graph"))  { certo_re_class_add(c, 33, 126); }
    else if (NAME("cntrl"))  { certo_re_class_add(c, 0, 31); certo_re_class_add(c, 127, 127); }
    else if (NAME("xdigit")) { certo_re_class_add(c, '0', '9'); certo_re_class_add(c, 'a', 'f'); certo_re_class_add(c, 'A', 'F'); }
    else if (NAME("word"))   { certo_re_class_add(c, 'a', 'z'); certo_re_class_add(c, 'A', 'Z'); certo_re_class_add(c, '0', '9'); certo_re_class_add(c, '_', '_'); }
    else return 0;
    #undef NAME
    return 1;
}

/* A simple (non-class) escape's character, or -1 if `e` is not one. */
static int certo_re_simple_escape(char e) {
    switch (e) {
        case 'n': return '\n'; case 't': return '\t'; case 'r': return '\r';
        case 'f': return '\f'; case 'v': return '\v'; case '0': return 0;
        default: return -1;
    }
}

static certo_re_node* certo_re_parse_alt(certo_re_parser* ps, int depth);

/* `[...]`; *p is just past the `[`. */
static certo_re_node* certo_re_parse_bracket(certo_re_parser* ps) {
    int ci = certo_re_class_new(ps);
    certo_re_class* cls = &ps->prog->cls[ci];
    const unsigned char* s = (const unsigned char*)ps->p;
    size_t n = strlen((const char*)s), i = 0;
    if (i < n && s[i] == '^') { cls->neg = 1; i++; }
    int first = 1;
    for (;;) {
        if (i >= n) { certo_re_fail(ps, "unterminated [ in bracket expression"); return NULL; }
        if (s[i] == ']' && !first) { i++; break; }
        first = 0;
        uint32_t lo;
        int len;
        if (s[i] == '[' && i + 1 < n && s[i + 1] == ':') {
            size_t j = i + 2;
            while (j + 1 < n && !(s[j] == ':' && s[j + 1] == ']')) j++;
            if (j + 1 >= n) { certo_re_fail(ps, "unterminated [: in bracket expression"); return NULL; }
            if (!certo_re_posix_class(cls, (const char*)s + i + 2, j - (i + 2))) { certo_re_fail(ps, "unknown character class name"); return NULL; }
            i = j + 2;
            continue;
        }
        if (s[i] == '\\') {
            if (i + 1 >= n) { certo_re_fail(ps, "trailing backslash"); return NULL; }
            char e = (char)s[i + 1];
            if (e == 'd' || e == 'D' || e == 'w' || e == 'W' || e == 's' || e == 'S') {
                certo_re_class_escape(cls, e); i += 2; continue;
            }
            int sc = certo_re_simple_escape(e);
            lo = sc >= 0 ? (uint32_t)sc : (uint32_t)(unsigned char)e;
            i += 2;
        } else {
            lo = certo_re_decode(s, n, i, &len); i += (size_t)len;
        }
        uint32_t hi = lo;
        if (i + 1 < n && s[i] == '-' && s[i + 1] != ']') {
            i++;
            if (s[i] == '\\') {
                if (i + 1 >= n) { certo_re_fail(ps, "trailing backslash"); return NULL; }
                int sc = certo_re_simple_escape((char)s[i + 1]);
                hi = sc >= 0 ? (uint32_t)sc : (uint32_t)(unsigned char)s[i + 1];
                i += 2;
            } else {
                hi = certo_re_decode(s, n, i, &len); i += (size_t)len;
            }
            if (hi < lo) { certo_re_fail(ps, "range out of order in bracket expression"); return NULL; }
        }
        certo_re_class_add(cls, lo, hi);
    }
    ps->p = (const char*)s + i;
    certo_re_node* node = certo_re_new(ps, N_CLASS);
    node->cls = ci;
    return node;
}

/* One atom, or NULL (with *no* error) at the end of a branch (`|`, `)` or end of pattern). */
static certo_re_node* certo_re_parse_atom(certo_re_parser* ps, int depth) {
    const unsigned char* s = (const unsigned char*)ps->p;
    unsigned char ch = *s;
    if (ch == '\0' || ch == '|') return NULL;
    if (ch == ')') {
        if (depth == 0) { certo_re_fail(ps, "unmatched )"); }
        return NULL;
    }
    if (ch == '*' || ch == '+' || ch == '?') { certo_re_fail(ps, "nothing to repeat"); return NULL; }
    if (ch == '(') {
        ps->p++;
        int capturing = 1;
        if (ps->p[0] == '?') {
            if (ps->p[1] == ':') { capturing = 0; ps->p += 2; }
            else { certo_re_fail(ps, "unsupported group syntax (only (?: ) is supported)"); return NULL; }
        }
        int idx = 0;
        if (capturing) idx = ++ps->prog->ngroups;
        certo_re_node* inner = certo_re_parse_alt(ps, depth + 1);
        if (ps->err) return NULL;
        if (*ps->p != ')') { certo_re_fail(ps, "unmatched ("); return NULL; }
        ps->p++;
        if (!capturing) return inner ? inner : certo_re_new(ps, N_EMPTY);
        certo_re_node* g = certo_re_new(ps, N_GROUP);
        g->a = inner ? inner : certo_re_new(ps, N_EMPTY);
        g->idx = idx;
        return g;
    }
    if (ch == '[') { ps->p++; return certo_re_parse_bracket(ps); }
    if (ch == '.') { ps->p++; return certo_re_new(ps, N_ANY); }
    if (ch == '^') { ps->p++; return certo_re_new(ps, N_BOL); }
    if (ch == '$') { ps->p++; return certo_re_new(ps, N_EOL); }
    if (ch == '\\') {
        char e = ps->p[1];
        if (e == '\0') { certo_re_fail(ps, "trailing backslash"); return NULL; }
        ps->p += 2;
        if (e == 'd' || e == 'D' || e == 'w' || e == 'W' || e == 's' || e == 'S') {
            int ci = certo_re_class_new(ps);
            certo_re_class_escape(&ps->prog->cls[ci], (char)(e | 0x20));   /* the positive set ... */
            ps->prog->cls[ci].neg = (e >= 'A' && e <= 'Z');                 /* ... negated for \D \W \S */
            certo_re_node* node = certo_re_new(ps, N_CLASS);
            node->cls = ci;
            return node;
        }
        if (e == 'b') return certo_re_new(ps, N_WB);
        if (e == 'B') return certo_re_new(ps, N_NWB);
        int sc = certo_re_simple_escape(e);
        if (sc >= 0) { certo_re_node* node = certo_re_new(ps, N_CHAR); node->c = (uint32_t)sc; return node; }
        if (e >= '1' && e <= '9') { certo_re_fail(ps, "backreferences are not supported"); return NULL; }
        if ((e >= 'a' && e <= 'z') || (e >= 'A' && e <= 'Z')) { certo_re_fail(ps, "unknown escape sequence"); return NULL; }
        certo_re_node* node = certo_re_new(ps, N_CHAR);
        node->c = (uint32_t)(unsigned char)e;
        return node;
    }
    int len;
    size_t n = strlen(ps->p);
    uint32_t cp = certo_re_decode(s, n, 0, &len);
    ps->p += len;
    certo_re_node* node = certo_re_new(ps, N_CHAR);
    node->c = cp;
    return node;
}

/* A `{n}`, `{n,}` or `{n,m}` quantifier at *p, if it is one. Returns its length, or 0. */
static int certo_re_interval(const char* p, int* min, int* max) {
    const char* b = p + 1;
    int n1 = 0, has1 = 0;
    while (*b >= '0' && *b <= '9') { if (n1 < 100000) n1 = n1 * 10 + (*b - '0'); b++; has1 = 1; }
    if (!has1) return 0;
    int n2 = n1;
    if (*b == ',') {
        b++;
        n2 = -1;
        int has2 = 0, v = 0;
        while (*b >= '0' && *b <= '9') { if (v < 100000) v = v * 10 + (*b - '0'); b++; has2 = 1; }
        if (has2) n2 = v;
    }
    if (*b != '}') return 0;
    *min = n1; *max = n2;   /* max -1: unbounded */
    return (int)(b - p) + 1;
}

static certo_re_node* certo_re_parse_repeat(certo_re_parser* ps, int depth) {
    certo_re_node* atom = certo_re_parse_atom(ps, depth);
    if (!atom || ps->err) return NULL;
    for (;;) {
        int min, max, qlen = 0;
        char q = *ps->p;
        if (q == '*')      { min = 0; max = -1; qlen = 1; }
        else if (q == '+') { min = 1; max = -1; qlen = 1; }
        else if (q == '?') { min = 0; max = 1;  qlen = 1; }
        else if (q == '{') {
            qlen = certo_re_interval(ps->p, &min, &max);
            if (qlen == 0) break;   /* an ordinary `{` */
            if (min > CERTO_RE_MAX_REPEAT || max > CERTO_RE_MAX_REPEAT) { certo_re_fail(ps, "repeat count too large (the limit is 1000)"); return NULL; }
            if (max >= 0 && max < min) { certo_re_fail(ps, "repeat counts out of order in {n,m}"); return NULL; }
        } else break;
        ps->p += qlen;
        certo_re_node* rep = certo_re_new(ps, N_REP);
        rep->a = atom; rep->min = min; rep->max = max;
        if (*ps->p == '?') { rep->greedy = 0; ps->p++; }
        atom = rep;
    }
    return atom;
}

static certo_re_node* certo_re_parse_cat(certo_re_parser* ps, int depth) {
    certo_re_node* head = NULL;
    for (;;) {
        certo_re_node* r = certo_re_parse_repeat(ps, depth);
        if (ps->err) return NULL;
        if (!r) break;
        if (!head) head = r;
        else { certo_re_node* cat = certo_re_new(ps, N_CAT); cat->a = head; cat->b = r; head = cat; }
    }
    return head;
}

static certo_re_node* certo_re_parse_alt(certo_re_parser* ps, int depth) {
    certo_re_node* left = certo_re_parse_cat(ps, depth);
    if (ps->err) return NULL;
    if (*ps->p == '|') {
        ps->p++;
        certo_re_node* right = certo_re_parse_alt(ps, depth);
        if (ps->err) return NULL;
        certo_re_node* alt = certo_re_new(ps, N_ALT);
        alt->a = left ? left : certo_re_new(ps, N_EMPTY);
        alt->b = right ? right : certo_re_new(ps, N_EMPTY);
        return alt;
    }
    return left;
}

/* ---- code generation ---- */

static int certo_re_emit(certo_re_parser* ps, int op, int x, int y, uint32_t c) {
    certo_re_prog* g = ps->prog;
    if (g->ncode >= CERTO_RE_MAX_PROG) { certo_re_fail(ps, "pattern too large"); return 0; }
    if (g->ncode == g->capcode) {
        g->capcode = g->capcode ? g->capcode * 2 : 64;
        g->code = (certo_re_inst*)realloc(g->code, (size_t)g->capcode * sizeof(*g->code));
        if (!g->code) certo_panic("out of memory");
    }
    certo_re_inst* in = &g->code[g->ncode];
    in->op = op; in->x = x; in->y = y; in->c = c;
    return g->ncode++;
}

static void certo_re_gen(certo_re_parser* ps, certo_re_node* n) {
    if (!n || ps->err) return;
    certo_re_prog* g = ps->prog;
    switch (n->type) {
        case N_EMPTY: break;
        case N_CHAR:  certo_re_emit(ps, RE_CHAR, 0, 0, n->c); break;
        case N_ANY:   certo_re_emit(ps, RE_ANY, 0, 0, 0); break;
        case N_CLASS: certo_re_emit(ps, RE_CLASS, n->cls, 0, 0); break;
        case N_BOL:   certo_re_emit(ps, RE_BOL, 0, 0, 0); break;
        case N_EOL:   certo_re_emit(ps, RE_EOL, 0, 0, 0); break;
        case N_WB:    certo_re_emit(ps, RE_WORDB, 0, 0, 0); break;
        case N_NWB:   certo_re_emit(ps, RE_NWORDB, 0, 0, 0); break;
        case N_CAT:   certo_re_gen(ps, n->a); certo_re_gen(ps, n->b); break;
        case N_GROUP:
            certo_re_emit(ps, RE_SAVE, 2 * n->idx, 0, 0);
            certo_re_gen(ps, n->a);
            certo_re_emit(ps, RE_SAVE, 2 * n->idx + 1, 0, 0);
            break;
        case N_ALT: {
            int split = certo_re_emit(ps, RE_SPLIT, 0, 0, 0);
            g->code[split].x = g->ncode;
            certo_re_gen(ps, n->a);
            int jmp = certo_re_emit(ps, RE_JMP, 0, 0, 0);
            g->code[split].y = g->ncode;
            certo_re_gen(ps, n->b);
            if (!ps->err) g->code[jmp].x = g->ncode;
            break;
        }
        case N_REP: {
            for (int i = 0; i < n->min && !ps->err; i++) certo_re_gen(ps, n->a);
            if (n->max < 0) {
                /* e*: L: SPLIT body,out; body: MARK k; e; CHKPROG k; JMP L; out: */
                int mark = g->nmarks++;
                int split = certo_re_emit(ps, RE_SPLIT, 0, 0, 0);
                int body = g->ncode;
                certo_re_emit(ps, RE_MARK, mark, 0, 0);
                certo_re_gen(ps, n->a);
                certo_re_emit(ps, RE_CHKPROG, mark, 0, 0);
                certo_re_emit(ps, RE_JMP, split, 0, 0);
                int out = g->ncode;
                if (ps->err) break;
                if (n->greedy) { g->code[split].x = body; g->code[split].y = out; }
                else           { g->code[split].x = out;  g->code[split].y = body; }
            } else {
                /* (max-min) optional copies, each of which can skip to the common end */
                int extra = n->max - n->min;
                int* splits = (int*)malloc((size_t)(extra > 0 ? extra : 1) * sizeof(int));
                if (!splits) certo_panic("out of memory");
                for (int i = 0; i < extra && !ps->err; i++) {
                    splits[i] = certo_re_emit(ps, RE_SPLIT, 0, 0, 0);
                    int body = g->ncode;
                    certo_re_gen(ps, n->a);
                    if (ps->err) break;
                    if (n->greedy) g->code[splits[i]].x = body; else g->code[splits[i]].y = body;
                }
                if (!ps->err) for (int i = 0; i < extra; i++) {
                    if (n->greedy) g->code[splits[i]].y = g->ncode; else g->code[splits[i]].x = g->ncode;
                }
                free(splits);
            }
            break;
        }
    }
}

static void certo_re_free(certo_re_prog* g) {
    if (!g) return;
    for (int i = 0; i < g->ncls; i++) free(g->cls[i].r);
    free(g->cls); free(g->code); free(g);
}

/* Compile `pat`. Returns NULL and sets *err on an invalid pattern. */
static certo_re_prog* certo_re_compile(const char* pat, const char** err) {
    certo_re_parser ps;
    memset(&ps, 0, sizeof(ps));
    ps.pat = pat; ps.p = pat;
    ps.prog = (certo_re_prog*)calloc(1, sizeof(certo_re_prog));
    if (!ps.prog) certo_panic("out of memory");
    certo_re_node* root = certo_re_parse_alt(&ps, 0);
    if (!ps.err && *ps.p == ')') certo_re_fail(&ps, "unmatched )");
    if (!ps.err) {
        certo_re_gen(&ps, root);
        certo_re_emit(&ps, RE_MATCH, 0, 0, 0);
    }
    for (int i = 0; i < ps.nnodes; i++) free(ps.nodes[i]);
    free(ps.nodes);
    if (ps.err) { *err = ps.err; certo_re_free(ps.prog); return NULL; }
    return ps.prog;
}

/* ---- matcher ---- */

typedef struct { int kind; int a; long b; } certo_re_bt;   /* kind 0: branch (pc a, pos b); 1: restore slot a to b */

static int certo_re_class_has(const certo_re_class* c, uint32_t cp) {
    int found = 0;
    for (int i = 0; i < c->n; i++) if (cp >= c->r[i].lo && cp <= c->r[i].hi) { found = 1; break; }
    return c->neg ? !found : found;
}

/* Try to match at exactly byte offset `start`. slots[0..2*(ngroups+1)) receive the capture
   offsets (-1 when unset). Returns 1 on a match, 0 on none; *over is set if the step limit
   was hit. */
static int certo_re_exec_at(const certo_re_prog* g, const char* text, size_t len, size_t start, long* slots, int* over) {
    int nslots = 2 * (g->ngroups + 1);
    long* marks = (long*)malloc((size_t)(g->nmarks > 0 ? g->nmarks : 1) * sizeof(long));
    if (!marks) certo_panic("out of memory");
    for (int i = 0; i < nslots; i++) slots[i] = -1;
    for (int i = 0; i < g->nmarks; i++) marks[i] = -1;
    certo_re_bt* stack = NULL; int sp = 0, scap = 0;
    int pc = 0; size_t pos = start;
    long steps = 0;
    int result = 0;
    const unsigned char* s = (const unsigned char*)text;
    for (;;) {
        if (++steps > CERTO_RE_STEP_LIMIT) { *over = 1; break; }
        const certo_re_inst* in = &g->code[pc];
        int ok = 1;
        switch (in->op) {
            case RE_CHAR: {
                int l;
                if (pos < len && certo_re_decode(s, len, pos, &l) == in->c) { pos += (size_t)l; pc++; } else ok = 0;
                break;
            }
            case RE_ANY: {
                int l;
                if (pos < len) { certo_re_decode(s, len, pos, &l); pos += (size_t)l; pc++; } else ok = 0;
                break;
            }
            case RE_CLASS: {
                int l;
                if (pos < len) {
                    uint32_t cp = certo_re_decode(s, len, pos, &l);
                    if (certo_re_class_has(&g->cls[in->x], cp)) { pos += (size_t)l; pc++; } else ok = 0;
                } else ok = 0;
                break;
            }
            case RE_BOL: if (pos == 0) pc++; else ok = 0; break;
            case RE_EOL: if (pos == len) pc++; else ok = 0; break;
            case RE_WORDB: case RE_NWORDB: {
                int before = pos > 0 && certo_re_isword(s[pos - 1]);
                int after = pos < len && certo_re_isword(s[pos]);
                int at_boundary = before != after;
                if ((in->op == RE_WORDB) == at_boundary) pc++; else ok = 0;
                break;
            }
            case RE_SPLIT:
                if (sp == scap) { scap = scap ? scap * 2 : 64; stack = (certo_re_bt*)realloc(stack, (size_t)scap * sizeof(*stack)); if (!stack) certo_panic("out of memory"); }
                stack[sp].kind = 0; stack[sp].a = in->y; stack[sp].b = (long)pos; sp++;
                pc = in->x;
                break;
            case RE_JMP: pc = in->x; break;
            case RE_SAVE: case RE_MARK: {
                long* slot = in->op == RE_SAVE ? &slots[in->x] : &marks[in->x];
                if (sp == scap) { scap = scap ? scap * 2 : 64; stack = (certo_re_bt*)realloc(stack, (size_t)scap * sizeof(*stack)); if (!stack) certo_panic("out of memory"); }
                stack[sp].kind = in->op == RE_SAVE ? 1 : 2; stack[sp].a = in->x; stack[sp].b = *slot; sp++;
                *slot = (long)pos;
                pc++;
                break;
            }
            case RE_CHKPROG: if (marks[in->x] == (long)pos) ok = 0; else pc++; break;
            case RE_MATCH:
                slots[0] = (long)start; slots[1] = (long)pos;
                result = 1;
                goto done;
        }
        if (!ok) {
            for (;;) {
                if (sp == 0) goto done;
                certo_re_bt e = stack[--sp];
                if (e.kind == 1) slots[e.a] = e.b;
                else if (e.kind == 2) marks[e.a] = e.b;
                else { pc = e.a; pos = (size_t)e.b; break; }
            }
        }
    }
done:
    free(stack); free(marks);
    return result;
}

/* Leftmost match starting at or after byte offset `from`. */
static int certo_re_search(const certo_re_prog* g, const char* text, size_t len, size_t from, long* slots, int* over) {
    size_t i = from;
    for (;;) {
        if (certo_re_exec_at(g, text, len, i, slots, over)) return 1;
        if (*over || i >= len) return 0;
        int l;
        certo_re_decode((const unsigned char*)text, len, i, &l);
        i += (size_t)l;
    }
}

/* ---- the Stdlib.Regex functions ---- */

static certo_re_prog* certo_re_compile_or_panic(certo_text_t pat) {
    const char* err = NULL;
    certo_re_prog* g = certo_re_compile(pat ? pat : "", &err);
    if (!g) {
        char msg[512];
        snprintf(msg, sizeof(msg), "invalid regular expression \"%.200s\": %s", pat ? pat : "", err);
        certo_panic(msg);
    }
    return g;
}

static void certo_re_check_over(int over) {
    if (over) certo_panic("regular expression match exceeded the step limit (the pattern backtracks too much)");
}

static char* certo_re_dup(const char* s, size_t n) {
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n);
    out[n] = '\0';
    return out;
}

static int64_t certo_regex_match(certo_text_t pat, certo_text_t str) {
    certo_re_prog* g = certo_re_compile_or_panic(pat);
    long* slots = (long*)malloc((size_t)(2 * (g->ngroups + 1)) * sizeof(long));
    int over = 0;
    const char* text = str ? str : "";
    int found = certo_re_search(g, text, strlen(text), 0, slots, &over);
    free(slots); certo_re_free(g);
    certo_re_check_over(over);
    return found;
}

static certo_text_t certo_regex_find(certo_text_t pat, certo_text_t str) {
    certo_re_prog* g = certo_re_compile_or_panic(pat);
    long* slots = (long*)malloc((size_t)(2 * (g->ngroups + 1)) * sizeof(long));
    int over = 0;
    const char* text = str ? str : "";
    certo_text_t out = "";
    if (certo_re_search(g, text, strlen(text), 0, slots, &over)) out = certo_re_dup(text + slots[0], (size_t)(slots[1] - slots[0]));
    free(slots); certo_re_free(g);
    certo_re_check_over(over);
    return out;
}

/* The capture groups of the first match, in order; a group that did not take part is "". */
static CertoList* certo_regex_captures(certo_text_t pat, certo_text_t str) {
    certo_re_prog* g = certo_re_compile_or_panic(pat);
    long* slots = (long*)malloc((size_t)(2 * (g->ngroups + 1)) * sizeof(long));
    int over = 0;
    const char* text = str ? str : "";
    CertoList* list = certo_list_new();
    if (certo_re_search(g, text, strlen(text), 0, slots, &over)) {
        for (int i = 1; i <= g->ngroups; i++) {
            long a = slots[2 * i], b = slots[2 * i + 1];
            list = certo_list_push(list, (a >= 0 && b >= a) ? (void*)certo_re_dup(text + a, (size_t)(b - a)) : (void*)"");
        }
    }
    int ng = g->ngroups; (void)ng;
    free(slots); certo_re_free(g);
    certo_re_check_over(over);
    return list;
}

/* Replaces only the *first* match (BACKLOG item 240); the replacement is literal text. */
static certo_text_t certo_regex_replace(certo_text_t pat, certo_text_t str, certo_text_t repl) {
    certo_re_prog* g = certo_re_compile_or_panic(pat);
    long* slots = (long*)malloc((size_t)(2 * (g->ngroups + 1)) * sizeof(long));
    int over = 0;
    const char* text = str ? str : "";
    size_t len = strlen(text);
    certo_text_t out;
    if (!certo_re_search(g, text, len, 0, slots, &over)) {
        out = certo_re_dup(text, len);
    } else {
        size_t pre = (size_t)slots[0], suf = len - (size_t)slots[1], rl = strlen(repl ? repl : "");
        char* buf = (char*)malloc(pre + rl + suf + 1);
        if (!buf) certo_panic("out of memory");
        memcpy(buf, text, pre);
        if (rl) memcpy(buf + pre, repl, rl);
        memcpy(buf + pre + rl, text + slots[1], suf);
        buf[pre + rl + suf] = '\0';
        out = buf;
    }
    free(slots); certo_re_free(g);
    certo_re_check_over(over);
    return out;
}

/* Splits at each match, like Text.split: the pieces between matches, with a final piece after
   the last one (empty if the text ends with a match). An empty match never splits at the
   start of a piece, so a pattern that can match nothing splits the text into characters. */
static CertoList* certo_regex_split(certo_text_t pat, certo_text_t str) {
    certo_re_prog* g = certo_re_compile_or_panic(pat);
    long* slots = (long*)malloc((size_t)(2 * (g->ngroups + 1)) * sizeof(long));
    const char* text = str ? str : "";
    size_t len = strlen(text);
    CertoList* list = certo_list_new();
    int over = 0;
    if (len == 0) {
        if (!certo_re_exec_at(g, text, len, 0, slots, &over)) list = certo_list_push(list, (void*)"");
    } else {
        size_t p = 0, q = 0;
        while (q < len) {
            if (certo_re_exec_at(g, text, len, q, slots, &over) && (size_t)slots[1] != p) {
                size_t e = (size_t)slots[1];
                list = certo_list_push(list, certo_re_dup(text + p, q - p));
                p = e;
                q = (e > q) ? e : q;
                if (e == q && e == p && slots[0] == slots[1]) { int l; certo_re_decode((const unsigned char*)text, len, q, &l); q += (size_t)l; }
            } else {
                if (over) break;
                int l;
                certo_re_decode((const unsigned char*)text, len, q, &l);
                q += (size_t)l;
            }
        }
        if (!over) list = certo_list_push(list, certo_re_dup(text + p, len - p));
    }
    free(slots); certo_re_free(g);
    certo_re_check_over(over);
    return list;
}
"##;
