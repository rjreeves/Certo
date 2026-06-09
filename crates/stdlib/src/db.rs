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

/* Connect to a PostgreSQL server.
 * Returns an opaque connection handle (cast to int64_t).
 * Returns 0 on failure. */
int64_t certo_db_connect(certo_text_t connstr) {
    PGconn *conn = PQconnectdb(connstr);
    if (PQstatus(conn) != CONNECTION_OK) {
        PQfinish(conn);
        return 0;
    }
    return (int64_t)(uintptr_t)conn;
}

/* Return the server version number (e.g. 150004 for 15.4).
 * Returns -1 if conn is 0 (failed connection). */
int64_t certo_db_server_version(int64_t handle) {
    if (handle == 0) return -1;
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    return (int64_t)PQserverVersion(conn);
}

/* Return the full server version string from SELECT version().
 * Returns an empty string on failure. */
certo_text_t certo_db_version_string(int64_t handle) {
    if (handle == 0) { char *e = malloc(1); e[0]='\0'; return e; }
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, "SELECT version()");
    if (PQresultStatus(res) != PGRES_TUPLES_OK) {
        PQclear(res);
        char *e = malloc(1); e[0]='\0'; return e;
    }
    const char *v = PQgetvalue(res, 0, 0);
    char *out = malloc(strlen(v) + 1);
    strcpy(out, v);
    PQclear(res);
    return out;
}

/* Close a connection. */
int64_t certo_db_close(int64_t handle) {
    if (handle != 0) PQfinish((PGconn *)(uintptr_t)handle);
    return 0;
}

/* Return a human-readable error message for the last operation. */
certo_text_t certo_db_error(int64_t handle) {
    if (handle == 0) {
        const char *msg = "connection failed";
        char *out = malloc(strlen(msg) + 1);
        strcpy(out, msg);
        return out;
    }
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    const char *msg = PQerrorMessage(conn);
    char *out = malloc(strlen(msg) + 1);
    strcpy(out, msg);
    return out;
}

/* Execute a query and return the first column of the first row as Text.
 * Returns empty string on failure. */
certo_text_t certo_db_query_one(int64_t handle, certo_text_t sql) {
    if (handle == 0) { char *e = malloc(1); e[0]='\0'; return e; }
    PGconn *conn = (PGconn *)(uintptr_t)handle;
    PGresult *res = PQexec(conn, sql);
    if (PQresultStatus(res) != PGRES_TUPLES_OK || PQntuples(res) == 0) {
        PQclear(res);
        char *e = malloc(1); e[0]='\0'; return e;
    }
    const char *v = PQgetvalue(res, 0, 0);
    char *out = malloc(strlen(v) + 1);
    strcpy(out, v);
    PQclear(res);
    return out;
}
"#;

/// Certo source declarations for Stdlib.Db.
pub const DB_CERTO: &str = r#"
module Stdlib.Db

/// Open a connection to a PostgreSQL server using a libpq connection string.
/// Returns a connection handle (0 = failure).
fn dbConnect(connstr: Text): Int [io]

/// Return the server version as an integer (major*10000 + minor*100 + patch).
/// e.g. PostgreSQL 15.4 → 150004. Returns -1 on a failed connection.
fn dbServerVersion(conn: Int): Int [io]

/// Return the full version banner from SELECT version().
fn dbVersionString(conn: Int): Text [io]

/// Return the last error message for a connection handle.
fn dbError(conn: Int): Text

/// Execute a SQL query and return the first column of the first row as Text.
fn dbQueryOne(conn: Int, sql: Text): Text [io]

/// Close a connection and release resources.
fn dbClose(conn: Int): Unit [io]
"#;
