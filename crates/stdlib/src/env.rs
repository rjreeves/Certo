pub const ENV_C: &str = r#"
/* ================================================================
   Stdlib.Env — environment variable access
   ================================================================ */

#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#include <errno.h>
#endif

certo_text_t certo_get_current_dir(void) {
#ifdef _WIN32
    DWORD needed = GetCurrentDirectoryA(0, NULL);
    if (needed == 0) certo_panic("failed to get current directory");
    char* buf = (char*)malloc((size_t)needed);
    if (!buf) certo_panic("out of memory");
    DWORD got = GetCurrentDirectoryA(needed, buf);
    if (got == 0 || got >= needed) { free(buf); certo_panic("failed to get current directory"); }
    return buf;
#else
    size_t cap = 256;
    char* buf = (char*)malloc(cap);
    if (!buf) certo_panic("out of memory");
    while (!getcwd(buf, cap)) {
        if (errno != ERANGE) { free(buf); certo_panic("failed to get current directory"); }
        cap *= 2;
        char* nb = (char*)realloc(buf, cap);
        if (!nb) { free(buf); certo_panic("out of memory"); }
        buf = nb;
    }
    return buf;
#endif
}

void* certo_get_env(certo_text_t key) {   /* Option<Text> */
    if (!key) return NULL;
    char* val = getenv(key);
    if (!val) return NULL;
    size_t len = strlen(val) + 1;
    char* copy = (char*)malloc(len);
    if (!copy) certo_panic("out of memory");
    memcpy(copy, val, len);
    return __certo_opt_box((int64_t)copy);   /* Some(value) */
}

int64_t certo_set_env(certo_text_t key, certo_text_t val) {
    if (!key || !val) return 0;
#ifdef _WIN32
    _putenv_s(key, val);
#else
    setenv(key, val, 1);
#endif
    return 0;
}

int64_t certo_unset_env(certo_text_t key) {
    if (!key) return 0;
#ifdef _WIN32
    _putenv_s(key, "");
#else
    unsetenv(key);
#endif
    return 0;
}
"#;
