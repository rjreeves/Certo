pub const ENV_C: &str = r#"
/* ================================================================
   Stdlib.Env — environment variable access
   ================================================================ */

#include <stdlib.h>
#include <string.h>

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
