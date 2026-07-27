pub const CSV_C: &str = r#"
/* ===== Stdlib.Csv — RFC 4180 CSV parser / serializer ===== */
#include <string.h>
#include <stdlib.h>

/* Parse one field starting at *p. Advances *p past the delimiter/newline.
   Returns a malloc'd string. Sets *eol if end of row was reached. */
static char *_csv_parse_field(const char **p, int *eol) {
    *eol = 0;
    const char *s = *p;
    char *out; size_t cap = 64, len = 0;
    out = (char*)malloc(cap);

    if (*s == '"') {
        s++; /* skip opening quote */
        while (*s) {
            if (*s == '"') {
                if (*(s+1) == '"') { /* escaped quote */
                    if (len+1 >= cap) { cap *= 2; out = (char*)realloc(out, cap); }
                    out[len++] = '"'; s += 2;
                } else { s++; break; } /* closing quote */
            } else {
                if (len+1 >= cap) { cap *= 2; out = (char*)realloc(out, cap); }
                out[len++] = *s++;
            }
        }
        /* skip optional comma after closing quote */
        if (*s == ',') s++;
        else if (*s == '\r' && *(s+1) == '\n') { s += 2; *eol = 1; }
        else if (*s == '\n') { s++; *eol = 1; }
        else if (*s == '\0') *eol = 1;
    } else {
        while (*s && *s != ',' && *s != '\n' && *s != '\r') {
            if (len+1 >= cap) { cap *= 2; out = (char*)realloc(out, cap); }
            out[len++] = *s++;
        }
        if (*s == ',') s++;
        else if (*s == '\r' && *(s+1) == '\n') { s += 2; *eol = 1; }
        else if (*s == '\n') { s++; *eol = 1; }
        else if (*s == '\0') *eol = 1;
    }
    out[len] = 0;
    *p = s;
    return out;
}

/* Csv.parse(text) → List<List<Text>> */
static CertoList *certo_csv_parse(certo_text_t text) {
    CertoList *rows = certo_list_new();
    const char *p = text;
    while (*p) {
        CertoList *row = certo_list_new();
        int eol = 0;
        while (!eol && *p) {
            char *field = _csv_parse_field(&p, &eol);
            certo_list_push(row, field);
        }
        certo_list_push(rows, row);
        /* skip trailing empty line at EOF */
        if (!*p && row->len == 1 && ((char*)row->data[0])[0] == '\0') {
            rows->len--;
        }
    }
    return rows;
}

/* Csv.serialize(rows) → Text */
static certo_text_t certo_csv_serialize(CertoList *rows) {
    size_t cap = 256, len = 0;
    char *out = (char*)malloc(cap);
    for (int64_t r = 0; r < rows->len; r++) {
        CertoList *row = (CertoList*)rows->data[r];
        for (int64_t c = 0; c < row->len; c++) {
            const char *field = (const char*)row->data[c];
            /* quote if field contains comma, quote, or newline */
            int needs_quote = 0;
            for (const char *ch = field; *ch; ch++) {
                if (*ch == ',' || *ch == '"' || *ch == '\n' || *ch == '\r') { needs_quote = 1; break; }
            }
            size_t flen = strlen(field);
            size_t need = flen * 2 + 4; /* worst case: all quotes */
            while (len + need >= cap) { cap *= 2; out = (char*)realloc(out, cap); }
            if (needs_quote) {
                out[len++] = '"';
                for (size_t i = 0; i < flen; i++) {
                    if (field[i] == '"') out[len++] = '"'; /* escape */
                    out[len++] = field[i];
                }
                out[len++] = '"';
            } else {
                memcpy(out+len, field, flen); len += flen;
            }
            if (c < row->len - 1) out[len++] = ',';
        }
        out[len++] = '\n';
    }
    out[len] = 0;
    return out;
}

/* Csv.header(rows) → List<Text>  — first row as header */
static CertoList *certo_csv_header(CertoList *rows) {
    if (rows->len == 0) return certo_list_new();
    return (CertoList*)rows->data[0];
}

/* Csv.rows(rows) → List<List<Text>>  — all rows except header */
static CertoList *certo_csv_rows(CertoList *rows) {
    CertoList *out = certo_list_new();
    for (int64_t i = 1; i < rows->len; i++) certo_list_push(out, rows->data[i]);
    return out;
}
"#;
