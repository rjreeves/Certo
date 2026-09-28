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
#include <windows.h>
#define CERTO_MKDIR(p) _mkdir(p)
#else
#include <sys/stat.h>
#include <unistd.h>
#include <dirent.h>
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

/* BACKLOG item 329 — tells a directory entry apart from a file. The
   underlying check already existed internally (certo_remove_dir_all_impl
   above uses the identical stat/S_ISDIR and GetFileAttributes checks to
   decide whether to recurse into a child entry or remove() it directly);
   this just exposes that same classification as a public builtin. Returns
   false (not an error signal) for a path that doesn't exist at all, same
   convention as certo_file_exists just above. */
bool certo_is_directory(certo_text_t path) {
    if (!path || !*path) return false;
#ifdef _WIN32
    DWORD attrs = GetFileAttributesA(path);
    return attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0;
#else
    struct stat st;
    return stat(path, &st) == 0 && S_ISDIR(st.st_mode);
#endif
}

/* Delete a file; returns true on success. */
bool certo_delete_file(certo_text_t path) {
    if (!path) return false;
    return remove(path) == 0;
}

/* Rename/move `from` to `to`, replacing `to` if it already exists.
 * Atomic when both paths are on the same volume/filesystem -- the
 * write-temp-file-then-rename pattern for safely updating a file other
 * processes might be reading concurrently depends on that guarantee, so
 * callers relying on atomicity must keep `from` and `to` on the same
 * volume (e.g. the same directory) themselves; this function does not
 * check or enforce that. */
bool certo_rename_file(certo_text_t from, certo_text_t to) {
    if (!from || !to) return false;
#ifdef _WIN32
    return MoveFileExA(from, to, MOVEFILE_REPLACE_EXISTING) != 0;
#else
    return rename(from, to) == 0;
#endif
}

/* Recursively remove a directory and everything inside it (like `rm -rf`
 * for directories). Keeps going and reports overall failure rather than
 * stopping at the first error, so a partially-removable tree still gets
 * cleaned up as much as possible. */
static bool certo_remove_dir_all_impl(const char* path) {
    bool ok = true;
#ifdef _WIN32
    char pattern[4096];
    snprintf(pattern, sizeof(pattern), "%s\\*", path);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) return false;
    do {
        if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0) continue;
        char child[4096];
        snprintf(child, sizeof(child), "%s\\%s", path, fd.cFileName);
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
            if (!certo_remove_dir_all_impl(child)) ok = false;
        } else {
            if (!DeleteFileA(child)) ok = false;
        }
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    if (!RemoveDirectoryA(path)) ok = false;
#else
    DIR* d = opendir(path);
    if (!d) return false;
    struct dirent* entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        char child[4096];
        snprintf(child, sizeof(child), "%s/%s", path, entry->d_name);
        struct stat st;
        if (stat(child, &st) == 0 && S_ISDIR(st.st_mode)) {
            if (!certo_remove_dir_all_impl(child)) ok = false;
        } else {
            if (remove(child) != 0) ok = false;
        }
    }
    closedir(d);
    if (rmdir(path) != 0) ok = false;
#endif
    return ok;
}

bool certo_remove_dir(certo_text_t path) {
    if (!path || !*path) return false;
    return certo_remove_dir_all_impl(path);
}

/* Remove a directory only if it's already empty — a single, atomic OS call
   (rmdir/RemoveDirectoryA both refuse outright on a non-empty directory, no
   listing required), unlike removeDir's own unconditional recursion. Exists
   because a caller wanting "only if empty" semantics had no atomic way to
   ask for that: listing entries first and calling removeDir only when the
   list comes back empty (Lume's own fs.remove_dir(path, false), the
   motivating case) is a real TOCTOU race — something else can add a file
   to the directory between the list and the removeDir call, and since
   removeDir always recurses, it would then silently delete that file too. */
bool certo_remove_empty_dir(certo_text_t path) {
    if (!path || !*path) return false;
#ifdef _WIN32
    return RemoveDirectoryA(path) != 0;
#else
    return rmdir(path) == 0;
#endif
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
