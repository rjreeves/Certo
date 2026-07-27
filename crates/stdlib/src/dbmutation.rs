/// Mutation builder — a fluent, schema-checked layer over `dbExec` for INSERT/UPDATE/
/// DELETE/UPSERT, mirroring `Query` (see `crates/stdlib/src/dbquery.rs`).
///
/// `Mutation.insertInto`/`.updateTable`/`.deleteFrom`/`.insertMany` start a mutation of a
/// given kind; `.set`/`.filter`/`.onConflict`/`.addRow` build it up; `.run` executes it.
/// Table names and every `.set`/`.filter`/`.onConflict` column, `.filter`'s operator, and
/// `.insertMany`'s column list must be string literals — `certo_dbschema::check_mutation`
/// validates them against the schema at compile time (E0519-E0521, plus reused E0509-E0511
/// from the query checker). Values are never literal-checked; they always travel as bound
/// `$N` parameters via the existing `dbExec`, so there is no SQL-injection surface here
/// regardless of where they come from.
///
/// Each builder call site is only valid for the mutation kind it makes sense on — `.set`
/// on insert/update, `.filter` on update/delete, `.onConflict` on insert, `.addRow` on
/// insertMany — enforced at compile time (E0520), not left to fail at the database.
pub const DBMUTATION_C: &str = r#"
/* ------------------------------------------------------------------ */
/* Stdlib.DbMutation — fluent Mutation builder over Stdlib.Db          */
/* ------------------------------------------------------------------ */

typedef enum {
    CERTO_MUT_INSERT,
    CERTO_MUT_INSERT_MANY,
    CERTO_MUT_UPDATE,
    CERTO_MUT_DELETE
} CertoMutationKind;

typedef struct {
    CertoMutationKind kind;
    certo_text_t      table;
    CertoList*        set_cols;      /* List<Text> — for insert/update .set() */
    CertoList*        set_vals;      /* List<Text>, parallel to set_cols */
    CertoList*        where_cols;    /* List<Text> — for update/delete .filter() */
    CertoList*        where_ops;     /* List<Text>, parallel to where_cols */
    CertoList*        where_vals;    /* List<Text>, parallel to where_cols */
    certo_text_t      conflict_col;  /* NULL = no upsert; else ON CONFLICT (col) */
    CertoList*        many_cols;     /* List<Text> — column names, for insertMany */
    CertoList*        many_rows;     /* List<List<Text>> — one element per .addRow() call */
} CertoMutation;

static CertoMutation* certo_mutation_new(certo_text_t table, CertoMutationKind kind) {
    CertoMutation* m = (CertoMutation*)malloc(sizeof(CertoMutation));
    if (!m) certo_panic("out of memory");
    m->kind         = kind;
    m->table        = certo_db_strdup(table);
    m->set_cols     = certo_list_new_empty();
    m->set_vals     = certo_list_new_empty();
    m->where_cols   = certo_list_new_empty();
    m->where_ops    = certo_list_new_empty();
    m->where_vals   = certo_list_new_empty();
    m->conflict_col = NULL;
    m->many_cols    = certo_list_new_empty();
    m->many_rows    = certo_list_new_empty();
    return m;
}

CertoMutation* certo_mutation_insert_into(certo_text_t table) {
    return certo_mutation_new(table, CERTO_MUT_INSERT);
}
CertoMutation* certo_mutation_update_table(certo_text_t table) {
    return certo_mutation_new(table, CERTO_MUT_UPDATE);
}
CertoMutation* certo_mutation_delete_from(certo_text_t table) {
    return certo_mutation_new(table, CERTO_MUT_DELETE);
}
CertoMutation* certo_mutation_insert_many(certo_text_t table, CertoList* columns) {
    CertoMutation* m = certo_mutation_new(table, CERTO_MUT_INSERT_MANY);
    m->many_cols = columns;
    return m;
}

/* Builder calls are functional (like certo_list_push / certo_query_*): each returns a new
 * CertoMutation, leaving the original untouched. */
static CertoMutation* certo_mutation_clone(CertoMutation* m) {
    CertoMutation* n = (CertoMutation*)malloc(sizeof(CertoMutation));
    if (!n) certo_panic("out of memory");
    *n = *m;
    return n;
}

CertoMutation* certo_mutation_set(CertoMutation* m, certo_text_t column, certo_text_t value) {
    CertoMutation* n = certo_mutation_clone(m);
    n->set_cols = certo_list_push(m->set_cols, (void*)certo_db_strdup(column));
    n->set_vals = certo_list_push(m->set_vals, (void*)certo_db_strdup(value));
    return n;
}

CertoMutation* certo_mutation_filter(CertoMutation* m, certo_text_t column, certo_text_t op, certo_text_t value) {
    CertoMutation* n = certo_mutation_clone(m);
    n->where_cols = certo_list_push(m->where_cols, (void*)certo_db_strdup(column));
    n->where_ops  = certo_list_push(m->where_ops,  (void*)certo_db_strdup(op));
    n->where_vals = certo_list_push(m->where_vals, (void*)certo_db_strdup(value));
    return n;
}

CertoMutation* certo_mutation_on_conflict(CertoMutation* m, certo_text_t column) {
    CertoMutation* n = certo_mutation_clone(m);
    n->conflict_col = certo_db_strdup(column);
    return n;
}

CertoMutation* certo_mutation_add_row(CertoMutation* m, CertoList* values) {
    CertoMutation* n = certo_mutation_clone(m);
    n->many_rows = certo_list_push(m->many_rows, (void*)values);
    return n;
}

static int64_t certo_mutation_run_insert(CertoMutation* m, int64_t conn) {
    int64_t ncols = m->set_cols ? m->set_cols->len : 0;
    size_t cap = 128;
    for (int64_t i = 0; i < ncols; i++) cap += strlen((const char*)m->set_cols->data[i]) + 16;
    if (m->conflict_col) cap += strlen(m->conflict_col) * 2 + ncols * 32 + 64;
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "INSERT INTO %s (", m->table);
    for (int64_t i = 0; i < ncols; i++)
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s", i == 0 ? "" : ", ", (const char*)m->set_cols->data[i]);
    pos += (size_t)snprintf(sql + pos, cap - pos, ") VALUES (");

    CertoList* params = certo_list_new_empty();
    for (int64_t i = 0; i < ncols; i++) {
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s$%" PRId64, i == 0 ? "" : ", ", i + 1);
        params = certo_list_push(params, (void*)certo_db_strdup((const char*)m->set_vals->data[i]));
    }
    pos += (size_t)snprintf(sql + pos, cap - pos, ")");

    if (m->conflict_col) {
        pos += (size_t)snprintf(sql + pos, cap - pos, " ON CONFLICT (%s) DO UPDATE SET ", m->conflict_col);
        for (int64_t i = 0; i < ncols; i++) {
            const char* c = (const char*)m->set_cols->data[i];
            pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s = EXCLUDED.%s", i == 0 ? "" : ", ", c, c);
        }
    }

    int64_t affected = certo_db_exec(conn, sql, params);
    free(sql);
    return affected;
}

static int64_t certo_mutation_run_insert_many(CertoMutation* m, int64_t conn) {
    int64_t ncols = m->many_cols ? m->many_cols->len : 0;
    int64_t nrows = m->many_rows ? m->many_rows->len : 0;
    if (nrows == 0) return 0;

    size_t cap = 128;
    for (int64_t i = 0; i < ncols; i++) cap += strlen((const char*)m->many_cols->data[i]) + 8;
    cap += (size_t)(nrows * ncols) * 12 + (size_t)nrows * 4;
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "INSERT INTO %s (", m->table);
    for (int64_t i = 0; i < ncols; i++)
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s", i == 0 ? "" : ", ", (const char*)m->many_cols->data[i]);
    pos += (size_t)snprintf(sql + pos, cap - pos, ") VALUES ");

    CertoList* params = certo_list_new_empty();
    int64_t param_n = 0;
    for (int64_t r = 0; r < nrows; r++) {
        CertoList* row = (CertoList*)m->many_rows->data[r];
        int64_t rowlen = row ? row->len : 0;
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s(", r == 0 ? "" : ", ");
        for (int64_t c = 0; c < rowlen; c++) {
            param_n++;
            pos += (size_t)snprintf(sql + pos, cap - pos, "%s$%" PRId64, c == 0 ? "" : ", ", param_n);
            params = certo_list_push(params, (void*)certo_db_strdup((const char*)row->data[c]));
        }
        pos += (size_t)snprintf(sql + pos, cap - pos, ")");
    }

    int64_t affected = certo_db_exec(conn, sql, params);
    free(sql);
    return affected;
}

static int64_t certo_mutation_run_update(CertoMutation* m, int64_t conn) {
    int64_t nset   = m->set_cols   ? m->set_cols->len   : 0;
    int64_t nwhere = m->where_cols ? m->where_cols->len : 0;
    size_t cap = 128;
    for (int64_t i = 0; i < nset;   i++) cap += strlen((const char*)m->set_cols->data[i])   + 16;
    for (int64_t i = 0; i < nwhere; i++) cap += strlen((const char*)m->where_cols->data[i]) + strlen((const char*)m->where_ops->data[i]) + 16;
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "UPDATE %s SET ", m->table);

    CertoList* params = certo_list_new_empty();
    int64_t param_n = 0;
    for (int64_t i = 0; i < nset; i++) {
        param_n++;
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s%s = $%" PRId64,
                                 i == 0 ? "" : ", ", (const char*)m->set_cols->data[i], param_n);
        params = certo_list_push(params, (void*)certo_db_strdup((const char*)m->set_vals->data[i]));
    }
    for (int64_t i = 0; i < nwhere; i++) {
        param_n++;
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s %s %s $%" PRId64,
                                 i == 0 ? " WHERE" : " AND",
                                 (const char*)m->where_cols->data[i],
                                 (const char*)m->where_ops->data[i],
                                 param_n);
        params = certo_list_push(params, (void*)certo_db_strdup((const char*)m->where_vals->data[i]));
    }

    int64_t affected = certo_db_exec(conn, sql, params);
    free(sql);
    return affected;
}

static int64_t certo_mutation_run_delete(CertoMutation* m, int64_t conn) {
    int64_t nwhere = m->where_cols ? m->where_cols->len : 0;
    size_t cap = 64;
    for (int64_t i = 0; i < nwhere; i++) cap += strlen((const char*)m->where_cols->data[i]) + strlen((const char*)m->where_ops->data[i]) + 16;
    char* sql = (char*)malloc(cap);
    if (!sql) certo_panic("out of memory");
    size_t pos = (size_t)snprintf(sql, cap, "DELETE FROM %s", m->table);

    CertoList* params = certo_list_new_empty();
    int64_t param_n = 0;
    for (int64_t i = 0; i < nwhere; i++) {
        param_n++;
        pos += (size_t)snprintf(sql + pos, cap - pos, "%s %s %s $%" PRId64,
                                 i == 0 ? " WHERE" : " AND",
                                 (const char*)m->where_cols->data[i],
                                 (const char*)m->where_ops->data[i],
                                 param_n);
        params = certo_list_push(params, (void*)certo_db_strdup((const char*)m->where_vals->data[i]));
    }

    int64_t affected = certo_db_exec(conn, sql, params);
    free(sql);
    return affected;
}

int64_t certo_mutation_run(CertoMutation* m, int64_t conn) {
    switch (m->kind) {
        case CERTO_MUT_INSERT:      return certo_mutation_run_insert(m, conn);
        case CERTO_MUT_INSERT_MANY: return certo_mutation_run_insert_many(m, conn);
        case CERTO_MUT_UPDATE:      return certo_mutation_run_update(m, conn);
        case CERTO_MUT_DELETE:      return certo_mutation_run_delete(m, conn);
        default:                    return -1;
    }
}
"#;
