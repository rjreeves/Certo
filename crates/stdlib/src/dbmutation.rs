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

/// Certo source declarations for Stdlib.DbMutation.
///
/// Not compiled by the typechecker (see `certo_stdlib::seed`, which is the actual source
/// of truth for these signatures) — this string exists for documentation/LSP purposes,
/// following the same split every other stdlib module in this crate uses.
pub const DBMUTATION_CERTO: &str = r#"
module Stdlib.DbMutation

// ── Mutation — opaque fluent builder for INSERT/UPDATE/DELETE/UPSERT ─
//
// Table names, and every `.set`/`.filter`/`.onConflict` column, `.filter`'s operator, and
// `.insertMany`'s column list, must be written as string literals — the compiler validates
// them against the schema (E0519-E0521, plus E0509-E0511 shared with `Query`). Values are
// never literal-checked; they always travel as bound parameters. Each method is only valid
// for the mutation kind it makes sense on (E0520) — see the examples below.
//
//   Mutation.insertInto("Orders")
//       |> Mutation.set("status", "pending")
//       |> Mutation.set("total", "19.99")
//       |> Mutation.run(conn)
//
//   Mutation.insertMany("Orders", ["status", "total"])
//       |> Mutation.addRow(["pending", "19.99"])
//       |> Mutation.addRow(["paid", "42.00"])
//       |> Mutation.run(conn)          // single round trip
//
//   Mutation.updateTable("Orders")
//       |> Mutation.set("status", "shipped")
//       |> Mutation.filter("id", "=", orderId)
//       |> Mutation.run(conn)
//
//   Mutation.insertInto("Prices")               // upsert
//       |> Mutation.set("productId", pid)
//       |> Mutation.set("amount", "9.99")
//       |> Mutation.onConflict("productId")
//       |> Mutation.run(conn)
//
//   Mutation.deleteFrom("Sessions")
//       |> Mutation.filter("expiresAt", "<", now)
//       |> Mutation.run(conn)

/// Start an `INSERT INTO table`. `table` must be a literal naming a declared `type` (E0519).
fn Mutation.insertInto(table: Text): Mutation

/// Start an `UPDATE table`. `table` must be a literal naming a declared `type` (E0519).
fn Mutation.updateTable(table: Text): Mutation

/// Start a `DELETE FROM table`. `table` must be a literal naming a declared `type` (E0519).
fn Mutation.deleteFrom(table: Text): Mutation

/// Start a multi-row `INSERT INTO table (columns...) VALUES (...), (...), ...` — add rows
/// with `.addRow`, then `.run` executes the whole batch in one round trip. `table` must be
/// a literal naming a declared `type` (E0519); `columns` must be a literal list of literal
/// column names on that table (E0509).
fn Mutation.insertMany(table: Text, columns: List<Text>): Mutation

/// Set `column = value` (INSERT/UPDATE only — E0520 on `insertMany`/`delete`). Repeatable.
/// `column` must be a literal column name on the mutation's table (E0509).
fn Mutation.set(m: Mutation, column: Text, value: Text): Mutation

/// Add a `column op value` condition (UPDATE/DELETE only — E0520 on `insert`/`insertMany`).
/// AND-combined with any other filters. `column` must be a literal column name (E0509);
/// `op` must be one of `"="`, `"!="`, `"<"`, `"<="`, `">"`, `">="`, `"like"` (E0511).
fn Mutation.filter(m: Mutation, column: Text, op: Text, value: Text): Mutation

/// Turn an insert into an upsert: `ON CONFLICT (column) DO UPDATE SET ...` for every column
/// already given to `.set` (INSERT only — E0520 otherwise). `column` must be a literal
/// column name on the mutation's table (E0509).
fn Mutation.onConflict(m: Mutation, column: Text): Mutation

/// Add one row of values to an `insertMany` batch (insertMany only — E0520 otherwise).
/// `values` should have the same length as the `columns` given to `.insertMany` — checked
/// at compile time when both are literal lists (E0521).
fn Mutation.addRow(m: Mutation, values: List<Text>): Mutation

/// Execute the mutation. Returns the number of rows affected (or inserted, for
/// `insert`/`insertMany`), or `-1` on error — check `dbError(conn)` for the reason.
fn Mutation.run(m: Mutation, conn: Int): Int [io]
"#;
