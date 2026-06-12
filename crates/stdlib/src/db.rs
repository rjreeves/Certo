/// PostgreSQL connectivity stdlib — wraps libpq.
///
/// Compiled into the generated C file when the program uses Stdlib.Db.
/// On Windows link with `-lpq` (from the PostgreSQL install).
/// On Linux/macOS link with `-lpq` from the system package.

pub const DB_C: &str = r#"
/* ------------------------------------------------------------------ */
/* Stdlib.Db — PostgreSQL via libpq                                    */
/* ------------------------------------------------------------------ */
#include <libpq-fe.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

/* ---- internal helpers -------------------------------------------- */

static char* certo_db_strdup(const char* s) {
    if (!s) { char *e = malloc(1); e[0]='\0'; return e; }
    size_t n = strlen(s);
    char *out = malloc(n + 1);
    memcpy(out, s, n + 1);
    return out;
}

/* Sentinel for SQL NULL parameters.  dbNull() returns a pointer to this
 * buffer.  certo_db_params detects it by pointer identity and passes NULL
 * to PQexecParams, which libpq treats as SQL NULL. */
static const char certo_db_null_sentinel_str[] = "\001CERTO_DB_NULL\001";

certo_text_t certo_db_null_param(void) {
    return (certo_text_t)certo_db_null_sentinel_str;
}

/* Build a const char** array from a CertoList* of certo_text_t.
 * Elements that are the null sentinel are mapped to NULL (→ SQL NULL).
 * Returns NULL and sets *nparams=0 if list is NULL. */
static const char** certo_db_params(CertoList* params, int* nparams) {
    if (!params || params->len == 0) { *nparams = 0; return NULL; }
    *nparams = (int)params->len;
    const char** arr = (const char**)malloc((size_t)*nparams * sizeof(char*));
    for (int i = 0; i < *nparams; i++) {
        const char* v = (const char*)params->data[i];
        arr[i] = (v == certo_db_null_sentinel_str) ? NULL : v;
    }
    return arr;
}

/* ---- connect / close / error ------------------------------------- */

/* Last connection-error message — set when certo_db_connect fails. */
static char* certo_db_last_connect_error = NULL;

/* Convert a postgres://user:pass@host:port/dbname URI to libpq keyword=value
 * format so older libpq versions (pre-9.2) that don't accept URIs still work.
 * Returns a newly malloc'd string, or NULL if the input is not a URI. */
static char* certo_db_uri_to_kv(const char* uri) {
    const char* pfx1 = "postgres://";
    const char* pfx2 = "postgresql://";
    const char* rest = NULL;
    if (strncmp(uri, pfx2, strlen(pfx2)) == 0) rest = uri + strlen(pfx2);
    else if (strncmp(uri, pfx1, strlen(pfx1)) == 0) rest = uri + strlen(pfx1);
    else return NULL;

    /* rest = [user[:pass]@]host[:port][/dbname] */
    char user[256]="", pass[256]="", host[256]="localhost", port[16]="5432", dbname[256]="";

    const char* at = strchr(rest, '@');
    if (at) {
        /* parse user[:pass] */
        size_t ulen = (size_t)(at - rest);
        char cred[512]; if (ulen >= sizeof(cred)) ulen = sizeof(cred)-1;
        memcpy(cred, rest, ulen); cred[ulen] = '\0';
        const char* colon = strchr(cred, ':');
        if (colon) {
            size_t ul = (size_t)(colon - cred);
            if (ul >= sizeof(user)) ul = sizeof(user)-1;
            memcpy(user, cred, ul); user[ul] = '\0';
            strncpy(pass, colon+1, sizeof(pass)-1);
        } else {
            strncpy(user, cred, sizeof(user)-1);
        }
        rest = at + 1;
    }

    /* rest = host[:port][/dbname] */
    const char* slash = strchr(rest, '/');
    if (slash) {
        strncpy(dbname, slash+1, sizeof(dbname)-1);
        /* strip query string from dbname */
        char* q = strchr(dbname, '?'); if (q) *q = '\0';
    }
    size_t hplen = slash ? (size_t)(slash - rest) : strlen(rest);
    char hostport[512]; if (hplen >= sizeof(hostport)) hplen = sizeof(hostport)-1;
    memcpy(hostport, rest, hplen); hostport[hplen] = '\0';

    /* host may be [ipv6] */
    if (hostport[0] == '[') {
        char* rb = strchr(hostport, ']');
        if (rb) {
            size_t hl = (size_t)(rb - hostport - 1);
            if (hl >= sizeof(host)) hl = sizeof(host)-1;
            memcpy(host, hostport+1, hl); host[hl] = '\0';
            if (*(rb+1) == ':') strncpy(port, rb+2, sizeof(port)-1);
        }
    } else {
        const char* col = strchr(hostport, ':');
        if (col) {
            size_t hl = (size_t)(col - hostport);
            if (hl >= sizeof(host)) hl = sizeof(host)-1;
            memcpy(host, hostport, hl); host[hl] = '\0';
            strncpy(port, col+1, sizeof(port)-1);
        } else {
            strncpy(host, hostport, sizeof(host)-1);
        }
    }

    /* Build keyword=value string */
    char* out = (char*)malloc(1024);
    int n = 0;
    n += snprintf(out+n, 1024-n, "host=%s port=%s", host, port);
    if (dbname[0]) n += snprintf(out+n, 1024-n, " dbname=%s", dbname);
    if (user[0])   n += snprintf(out+n, 1024-n, " user=%s", user);
    if (pass[0])   n += snprintf(out+n, 1024-n, " password=%s", pass);
    return out;
}

int64_t certo_db_connect(certo_text_t connstr) {
    char* kv = certo_db_uri_to_kv(connstr);
    PGconn *conn = PQconnectdb(kv ? kv : connstr);
    free(kv);
    if (PQstatus(conn) != CONNECTION_OK) {
        free(certo_db_last_connect_error);
        certo_db_last_connect_error = certo_db_strdup(PQerrorMessage(conn));
        PQfinish(conn);
        return 0;
    }
    return (int64_t)(uintptr_t)conn;
}

int64_t certo_db_close(int64_t handle) {
    if (handle != 0) PQfinish((PGconn *)(uintptr_t)handle);
    return 0;
}

certo_text_t certo_db_error(int64_t handle) {
    if (handle == 0) {
        return certo_db_last_connect_error ? certo_db_last_connect_error : "connection failed";
    }
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    return certo_db_strdup(PQerrorMessage(conn));
}

/* ---- server info -------------------------------------------------- */

int64_t certo_db_server_version(int64_t handle) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    return (int64_t)PQserverVersion(conn);
}

certo_text_t certo_db_version_string(int64_t handle) {
    if (handle == 0) return certo_db_strdup("");
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, "SELECT version()");
    certo_text_t out;
    if (PQresultStatus(res) != PGRES_TUPLES_OK) {
        out = certo_db_strdup("");
    } else {
        out = certo_db_strdup(PQgetvalue(res, 0, 0));
    }
    PQclear(res);
    return out;
}

/* ---- exec (INSERT / UPDATE / DELETE) ----------------------------- */

/* Execute a statement with $1..$N parameters.
 * Returns the number of rows affected, or -1 on error. */
int64_t certo_db_exec(int64_t handle, certo_text_t sql, CertoList* params) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    int nparams;
    const char **pv = certo_db_params(params, &nparams);
    PGresult *res = PQexecParams(conn, sql, nparams, NULL, pv, NULL, NULL, 0);
    free(pv);
    int64_t affected = -1;
    ExecStatusType st = PQresultStatus(res);
    if (st == PGRES_COMMAND_OK || st == PGRES_TUPLES_OK) {
        const char *rows = PQcmdTuples(res);
        affected = (rows && rows[0]) ? (int64_t)atoll(rows) : 0;
    }
    PQclear(res);
    return affected;
}

/* ---- query (SELECT → List<List<Text>>) --------------------------- */

/* Run a SELECT with $1..$N parameters.
 * Returns a List<List<Text?>> — outer list is rows, inner is columns.
 * SQL NULL cells are stored as NULL pointers (Certo None); non-null cells
 * are heap-copied strings (Certo Some(text)).
 * Returns an empty list on error. */
CertoList* certo_db_query(int64_t handle, certo_text_t sql, CertoList* params) {
    CertoList* result = certo_list_new_empty();
    if (handle == 0) return result;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    int nparams;
    const char **pv = certo_db_params(params, &nparams);
    PGresult *res = PQexecParams(conn, sql, nparams, NULL, pv, NULL, NULL, 0);
    free(pv);
    if (PQresultStatus(res) != PGRES_TUPLES_OK) {
        PQclear(res);
        return result;
    }
    int nrows = PQntuples(res);
    int ncols = PQnfields(res);
    for (int r = 0; r < nrows; r++) {
        CertoList* row = certo_list_new_empty();
        for (int c = 0; c < ncols; c++) {
            /* NULL pointer = Certo None; heap string = Certo Some(text) */
            void *cell = PQgetisnull(res, r, c)
                ? NULL
                : (void*)certo_db_strdup(PQgetvalue(res, r, c));
            row = certo_list_push(row, cell);
        }
        result = certo_list_push(result, (void*)row);
    }
    PQclear(res);
    return result;
}

/* Convenience: return the first row, or NULL (None) if no rows. */
CertoList* certo_db_query_row(int64_t handle, certo_text_t sql, CertoList* params) {
    CertoList* rows = certo_db_query(handle, sql, params);
    if (!rows || rows->len == 0) return NULL;
    return (CertoList*)rows->data[0];
}

/* Convenience: return the first column of the first row as Text?, or None.
 * Returns NULL (None) when there are no rows or the cell is SQL NULL.
 * Returns a heap-copied string pointer (Some(text)) otherwise. */
void* certo_db_query_one(int64_t handle, certo_text_t sql) {
    if (handle == 0) return NULL;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, sql);
    void *out;
    if (PQresultStatus(res) != PGRES_TUPLES_OK || PQntuples(res) == 0
            || PQgetisnull(res, 0, 0)) {
        out = NULL;
    } else {
        out = (void*)certo_db_strdup(PQgetvalue(res, 0, 0));
    }
    PQclear(res);
    return out;
}

/* ---- column names ------------------------------------------------- */

/* Return the column names for a query as List<Text>. */
CertoList* certo_db_columns(int64_t handle, certo_text_t sql) {
    CertoList* result = certo_list_new_empty();
    if (handle == 0) return result;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, sql);
    if (PQresultStatus(res) != PGRES_TUPLES_OK) {
        PQclear(res);
        return result;
    }
    int ncols = PQnfields(res);
    for (int c = 0; c < ncols; c++) {
        result = certo_list_push(result, (void*)certo_db_strdup(PQfname(res, c)));
    }
    PQclear(res);
    return result;
}

/* ---- typed query -------------------------------------------------- */

/* Run a SELECT and map each row through a Certo function.
 * mapper :: List<Text> -> T  (CertoFn1 convention: void* -> void*)
 * Returns List<T>. */
typedef void* (*CertoFn1)(void*);

CertoList* certo_db_query_typed(int64_t handle, certo_text_t sql,
                                CertoList* params, CertoFn1 mapper) {
    CertoList* rows = certo_db_query(handle, sql, params);
    if (!mapper) return rows;
    CertoList* result = certo_list_new_empty();
    for (size_t i = 0; i < rows->len; i++) {
        void* mapped = mapper(rows->data[i]);
        result = certo_list_push(result, mapped);
    }
    return result;
}

/* ---- transactions ------------------------------------------------- */

int64_t certo_db_begin(int64_t handle) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, "BEGIN");
    int64_t ok = PQresultStatus(res) == PGRES_COMMAND_OK ? 1 : 0;
    PQclear(res);
    return ok;
}

int64_t certo_db_commit(int64_t handle) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, "COMMIT");
    int64_t ok = PQresultStatus(res) == PGRES_COMMAND_OK ? 1 : 0;
    PQclear(res);
    return ok;
}

int64_t certo_db_rollback(int64_t handle) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, "ROLLBACK");
    int64_t ok = PQresultStatus(res) == PGRES_COMMAND_OK ? 1 : 0;
    PQclear(res);
    return ok;
}

/* ---- withTransaction ---------------------------------------------- */

/* Monotonically increasing counter for savepoint names.
 * Not thread-safe — acceptable while Certo's async runtime is single-threaded. */
static int64_t certo_svp_seq = 0;

/* Execute a zero-arg thunk inside a transaction on the given connection.
 *
 * Outer call  (no active txn):  issues BEGIN / COMMIT or ROLLBACK.
 * Nested call (already in txn): issues SAVEPOINT / RELEASE or
 *                               ROLLBACK TO SAVEPOINT + RELEASE, leaving
 *                               the outer transaction intact on failure.
 *
 * Returns the Result<T,E> from the thunk unchanged in both cases, so ?
 * propagation in Certo code works naturally at any nesting depth.
 *
 * certo_fn_t is void(*)(void) but the compiled thunk returns void*
 * (a certo_result_t*).  The cast is safe on every ABI Certo targets
 * because void* and void share the same return register (rax / r0). */
void* certo_with_transaction(int64_t handle, certo_fn_t thunk) {
    if (handle == 0 || !thunk) {
        return certo_err((intptr_t)(certo_text_t)"withTransaction: invalid connection");
    }
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    typedef void* (*thunk_t)(void);

    PGTransactionStatusType txstatus = PQtransactionStatus(conn);

    if (txstatus == PQTRANS_INERROR) {
        return certo_err((intptr_t)(certo_text_t)
            "withTransaction: connection is in error state — call dbRollback first");
    }
    if (txstatus == PQTRANS_UNKNOWN) {
        return certo_err((intptr_t)(certo_text_t)
            "withTransaction: connection is invalid");
    }

    if (txstatus == PQTRANS_INTRANS) {
        /* ---- nested: use a savepoint --------------------------------- */
        char svp_name[40];
        snprintf(svp_name, sizeof(svp_name), "certo_svp_%" PRId64, ++certo_svp_seq);

        char sql[80];
        snprintf(sql, sizeof(sql), "SAVEPOINT %s", svp_name);
        PGresult *svp = PQexec(conn, sql);
        if (PQresultStatus(svp) != PGRES_COMMAND_OK) {
            char *msg = certo_db_strdup(PQerrorMessage(conn));
            PQclear(svp);
            return certo_err((intptr_t)(certo_text_t)msg);
        }
        PQclear(svp);

        void *result = ((thunk_t)thunk)();

        if (__result_is_ok(result)) {
            snprintf(sql, sizeof(sql), "RELEASE SAVEPOINT %s", svp_name);
            PGresult *rel = PQexec(conn, sql);
            PQclear(rel);
        } else {
            /* Roll back to the savepoint, then release it to free server resources. */
            snprintf(sql, sizeof(sql), "ROLLBACK TO SAVEPOINT %s", svp_name);
            PGresult *rb = PQexec(conn, sql);
            PQclear(rb);
            snprintf(sql, sizeof(sql), "RELEASE SAVEPOINT %s", svp_name);
            PGresult *rel = PQexec(conn, sql);
            PQclear(rel);
        }

        return result;
    } else {
        /* ---- outer: use BEGIN / COMMIT / ROLLBACK -------------------- */
        PGresult *begin = PQexec(conn, "BEGIN");
        if (PQresultStatus(begin) != PGRES_COMMAND_OK) {
            char *msg = certo_db_strdup(PQerrorMessage(conn));
            PQclear(begin);
            return certo_err((intptr_t)(certo_text_t)msg);
        }
        PQclear(begin);

        void *result = ((thunk_t)thunk)();

        if (__result_is_ok(result)) {
            PGresult *res = PQexec(conn, "COMMIT");
            PQclear(res);
        } else {
            PGresult *res = PQexec(conn, "ROLLBACK");
            PQclear(res);
        }

        return result;
    }
}

/* ---- withConnection ----------------------------------------------- */

/* Open a connection, call body(conn), then close unconditionally.
 *
 * On connect failure returns Err with the libpq error message.
 * On success or failure of the body, the connection is always closed
 * before returning — no leak is possible.
 *
 * body is fn(Int): Result<T,E>.  certo_fn_t is void(*)(void) but the
 * compiled lambda actually takes int64_t and returns void*.  The cast
 * is safe: the argument goes in the first integer register (rdi / r0)
 * and the void* result comes back in rax / r0 regardless of declared
 * return type. */
void* certo_with_connection(certo_text_t connstr, certo_fn_t body) {
    if (!body) {
        return certo_err((intptr_t)(certo_text_t)
            "withConnection: invalid body");
    }

    int64_t handle = certo_db_connect(connstr);
    if (handle == 0) {
        const char *raw = certo_db_last_connect_error
            ? certo_db_last_connect_error : "withConnection: connection failed";
        return certo_err((intptr_t)(certo_text_t)certo_db_strdup(raw));
    }

    typedef void* (*body_t)(int64_t);
    void *result = ((body_t)body)(handle);

    certo_db_close(handle);
    return result;
}
"#;

/// Certo source declarations for Stdlib.Db.
pub const DB_CERTO: &str = r#"
// ── DbRow marker trait ──────────────────────────────────────────────────
// Types generated by `certo db pull` automatically implement this trait.
// It is the bound on `dbQueryTyped<T: DbRow>` — only schema-derived types
// can be used as the typed result, catching mapper/schema mismatches at
// compile time.
trait DbRow {}

module Stdlib.Db

// ── Connection ──────────────────────────────────────────────────────

/// Open a connection using a libpq connection string.
/// Returns a connection handle; 0 means the connection failed.
/// Check dbError(conn) for the reason.
fn dbConnect(connstr: Text): Int [io]

/// Close a connection and free its resources.
fn dbClose(conn: Int): Unit [io]

/// Return the last error message on a connection handle.
fn dbError(conn: Int): Text

// ── Server info ─────────────────────────────────────────────────────

/// Server version as an integer: major×10000 + minor×100 + patch.
/// PostgreSQL 15.4 → 150004. Returns -1 on a failed connection.
fn dbServerVersion(conn: Int): Int [io]

/// Full version banner from SELECT version().
fn dbVersionString(conn: Int): Text [io]

// ── Null sentinel ────────────────────────────────────────────────────

/// Return a sentinel value that dbExec treats as SQL NULL for that parameter.
/// Use this when a nullable field is null in an insert or update:
///
///   dbExec(conn, "INSERT INTO t (name, note) VALUES ($1, $2)",
///          [record.name,
///           if record.note == null then dbNull() else record.note ?? ""])
fn dbNull(): Text

// ── Exec (INSERT / UPDATE / DELETE) ─────────────────────────────────

/// Execute a statement with positional parameters ($1, $2, …).
/// Returns the number of rows affected, or -1 on error.
///
/// Example:
///   dbExec(conn, "INSERT INTO users(name, email) VALUES ($1, $2)",
///          ["Alice", "alice@example.com"])
fn dbExec(conn: Int, sql: Text, params: List<Text>): Int [io]

// ── Query (SELECT) ───────────────────────────────────────────────────

/// Run a SELECT with positional parameters.
/// Returns all rows as List<List<Text?>> — outer list is rows, inner is columns.
/// SQL NULL cells are None; non-null cells are Some(text).
/// Use `?? ""` (or any default) to coerce a nullable cell to a plain Text.
///
/// Example:
///   val rows = dbQuery(conn, "SELECT id, name FROM users WHERE age > $1", ["18"])
///   for row in rows {
///       val id   = List.getOrPanic(row, 0) ?? ""
///       val name = List.getOrPanic(row, 1) ?? "(unknown)"
///       println(id ++ " " ++ name)
///   }
fn dbQuery(conn: Int, sql: Text, params: List<Text>): List<List<Text?>> [io]

/// Run a SELECT, apply `mapper` to every row, and return a typed list.
/// This is the preferred API when the schema is known — the return type is
/// inferred from the mapper, so the compiler verifies the result type.
/// Each row is List<Text?> so the mapper can handle nullable columns explicitly.
///
/// Example:
///   val users = dbQueryTyped(conn,
///       "SELECT id, name, email FROM users WHERE active = $1",
///       ["true"], usersFromRow)
///   // users : List<Users>  — verified by the compiler
fn dbQueryTyped<T: DbRow>(conn: Int, sql: Text, params: List<Text>, mapper: fn(List<Text?>): T): List<T> [io]

/// Run a SELECT and return only the first row as List<Text?>?, or None if no rows.
fn dbQueryRow(conn: Int, sql: Text, params: List<Text>): List<Text?>? [io]

/// Run a no-parameter SELECT and return the first column of the first row as Text?,
/// or None when there are no rows or the cell is SQL NULL.
/// Useful for scalar queries like COUNT or server metadata.
fn dbQueryOne(conn: Int, sql: Text): Text? [io]

/// Return the column names for a query as List<Text>.
fn dbColumns(conn: Int, sql: Text): List<Text> [io]

// ── Transactions ────────────────────────────────────────────────────

/// Begin a transaction. Returns 1 on success, 0 on failure.
fn dbBegin(conn: Int): Int [io]

/// Commit the current transaction. Returns 1 on success, 0 on failure.
fn dbCommit(conn: Int): Int [io]

/// Roll back the current transaction. Returns 1 on success, 0 on failure.
fn dbRollback(conn: Int): Int [io]

// ── withTransaction ─────────────────────────────────────────────────

/// Execute `body` inside a transaction on `conn`.
///
/// Commits if `body` returns `Ok(_)`, rolls back if it returns `Err(_)`.
/// The `Result` from `body` is returned unchanged, so `?` propagation
/// works naturally inside the block.
///
/// Example — atomic transfer between accounts:
///
///   withTransaction(conn) {
///       dbExec(conn, "UPDATE accounts SET balance = balance - $1 WHERE id = $2",
///              [amountStr, fromId])?
///       dbExec(conn, "UPDATE accounts SET balance = balance + $1 WHERE id = $2",
///              [amountStr, toId])?
///       Ok(Unit)
///   }
fn withTransaction(conn: Int, body: fn(): Result<T, E>): Result<T, E> [io]

/// Open a connection to `url`, run `body(conn)`, then close the connection —
/// whether `body` succeeds or fails.  The connection is never leaked.
///
/// Returns `Err` immediately if the connection cannot be established.
/// Otherwise returns the `Result` produced by `body` unchanged.
///
/// Pair with `withTransaction` for full resource safety:
///
///   withConnection(DATABASE_URL) { conn =>
///       withTransaction(conn) {
///           dbExec(conn, "INSERT INTO orders (total) VALUES ($1)", [total])?
///           Ok(Unit)
///       }
///   }
fn withConnection(url: Text, body: fn(Int): Result<T, E>): Result<T, E> [io]
"#;
