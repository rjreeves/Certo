pub const CREDENTIAL_C: &str = r#"
/* ================================================================
   Stdlib.Credential — Windows Credential Manager (Generic creds).
   Windows: advapi32 CredRead/CredWrite/CredDelete.
   Other platforms: stubs returning None/false (env-var fallback).
   Concatenated after BYTES_C so CertoBytes / certo_bytes_alloc exist.
   ================================================================ */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#ifdef _WIN32
#include <windows.h>
#include <wincred.h>
#ifdef _MSC_VER
#pragma comment(lib, "advapi32.lib")
#endif

/* UTF-8 -> wide (caller frees). Private copy to avoid coupling module order. */
static LPWSTR cred_utf8_to_wide(const char* s) {
    if (!s) return NULL;
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0) return NULL;
    LPWSTR w = (LPWSTR)malloc((size_t)n * sizeof(WCHAR));
    if (!w) certo_panic("out of memory");
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w, n);
    return w;
}

/* `wlen` WCHARs (not necessarily NUL-terminated) -> UTF-8 heap string. */
static char* cred_wide_to_utf8_n(const WCHAR* w, int wlen) {
    if (!w || wlen <= 0) { char* e = (char*)malloc(1); if (!e) certo_panic("out of memory"); e[0] = 0; return e; }
    int n = WideCharToMultiByte(CP_UTF8, 0, w, wlen, NULL, 0, NULL, NULL);
    char* s = (char*)malloc((size_t)n + 1);
    if (!s) certo_panic("out of memory");
    WideCharToMultiByte(CP_UTF8, 0, w, wlen, s, n, NULL, NULL);
    s[n] = 0;
    return s;
}

/* Raw credential blob as Option<Bytes>. */
void* certo_credential_get_bytes(certo_text_t target) {
    if (!target) return NULL;
    LPWSTR wtarget = cred_utf8_to_wide(target);
    PCREDENTIALW cred = NULL;
    BOOL ok = CredReadW(wtarget, CRED_TYPE_GENERIC, 0, &cred);
    free(wtarget);
    if (!ok || !cred) return NULL;
    CertoBytes* b = certo_bytes_alloc((int64_t)cred->CredentialBlobSize);
    if (cred->CredentialBlobSize)
        memcpy(b->data, cred->CredentialBlob, cred->CredentialBlobSize);
    CredFree(cred);
    return __certo_opt_box((int64_t)b);   /* Some(bytes) */
}

/* Best-effort text password as Option<Text>. Tries UTF-16LE (interleaved NULs),
   else UTF-8; trailing NULs drop out via the NUL-terminated Text representation. */
void* certo_credential_get(certo_text_t target) {
    if (!target) return NULL;
    LPWSTR wtarget = cred_utf8_to_wide(target);
    PCREDENTIALW cred = NULL;
    BOOL ok = CredReadW(wtarget, CRED_TYPE_GENERIC, 0, &cred);
    free(wtarget);
    if (!ok || !cred) return NULL;

    DWORD n    = cred->CredentialBlobSize;
    BYTE* blob = cred->CredentialBlob;

    /* Detect ASCII UTF-16LE: even length with zero in every odd byte. */
    bool utf16 = (n >= 2) && (n % 2 == 0);
    if (utf16) {
        for (DWORD i = 1; i < n; i += 2) { if (blob[i] != 0) { utf16 = false; break; } }
    }

    char* out;
    if (utf16) {
        out = cred_wide_to_utf8_n((const WCHAR*)blob, (int)(n / 2));
    } else {
        out = (char*)malloc((size_t)n + 1);
        if (!out) certo_panic("out of memory");
        if (n) memcpy(out, blob, n);
        out[n] = 0;
    }
    CredFree(cred);
    return __certo_opt_box((int64_t)out);   /* Some(text) */
}

/* Store a Generic credential (UTF-8 blob). Returns true on success. */
bool certo_credential_set(certo_text_t target, certo_text_t secret) {
    if (!target || !secret) return false;
    LPWSTR wtarget = cred_utf8_to_wide(target);
    CREDENTIALW cred; memset(&cred, 0, sizeof(cred));
    cred.Type               = CRED_TYPE_GENERIC;
    cred.TargetName         = wtarget;
    cred.CredentialBlobSize = (DWORD)strlen(secret);
    cred.CredentialBlob     = (LPBYTE)secret;
    cred.Persist            = CRED_PERSIST_LOCAL_MACHINE;
    BOOL ok = CredWriteW(&cred, 0);
    free(wtarget);
    return ok ? true : false;
}

/* Delete a Generic credential. Returns true on success. */
bool certo_credential_delete(certo_text_t target) {
    if (!target) return false;
    LPWSTR wtarget = cred_utf8_to_wide(target);
    BOOL ok = CredDeleteW(wtarget, CRED_TYPE_GENERIC, 0);
    free(wtarget);
    return ok ? true : false;
}

#else
/* ---- POSIX stubs: no credential store, fall back to env vars ----- */
void* certo_credential_get_bytes(certo_text_t target) { (void)target; return NULL; }
void* certo_credential_get      (certo_text_t target) { (void)target; return NULL; }
bool  certo_credential_set(certo_text_t target, certo_text_t secret) { (void)target; (void)secret; return false; }
bool  certo_credential_delete(certo_text_t target) { (void)target; return false; }
#endif
"#;

pub const CREDENTIAL_CERTO: &str = r#"
module Stdlib.Credential

// Windows Credential Manager (Generic credentials). On non-Windows platforms
// every function returns None/false so callers can fall back to env vars.

/// Raw credential blob for a Generic credential `target`, or None if absent.
fn Credential.getBytes(target: Text): Bytes? [io]

/// Best-effort text secret for `target` (tries UTF-8, then UTF-16LE).
/// None if the credential is absent.
fn Credential.get(target: Text): Text? [io]

/// Store a Generic credential. Returns true on success.
fn Credential.set(target: Text, secret: Text): Bool [io]

/// Delete a Generic credential. Returns true on success.
fn Credential.delete(target: Text): Bool [io]
"#;
