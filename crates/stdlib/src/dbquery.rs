/// Query builder — a fluent, schema-checked layer over Stdlib.Db.
///
/// `Query.from`/`.filter`/`.orderBy`/`.join`/`.leftJoin`/`.groupBy`/`.aggregate`/`.having`/
/// `.limit`/`.offset` build up a query; `.list`/`.first`/`.count`/`.sum`/`.avg`/`.min`/`.max`/
/// `.groupedList` run it. The table/column/operator/direction/aggregate-function/alias
/// arguments must be string literals at the call site — `certo_dbschema::check_query`
/// validates those literals against the schema (tables/columns declared as `type`s in the
/// module, join columns qualified as `"Table.column"`) at compile time, and rejects anything
/// that isn't a literal, because a computed value can't be verified statically. Values passed
/// to `.filter`/`.having` are never literal-checked — they always travel as bound `$N`
/// parameters, so there is no SQL-injection surface here regardless of where they come from.
///
/// Two query "shapes":
///   - Row queries (no `.groupBy`/`.aggregate`) run `SELECT * FROM <table> [JOIN ...] ...`,
///     so — like hand-written `dbQuery` — the physical column order must match the order
///     fields are declared in the `type`. Read with `.list`/`.first`/`.count`.
///   - Grouped/aggregated queries (after `.groupBy` and/or `.aggregate`) run
///     `SELECT <group cols>, <agg(col) AS alias>... FROM ... GROUP BY ... [HAVING ...]`.
///     Read with `.groupedList` — `.list`/`.first`/`.count`/`.sum`/`.avg`/`.min`/`.max` are
///     rejected at compile time (E0516) since they assume the row-query shape.
///
/// `.from`/`.join`/`.leftJoin` default a table's alias to its own name, matching the common
/// case where a table appears once. `.fromAs`/`.joinAs`/`.leftJoinAs` take an explicit alias
/// instead — the only way to join the same table to itself (a self-join), since two
/// occurrences of the same table need two distinct aliases to be referenceable at all.
/// Reusing an alias already in scope is a compile error (E0526), which is also what makes
/// plain (non-aliased) `.join` of the same table twice a clear error rather than silently
/// broken SQL.
pub const DBQUERY_C: &str = r#"
/* ------------------------------------------------------------------ */
/* Stdlib.DbQuery — fluent Query builder over Stdlib.Db                */
/* ------------------------------------------------------------------ */
#include <ctype.h>

typedef struct {
    certo_text_t table;
    certo_text_t alias;       /* defaults to table (see certo_query_from) — distinct aliases
                                 are what make self-joins possible (certo_query_from_as/join_as) */
    CertoList*   joins;       /* List<Text> — "JOIN Table AS alias ON a = b" / "LEFT JOIN ..." fragments */
    CertoList*   wheres;      /* List<Text> — "col op $N" fragments, in order */
    CertoList*   params;      /* List<Text> — bound values, parallel to $N placeholders (WHERE + HAVING) */
    CertoList*   group_by;    /* List<Text> — GROUP BY column names, in order */
    CertoList*   aggregates;  /* List<Text> — "AGG(col) AS alias" fragments, in order */
    CertoList*   having;      /* List<Text> — "AGG(col) op $N" fragments, in order */
    certo_text_t order_col;   /* NULL = no ORDER BY */
    int64_t      order_desc;  /* 0 = ASC, 1 = DESC */
    int64_t      limit_n;     /* -1 = no LIMIT */
    int64_t      offset_n;    /* -1 = no OFFSET */
} CertoQuery;

CertoQuery* certo_query_from_as(certo_text_t table, certo_text_t alias) {
    CertoQuery* q = (CertoQuery*)malloc(sizeof(CertoQuery));
    if (!q) certo_panic("out of memory");
    q->table      = certo_db_strdup(table);
    q->alias      = certo_db_strdup(alias);
    q->joins      = certo_list_new_empty();
    q->wheres     = certo_list_new_empty();
    q->params     = certo_list_new_empty();
    q->group_by   = certo_list_new_empty();
    q->aggregates = certo_list_new_empty();
    q->having     = certo_list_new_empty();
    q->order_col  = NULL;
    q->order_desc = 0;
    q->limit_n    = -1;
    q->offset_n   = -1;
    return q;
}

CertoQuery* certo_query_from(certo_text_t table) {
    return certo_query_from_as(table, table);
}

/* Builder calls are functional (like certo_list_push): each returns a new CertoQuery,
 * leaving the original untouched, consistent with every other Certo collection type. */
static CertoQuery* certo_query_clone(CertoQuery* q) {
    CertoQuery* n = (CertoQuery*)malloc(sizeof(CertoQuery));
    if (!n) certo_panic("out of memory");
    *n = *q;
    return n;
}

CertoQuery* certo_query_filter(CertoQuery* q, certo_text_t column, certo_text_t op, certo_text_t value) {
    CertoQuery* n = certo_query_clone(q);
    int64_t param_n = (q->params ? q->params->len : 0) + 1;
    char frag[160];
    snprintf(frag, sizeof(frag), "%s %s $%" PRId64, column, op, param_n);
    n->wheres = certo_list_push(q->wheres, (void*)certo_db_strdup(frag));
    n->params = certo_list_push(q->params, (void*)certo_db_strdup(value));
    return n;
}

CertoQuery* certo_query_order_by(CertoQuery* q, certo_text_t column, certo_text_t dir) {
    CertoQuery* n = certo_query_clone(q);
    n->order_col  = certo_db_strdup(column);
    n->order_desc = (strcmp(dir, "desc") == 0) ? 1 : 0;
    return n;
}

CertoQuery* certo_query_limit(CertoQuery* q, int64_t limit_n) {
    CertoQuery* n = certo_query_clone(q);
    n->limit_n = limit_n;
    return n;
}

CertoQuery* certo_query_offset(CertoQuery* q, int64_t offset_n) {
    CertoQuery* n = certo_query_clone(q);
    n->offset_n = offset_n;
    return n;
}

CertoQuery* certo_query_join_as(CertoQuery* q, certo_text_t table, certo_text_t alias, certo_text_t left_col, certo_text_t right_col) {
    CertoQuery* n = certo_query_clone(q);
    char frag[220];
    snprintf(frag, sizeof(frag), "JOIN %s AS %s ON %s = %s", table, alias, left_col, right_col);
    n->joins = certo_list_push(q->joins, (void*)certo_db_strdup(frag));
    return n;
}

CertoQuery* certo_query_join(CertoQuery* q, certo_text_t table, certo_text_t left_col, certo_text_t right_col) {
    return certo_query_join_as(q, table, table, left_col, right_col);
}

CertoQuery* certo_query_left_join_as(CertoQuery* q, certo_text_t table, certo_text_t alias, certo_text_t left_col, certo_text_t right_col) {
    CertoQuery* n = certo_query_clone(q);
    char frag[220];
    snprintf(frag, sizeof(frag), "LEFT JOIN %s AS %s ON %s = %s", table, alias, left_col, right_col);
    n->joins = certo_list_push(q->joins, (void*)certo_db_strdup(frag));
    return n;
}

CertoQuery* certo_query_left_join(CertoQuery* q, certo_text_t table, certo_text_t left_col, certo_text_t right_col) {
    return certo_query_left_join_as(q, table, table, left_col, right_col);
}

CertoQuery* certo_query_group_by(CertoQuery* q, certo_text_t column) {
    CertoQuery* n = certo_query_clone(q);
    n->group_by = certo_list_push(q->group_by, (void*)certo_db_strdup(column));
    return n;
}

/* "COUNT(*)" for count(*,"*"), else "SUM(col)"/"AVG(col)"/"MIN(col)"/"MAX(col)" —
 * uppercased so the generated SQL reads naturally. Caller frees the result. */
static char* certo_query_agg_expr(certo_text_t agg_fn, certo_text_t column) {
    char* buf = (char*)malloc(128);
    if (!buf) certo_panic("out of memory");
    if (strcmp(agg_fn, "count") == 0 && strcmp(column, "*") == 0) {
        snprintf(buf, 128, "COUNT(*)");
    } else {
        char up[16];
        size_t i = 0;
        for (; agg_fn[i] && i + 1 < sizeof(up); i++) up[i] = (char)toupper((unsigned char)agg_fn[i]);
        up[i] = '\0';
        snprintf(buf, 128, "%s(%s)", up, column);
    }
    return buf;
}

CertoQuery* certo_query_aggregate(CertoQuery* q, certo_text_t agg_fn, certo_text_t column, certo_text_t alias) {
    CertoQuery* n = certo_query_clone(q);
    char* expr = certo_query_agg_expr(agg_fn, column);
    char frag[160];
    snprintf(frag, sizeof(frag), "%s AS %s", expr, alias);
    free(expr);
    n->aggregates = certo_list_push(q->aggregates, (void*)certo_db_strdup(frag));
    return n;
}

CertoQuery* certo_query_having(CertoQuery* q, certo_text_t agg_fn, certo_text_t column, certo_text_t op, certo_text_t value) {
    CertoQuery* n = certo_query_clone(q);
    char* expr = certo_query_agg_expr(agg_fn, column);
    int64_t param_n = (q->params ? q->params->len : 0) + 1;
    char frag[160];
    snprintf(frag, sizeof(frag), "%s %s $%" PRId64, expr, op, param_n);
    free(expr);
    n->having = certo_list_push(q->having, (void*)certo_db_strdup(frag));
    n->params = certo_list_push(q->params, (void*)certo_db_strdup(value));
    return n;
}

/* Shared WHERE-clause fragment builder used by SELECT, COUNT, and scalar-aggregate queries. */
static size_t certo_query_where_clause(CertoQuery* q, char* out, size_t cap, size_t pos) {
    int64_t nwheres = q->wheres ? q->wheres->len : 0;
    for (int64_t i = 0; i < nwheres; i++) {
        pos += (size_t)snprintf(out + pos, cap - pos, "%s %s",
                                 i == 0 ? " WHERE" : " AND",
                                 (const char*)q->wheres->data[i]);
    }
    return pos;
}

static size_t certo_query_sql_capacity(CertoQuery* q) {
    size_t cap = 256;
    int64_t n;
    n = q->joins      ? q->joins->len      : 0; for (int64_t i = 0; i < n; i++) cap += strlen((const char*)q->joins->data[i])      + 8;
    n = q->wheres     ? q->wheres->len     : 0; for (int64_t i = 0; i < n; i++) cap += strlen((const char*)q->wheres->data[i])     + 8;
    n = q->group_by   ? q->group_by->len   : 0; for (int64_t i = 0; i < n; i++) cap += strlen((const char*)q->group_by->data[i])   + 8;
    n = q->aggregates ? q->aggregates->len : 0; for (int64_t i = 0; i < n; i++) cap += strlen((const char*)q->aggregates->data[i]) + 8;
    n = q->having     ? q->having->len     : 0; for (int64_t i = 0; i < n; i++) cap += strlen((const char*)q->having->data[i])     + 8;
    return cap;
}

/* Row shape:     "SELECT * FROM t [JOIN ...] [WHERE ...] [ORDER BY ...] [LIMIT n] [OFFSET n]"
 * Grouped shape: "SELECT <group cols>, <agg AS alias>... FROM t [JOIN ...] [WHERE ...]
 *                 GROUP BY <group cols> [HAVING ...] [ORDER BY ...] [LIMIT n] [OFFSET n]"
 * Switches shape automatically based on whether `.groupBy`/`.aggregate` were ever called —
 * `certo_dbschema::check_query` is what actually prevents mixing shapes incorrectly
 * (E0516), this function just builds whatever the query's accumulated state describes. */
static char* certo_query_build_sql(CertoQuery* q) {
    size_t cap = certo_query_sql_capacity(q);
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");

    int64_t ngroup = q->group_by   ? q->group_by->len   : 0;
    int64_t naggs  = q->aggregates ? q->aggregates->len : 0;
    int64_t njoins = q->joins      ? q->joins->len      : 0;
    size_t pos;

    if (ngroup > 0 || naggs > 0) {
        pos = (size_t)snprintf(sql, cap, "SELECT ");
        int64_t first = 1;
        for (int64_t i = 0; i < ngroup; i++) {
            pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s", first ? "" : ", ", (const char*)q->group_by->data[i]);
            first = 0;
        }
        for (int64_t i = 0; i < naggs; i++) {
            pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s", first ? "" : ", ", (const char*)q->aggregates->data[i]);
            first = 0;
        }
        pos += (size_t)snprintf(sql + pos, cap - pos, " FROM %s AS %s", q->table, q->alias);
    } else {
        pos = (size_t)snprintf(sql, cap, "SELECT * FROM %s AS %s", q->table, q->alias);
    }

    for (int64_t i = 0; i < njoins; i++)
        pos += (size_t)snprintf(sql + pos, cap - pos, " %s", (const char*)q->joins->data[i]);

    pos = certo_query_where_clause(q, sql, cap, pos);

    if (ngroup > 0) {
        pos += (size_t)snprintf(sql + pos, cap - pos, " GROUP BY ");
        for (int64_t i = 0; i < ngroup; i++) {
            pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s", i == 0 ? "" : ", ", (const char*)q->group_by->data[i]);
        }
    }

    int64_t nhaving = q->having ? q->having->len : 0;
    for (int64_t i = 0; i < nhaving; i++) {
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s %s", i == 0 ? " HAVING" : " AND", (const char*)q->having->data[i]);
    }

    if (q->order_col) {
        pos += (size_t)snprintf(sql + pos, cap - pos, " ORDER BY %s %s",
                                 q->order_col, q->order_desc ? "DESC" : "ASC");
    }
    if (q->limit_n >= 0)  pos += (size_t)snprintf(sql + pos, cap - pos, " LIMIT %"  PRId64, q->limit_n);
    if (q->offset_n >= 0) pos += (size_t)snprintf(sql + pos, cap - pos, " OFFSET %" PRId64, q->offset_n);
    return sql;
}

certo_text_t certo_query_sql(CertoQuery* q) {
    return certo_query_build_sql(q);
}

CertoList* certo_query_list(CertoQuery* q, int64_t conn, certo_fn_t mapper) {
    char* sql = certo_query_build_sql(q);
    CertoList* result = certo_db_query_typed(conn, sql, q->params, mapper);
    free(sql);
    return result;
}

/* Returns Some(mapped row) or None — implemented as `.limit(1).list(...)`, not
 * a separate SQL path, so it can never drift from `.list`'s query construction. */
void* certo_query_first(CertoQuery* q, int64_t conn, certo_fn_t mapper) {
    CertoQuery* limited = certo_query_limit(q, 1);
    CertoList* rows = certo_query_list(limited, conn, mapper);
    if (!rows || rows->len == 0) return NULL;
    return __certo_opt_box((int64_t)rows->data[0]);
}

/* Grouped/aggregated queries run through the exact same `certo_query_build_sql`/
 * `certo_db_query_typed` path as `.list` — the only reason this is a separate C symbol
 * is that it's a separate Certo-level name (no `DbRow` bound: aggregate results are
 * synthetic shapes, not schema tables). */
CertoList* certo_query_grouped_list(CertoQuery* q, int64_t conn, certo_fn_t mapper) {
    return certo_query_list(q, conn, mapper);
}

int64_t certo_query_count(CertoQuery* q, int64_t conn) {
    size_t cap = certo_query_sql_capacity(q);
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "SELECT COUNT(*) FROM %s AS %s", q->table, q->alias);
    int64_t njoins = q->joins ? q->joins->len : 0;
    for (int64_t i = 0; i < njoins; i++)
        pos += (size_t)snprintf(sql + pos, cap - pos, " %s", (const char*)q->joins->data[i]);
    certo_query_where_clause(q, sql, cap, pos);

    CertoList* rows = certo_db_query(conn, sql, q->params);
    free(sql);
    if (!rows || rows->len == 0) return 0;
    CertoList* row0 = (CertoList*)rows->data[0];
    if (!row0 || row0->len == 0 || !row0->data[0]) return 0;
    certo_text_t text = (certo_text_t)(*(int64_t*)row0->data[0]);
    return atoll(text);
}

/* Shared by sum/avg/min/max: "SELECT AGG(col) FROM t [JOIN ...] [WHERE ...]" — a single
 * scalar cell. Returns NULL (Certo None) when there are no matching rows or the aggregate
 * itself is SQL NULL (e.g. SUM/AVG/MIN/MAX over zero rows). Caller must strdup if it needs
 * the string past the next call. */
static certo_text_t certo_query_scalar_agg(CertoQuery* q, int64_t conn, certo_text_t agg_fn, certo_text_t column) {
    char* expr = certo_query_agg_expr(agg_fn, column);
    size_t cap = certo_query_sql_capacity(q) + 64;
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "SELECT %s FROM %s AS %s", expr, q->table, q->alias);
    free(expr);
    int64_t njoins = q->joins ? q->joins->len : 0;
    for (int64_t i = 0; i < njoins; i++)
        pos += (size_t)snprintf(sql + pos, cap - pos, " %s", (const char*)q->joins->data[i]);
    certo_query_where_clause(q, sql, cap, pos);

    CertoList* rows = certo_db_query(conn, sql, q->params);
    free(sql);
    if (!rows || rows->len == 0) return NULL;
    CertoList* row0 = (CertoList*)rows->data[0];
    if (!row0 || row0->len == 0 || !row0->data[0]) return NULL;
    return (certo_text_t)(*(int64_t*)row0->data[0]);
}

void* certo_query_sum(CertoQuery* q, certo_text_t column, int64_t conn) {
    certo_text_t r = certo_query_scalar_agg(q, conn, "sum", column);
    return r ? __certo_opt_box((int64_t)certo_db_strdup(r)) : NULL;
}
void* certo_query_avg(CertoQuery* q, certo_text_t column, int64_t conn) {
    certo_text_t r = certo_query_scalar_agg(q, conn, "avg", column);
    return r ? __certo_opt_box((int64_t)certo_db_strdup(r)) : NULL;
}
void* certo_query_min(CertoQuery* q, certo_text_t column, int64_t conn) {
    certo_text_t r = certo_query_scalar_agg(q, conn, "min", column);
    return r ? __certo_opt_box((int64_t)certo_db_strdup(r)) : NULL;
}
void* certo_query_max(CertoQuery* q, certo_text_t column, int64_t conn) {
    certo_text_t r = certo_query_scalar_agg(q, conn, "max", column);
    return r ? __certo_opt_box((int64_t)certo_db_strdup(r)) : NULL;
}
"#;
