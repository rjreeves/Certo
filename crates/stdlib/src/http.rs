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
"#;

pub const HTTP_CERTO: &str = r#"
module Stdlib.Http

/// GET request. Returns an HttpResponse.
fn Http.get(url: Text): HttpResponse [io]

/// POST request with a body and content-type header.
fn Http.post(url: Text, body: Text, contentType: Text): HttpResponse [io]

/// PUT request with a body and content-type header.
fn Http.put(url: Text, body: Text, contentType: Text): HttpResponse [io]

/// DELETE request.
fn Http.delete(url: Text): HttpResponse [io]

/// HTTP status code (e.g. 200, 404).
fn HttpResponse.status(r: HttpResponse): Int

/// Response body as text.
fn HttpResponse.body(r: HttpResponse): Text

/// Value of the Content-Type response header.
fn HttpResponse.contentType(r: HttpResponse): Text

/// True if status is 2xx.
fn HttpResponse.ok(r: HttpResponse): Bool
"#;
