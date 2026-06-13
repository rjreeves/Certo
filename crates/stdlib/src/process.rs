pub const PROCESS_C: &str = r#"
/* ================================================================
   Stdlib.Process — run external commands and capture output
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int64_t      exit_code;
    certo_text_t out;
    certo_text_t err;
} CertoProcessResult;

/* Read all of a FILE* into a heap string. */
static certo_text_t read_pipe(FILE* f) {
    if (!f) return "";
    char*  buf = NULL;
    size_t cap = 0;
    size_t len = 0;
    char   tmp[4096];
    size_t got;
    while ((got = fread(tmp, 1, sizeof(tmp), f)) > 0) {
        if (len + got + 1 > cap) {
            cap = (len + got + 1) * 2;
            char* nb = (char*)realloc(buf, cap);
            if (!nb) { free(buf); certo_panic("out of memory"); }
            buf = nb;
        }
        memcpy(buf + len, tmp, got);
        len += got;
    }
    if (!buf) { buf = (char*)malloc(1); if (!buf) certo_panic("out of memory"); }
    buf[len] = '\0';
    return buf;
}

/* Append a shell-quoted argument to buf[pos], growing buf as needed. */
static void append_arg(char** buf, size_t* cap, size_t* pos, certo_text_t arg) {
    size_t arg_len = arg ? strlen(arg) : 0;
    /* Worst case: every char is a quote => 2x + surrounding quotes + space + NUL */
    size_t need = *pos + arg_len * 2 + 4;
    if (need > *cap) {
        *cap = need * 2;
        char* nb = (char*)realloc(*buf, *cap);
        if (!nb) certo_panic("out of memory");
        *buf = nb;
    }
    (*buf)[(*pos)++] = '"';
    if (arg) {
        for (size_t i = 0; i < arg_len; i++) {
            if (arg[i] == '"') (*buf)[(*pos)++] = '\\';
            (*buf)[(*pos)++] = arg[i];
        }
    }
    (*buf)[(*pos)++] = '"';
}

CertoProcessResult* certo_process_exec(certo_text_t cmd, CertoList* args) {
    /* Build command string: cmd "arg1" "arg2" ... */
    size_t cap = 256;
    size_t pos = 0;
    char*  cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");

    append_arg(&cbuf, &cap, &pos, cmd);

    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) {
                cap *= 2;
                char* nb = (char*)realloc(cbuf, cap);
                if (!nb) certo_panic("out of memory");
                cbuf = nb;
            }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

    /* Redirect stderr to a temp file so we can capture it separately. */
    char out_tmp[64], err_tmp[64];
#ifdef _WIN32
    snprintf(out_tmp, sizeof(out_tmp), "%s\\certo_out_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount());
    snprintf(err_tmp, sizeof(err_tmp), "%s\\certo_err_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount() + 1);
#else
    snprintf(out_tmp, sizeof(out_tmp), "/tmp/certo_out_%d.tmp", (int)getpid());
    snprintf(err_tmp, sizeof(err_tmp), "/tmp/certo_err_%d.tmp", (int)getpid());
#endif

    size_t full_cap = pos + strlen(out_tmp) + strlen(err_tmp) + 32;
    char*  full_cmd = (char*)malloc(full_cap);
    if (!full_cmd) certo_panic("out of memory");
    snprintf(full_cmd, full_cap, "%s > \"%s\" 2> \"%s\"", cbuf, out_tmp, err_tmp);
    free(cbuf);

    int rc = system(full_cmd);
    free(full_cmd);

    /* Read captured output */
    FILE* fo = fopen(out_tmp, "rb");
    FILE* fe = fopen(err_tmp, "rb");
    certo_text_t out_str = read_pipe(fo);
    certo_text_t err_str = read_pipe(fe);
    if (fo) fclose(fo);
    if (fe) fclose(fe);
    remove(out_tmp);
    remove(err_tmp);

    CertoProcessResult* res = (CertoProcessResult*)malloc(sizeof(CertoProcessResult));
    if (!res) certo_panic("out of memory");
#ifdef _WIN32
    res->exit_code = (int64_t)rc;
#else
    res->exit_code = WIFEXITED(rc) ? (int64_t)WEXITSTATUS(rc) : -1;
#endif
    res->out = out_str;
    res->err = err_str;
    return res;
}

int64_t      certo_process_result_exit_code(CertoProcessResult* r) { return r ? r->exit_code : -1; }
certo_text_t certo_process_result_stdout(CertoProcessResult* r)    { return r ? r->out : ""; }
certo_text_t certo_process_result_stderr(CertoProcessResult* r)    { return r ? r->err : ""; }

/* ------------------------------------------------------------------ *
 * Process.execWithInput — like exec but writes `input` to stdin.
 * ------------------------------------------------------------------ */
CertoProcessResult* certo_process_exec_with_input(certo_text_t cmd, CertoList* args,
                                                   certo_text_t input) {
    size_t cap = 256, pos = 0;
    char* cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");
    append_arg(&cbuf, &cap, &pos, cmd);
    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) { cap *= 2; char* nb = (char*)realloc(cbuf, cap); if (!nb) certo_panic("oom"); cbuf = nb; }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

    char out_tmp[64], err_tmp[64];
#ifdef _WIN32
    snprintf(out_tmp, sizeof(out_tmp), "%s\\certo_out_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount());
    snprintf(err_tmp, sizeof(err_tmp), "%s\\certo_err_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount() + 1);
#else
    snprintf(out_tmp, sizeof(out_tmp), "/tmp/certo_out_%d.tmp", (int)getpid());
    snprintf(err_tmp, sizeof(err_tmp), "/tmp/certo_err_%d.tmp", (int)getpid());
#endif

    /* Use popen to write stdin, redirect stdout/stderr to temp files. */
    size_t full_cap = pos + strlen(out_tmp) + strlen(err_tmp) + 32;
    char* full_cmd = (char*)malloc(full_cap);
    if (!full_cmd) certo_panic("out of memory");
    snprintf(full_cmd, full_cap, "%s > \"%s\" 2> \"%s\"", cbuf, out_tmp, err_tmp);
    free(cbuf);

#ifdef _WIN32
    FILE* proc = _popen(full_cmd, "w");
#else
    FILE* proc = popen(full_cmd, "w");
#endif
    free(full_cmd);

    if (proc && input && *input) {
        fputs(input, proc);
    }

    int rc = 0;
#ifdef _WIN32
    if (proc) rc = _pclose(proc);
#else
    if (proc) rc = pclose(proc);
#endif

    FILE* fo = fopen(out_tmp, "rb");
    FILE* fe = fopen(err_tmp, "rb");
    certo_text_t out_str = read_pipe(fo);
    certo_text_t err_str = read_pipe(fe);
    if (fo) fclose(fo);
    if (fe) fclose(fe);
    remove(out_tmp);
    remove(err_tmp);

    CertoProcessResult* res = (CertoProcessResult*)malloc(sizeof(CertoProcessResult));
    if (!res) certo_panic("out of memory");
#ifdef _WIN32
    res->exit_code = (int64_t)rc;
#else
    res->exit_code = WIFEXITED(rc) ? (int64_t)WEXITSTATUS(rc) : -1;
#endif
    res->out = out_str;
    res->err = err_str;
    return res;
}

/* ------------------------------------------------------------------ *
 * Process.lines — stream stdout line-by-line to a callback.
 * handler(line: Text): Unit  called once per line, without newline.
 * Returns exit code.
 * ------------------------------------------------------------------ */
typedef void (*CertoLineHandler)(certo_text_t line, void* ctx);

int64_t certo_process_lines(certo_text_t cmd, CertoList* args,
                             CertoLineHandler handler, void* ctx) {
    size_t cap = 256, pos = 0;
    char* cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");
    append_arg(&cbuf, &cap, &pos, cmd);
    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) { cap *= 2; char* nb = (char*)realloc(cbuf, cap); if (!nb) certo_panic("oom"); cbuf = nb; }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

    /* Redirect stderr to /dev/null so only stdout streams to us. */
#ifdef _WIN32
    size_t full_cap = pos + 20;
    char* full_cmd = (char*)malloc(full_cap);
    snprintf(full_cmd, full_cap, "%s 2>NUL", cbuf);
    FILE* proc = _popen(full_cmd, "r");
#else
    size_t full_cap = pos + 20;
    char* full_cmd = (char*)malloc(full_cap);
    snprintf(full_cmd, full_cap, "%s 2>/dev/null", cbuf);
    FILE* proc = popen(full_cmd, "r");
#endif
    free(cbuf);
    free(full_cmd);

    if (!proc) return -1;

    char line_buf[4096];
    while (fgets(line_buf, sizeof(line_buf), proc)) {
        /* Strip trailing newline. */
        size_t len = strlen(line_buf);
        if (len > 0 && line_buf[len-1] == '\n') line_buf[--len] = '\0';
        if (len > 0 && line_buf[len-1] == '\r') line_buf[--len] = '\0';
        handler((certo_text_t)line_buf, ctx);
    }

    int rc = 0;
#ifdef _WIN32
    rc = _pclose(proc);
    return (int64_t)rc;
#else
    rc = pclose(proc);
    return WIFEXITED(rc) ? (int64_t)WEXITSTATUS(rc) : -1;
#endif
}

/* Certo calls this as: Process.lines(cmd, args, handler)
 * handler is a Certo fn(Text): Unit closure pointer passed as void*.
 * We wrap it so the C signature matches what Certo codegen expects. */
typedef void (*CertoClosure)(void* env, certo_text_t arg);
typedef struct { CertoClosure fn; void* env; } CertoFnText;

int64_t certo_process_lines_certo(certo_text_t cmd, CertoList* args, CertoFnText* handler) {
    /* Inline the loop so we can call the Certo closure directly. */
    size_t cap = 256, pos = 0;
    char* cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");
    append_arg(&cbuf, &cap, &pos, cmd);
    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) { cap *= 2; char* nb = (char*)realloc(cbuf, cap); if (!nb) certo_panic("oom"); cbuf = nb; }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

#ifdef _WIN32
    size_t full_cap = pos + 20;
    char* full_cmd = (char*)malloc(full_cap);
    snprintf(full_cmd, full_cap, "%s 2>NUL", cbuf);
    FILE* proc = _popen(full_cmd, "r");
#else
    size_t full_cap = pos + 20;
    char* full_cmd = (char*)malloc(full_cap);
    snprintf(full_cmd, full_cap, "%s 2>/dev/null", cbuf);
    FILE* proc = popen(full_cmd, "r");
#endif
    free(cbuf);
    free(full_cmd);
    if (!proc) return -1;

    char line_buf[4096];
    while (fgets(line_buf, sizeof(line_buf), proc)) {
        size_t len = strlen(line_buf);
        if (len > 0 && line_buf[len-1] == '\n') line_buf[--len] = '\0';
        if (len > 0 && line_buf[len-1] == '\r') line_buf[--len] = '\0';
        char* line_copy = (char*)malloc(len + 1);
        if (!line_copy) certo_panic("out of memory");
        memcpy(line_copy, line_buf, len + 1);
        handler->fn(handler->env, (certo_text_t)line_copy);
    }

    int rc = 0;
#ifdef _WIN32
    rc = _pclose(proc);
    return (int64_t)rc;
#else
    rc = pclose(proc);
    return WIFEXITED(rc) ? (int64_t)WEXITSTATUS(rc) : -1;
#endif
}
"#;

pub const PROCESS_CERTO: &str = r#"
module Stdlib.Process

/// Run an external command with the given arguments.
/// Returns a ProcessResult with exitCode, stdout, and stderr captured.
fn Process.exec(cmd: Text, args: List<Text>): ProcessResult [io]

/// Run an external command, writing `input` to its stdin.
/// Returns a ProcessResult with exitCode, stdout, and stderr captured.
fn Process.execWithInput(cmd: Text, args: List<Text>, input: Text): ProcessResult [io]

/// Stream stdout of an external command line by line.
/// `handler` is called once per line (newline stripped).
/// Returns the exit code.
fn Process.lines(cmd: Text, args: List<Text>, handler: fn(Text): Unit): Int [io]

/// Exit code from a ProcessResult.
fn ProcessResult.exitCode(r: ProcessResult): Int

/// Captured standard output from a ProcessResult.
fn ProcessResult.stdout(r: ProcessResult): Text

/// Captured standard error from a ProcessResult.
fn ProcessResult.stderr(r: ProcessResult): Text
"#;
