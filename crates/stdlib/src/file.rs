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

/* ---- File: open-handle API (BACKLOG item 192) ----
   `CertoFile` is an opaque `FILE*`, same `void*`-handle convention as
   `__CertoTask`/`Channel` — no wrapper struct needed since a C pointer is
   already pointer-sized. Opens in "r+b" (read/write, must already exist,
   no truncation) rather than a fixed read-only or write-only mode, so the
   one handle a `use file = File.open(path) { ... }` block gets can serve
   either `.readAll()` or `.write(...)` depending on what the caller does
   with it. Failure (missing file, permissions) is `None`, matching every
   sibling function in this file (`certo_read_file`/`certo_write_file`) —
   deliberately not a new `Result<File, IOError>`, since no `IOError` type
   exists anywhere in this codebase and every other file function already
   uses this same `Option`/`Bool` convention. */
typedef void* CertoFile;

void* certo_file_open(certo_text_t path) {   /* Option<File> */
    if (!path) return NULL;
    FILE* f = fopen(path, "r+b");
    if (!f) return NULL;
    return __certo_opt_box((int64_t)f);   /* Some(file) */
}

/* Reads everything from the current position to end-of-file (not the whole
   file from the start — a second `.readAll()` on the same handle correctly
   returns "" once already at EOF, same "read what's left" semantics a real
   file handle should have). `File.open`'s "r+b" mode means this same
   stream may also be written to, and C's stdio requires an intervening
   file-positioning call between a write and a following read on such a
   stream (C99 §7.19.5.3) — the leading no-op `fseek(f, 0, SEEK_CUR)`
   satisfies that unconditionally, regardless of what the previous
   operation on this handle was. */
void* certo_file_read_all(CertoFile file) {   /* Option<Text> */
    FILE* f = (FILE*)file;
    if (!f) return NULL;
    fseek(f, 0, SEEK_CUR);
    long cur = ftell(f);
    if (cur < 0) return NULL;
    fseek(f, 0, SEEK_END);
    long end = ftell(f);
    fseek(f, cur, SEEK_SET);
    if (end < cur) return NULL;
    size_t remaining = (size_t)(end - cur);
    char* buf = (char*)malloc(remaining + 1);
    if (!buf) certo_panic("out of memory");
    size_t got = fread(buf, 1, remaining, f);
    buf[got] = '\0';
    return __certo_opt_box((int64_t)buf);   /* Some(contents) */
}

/* Writes at the handle's current position; returns true on success. Same
   read/write-mixing requirement as `certo_file_read_all` above, in the
   other direction — without this leading `fseek`, a write immediately
   following a read on the same "r+b" stream is undefined behavior per
   C99 §7.19.5.3, confirmed to actually lose the write silently (not just
   a theoretical concern) when caught by direct testing before this was
   added: reading a file's existing content then writing more to it
   reported success but left the file completely unchanged. */
bool certo_file_write(CertoFile file, certo_text_t content) {
    FILE* f = (FILE*)file;
    if (!f || !content) return false;
    fseek(f, 0, SEEK_CUR);
    size_t len = strlen(content);
    return fwrite(content, 1, len, f) == len;
}

int64_t certo_file_close(CertoFile file) {
    FILE* f = (FILE*)file;
    if (f) fclose(f);
    return 0;
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
