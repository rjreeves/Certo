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
    certo_text_t content_type;
} CertoHttpResponse;

static CertoHttpResponse* http_response_new(int64_t status, char* body, char* ct) {
    CertoHttpResponse* r = (CertoHttpResponse*)malloc(sizeof(CertoHttpResponse));
    if (!r) certo_panic("out of memory");
    r->status       = status;
    r->body         = body ? body : (char*)"";
    r->content_type = ct   ? ct   : (char*)"";
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
    if (!parse_url(url, &pu)) return http_response_new(0, NULL, NULL);

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

    result = http_response_new((int64_t)status_code, body_out, ct);

    WinHttpCloseHandle(hreq);
    WinHttpCloseHandle(hconn);
    WinHttpCloseHandle(hsess);

cleanup:
    free(w_host); free(w_path); free(w_method);
    free(pu.host); free(pu.path);
    if (!result) result = http_response_new(0, NULL, NULL);
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

#else
/* ---- POSIX stub ------------------------------------------------- */
CertoHttpResponse* certo_http_get   (certo_text_t url)                                              { (void)url; certo_panic("Http not supported on this platform — link libcurl and implement."); return NULL; }
CertoHttpResponse* certo_http_post  (certo_text_t url, certo_text_t body, certo_text_t ct)          { (void)url; (void)body; (void)ct; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_put   (certo_text_t url, certo_text_t body, certo_text_t ct)          { (void)url; (void)body; (void)ct; certo_panic("Http not supported on this platform."); return NULL; }
CertoHttpResponse* certo_http_delete(certo_text_t url)                                              { (void)url; certo_panic("Http not supported on this platform."); return NULL; }
#endif

/* ---- Accessors (platform-independent) -------------------------- */

int64_t      certo_http_response_status      (CertoHttpResponse* r) { return r ? r->status       : 0; }
certo_text_t certo_http_response_body        (CertoHttpResponse* r) { return r ? r->body         : ""; }
certo_text_t certo_http_response_content_type(CertoHttpResponse* r) { return r ? r->content_type : ""; }
bool         certo_http_response_ok          (CertoHttpResponse* r) { return r && r->status >= 200 && r->status < 300; }

/* ================================================================
   Stdlib.Http — server (Windows-only via WinSock2)
   ================================================================ */

/* An incoming HTTP request. */
typedef struct {
    certo_text_t method;
    certo_text_t path;
    certo_text_t query;     /* everything after '?' in the URL, or "" */
    certo_text_t body;
    CertoList*   headers;   /* List<List<Text>>: each inner = [name, value] */
} CertoHttpRequest;

typedef CertoHttpResponse* (*CertoHttpHandler)(CertoHttpRequest*);

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
    return http_response_new(status, b, c);
}

CertoHttpResponse* certo_http_ok        (certo_text_t body, certo_text_t ct)  { return certo_http_respond(200, body, ct); }
CertoHttpResponse* certo_http_not_found (certo_text_t body)                   { return certo_http_respond(404, body, "text/plain"); }
CertoHttpResponse* certo_http_bad_req   (certo_text_t body)                   { return certo_http_respond(400, body, "text/plain"); }
CertoHttpResponse* certo_http_srv_error (certo_text_t body)                   { return certo_http_respond(500, body, "text/plain"); }

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
        if (k && _stricmp(k, name) == 0) return (certo_text_t)pair->data[1];
    }
    return "";
}

#ifdef _WIN32
/* ---- internal: heap-duplicate a string -------------------------- */
static char* http_srv_strdup(const char* s) {
    if (!s) { char* e = (char*)malloc(1); e[0]='\0'; return e; }
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n + 1);
    return out;
}

/* ---- internal: read until CRLF CRLF ----------------------------- */
static char* http_srv_read_request(SOCKET sock) {
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

/* ---- internal: read exactly `len` bytes of body ----------------- */
static char* http_srv_read_body(SOCKET sock, int64_t len) {
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

/* ---- internal: parse raw headers text into CertoHttpRequest ----- */
static CertoHttpRequest* http_srv_parse(const char* raw, SOCKET sock) {
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

            if (_stricmp(key, "content-length") == 0)
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

/* ---- internal: status text ------------------------------------- */
static const char* http_status_text(int64_t code) {
    switch (code) {
        case 200: return "OK";
        case 201: return "Created";
        case 204: return "No Content";
        case 301: return "Moved Permanently";
        case 302: return "Found";
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

/* ---- internal: send response ----------------------------------- */
static void http_srv_send(SOCKET sock, CertoHttpResponse* resp) {
    const char* body = resp && resp->body         ? resp->body         : "";
    const char* ct   = resp && resp->content_type ? resp->content_type : "text/plain";
    int64_t     code = resp ? resp->status : 500;
    size_t      blen = strlen(body);

    char header[512];
    snprintf(header, sizeof(header),
        "HTTP/1.1 %lld %s\r\n"
        "Content-Type: %s\r\n"
        "Content-Length: %zu\r\n"
        "Connection: close\r\n"
        "\r\n",
        (long long)code, http_status_text(code), ct, blen);

    send(sock, header, (int)strlen(header), 0);
    if (blen > 0) send(sock, body, (int)blen, 0);
}

/* ---- public: blocking serve loop ------------------------------- */
void certo_http_serve(int64_t port, certo_fn_t raw_handler) {
    CertoHttpHandler handler = (CertoHttpHandler)raw_handler;

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
        if (client == INVALID_SOCKET) continue;

        char* raw = http_srv_read_request(client);
        CertoHttpRequest* req = http_srv_parse(raw, client);
        free(raw);

        CertoHttpResponse* resp = handler(req);
        http_srv_send(client, resp);

        closesocket(client);
    }
    /* unreachable — server runs until process exits */
}

#else
/* ---- POSIX stub ------------------------------------------------- */
void certo_http_serve(int64_t port, certo_fn_t handler) {
    (void)port; (void)handler;
    certo_panic("Http.serve is not yet supported on this platform");
}
#endif
"#;

pub const HTTP_CERTO: &str = r#"
module Stdlib.Http

// ── Client ──────────────────────────────────────────────────────────

/// GET request.
fn Http.get(url: Text): HttpResponse [io]

/// POST request with a body and Content-Type header.
fn Http.post(url: Text, body: Text, contentType: Text): HttpResponse [io]

/// PUT request with a body and Content-Type header.
fn Http.put(url: Text, body: Text, contentType: Text): HttpResponse [io]

/// DELETE request.
fn Http.delete(url: Text): HttpResponse [io]

// ── HttpResponse ────────────────────────────────────────────────────

/// HTTP status code (e.g. 200, 404).
fn HttpResponse.status(r: HttpResponse): Int

/// Response body as text.
fn HttpResponse.body(r: HttpResponse): Text

/// Value of the Content-Type response header.
fn HttpResponse.contentType(r: HttpResponse): Text

/// True if status is 2xx.
fn HttpResponse.ok(r: HttpResponse): Bool

// ── Server ──────────────────────────────────────────────────────────

/// Start a blocking HTTP server on the given port.
/// The handler receives each request and must return a response.
/// The server runs until the process exits.
///
/// Example:
///   Http.serve(8080, fn(req: HttpRequest): HttpResponse = {
///       val path = HttpRequest.path(req)
///       if path == "/" then Http.respond(200, "Hello!", "text/plain")
///       else Http.respond(404, "Not Found", "text/plain")
///   })
fn Http.serve(port: Int, handler: fn(HttpRequest): HttpResponse): Unit [io]

/// Build an HttpResponse with an explicit status code.
fn Http.respond(status: Int, body: Text, contentType: Text): HttpResponse

/// 200 OK
fn Http.ok(body: Text, contentType: Text): HttpResponse

/// 404 Not Found
fn Http.notFound(body: Text): HttpResponse

/// 400 Bad Request
fn Http.badRequest(body: Text): HttpResponse

/// 500 Internal Server Error
fn Http.serverError(body: Text): HttpResponse

// ── HttpRequest ─────────────────────────────────────────────────────

/// HTTP method (GET, POST, PUT, DELETE, …).
fn HttpRequest.method(r: HttpRequest): Text

/// Request path, e.g. "/api/users".
fn HttpRequest.path(r: HttpRequest): Text

/// Query string, e.g. "page=2&limit=10". Empty string if none.
fn HttpRequest.query(r: HttpRequest): Text

/// Request body (for POST/PUT).
fn HttpRequest.body(r: HttpRequest): Text

/// Look up a request header by name (case-insensitive). Returns "" if absent.
fn HttpRequest.header(r: HttpRequest, name: Text): Text

/// All request headers as List<List<Text>>. Each inner list is [name, value].
fn HttpRequest.headers(r: HttpRequest): List<List<Text>>
"#;
