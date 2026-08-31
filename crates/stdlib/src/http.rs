pub const HTTP_C: &str = r#"
/* ================================================================
   Stdlib.Http — simple HTTP/HTTPS client
   Windows: WinHTTP (built-in, no extra deps).
   Other platforms: stub (panics with helpful message).
   ================================================================ */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

typedef struct {
    int64_t      status;
    certo_text_t body;
    int64_t      body_length;
    certo_text_t content_type;
    /* BACKLOG item 239 — NULL for an ordinary response; set only by
       certo_http_redirect below. A dedicated field rather than overloading
       `body`'s meaning, so every existing constructor/accessor is
       completely unaffected. */
    certo_text_t location;
} CertoHttpResponse;

static CertoHttpResponse* http_response_new(int64_t status, char* body, int64_t body_length, char* ct) {
    CertoHttpResponse* r = (CertoHttpResponse*)malloc(sizeof(CertoHttpResponse));
    if (!r) certo_panic("out of memory");
    r->status       = status;
    r->body         = body ? body : (char*)"";
    r->body_length  = body_length;
    r->content_type = ct   ? ct   : (char*)"";
    r->location     = NULL;
    return r;
}

#ifdef _WIN32
#include <windows.h>
#include <winhttp.h>
#ifdef _MSC_VER
#pragma comment(lib, "winhttp.lib")
#endif

/* Convert UTF-8 to wide string (caller frees). */
static LPWSTR utf8_to_wide(const char* s) {
    if (!s) return NULL;
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0) return NULL;
    LPWSTR w = (LPWSTR)malloc((size_t)n * sizeof(WCHAR));
    if (!w) certo_panic("out of memory");
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w, n);
    return w;
}

/* Convert wide string to UTF-8 heap string (caller frees). */
static char* wide_to_utf8(LPCWSTR w) {
    if (!w) return NULL;
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    if (n <= 0) return NULL;
    char* s = (char*)malloc((size_t)n);
    if (!s) certo_panic("out of memory");
    WideCharToMultiByte(CP_UTF8, 0, w, -1, s, n, NULL, NULL);
    return s;
}

/* Parse URL into components (scheme, host, port, path+query). */
typedef struct { bool https; char* host; INTERNET_PORT port; char* path; } ParsedUrl;

static bool parse_url(const char* url, ParsedUrl* out) {
    out->host = NULL; out->path = NULL;
    bool https = false;
    const char* rest = url;
    if (strncmp(url, "https://", 8) == 0) { https = true; rest = url + 8; }
    else if (strncmp(url, "http://", 7) == 0) { rest = url + 7; }
    else return false;
    out->https = https;
    out->port  = https ? INTERNET_DEFAULT_HTTPS_PORT : INTERNET_DEFAULT_HTTP_PORT;

    const char* slash = strchr(rest, '/');
    size_t host_len = slash ? (size_t)(slash - rest) : strlen(rest);

    /* Check for explicit port */
    const char* colon = (const char*)memchr(rest, ':', host_len);
    if (colon) {
        out->port  = (INTERNET_PORT)atoi(colon + 1);
        host_len   = (size_t)(colon - rest);
    }

    out->host = (char*)malloc(host_len + 1);
    if (!out->host) certo_panic("out of memory");
    memcpy(out->host, rest, host_len);
    out->host[host_len] = '\0';

    const char* path_start = slash ? slash : "/";
    size_t path_len = strlen(path_start);
    out->path = (char*)malloc(path_len + 1);
    if (!out->path) certo_panic("out of memory");
    memcpy(out->path, path_start, path_len + 1);
    return true;
}

/* Core request function. */
static CertoHttpResponse* winhttp_request(
    const char* method,
    const char* url,
    const char* extra_headers, /* may be NULL */
    const char* body,          /* may be NULL */
    size_t body_len
) {
    ParsedUrl pu = {0};
    if (!parse_url(url, &pu)) return http_response_new(0, NULL, 0, NULL);

    LPWSTR w_host   = utf8_to_wide(pu.host);
    LPWSTR w_path   = utf8_to_wide(pu.path);
    LPWSTR w_method = utf8_to_wide(method);

    HINTERNET hsess = WinHttpOpen(L"Certo/1.0",
        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
        WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    CertoHttpResponse* result = NULL;

    if (!hsess) goto cleanup;

    HINTERNET hconn = WinHttpConnect(hsess, w_host, pu.port, 0);
    if (!hconn) { WinHttpCloseHandle(hsess); goto cleanup; }

    DWORD flags = pu.https ? WINHTTP_FLAG_SECURE : 0;
    HINTERNET hreq = WinHttpOpenRequest(hconn, w_method, w_path,
        NULL, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, flags);
    if (!hreq) { WinHttpCloseHandle(hconn); WinHttpCloseHandle(hsess); goto cleanup; }

    /* Add extra headers if provided */
    if (extra_headers && *extra_headers) {
        LPWSTR w_hdrs = utf8_to_wide(extra_headers);
        WinHttpAddRequestHeaders(hreq, w_hdrs, (DWORD)-1L,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE);
        free(w_hdrs);
    }

    BOOL sent = WinHttpSendRequest(hreq,
        WINHTTP_NO_ADDITIONAL_HEADERS, 0,
        (LPVOID)body, (DWORD)body_len, (DWORD)body_len, 0);

    if (sent) WinHttpReceiveResponse(hreq, NULL);

    /* Status code */
    DWORD status_code = 0;
    DWORD status_size = sizeof(DWORD);
    WinHttpQueryHeaders(hreq,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_HEADER_NAME_BY_INDEX, &status_code, &status_size,
        WINHTTP_NO_HEADER_INDEX);

    /* Content-Type */
    WCHAR ct_buf[256] = {0};
    DWORD ct_size = sizeof(ct_buf);
    WinHttpQueryHeaders(hreq, WINHTTP_QUERY_CONTENT_TYPE,
        WINHTTP_HEADER_NAME_BY_INDEX, ct_buf, &ct_size,
        WINHTTP_NO_HEADER_INDEX);
    char* ct = wide_to_utf8(ct_buf);

    /* Read body */
    char*  body_out = NULL;
    size_t body_cap = 0;
    size_t body_pos = 0;
    DWORD  avail    = 0;
    while (WinHttpQueryDataAvailable(hreq, &avail) && avail > 0) {
        if (body_pos + avail + 1 > body_cap) {
            body_cap = (body_pos + avail + 1) * 2;
            body_out = (char*)realloc(body_out, body_cap);
            if (!body_out) certo_panic("out of memory");
        }
        DWORD read = 0;
        WinHttpReadData(hreq, body_out + body_pos, avail, &read);
        body_pos += read;
    }
    if (!body_out) { body_out = (char*)malloc(1); if (!body_out) certo_panic("out of memory"); }
    body_out[body_pos] = '\0';

    result = http_response_new((int64_t)status_code, body_out, (int64_t)body_pos, ct);

    WinHttpCloseHandle(hreq);
    WinHttpCloseHandle(hconn);
    WinHttpCloseHandle(hsess);

cleanup:
    free(w_host); free(w_path); free(w_method);
    free(pu.host); free(pu.path);
    if (!result) result = http_response_new(0, NULL, 0, NULL);
    return result;
}

CertoHttpResponse* certo_http_get(certo_text_t url) {
    return winhttp_request("GET", url, NULL, NULL, 0);
}

CertoHttpResponse* certo_http_post(certo_text_t url, certo_text_t body, certo_text_t content_type) {
    char hdr[256] = {0};
    if (content_type && *content_type)
        snprintf(hdr, sizeof(hdr), "Content-Type: %s", content_type);
    size_t blen = body ? strlen(body) : 0;
    return winhttp_request("POST", url, *hdr ? hdr : NULL, body, blen);
}

CertoHttpResponse* certo_http_put(certo_text_t url, certo_text_t body, certo_text_t content_type) {
    char hdr[256] = {0};
    if (content_type && *content_type)
        snprintf(hdr, sizeof(hdr), "Content-Type: %s", content_type);
    size_t blen = body ? strlen(body) : 0;
    return winhttp_request("PUT", url, *hdr ? hdr : NULL, body, blen);
}

CertoHttpResponse* certo_http_delete(certo_text_t url) {
    return winhttp_request("DELETE", url, NULL, NULL, 0);
}

/* Flatten a List<List<Text>> of [name,value] pairs into WinHTTP's CRLF-joined
   header string. Caller frees the returned buffer (may be NULL if no headers). */
static char* http_flatten_headers(CertoList* headers) {
    char*  hdrs = NULL;
    size_t hlen = 0, hcap = 0;
    if (!headers) return NULL;
    for (int64_t i = 0; i < headers->len; i++) {
        CertoList* pair = (CertoList*)headers->data[i];
        if (!pair || pair->len < 2) continue;
        const char* k = pair->data[0] ? (const char*)pair->data[0] : "";
        const char* v = pair->data[1] ? (const char*)pair->data[1] : "";
        size_t need = strlen(k) + strlen(v) + 4; /* "k: v\r\n" */
        if (hlen + need + 1 > hcap) {
            hcap = (hlen + need + 1) * 2;
            hdrs = (char*)realloc(hdrs, hcap);
            if (!hdrs) certo_panic("out of memory");
        }
        hlen += (size_t)snprintf(hdrs + hlen, hcap - hlen, "%s: %s\r\n", k, v);
    }
    return hdrs;
}

/* General request with an explicit method and caller-supplied headers.
   `headers` is a List<List<Text>>: each inner list is [name, value].
   Body is Text (NUL-terminated). For binary bodies use certo_http_request_bytes. */
CertoHttpResponse* certo_http_request(certo_text_t method, certo_text_t url, CertoList* headers, certo_text_t body) {
    char* hdrs = http_flatten_headers(headers);
    size_t blen = body ? strlen(body) : 0;
    CertoHttpResponse* r = winhttp_request(method, url, hdrs, body, blen);
    free(hdrs);
    return r;
}

/* Same, but with a binary Bytes body (length-carrying, NUL-safe). */
CertoHttpResponse* certo_http_request_bytes(certo_text_t method, certo_text_t url, CertoList* headers, CertoBytes* body) {
    char* hdrs = http_flatten_headers(headers);
    const char* data = body ? (const char*)body->data : NULL;
    size_t blen = body ? (size_t)body->len : 0;
    CertoHttpResponse* r = winhttp_request(method, url, hdrs, data, blen);
    free(hdrs);
    return r;
}

#else
/* ---- POSIX stub ------------------------------------------------- */
CertoHttpResponse* certo_http_get   (certo_text_t url)                                              { (void)url; certo_panic("Http not supported on this platform — link libcurl and implement."); return NULL; }
CertoHttpResponse* certo_http_post  (certo_text_t url, certo_text_t body, certo_text_t ct)          { (void)url; (void)body; (void)ct; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_put   (certo_text_t url, certo_text_t body, certo_text_t ct)          { (void)url; (void)body; (void)ct; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_delete(certo_text_t url)                                              { (void)url; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_request(certo_text_t method, certo_text_t url, CertoList* headers, certo_text_t body) { (void)method; (void)url; (void)headers; (void)body; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_request_bytes(certo_text_t method, certo_text_t url, CertoList* headers, CertoBytes* body) { (void)method; (void)url; (void)headers; (void)body; certo_panic("Http not supported on this platform."); return NULL; }
#endif

/* Portable case-insensitive string compare — `_stricmp` is MSVC/Windows-only;
   POSIX's equivalent lives in <strings.h> as `strcasecmp`. Used below by
   `certo_http_request_header` (platform-independent — not itself guarded by
   `#ifdef _WIN32`, so it must compile everywhere) and by the server's own
   request parser further down. */
#ifdef _WIN32
#define certo_stricmp _stricmp
#else
#include <strings.h>
#define certo_stricmp strcasecmp
#endif

/* ---- Accessors (platform-independent) -------------------------- */

int64_t      certo_http_response_status      (CertoHttpResponse* r) { return r ? r->status       : 0; }
certo_text_t certo_http_response_body        (CertoHttpResponse* r) { return r ? r->body         : ""; }
int64_t      certo_http_response_body_length (CertoHttpResponse* r) { return r ? r->body_length  : 0; }
certo_text_t certo_http_response_content_type(CertoHttpResponse* r) { return r ? r->content_type : ""; }
bool         certo_http_response_ok          (CertoHttpResponse* r) { return r && r->status >= 200 && r->status < 300; }

/* ================================================================
   Stdlib.Http — server. Windows: WinSock2. POSIX: BSD sockets.
   Both are a plain blocking, single-connection-at-a-time accept loop —
   no threading/concurrency, no keep-alive (mirrors Windows exactly).
   ================================================================ */

/* An incoming HTTP request. */
typedef struct {
    certo_text_t method;
    certo_text_t path;
    certo_text_t query;     /* everything after '?' in the URL, or "" */
    certo_text_t body;
    CertoList*   headers;   /* List<List<Text>>: each inner = [name, value] */
} CertoHttpRequest;

typedef CertoHttpResponse* (*CertoHttpHandler)(void*, CertoHttpRequest*);

/* Convenience response constructors */
CertoHttpResponse* certo_http_respond(int64_t status, certo_text_t body, certo_text_t ct) {
    char* b = NULL;
    if (body) {
        size_t n = strlen(body);
        b = (char*)malloc(n + 1);
        if (!b) certo_panic("out of memory");
        memcpy(b, body, n + 1);
    }
    char* c = NULL;
    if (ct) {
        size_t n = strlen(ct);
        c = (char*)malloc(n + 1);
        if (!c) certo_panic("out of memory");
        memcpy(c, ct, n + 1);
    }
    return http_response_new(status, b, body ? (int64_t)strlen(body) : 0, c);
}

CertoHttpResponse* certo_http_ok        (certo_text_t body, certo_text_t ct)  { return certo_http_respond(200, body, ct); }
CertoHttpResponse* certo_http_not_found (certo_text_t body)                   { return certo_http_respond(404, body, "text/plain"); }
CertoHttpResponse* certo_http_bad_req   (certo_text_t body)                   { return certo_http_respond(400, body, "text/plain"); }
CertoHttpResponse* certo_http_srv_error (certo_text_t body)                   { return certo_http_respond(500, body, "text/plain"); }

/* BACKLOG item 239 — a real server-side redirect (303 See Other, the
   standard "redirect after a successful POST" status so the browser
   re-fetches the target with GET rather than resubmitting the form),
   needed for a `form`'s `onSuccess: navigate(View)` to actually navigate
   the client anywhere — every generated `form` is a plain, non-htmx
   `<form method="POST">`, so an ordinary HTTP redirect (not an
   htmx-specific `HX-Redirect` header) is the correct mechanism a real
   browser already understands. */
CertoHttpResponse* certo_http_redirect(certo_text_t url) {
    CertoHttpResponse* r = http_response_new(303, (char*)"", 0, (char*)"text/plain");
    if (url) {
        size_t n = strlen(url);
        char* u = (char*)malloc(n + 1);
        if (!u) certo_panic("out of memory");
        memcpy(u, url, n + 1);
        r->location = u;
    }
    return r;
}
/* Aliases matching camelCase Certo names → c_fn_name output */
static inline CertoHttpResponse* certo_http_bad_request    (certo_text_t b) { return certo_http_bad_req(b); }
static inline CertoHttpResponse* certo_http_server_error   (certo_text_t b) { return certo_http_srv_error(b); }

/* HttpRequest accessors */
certo_text_t certo_http_request_method (CertoHttpRequest* r) { return r ? r->method  : ""; }
certo_text_t certo_http_request_path   (CertoHttpRequest* r) { return r ? r->path    : ""; }
certo_text_t certo_http_request_query  (CertoHttpRequest* r) { return r ? r->query   : ""; }
certo_text_t certo_http_request_body   (CertoHttpRequest* r) { return r ? r->body    : ""; }
CertoList*   certo_http_request_headers(CertoHttpRequest* r) {
    return (r && r->headers) ? r->headers : certo_list_new_empty();
}

/* Look up a header by name (case-insensitive). Returns "" if not found. */
certo_text_t certo_http_request_header(CertoHttpRequest* r, certo_text_t name) {
    if (!r || !r->headers || !name) return "";
    for (int64_t i = 0; i < r->headers->len; i++) {
        CertoList* pair = (CertoList*)r->headers->data[i];
        if (!pair || pair->len < 2) continue;
        const char* k = (const char*)pair->data[0];
        if (k && certo_stricmp(k, name) == 0) return (certo_text_t)pair->data[1];
    }
    return "";
}

/* ---- internal: heap-duplicate a string (pure logic, no socket API,
   shared verbatim by both platforms) -------------------------------- */
static char* http_srv_strdup(const char* s) {
    if (!s) { char* e = (char*)malloc(1); e[0]='\0'; return e; }
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n + 1);
    return out;
}

/* ---- internal: status text (pure logic, shared) ------------------ */
static const char* http_status_text(int64_t code) {
    switch (code) {
        case 200: return "OK";
        case 201: return "Created";
        case 204: return "No Content";
        case 301: return "Moved Permanently";
        case 302: return "Found";
        case 303: return "See Other";
        case 304: return "Not Modified";
        case 400: return "Bad Request";
        case 401: return "Unauthorized";
        case 403: return "Forbidden";
        case 404: return "Not Found";
        case 405: return "Method Not Allowed";
        case 409: return "Conflict";
        case 422: return "Unprocessable Entity";
        case 429: return "Too Many Requests";
        case 500: return "Internal Server Error";
        case 502: return "Bad Gateway";
        case 503: return "Service Unavailable";
        default:  return "Unknown";
    }
}

/* `recv`/`send` are BSD-sockets API with the same signature on Winsock and
   POSIX — only the socket handle's own type, its "invalid" sentinel, and how
   it's closed differ. `certo_socket_t` lets every helper below (`http_srv_*`)
   be written once and shared, instead of duplicating the whole read/parse/
   send pipeline per platform the way the client-side (WinHTTP vs POSIX-stub)
   functions above do. */
#ifdef _WIN32
typedef SOCKET certo_socket_t;
#define CERTO_INVALID_SOCKET INVALID_SOCKET
static void certo_closesocket(certo_socket_t s) { closesocket(s); }
#else
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
typedef int certo_socket_t;
#define CERTO_INVALID_SOCKET (-1)
static void certo_closesocket(certo_socket_t s) { close(s); }
#endif

/* ---- internal: read until CRLF CRLF (shared) --------------------- */
static char* http_srv_read_request(certo_socket_t sock) {
    size_t cap = 4096, pos = 0;
    char* buf = (char*)malloc(cap);
    if (!buf) certo_panic("out of memory");
    for (;;) {
        if (pos + 1 >= cap) {
            cap *= 2;
            char* nb = (char*)realloc(buf, cap);
            if (!nb) { free(buf); certo_panic("out of memory"); }
            buf = nb;
        }
        int n = recv(sock, buf + pos, 1, 0);
        if (n <= 0) break;
        pos++;
        if (pos >= 4
            && buf[pos-4] == '\r' && buf[pos-3] == '\n'
            && buf[pos-2] == '\r' && buf[pos-1] == '\n') break;
    }
    buf[pos] = '\0';
    return buf;
}

/* ---- internal: read exactly `len` bytes of body (shared) --------- */
static char* http_srv_read_body(certo_socket_t sock, int64_t len) {
    if (len <= 0) { char* e = (char*)malloc(1); e[0]='\0'; return e; }
    char* buf = (char*)malloc((size_t)len + 1);
    if (!buf) certo_panic("out of memory");
    int64_t got = 0;
    while (got < len) {
        int n = recv(sock, buf + got, (int)(len - got), 0);
        if (n <= 0) break;
        got += n;
    }
    buf[got] = '\0';
    return buf;
}

/* ---- internal: parse raw headers text into CertoHttpRequest (shared) */
static CertoHttpRequest* http_srv_parse(const char* raw, certo_socket_t sock) {
    CertoHttpRequest* req = (CertoHttpRequest*)malloc(sizeof(CertoHttpRequest));
    if (!req) certo_panic("out of memory");
    req->method  = http_srv_strdup("");
    req->path    = http_srv_strdup("");
    req->query   = http_srv_strdup("");
    req->body    = http_srv_strdup("");
    req->headers = certo_list_new_empty();

    /* Parse request line: METHOD SP path[?query] SP HTTP/x.y */
    const char* p = raw;
    const char* sp1 = strchr(p, ' ');
    if (!sp1) return req;

    size_t mlen = (size_t)(sp1 - p);
    char* method = (char*)malloc(mlen + 1);
    if (!method) certo_panic("out of memory");
    memcpy(method, p, mlen);
    method[mlen] = '\0';
    req->method = method;

    p = sp1 + 1;
    const char* sp2 = strchr(p, ' ');
    if (!sp2) return req;

    size_t pqlen = (size_t)(sp2 - p);
    char* pq = (char*)malloc(pqlen + 1);
    if (!pq) certo_panic("out of memory");
    memcpy(pq, p, pqlen);
    pq[pqlen] = '\0';

    char* qmark = strchr(pq, '?');
    if (qmark) {
        *qmark = '\0';
        req->path  = pq;
        req->query = http_srv_strdup(qmark + 1);
    } else {
        req->path  = pq;
        req->query = http_srv_strdup("");
    }

    /* Skip to end of request line */
    p = sp2 + 1;
    while (*p && *p != '\n') p++;
    if (*p) p++;

    /* Parse header lines */
    int64_t content_length = 0;
    while (*p && !(*p == '\r' && *(p+1) == '\n')) {
        const char* eol = strstr(p, "\r\n");
        if (!eol) break;
        const char* colon = (const char*)memchr(p, ':', (size_t)(eol - p));
        if (colon) {
            size_t klen = (size_t)(colon - p);
            char* key = (char*)malloc(klen + 1);
            if (!key) certo_panic("out of memory");
            memcpy(key, p, klen);
            key[klen] = '\0';

            const char* vstart = colon + 1;
            while (*vstart == ' ') vstart++;
            size_t vlen = (size_t)(eol - vstart);
            char* val = (char*)malloc(vlen + 1);
            if (!val) certo_panic("out of memory");
            memcpy(val, vstart, vlen);
            val[vlen] = '\0';

            if (certo_stricmp(key, "content-length") == 0)
                content_length = atoll(val);

            CertoList* pair = certo_list_new_empty();
            pair = certo_list_push(pair, (void*)key);
            pair = certo_list_push(pair, (void*)val);
            req->headers = certo_list_push(req->headers, (void*)pair);
        }
        p = eol + 2;
    }

    /* Read body if Content-Length > 0 */
    if (content_length > 0)
        req->body = http_srv_read_body(sock, content_length);

    return req;
}

/* ---- internal: send response (shared) --------------------------- */
static void http_srv_send(certo_socket_t sock, CertoHttpResponse* resp) {
    const char* body = resp && resp->body         ? resp->body         : "";
    const char* ct   = resp && resp->content_type ? resp->content_type : "text/plain";
    int64_t     code = resp ? resp->status : 500;
    size_t      blen = strlen(body);
    /* BACKLOG item 239 — a Location header, only ever set by
       certo_http_redirect; every other response constructor leaves this
       NULL, so this is a strict addition with no effect on any existing
       response. */
    const char* loc  = resp ? resp->location : NULL;

    char header[768];
    if (loc) {
        snprintf(header, sizeof(header),
            "HTTP/1.1 %lld %s\r\n"
            "Location: %s\r\n"
            "Content-Type: %s\r\n"
            "Content-Length: %zu\r\n"
            "Connection: close\r\n"
            "\r\n",
            (long long)code, http_status_text(code), loc, ct, blen);
    } else {
        snprintf(header, sizeof(header),
            "HTTP/1.1 %lld %s\r\n"
            "Content-Type: %s\r\n"
            "Content-Length: %zu\r\n"
            "Connection: close\r\n"
            "\r\n",
            (long long)code, http_status_text(code), ct, blen);
    }

    send(sock, header, (int)strlen(header), 0);
    if (blen > 0) send(sock, body, (int)blen, 0);
}

/* ---- live-query push channel (BACKLOG item 88, stage 2/3) --------
   `/__certo_live` is a reserved path, intercepted before the user's own
   handler ever sees it (a real, documented collision risk if a user's
   own app happens to route that exact path — accepted for this first
   version). A connecting client holds the connection open and blocks on
   a shared generation counter; `certo_http_live_notify()` (called by
   generated write-handler code after a successful DB write) bumps the
   counter and broadcasts, waking every open `/__certo_live` connection
   at once. No registry of open sockets is needed at all: a broadcast
   wakes every waiter, and each waiter only ever touches the one socket
   it already owns — reusing the exact mutex+condvar split (Win32
   CRITICAL_SECTION+CONDITION_VARIABLE / POSIX pthread mutex+cond)
   `crates/stdlib/src/channel.rs`'s `CertoChannel` already proves out.
   A periodic timed wake (not just the broadcast) sends an SSE comment
   (`: ping`) even with no real change — both a standard keep-alive and
   the only way a thread ever notices its client disconnected, since
   nothing else here polls the socket; a client that vanishes between
   pings leaks its thread until the next ping/notify's failed `send()`
   cleans it up — bounded, not unbounded, but not instant either, a
   deliberate simplification for this first version. */
#define CERTO_LIVE_PATH "/__certo_live"
#define CERTO_LIVE_PING_MS 25000

#if defined(_WIN32)
static CRITICAL_SECTION g_live_lock;
static CONDITION_VARIABLE g_live_cond;
#else
static pthread_mutex_t g_live_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t  g_live_cond = PTHREAD_COND_INITIALIZER;
#endif
static volatile int64_t g_live_generation = 0;

/* Called once, synchronously, before `certo_http_serve`'s accept loop
   starts spawning connection threads — Win32's CRITICAL_SECTION/
   CONDITION_VARIABLE (unlike pthread's static initializer macros) have
   no compile-time-initialized form, so this must run before any
   connection thread could possibly reach the SSE code path below. */
static void __certo_live_init(void) {
#if defined(_WIN32)
    InitializeCriticalSection(&g_live_lock);
    InitializeConditionVariable(&g_live_cond);
#endif
}

/* Public: called by generated write-handler code after a successful DB
   write. A no-op (just an uncontended lock/unlock and a broadcast to
   zero waiters) when no client is connected to `/__certo_live` at all.
   Returns `int64_t` (always 0), not `void` — every stdlib function whose
   Certo signature is `Unit` still compiles to a real `int64_t` return
   (codegen always assigns a call's result to an `int64_t`-typed local
   regardless of Certo-level Unit semantics), the exact bug item 192
   already found and fixed once for `certo_file_close`. */
int64_t certo_http_live_notify(void) {
#if defined(_WIN32)
    EnterCriticalSection(&g_live_lock);
    g_live_generation++;
    LeaveCriticalSection(&g_live_lock);
    WakeAllConditionVariable(&g_live_cond);
#else
    pthread_mutex_lock(&g_live_lock);
    g_live_generation++;
    pthread_mutex_unlock(&g_live_lock);
    pthread_cond_broadcast(&g_live_cond);
#endif
    return 0;
}

/* Runs on the connection's own thread — holds the connection open
   indefinitely, so it must own closing the socket on every exit path
   (the caller, `__certo_http_handle_connection`, must not also close it). */
static void __certo_http_serve_sse(certo_socket_t client) {
    static const char* headers =
        "HTTP/1.1 200 OK\r\n"
        "Content-Type: text/event-stream\r\n"
        "Cache-Control: no-cache\r\n"
        "Connection: keep-alive\r\n"
        "\r\n";
    if (send(client, headers, (int)strlen(headers), 0) <= 0) {
        certo_closesocket(client);
        return;
    }

#if defined(_WIN32)
    EnterCriticalSection(&g_live_lock);
#else
    pthread_mutex_lock(&g_live_lock);
#endif
    int64_t last_seen = g_live_generation;
    for (;;) {
#if defined(_WIN32)
        SleepConditionVariableCS(&g_live_cond, &g_live_lock, CERTO_LIVE_PING_MS);
        LeaveCriticalSection(&g_live_lock);
#else
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_sec  += CERTO_LIVE_PING_MS / 1000;
        pthread_cond_timedwait(&g_live_cond, &g_live_lock, &ts);
        pthread_mutex_unlock(&g_live_lock);
#endif
        bool changed = (g_live_generation != last_seen);
        last_seen = g_live_generation;
        const char* frame = changed ? "data: refresh\n\n" : ": ping\n\n";
        if (send(client, frame, (int)strlen(frame), 0) <= 0) {
            certo_closesocket(client);
            return;
        }
#if defined(_WIN32)
        EnterCriticalSection(&g_live_lock);
#else
        pthread_mutex_lock(&g_live_lock);
#endif
    }
}

/* ---- concurrent connections (BACKLOG item 88, stage 1) -----------
   Each accepted connection now runs on its own OS thread instead of
   being handled inline in the accept loop — `__certo_thread_spawn`/
   `__certo_thread_t` (Win32 threads / pthreads) already exist in
   `RUNTIME_HEADER` (`crates/codegen/src/emit_module.rs`), emitted into
   every compiled program *before* this stdlib module's own C, so they're
   already visible here — no new concurrency primitive needed, just
   reusing the one `spawn`/`parallel`/`withTimeout` already rely on.
   Fire-and-forget: the server thread never joins a connection thread
   (it must keep accepting new connections concurrently), so each
   connection thread is detached immediately after spawning — otherwise
   its OS resources (a Win32 HANDLE, a pthread's join state) would never
   be reclaimed until process exit, a real leak under sustained traffic.
   No concurrent-connection limit or backpressure exists yet — a
   deliberate, documented simplification for this first version, the
   same kind of accepted scope-limit already used elsewhere in this file
   (e.g. `parallel(timeout)`'s own leaked-thread-on-timeout above). */
typedef struct {
    certo_socket_t client;
    certo_fn_t     raw_handler;
} __certo_http_conn_ctx_t;

static void* __certo_http_handle_connection(void* arg) {
    __certo_http_conn_ctx_t* ctx = (__certo_http_conn_ctx_t*)arg;
    certo_socket_t   client  = ctx->client;
    CertoHttpHandler handler = (CertoHttpHandler)ctx->raw_handler.fn;
    void*            env     = ctx->raw_handler.env;
    free(ctx);

    char* raw = http_srv_read_request(client);
    CertoHttpRequest* req = http_srv_parse(raw, client);
    free(raw);

    if (strcmp(req->path, CERTO_LIVE_PATH) == 0) {
        __certo_http_serve_sse(client);   /* owns closing the socket itself */
        return NULL;
    }

    CertoHttpResponse* resp = handler(env, req);
    http_srv_send(client, resp);

    /* BACKLOG item 226 — this thread may have opened an ambient DB
     * connection (`db.transaction`/`db.<table>.<method>`) while handling
     * this one request; close it now rather than leaking it when the OS
     * eventually reclaims this (detached, never-reused) thread. No-op for
     * a request that never touched the DB. Guarded since ordinary non-DB
     * programs never define this symbol at all (crates/codegen/src/
     * emit_module.rs's own matching guard on the forward declaration). */
    #ifdef CERTO_DB_ENABLED
    __certo_db_thread_teardown();
    #endif

    certo_closesocket(client);
    return NULL;
}

static void __certo_http_spawn_connection(certo_socket_t client, certo_fn_t raw_handler) {
    __certo_http_conn_ctx_t* ctx = (__certo_http_conn_ctx_t*)malloc(sizeof(__certo_http_conn_ctx_t));
    if (!ctx) certo_panic("out of memory");
    ctx->client      = client;
    ctx->raw_handler = raw_handler;
    __certo_thread_t th = __certo_thread_spawn(__certo_http_handle_connection, ctx);
#ifdef _WIN32
    CloseHandle(th);       /* fire-and-forget: release the handle, the thread keeps running */
#else
    pthread_detach(th);    /* fire-and-forget: reclaim resources on exit without a join */
#endif
}

#ifdef _WIN32
/* ---- public: blocking serve loop (Windows / WinSock2) ------------ */
int64_t certo_http_serve(int64_t port, certo_fn_t raw_handler) {
    __certo_live_init();

    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0)
        certo_panic("WSAStartup failed");

    SOCKET srv = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (srv == INVALID_SOCKET) certo_panic("socket() failed");

    BOOL reuse = TRUE;
    setsockopt(srv, SOL_SOCKET, SO_REUSEADDR, (char*)&reuse, sizeof(reuse));

    struct sockaddr_in addr = {0};
    addr.sin_family      = AF_INET;
    addr.sin_addr.s_addr = INADDR_ANY;
    addr.sin_port        = htons((u_short)port);

    if (bind(srv, (struct sockaddr*)&addr, sizeof(addr)) != 0)
        certo_panic("bind() failed — port may already be in use");
    if (listen(srv, SOMAXCONN) != 0)
        certo_panic("listen() failed");

    for (;;) {
        struct sockaddr_in client_addr = {0};
        int addr_len = sizeof(client_addr);
        SOCKET client = accept(srv, (struct sockaddr*)&client_addr, &addr_len);
        if (client == CERTO_INVALID_SOCKET) continue;

        __certo_http_spawn_connection(client, raw_handler);
    }
    /* unreachable — server runs until process exits */
    return 0;
}

#else
/* ---- public: blocking serve loop (POSIX / BSD sockets) ----------- */
int64_t certo_http_serve(int64_t port, certo_fn_t raw_handler) {
    __certo_live_init();    /* no-op on POSIX — pthread's static initializers already ran */

    int srv = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (srv == CERTO_INVALID_SOCKET) certo_panic("socket() failed");

    int reuse = 1;
    setsockopt(srv, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family      = AF_INET;
    addr.sin_addr.s_addr = INADDR_ANY;
    addr.sin_port        = htons((uint16_t)port);

    if (bind(srv, (struct sockaddr*)&addr, sizeof(addr)) != 0)
        certo_panic("bind() failed — port may already be in use");
    if (listen(srv, SOMAXCONN) != 0)
        certo_panic("listen() failed");

    for (;;) {
        struct sockaddr_in client_addr;
        socklen_t addr_len = sizeof(client_addr);
        int client = accept(srv, (struct sockaddr*)&client_addr, &addr_len);
        if (client == CERTO_INVALID_SOCKET) continue;

        __certo_http_spawn_connection(client, raw_handler);
    }
    /* unreachable — server runs until process exits */
    return 0;
}
#endif
"#;
