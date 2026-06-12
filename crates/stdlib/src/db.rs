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
 * Returns a List<List<Text>> — outer list is rows, inner is columns.
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
            char *cell = certo_db_strdup(
                PQgetisnull(res, r, c) ? "" : PQgetvalue(res, r, c));
            row = certo_list_push(row, (void*)cell);
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

/* Convenience: return the first column of the first row as Text, or "". */
certo_text_t certo_db_query_one(int64_t handle, certo_text_t sql) {
    if (handle == 0) return certo_db_strdup("");
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, sql);
    certo_text_t out;
    if (PQresultStatus(res) != PGRES_TUPLES_OK || PQntuples(res) == 0) {
        out = certo_db_strdup("");
    } else {
        out = certo_db_strdup(PQgetvalue(res, 0, 0));
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
"#;

/// Certo source declarations for Stdlib.Db.
pub const DB_CERTO: &str = r#"
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
/// Returns all rows as List<List<Text>> — outer list is rows, inner is columns.
/// NULL cells are returned as empty strings.
///
/// Example:
///   val rows = dbQuery(conn, "SELECT id, name FROM users WHERE age > $1", ["18"])
///   for row in rows {
///       println(List.getOrPanic(row, 0) ++ " " ++ List.getOrPanic(row, 1))
///   }
fn dbQuery(conn: Int, sql: Text, params: List<Text>): List<List<Text>> [io]

/// Run a SELECT, apply `mapper` to every row, and return a typed list.
/// This is the preferred API when the schema is known — the return type is
/// inferred from the mapper, so the compiler verifies the result type.
///
/// Example:
///   val users = dbQueryTyped(conn,
///       "SELECT id, name, email FROM users WHERE active = $1",
///       ["true"], usersFromRow)
///   // users : List<Users>  — verified by the compiler
fn dbQueryTyped(conn: Int, sql: Text, params: List<Text>, mapper: fn(List<Text>): T): List<T> [io]

/// Run a SELECT and return only the first row as List<Text>, or None.
fn dbQueryRow(conn: Int, sql: Text, params: List<Text>): List<Text>? [io]

/// Run a no-parameter SELECT and return the first column of the first row.
/// Useful for scalar queries like COUNT or server metadata.
fn dbQueryOne(conn: Int, sql: Text): Text [io]

/// Return the column names for a query as List<Text>.
fn dbColumns(conn: Int, sql: Text): List<Text> [io]

// ── Transactions ────────────────────────────────────────────────────

/// Begin a transaction. Returns 1 on success, 0 on failure.
fn dbBegin(conn: Int): Int [io]

/// Commit the current transaction. Returns 1 on success, 0 on failure.
fn dbCommit(conn: Int): Int [io]

/// Roll back the current transaction. Returns 1 on success, 0 on failure.
fn dbRollback(conn: Int): Int [io]
"#;
