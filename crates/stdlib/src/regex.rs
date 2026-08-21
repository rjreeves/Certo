pub const REGEX_C: &str = r##"
/* ===== Stdlib.Regex ===== */
/* Backtracking regex engine — supports: . * + ? ^ $ [] () captures */
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

#define CERTO_RE_NCAP 10

typedef struct {
    const char *start;
    int         len;
} certo_re_cap;

/* Forward declarations */
static const char *_re_match(const char *pat, const char *str, certo_re_cap *caps, int ncaps);

static int _re_matchchar(const char *pat, char c, int *consumed) {
    *consumed = 1;
    if (*pat == '.') return (c != '\0');
    if (*pat == '[') {
        const char *p = pat + 1; int neg = 0, found = 0;
        if (*p == '^') { neg = 1; p++; }
        while (*p && *p != ']') {
            if (*(p+1) == '-' && *(p+2) && *(p+2) != ']') {
                if (c >= *p && c <= *(p+2)) found = 1;
                p += 3;
            } else {
                if (*p == c) found = 1;
                p++;
            }
        }
        *consumed = (int)(p - pat) + 1; /* skip past ] */
        return neg ? !found : found;
    }
    return (*pat == c);
}

static int _re_patlen(const char *pat) {
    if (*pat == '[') {
        int i = 1;
        if (pat[i] == '^') i++;
        while (pat[i] && pat[i] != ']') i++;
        return i + 1;
    }
    return 1;
}

static const char *_re_match(const char *pat, const char *str, certo_re_cap *caps, int ncaps) {
    while (1) {
        if (*pat == '\0') return str;
        if (*pat == '$' && *(pat+1) == '\0') return (*str == '\0') ? str : NULL;
        if (*pat == '(') {
            /* find matching close, try to match inner */
            int depth = 1, i = 1;
            while (pat[i] && depth > 0) { if (pat[i]=='(') depth++; else if (pat[i]==')') depth--; i++; }
            /* inner is pat+1 .. pat+i-1, then rest is pat+i */
            for (int ci = 0; ci < ncaps; ci++) {
                if (caps[ci].start == NULL) {
                    caps[ci].start = str;
                    char *inner = (char*)malloc(i); strncpy(inner, pat+1, i-2); inner[i-2]='\0';
                    const char *after = _re_match(inner, str, caps+ci+1, ncaps-ci-1);
                    free(inner);
                    if (after) {
                        caps[ci].len = (int)(after - str);
                        const char *rest = _re_match(pat+i, after, caps, ncaps);
                        if (rest) return rest;
                    }
                    caps[ci].start = NULL; caps[ci].len = 0;
                    return NULL;
                }
            }
            return NULL;
        }
        int consumed = 0;
        int plen = _re_patlen(pat);
        char quant = *(pat + plen);
        if (quant == '*' || quant == '+' || quant == '?') {
            int min = (quant == '+') ? 1 : 0;
            int max = (quant == '?') ? 1 : 256*256;
            const char *s = str; int count = 0;
            while (count < max && *s) {
                if (!_re_matchchar(pat, *s, &consumed)) break;
                s++; count++;
            }
            /* greedy: try longest first */
            while (count >= min) {
                const char *rest = _re_match(pat+plen+1, str+count, caps, ncaps);
                if (rest) return rest;
                count--;
            }
            return NULL;
        }
        if (!_re_matchchar(pat, *str, &consumed)) return NULL;
        pat += plen; str++;
    }
}

static int _re_find(const char *pat, const char *str, certo_re_cap *caps, int ncaps, const char **match_start, const char **match_end) {
    int anchored = (*pat == '^');
    const char *p = anchored ? pat+1 : pat;
    const char *s = str;
    do {
        for (int i = 0; i < ncaps; i++) { caps[i].start = NULL; caps[i].len = 0; }
        const char *end = _re_match(p, s, caps, ncaps);
        if (end) {
            if (match_start) *match_start = s;
            if (match_end) *match_end = end;
            return 1;
        }
        s++;
    } while (!anchored && *s);
    return 0;
}

static int64_t certo_regex_match(certo_text_t pat, certo_text_t str) {
    certo_re_cap caps[CERTO_RE_NCAP] = {0};
    return _re_find(pat, str, caps, CERTO_RE_NCAP, NULL, NULL);
}

static certo_text_t certo_regex_find(certo_text_t pat, certo_text_t str) {
    certo_re_cap caps[CERTO_RE_NCAP] = {0};
    const char *ms, *me;
    if (!_re_find(pat, str, caps, CERTO_RE_NCAP, &ms, &me)) return "";
    int len = (int)(me - ms);
    char *out = (char*)malloc(len+1); strncpy(out, ms, len); out[len] = 0;
    return out;
}

static CertoList* certo_regex_captures(certo_text_t pat, certo_text_t str) {
    certo_re_cap caps[CERTO_RE_NCAP] = {0};
    _re_find(pat, str, caps, CERTO_RE_NCAP, NULL, NULL);
    CertoList *list = certo_list_new();
    for (int i = 0; i < CERTO_RE_NCAP; i++) {
        if (!caps[i].start) break;
        char *s = (char*)malloc(caps[i].len+1);
        strncpy(s, caps[i].start, caps[i].len); s[caps[i].len]=0;
        list = certo_list_push(list, s);
    }
    return list;
}

static certo_text_t certo_regex_replace(certo_text_t pat, certo_text_t str, certo_text_t repl) {
    size_t cap = strlen(str)*2 + 64; char *out = (char*)malloc(cap); size_t j = 0;
    const char *s = str;
    while (*s) {
        certo_re_cap caps[CERTO_RE_NCAP] = {0};
        const char *ms, *me;
        if (_re_find(pat, s, caps, CERTO_RE_NCAP, &ms, &me)) {
            /* copy prefix */
            size_t pre = ms - s;
            if (j + pre + strlen(repl) + (strlen(s) - (me-s)) + 2 > cap) {
                cap *= 2; out = (char*)realloc(out, cap);
            }
            memcpy(out+j, s, pre); j += pre;
            size_t rl = strlen(repl); memcpy(out+j, repl, rl); j += rl;
            s = me;
        } else {
            if (j + 1 >= cap) { cap *= 2; out = (char*)realloc(out, cap); }
            out[j++] = *s++;
        }
    }
    out[j] = 0; return out;
}

static CertoList* certo_regex_split(certo_text_t pat, certo_text_t str) {
    CertoList *list = certo_list_new();
    const char *s = str;
    while (*s) {
        certo_re_cap caps[CERTO_RE_NCAP] = {0};
        const char *ms, *me;
        if (_re_find(pat, s, caps, CERTO_RE_NCAP, &ms, &me) && ms == s) {
            /* zero-length match guard */
            if (me == ms) { char *seg = (char*)malloc(2); seg[0]=*s; seg[1]=0; list = certo_list_push(list, seg); s++; continue; }
            char *seg = (char*)malloc(1); seg[0]=0; list = certo_list_push(list, seg);
            s = me;
        } else if (_re_find(pat, s, caps, CERTO_RE_NCAP, &ms, &me)) {
            int len = (int)(ms - s); char *seg = (char*)malloc(len+1); strncpy(seg, s, len); seg[len]=0;
            list = certo_list_push(list, seg); s = me;
        } else {
            char *seg = (char*)malloc(strlen(s)+1); strcpy(seg, s); list = certo_list_push(list, seg);
            break;
        }
    }
    return list;
}
"##;
