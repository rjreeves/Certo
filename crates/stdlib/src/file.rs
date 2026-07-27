pub const FILE_C: &str = r#"
/* ================================================================
   Stdlib.File — file I/O and directory listing
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#ifdef _WIN32
#include <direct.h>
#define CERTO_MKDIR(p) _mkdir(p)
#else
#include <sys/stat.h>
#define CERTO_MKDIR(p) mkdir(p, 0777)
#endif

/* Create a directory and any missing parents (mkdir -p). Accepts '/' or '\\'
   separators. Returns true if the directory exists afterwards. */
bool certo_make_dir(certo_text_t path) {
    if (!path || !*path) return false;
    size_t n = strlen(path);
    char* tmp = (char*)malloc(n + 1);
    if (!tmp) certo_panic("out of memory");
    memcpy(tmp, path, n + 1);
    for (size_t i = 1; i < n; i++) {
        if (tmp[i] == '/' || tmp[i] == '\\') {
            char c = tmp[i];
            tmp[i] = '\0';
            CERTO_MKDIR(tmp);            /* ignore intermediate errors (already exists) */
            tmp[i] = c;
        }
    }
    int r = CERTO_MKDIR(tmp);
    bool ok = (r == 0) || (errno == EEXIST);
    free(tmp);
    return ok;
}

/* Returns file contents as a heap string, or NULL on error. */
void* certo_read_file(certo_text_t path) {   /* Option<Text> */
    if (!path) return NULL;
    FILE* f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < 0) { fclose(f); return NULL; }
    char* buf = (char*)malloc((size_t)size + 1);
    if (!buf) { fclose(f); certo_panic("out of memory"); }
    size_t got = fread(buf, 1, (size_t)size, f);
    buf[got] = '\0';
    fclose(f);
    return __certo_opt_box((int64_t)buf);   /* Some(contents) */
}

/* Write text to path; returns true on success. */
bool certo_write_file(certo_text_t path, certo_text_t content) {
    if (!path || !content) return false;
    FILE* f = fopen(path, "wb");
    if (!f) return false;
    size_t len = strlen(content);
    bool ok = fwrite(content, 1, len, f) == len;
    fclose(f);
    return ok;
}

/* Append text to path; returns true on success. */
bool certo_append_file(certo_text_t path, certo_text_t content) {
    if (!path || !content) return false;
    FILE* f = fopen(path, "ab");
    if (!f) return false;
    size_t len = strlen(content);
    bool ok = fwrite(content, 1, len, f) == len;
    fclose(f);
    return ok;
}

bool certo_file_exists(certo_text_t path) {
    if (!path) return false;
    FILE* f = fopen(path, "rb");
    if (!f) return false;
    fclose(f);
    return true;
}

/* Delete a file; returns true on success. */
bool certo_delete_file(certo_text_t path) {
    if (!path) return false;
    return remove(path) == 0;
}

/* List directory entries (excluding . and ..).
   Returns a List<Text> or NULL on error. */
void* certo_list_dir(certo_text_t path) {   /* Option<List<Text>> */
    if (!path) return NULL;
    CertoList* out = certo_list_new_empty();
#ifdef _WIN32
    char pattern[4096];
    snprintf(pattern, sizeof(pattern), "%s\\*", path);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) return NULL;
    do {
        if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0) continue;
        size_t len = strlen(fd.cFileName) + 1;
        char* name = (char*)malloc(len);
        if (!name) { FindClose(h); certo_panic("out of memory"); }
        memcpy(name, fd.cFileName, len);
        out = certo_list_push(out, name);
    } while (FindNextFileA(h, &fd));
    FindClose(h);
#else
    DIR* d = opendir(path);
    if (!d) return NULL;
    struct dirent* entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        size_t len = strlen(entry->d_name) + 1;
        char* name = (char*)malloc(len);
        if (!name) { closedir(d); certo_panic("out of memory"); }
        memcpy(name, entry->d_name, len);
        out = certo_list_push(out, name);
    }
    closedir(d);
#endif
    return __certo_opt_box((int64_t)out);   /* Some(entries) */
}
"#;
