#include <sqlite3.h>
#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef const char *certo_text_t;

typedef struct { char *p; size_t n, cap; } Buf;

static void grow(Buf *b, size_t extra) {
    if (b->n + extra + 1 <= b->cap) return;
    size_t cap = b->cap ? b->cap : 4096;
    while (cap < b->n + extra + 1) cap *= 2;
    b->p = (char *)realloc(b->p, cap);
    b->cap = cap;
}

static void addn(Buf *b, const char *s, size_t n) {
    grow(b, n); memcpy(b->p + b->n, s, n); b->n += n; b->p[b->n] = 0;
}

static void add(Buf *b, const char *s) { addn(b, s, strlen(s)); }

static void addf(Buf *b, const char *fmt, long long a, long long c) {
    char tmp[160]; int n = snprintf(tmp, sizeof tmp, fmt, a, c); if (n > 0) addn(b, tmp, (size_t)n);
}

static void html(Buf *b, const char *s) {
    if (!s) { add(b, "<span style=\"color:#66717f\">NULL</span>"); return; }
    for (; *s; ++s) switch (*s) {
        case '&': add(b, "&amp;"); break; case '<': add(b, "&lt;"); break;
        case '>': add(b, "&gt;"); break; case '"': add(b, "&quot;"); break;
        case '\'': add(b, "&#39;"); break; default: addn(b, s, 1);
    }
}

static char *decode(const char *s) {
    size_t n = strlen(s), j = 0; char *out = (char *)malloc(n + 1);
    for (size_t i = 0; i < n; ++i) {
        if (s[i] == '+') out[j++] = ' ';
        else if (s[i] == '%' && i + 2 < n && isxdigit((unsigned char)s[i+1]) && isxdigit((unsigned char)s[i+2])) {
            char h[3] = {s[i+1], s[i+2], 0}; out[j++] = (char)strtol(h, NULL, 16); i += 2;
        } else out[j++] = s[i];
    }
    out[j] = 0; return out;
}

static const char *allowed_sort(const char *table, const char *sort) {
    static const char *files[] = {"id","backup_id","relative_path","file_size","modified_time_unix","blake3"};
    static const char *backups[] = {"id","source_folder","archive_path","archive_blake3","created_at_unix","compression_method","compression_level","include_hidden","total_files","total_directories","total_input_bytes","archive_bytes"};
    const char **cols = strcmp(table, "backups") == 0 ? backups : files;
    size_t count = strcmp(table, "backups") == 0 ? 12 : 6;
    for (size_t i = 0; i < count; ++i) if (strcmp(sort, cols[i]) == 0) return cols[i];
    return "id";
}

static void title(Buf *b, const char *name) {
    int upper = 1; for (; *name; ++name) { char c = *name == '_' ? ' ' : *name; if (upper) c = (char)toupper((unsigned char)c); addn(b, &c, 1); upper = c == ' '; }
}

static void human_bytes(Buf *b, sqlite3_int64 value) {
    const char *units[] = {"B","KB","MB","GB","TB"}; double v = (double)value; int u = 0;
    while (v >= 1024.0 && u < 4) { v /= 1024.0; ++u; }
    char tmp[64]; snprintf(tmp, sizeof tmp, u ? "%.1f %s" : "%.0f %s", v, units[u]); add(b, tmp);
}

static void local_time(Buf *b, sqlite3_int64 value) {
    time_t raw = (time_t)value; struct tm tmv;
#ifdef _WIN32
    localtime_s(&tmv, &raw);
#else
    localtime_r(&raw, &tmv);
#endif
    char tmp[64]; strftime(tmp, sizeof tmp, "%d %b %Y, %H:%M:%S", &tmv); add(b, tmp);
}

static void error_page(Buf *b, const char *message) {
    add(b, "<div style=\"border:1px solid #632f35;background:#28171a;color:#ffb5bd;padding:14px;border-radius:9px\">"); html(b, message); add(b, "</div>");
}

certo_text_t vickiSqliteTableHtml(certo_text_t database, certo_text_t table_arg, certo_text_t search_arg,
                                  certo_text_t sort_arg, certo_text_t direction, int64_t page, int64_t page_size) {
    Buf b = {0}; const char *table = strcmp(table_arg, "backups") == 0 ? "backups" : "backup_files";
    const char *sort = allowed_sort(table, sort_arg ? sort_arg : "");
    const char *dir = direction && strcmp(direction, "asc") == 0 ? "ASC" : "DESC";
    if (page < 1) page = 1; if (page_size < 1) page_size = 50; if (page_size > 250) page_size = 250;
    char *search = decode(search_arg ? search_arg : ""); sqlite3 *db = NULL;
    int flags = SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX;
    if (sqlite3_open_v2(database, &db, flags, NULL) != SQLITE_OK) { error_page(&b, db ? sqlite3_errmsg(db) : "Could not open database"); if (db) sqlite3_close(db); free(search); return b.p; }

    const char *where_files = " WHERE CAST(id AS TEXT) LIKE ?1 OR CAST(backup_id AS TEXT) LIKE ?1 OR relative_path LIKE ?1 OR blake3 LIKE ?1";
    const char *where_backups = " WHERE CAST(id AS TEXT) LIKE ?1 OR source_folder LIKE ?1 OR archive_path LIKE ?1 OR archive_blake3 LIKE ?1 OR compression_method LIKE ?1";
    const char *where = *search ? (strcmp(table,"backups") == 0 ? where_backups : where_files) : "";
    char count_sql[700]; snprintf(count_sql, sizeof count_sql, "SELECT COUNT(*) FROM %s%s", table, where);
    sqlite3_stmt *count_stmt = NULL; long long total = 0;
    if (sqlite3_prepare_v2(db, count_sql, -1, &count_stmt, NULL) == SQLITE_OK) {
        char pattern[1024]; snprintf(pattern, sizeof pattern, "%%%s%%", search); if (*search) sqlite3_bind_text(count_stmt, 1, pattern, -1, SQLITE_TRANSIENT);
        if (sqlite3_step(count_stmt) == SQLITE_ROW) total = sqlite3_column_int64(count_stmt, 0);
    }
    sqlite3_finalize(count_stmt);

    char sql[900]; snprintf(sql, sizeof sql, "SELECT * FROM %s%s ORDER BY %s %s LIMIT ?2 OFFSET ?3", table, where, sort, dir);
    sqlite3_stmt *stmt = NULL;
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, NULL) != SQLITE_OK) { error_page(&b, sqlite3_errmsg(db)); sqlite3_close(db); free(search); return b.p; }
    char pattern[1024]; snprintf(pattern, sizeof pattern, "%%%s%%", search); if (*search) sqlite3_bind_text(stmt, 1, pattern, -1, SQLITE_TRANSIENT);
    sqlite3_bind_int64(stmt, 2, page_size); sqlite3_bind_int64(stmt, 3, (page - 1) * page_size);
    int cols = sqlite3_column_count(stmt);

    add(&b, "<div class=\"eyebrow\">SQLite / master.db</div><h1>"); title(&b, table); add(&b, "</h1><div class=\"meta\">");
    addf(&b, "%lld rows · %lld columns", total, cols); add(&b, " · Read-only</div>");
    add(&b, "<form class=\"toolbar\" method=\"get\"><input type=\"hidden\" name=\"table\" value=\""); html(&b, table); add(&b, "\">");
    add(&b, "<input class=\"search\" type=\"search\" name=\"q\" placeholder=\"Search this table…\" value=\""); html(&b, search); add(&b, "\"><button class=\"button\">Search</button></form>");
    add(&b, "<section class=\"frame\"><div class=\"scroll\"><table><thead><tr>");
    for (int c = 0; c < cols; ++c) { const char *name = sqlite3_column_name(stmt,c); add(&b,"<th><a href=\"/?table="); html(&b,table); add(&b,"&q="); html(&b,search_arg?search_arg:""); add(&b,"&sort="); html(&b,name); add(&b,"&dir="); add(&b, strcmp(sort,name)==0 && strcmp(dir,"ASC")==0 ? "desc" : "asc"); add(&b,"\">"); title(&b,name); add(&b,"</a></th>"); }
    add(&b, "</tr></thead><tbody>"); int shown = 0;
    while (sqlite3_step(stmt) == SQLITE_ROW) { ++shown; add(&b,"<tr>"); for (int c=0;c<cols;++c) { const char *name=sqlite3_column_name(stmt,c); int numeric=sqlite3_column_type(stmt,c)==SQLITE_INTEGER; add(&b,numeric?"<td class=\"num\">":"<td>"); if(sqlite3_column_type(stmt,c)==SQLITE_NULL) html(&b,NULL); else if(strstr(name,"_unix")) local_time(&b,sqlite3_column_int64(stmt,c)); else if(strstr(name,"bytes")||strcmp(name,"file_size")==0) human_bytes(&b,sqlite3_column_int64(stmt,c)); else { const char *v=(const char*)sqlite3_column_text(stmt,c); if(strstr(name,"blake3")){add(&b,"<span class=\"hash\" title=\"");html(&b,v);add(&b,"\">");if(v&&strlen(v)>12)addn(&b,v,12);else html(&b,v);add(&b,"…</span>");}else {if(strstr(name,"path")||strcmp(name,"source_folder")==0)add(&b,"<span class=\"path\">");html(&b,v);if(strstr(name,"path")||strcmp(name,"source_folder")==0)add(&b,"</span>");}} add(&b,"</td>"); } add(&b,"</tr>"); }
    if (!shown) { addf(&b,"<tr><td colspan=\"%lld\"><div class=\"empty\">No matching rows</div></td></tr>",cols,0); }
    sqlite3_finalize(stmt); sqlite3_close(db);
    long long pages = total ? (total + page_size - 1) / page_size : 1, start = total ? (page-1)*page_size+1 : 0, end = page*page_size < total ? page*page_size : total;
    add(&b,"</tbody></table></div><footer class=\"foot\"><span>");addf(&b,"Showing %lld–%lld",start,end);addf(&b," of %lld</span><div class=\"pages\">",total,0);
    if(page>1){add(&b,"<a href=\"/?table=");html(&b,table);add(&b,"&q=");html(&b,search_arg?search_arg:"");add(&b,"&sort=");html(&b,sort);add(&b,"&dir=");html(&b,direction);addf(&b,"&page=%lld\">← Previous</a>",page-1,0);} addf(&b,"<span>Page %lld of %lld</span>",page,pages);
    if(page<pages){add(&b,"<a href=\"/?table=");html(&b,table);add(&b,"&q=");html(&b,search_arg?search_arg:"");add(&b,"&sort=");html(&b,sort);add(&b,"&dir=");html(&b,direction);addf(&b,"&page=%lld\">Next →</a>",page+1,0);} add(&b,"</div></footer></section>"); free(search); return b.p;
}
