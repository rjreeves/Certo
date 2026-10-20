pub const PROCESS_C: &str = r#"
/* ================================================================
   Stdlib.Process — run external commands and capture output
   ================================================================ */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#include <sys/wait.h>
#include <signal.h>
#include <time.h>
#include <fcntl.h>
#endif

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
#ifdef _WIN32
        size_t slashes = 0;
        for (size_t i = 0; i < arg_len; i++) {
            if (arg[i] == '\\') {
                slashes++;
            } else if (arg[i] == '"') {
                while (slashes > 0) {
                    slashes--;
                    (*buf)[(*pos)++] = '\\';
                    (*buf)[(*pos)++] = '\\';
                }
                (*buf)[(*pos)++] = '\\';
                (*buf)[(*pos)++] = '"';
                slashes = 0;
            } else {
                while (slashes > 0) {
                    slashes--;
                    (*buf)[(*pos)++] = '\\';
                }
                (*buf)[(*pos)++] = arg[i];
                slashes = 0;
            }
        }
        /* Backslashes before the closing quote must be doubled. */
        while (slashes > 0) {
            slashes--;
            (*buf)[(*pos)++] = '\\';
            (*buf)[(*pos)++] = '\\';
        }
#else
        for (size_t i = 0; i < arg_len; i++) {
            if (arg[i] == '"') (*buf)[(*pos)++] = '\\';
            (*buf)[(*pos)++] = arg[i];
        }
#endif
    }
    (*buf)[(*pos)++] = '"';
}

#ifdef _WIN32
/* Run `command` through the command interpreter exactly as system() does
   (`cmd.exe /c <command>`, same quote-stripping rules), but without letting
   Windows create a visible console window — BACKLOG item 427.

   system() starts cmd.exe with no creation flags, so when the calling process
   has no console of its own (a `--windows-gui` app, a service) Windows gives
   the child a brand-new console window that flashes up and disappears. That
   is what made a GUI installer calling Process.exec three times flash two or
   three black consoles. CREATE_NO_WINDOW runs the same cmd.exe invisibly.
   stdout/stderr need no handle of their own because the command string
   redirects both to files; stdin still reaches the child when the parent has
   one. Returns cmd.exe's exit code, or -1 if it could not be started. */
static int certo_system_hidden(const char* command) {
    const char* comspec = getenv("COMSPEC");
    if (!comspec || !*comspec) comspec = "cmd.exe";
    size_t n = strlen(command) + 32;
    char* line = (char*)malloc(n);
    if (!line) certo_panic("out of memory");
    snprintf(line, n, "cmd.exe /c %s", command);

    SECURITY_ATTRIBUTES sa;
    memset(&sa, 0, sizeof(sa));
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;
    HANDLE hNul = CreateFileA("NUL", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                              &sa, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);

    /* Hand the child our stdin if we have a usable one (made inheritable via
       a duplicate, so our own handle's flags are left alone); otherwise NUL. */
    HANDLE hIn = hNul;
    HANDLE hInDup = NULL;
    HANDLE hOwnIn = GetStdHandle(STD_INPUT_HANDLE);
    if (hOwnIn && hOwnIn != INVALID_HANDLE_VALUE &&
        DuplicateHandle(GetCurrentProcess(), hOwnIn, GetCurrentProcess(), &hInDup, 0, TRUE, DUPLICATE_SAME_ACCESS)) {
        hIn = hInDup;
    }

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    memset(&pi, 0, sizeof(pi));
    si.cb = sizeof(si);
    si.dwFlags    = STARTF_USESTDHANDLES;
    si.hStdInput  = hIn;
    si.hStdOutput = hNul;
    si.hStdError  = hNul;

    BOOL ok = CreateProcessA(comspec, line, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(line);
    if (hInDup) CloseHandle(hInDup);
    if (hNul != INVALID_HANDLE_VALUE) CloseHandle(hNul);
    if (!ok) return -1;

    CloseHandle(pi.hThread);
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    return (int)code;
}
#endif

#ifdef _WIN32
#include <io.h>
#include <fcntl.h>

/* BACKLOG item 428 — the pipe-based Process APIs (`execWithInput`, `lines`) used
   _popen, which starts `cmd.exe` with no way to pass CREATE_NO_WINDOW, so from a
   GUI-subsystem program every call flashed a console window (item 427 fixed the same
   thing for `exec`/`run`). These helpers start the same `cmd.exe /c <command>` as
   certo_system_hidden — hidden, same quoting — but with an optional pipe on the
   child's stdin (`stdin_pipe`: the caller writes to `pipe`) or stdout (`stdout_pipe`:
   the caller reads from `pipe`), at most one. stderr is NUL (the command redirects it).
   Returns 0, or -1 if the process could not be started. */
typedef struct { HANDLE process; HANDLE pipe; } CertoHiddenProc;

static int certo_hidden_start(const char* command, int stdin_pipe, int stdout_pipe, CertoHiddenProc* h) {
    const char* comspec = getenv("COMSPEC");
    if (!comspec || !*comspec) comspec = "cmd.exe";
    size_t n = strlen(command) + 32;
    char* line = (char*)malloc(n);
    if (!line) certo_panic("out of memory");
    snprintf(line, n, "cmd.exe /c %s", command);

    SECURITY_ATTRIBUTES sa;
    memset(&sa, 0, sizeof(sa));
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;
    HANDLE hNul = CreateFileA("NUL", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                              &sa, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);

    HANDLE hIn = hNul, hOut = hNul, hInDup = NULL;
    HANDLE pipe_child = NULL, pipe_parent = NULL;
    if (stdin_pipe || stdout_pipe) {
        HANDLE rd = NULL, wr = NULL;
        if (!CreatePipe(&rd, &wr, &sa, 0)) {
            free(line);
            if (hNul != INVALID_HANDLE_VALUE) CloseHandle(hNul);
            return -1;
        }
        if (stdin_pipe) { pipe_child = rd; pipe_parent = wr; hIn = rd; }
        else            { pipe_child = wr; pipe_parent = rd; hOut = wr; }
        /* Our end must not be inherited: a child that held it open would keep the
           pipe from ever reaching end-of-file. */
        SetHandleInformation(pipe_parent, HANDLE_FLAG_INHERIT, 0);
    }
    if (!stdin_pipe) {
        /* Hand the child our stdin when we have a usable one, as certo_system_hidden does. */
        HANDLE hOwnIn = GetStdHandle(STD_INPUT_HANDLE);
        if (hOwnIn && hOwnIn != INVALID_HANDLE_VALUE &&
            DuplicateHandle(GetCurrentProcess(), hOwnIn, GetCurrentProcess(), &hInDup, 0, TRUE, DUPLICATE_SAME_ACCESS)) {
            hIn = hInDup;
        }
    }

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    memset(&pi, 0, sizeof(pi));
    si.cb = sizeof(si);
    si.dwFlags    = STARTF_USESTDHANDLES;
    si.hStdInput  = hIn;
    si.hStdOutput = hOut;
    si.hStdError  = hNul;

    BOOL ok = CreateProcessA(comspec, line, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(line);
    if (hInDup) CloseHandle(hInDup);
    if (pipe_child) CloseHandle(pipe_child);
    if (hNul != INVALID_HANDLE_VALUE) CloseHandle(hNul);
    if (!ok) {
        if (pipe_parent) CloseHandle(pipe_parent);
        return -1;
    }
    CloseHandle(pi.hThread);
    h->process = pi.hProcess;
    h->pipe    = pipe_parent;
    return 0;
}

/* Close the pipe if still open, wait for the child, return its exit code. */
static int certo_hidden_finish(CertoHiddenProc* h) {
    if (h->pipe) { CloseHandle(h->pipe); h->pipe = NULL; }
    WaitForSingleObject(h->process, INFINITE);
    DWORD code = 0;
    GetExitCodeProcess(h->process, &code);
    CloseHandle(h->process);
    h->process = NULL;
    return (int)code;
}

/* _popen(command, "r") without the console window: a text-mode stream over the child's
   stdout, and the process handle to give back to certo_pclose_hidden. NULL on failure. */
static FILE* certo_popen_hidden_r(const char* command, HANDLE* proc) {
    CertoHiddenProc h = { NULL, NULL };
    if (certo_hidden_start(command, 0, 1, &h) != 0) return NULL;
    int fd = _open_osfhandle((intptr_t)h.pipe, _O_RDONLY | _O_TEXT);
    FILE* f = fd < 0 ? NULL : _fdopen(fd, "r");
    if (!f) {
        if (fd < 0) { CloseHandle(h.pipe); }
        h.pipe = NULL;
        certo_hidden_finish(&h);
        return NULL;
    }
    *proc = h.process;
    return f;
}

/* _pclose for certo_popen_hidden_r: close the stream (and with it the pipe), then
   wait for the child and return its exit code. */
static int certo_pclose_hidden(FILE* f, HANDLE process) {
    fclose(f);
    CertoHiddenProc h = { process, NULL };
    return certo_hidden_finish(&h);
}

/* Write `text` to the child's stdin pipe the way fputs on a text-mode _popen stream did:
   each LF becomes CRLF. */
static void certo_hidden_write_text(HANDLE pipe, const char* text) {
    size_t len = strlen(text);
    char* buf = (char*)malloc(len * 2 + 1);
    if (!buf) certo_panic("out of memory");
    size_t n = 0;
    for (size_t i = 0; i < len; i++) {
        if (text[i] == '\n' && (i == 0 || text[i - 1] != '\r')) buf[n++] = '\r';
        buf[n++] = text[i];
    }
    size_t off = 0;
    while (off < n) {
        DWORD w = 0;
        if (!WriteFile(pipe, buf + off, (DWORD)(n - off), &w, NULL) || w == 0) break;
        off += w;
    }
    free(buf);
}
#endif

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

    size_t full_cap = pos + strlen(out_tmp) + strlen(err_tmp) + 36;
    char*  full_cmd = (char*)malloc(full_cap);
    if (!full_cmd) certo_panic("out of memory");
#ifdef _WIN32
    snprintf(full_cmd, full_cap, "\"%s > \"%s\" 2> \"%s\"\"", cbuf, out_tmp, err_tmp);
#else
    snprintf(full_cmd, full_cap, "%s > \"%s\" 2> \"%s\"", cbuf, out_tmp, err_tmp);
#endif
    free(cbuf);

#ifdef _WIN32
    int rc = certo_system_hidden(full_cmd);
#else
    int rc = system(full_cmd);
#endif
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
 * Process.execInherit — run a command with stdin/stdout/stderr
 * connected directly to this process's own handles (no capture, no
 * buffering). Built for shims: the child sees a real console, prompts
 * and progress bars work, and the exact exit code comes back.
 * ------------------------------------------------------------------ */
int64_t certo_process_exec_inherit(certo_text_t cmd, CertoList* args, certo_text_t working_dir) {
#ifdef _WIN32
    size_t cap = 256, pos = 0;
    char* cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");
    append_arg(&cbuf, &cap, &pos, cmd);
    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) { cap *= 2; char* nb = (char*)realloc(cbuf, cap); if (!nb) certo_panic("out of memory"); cbuf = nb; }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    memset(&pi, 0, sizeof(pi));
    si.cb = sizeof(si);
    si.dwFlags    = STARTF_USESTDHANDLES;
    si.hStdInput  = GetStdHandle(STD_INPUT_HANDLE);
    si.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
    si.hStdError  = GetStdHandle(STD_ERROR_HANDLE);

    BOOL ok = CreateProcessA(
        NULL, cbuf, NULL, NULL,
        TRUE  /* inherit handles, incl. stdio set above */,
        0, NULL, (working_dir && working_dir[0]) ? working_dir : NULL, &si, &pi
    );
    free(cbuf);

    if (!ok) {
        fprintf(stderr, "failed to launch '%s' (error %lu)\n", cmd, (unsigned long)GetLastError());
        return 127;
    }
    CloseHandle(pi.hThread);
    /* The child shares this console, so Ctrl+C reaches the child AND us. A
     * launcher must not die first: a child that handles Ctrl+C itself (a
     * shell, a REPL) would be left running with nothing waiting on it, still
     * reading the terminal, and its exit status lost. Ignore Ctrl+C while we
     * wait and let the child decide; we then return whatever it exits with.
     * This is done AFTER CreateProcess on purpose: the "ignore Ctrl+C" flag
     * is inherited by processes created while it is set, which would stop a
     * child that relies on the default from dying. Ctrl+Break is left alone
     * as the way to force-stop a launcher. */
    SetConsoleCtrlHandler(NULL, TRUE);
    WaitForSingleObject(pi.hProcess, INFINITE);
    SetConsoleCtrlHandler(NULL, FALSE);
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    return (int64_t)code;
#else
    int64_t argc = 1 + (args ? args->len : 0);
    char** argv = (char**)malloc(sizeof(char*) * (size_t)(argc + 1));
    if (!argv) certo_panic("out of memory");
    argv[0] = (char*)cmd;
    for (int64_t i = 0; i < (args ? args->len : 0); i++) {
        argv[1 + i] = (char*)args->data[i];
    }
    argv[argc] = NULL;

    pid_t pid = fork();
    if (pid < 0) {
        free(argv);
        return -1;
    }
    if (pid == 0) {
        if (working_dir && working_dir[0] && chdir(working_dir) != 0) _exit(127);
        execvp(cmd, argv);
        _exit(127); /* only reached if exec failed */
    }
    free(argv);
    /* Same reasoning as the Windows branch: Ctrl+C (SIGINT) and Ctrl+\ (SIGQUIT)
     * go to the whole foreground process group, child included, so ignore them
     * here while we wait -- as system(3) does -- and restore afterwards. Done
     * after fork() so the child keeps the default disposition. */
    struct sigaction ign, old_int, old_quit;
    memset(&ign, 0, sizeof(ign));
    ign.sa_handler = SIG_IGN;
    sigemptyset(&ign.sa_mask);
    sigaction(SIGINT, &ign, &old_int);
    sigaction(SIGQUIT, &ign, &old_quit);
    int status = 0;
    waitpid(pid, &status, 0);
    sigaction(SIGINT, &old_int, NULL);
    sigaction(SIGQUIT, &old_quit, NULL);
    return WIFEXITED(status) ? (int64_t)WEXITSTATUS(status) : -1;
#endif
}

int64_t certo_process_quit(int64_t code) {
    exit((int)code);
    return 0;
}

static int64_t certo_process_spawn_detached_with_flags(certo_text_t cmd, CertoList* args, certo_text_t working_dir, uint32_t flags) {
#ifdef _WIN32
    size_t cap = 256;
    size_t pos = 0;
    char* cbuf = (char*)malloc(cap);
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

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    memset(&pi, 0, sizeof(pi));
    si.cb = sizeof(si);

    BOOL ok = CreateProcessA(
        NULL,
        cbuf,
        NULL,
        NULL,
        FALSE,
        flags,
        NULL,
        (working_dir && *working_dir) ? working_dir : NULL,
        &si,
        &pi
    );
    free(cbuf);

    if (!ok) return (int64_t)GetLastError();
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return 0;
#else
    /* BACKLOG item 333 — this used to be a stub that discarded every
       argument and always returned -1, never actually spawning anything.
       `flags` has no POSIX equivalent (it only ever carries Windows'
       CREATE_NEW_CONSOLE/CREATE_NO_WINDOW) so it's intentionally unused
       here. A real detached spawn needs the standard double-fork
       idiom: forking once and having that first child fork *again* then
       exit immediately reparents the real grandchild to init (or the
       nearest subreaper), so it never becomes a zombie under *this*
       process once it exits — unlike a single fork with no wait() at
       all, which would leak a zombie per spawnDetached call for the
       calling program's entire remaining lifetime. The short-lived first
       child is reaped immediately below via waitpid, so this returns
       promptly, not after the spawned command finishes. */
    (void)flags;
    int64_t argc = 1 + (args ? args->len : 0);
    char** argv = (char**)malloc(sizeof(char*) * (size_t)(argc + 1));
    if (!argv) certo_panic("out of memory");
    argv[0] = (char*)cmd;
    for (int64_t i = 0; i < (args ? args->len : 0); i++) argv[1 + i] = (char*)args->data[i];
    argv[argc] = NULL;

    pid_t first = fork();
    if (first < 0) { free(argv); return -1; }
    if (first == 0) {
        pid_t grandchild = fork();
        if (grandchild < 0) _exit(1);
        if (grandchild == 0) {
            setsid();   /* detach from the parent's session/controlling terminal */
            if (working_dir && *working_dir && chdir(working_dir) != 0) _exit(127);
            /* No stdio handle to hand back to on a fire-and-forget spawn —
               redirect to /dev/null so the child never blocks writing to
               (or reading from) a terminal that may no longer exist by the
               time it runs. */
            int devnull = open("/dev/null", O_RDWR);
            if (devnull >= 0) {
                dup2(devnull, STDIN_FILENO);
                dup2(devnull, STDOUT_FILENO);
                dup2(devnull, STDERR_FILENO);
            }
            execvp(cmd, argv);
            _exit(127);
        }
        _exit(0);   /* first child's only job was forking the grandchild */
    }
    free(argv);
    int status = 0;
    waitpid(first, &status, 0);   /* reap the short-lived first child */
    return 0;
#endif
}

int64_t certo_process_spawn_detached(certo_text_t cmd, CertoList* args, certo_text_t working_dir) {
#ifdef _WIN32
    return certo_process_spawn_detached_with_flags(cmd, args, working_dir, CREATE_NEW_CONSOLE);
#else
    return certo_process_spawn_detached_with_flags(cmd, args, working_dir, 0);
#endif
}

int64_t certo_process_spawn_detached_hidden(certo_text_t cmd, CertoList* args, certo_text_t working_dir) {
#ifdef _WIN32
    return certo_process_spawn_detached_with_flags(cmd, args, working_dir, CREATE_NO_WINDOW);
#else
    return certo_process_spawn_detached_with_flags(cmd, args, working_dir, 0);
#endif
}

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
    size_t full_cap = pos + strlen(out_tmp) + strlen(err_tmp) + 36;
    char* full_cmd = (char*)malloc(full_cap);
    if (!full_cmd) certo_panic("out of memory");
#ifdef _WIN32
    snprintf(full_cmd, full_cap, "\"%s > \"%s\" 2> \"%s\"\"", cbuf, out_tmp, err_tmp);
#else
    snprintf(full_cmd, full_cap, "%s > \"%s\" 2> \"%s\"", cbuf, out_tmp, err_tmp);
#endif
    free(cbuf);

    int rc = 0;
#ifdef _WIN32
    /* BACKLOG item 428: a hidden child with a stdin pipe instead of _popen. */
    {
        CertoHiddenProc hp = { NULL, NULL };
        if (certo_hidden_start(full_cmd, 1, 0, &hp) == 0) {
            if (input && *input) certo_hidden_write_text(hp.pipe, input);
            rc = certo_hidden_finish(&hp);   /* closes the pipe (end of input), then waits */
        } else {
            rc = -1;
        }
    }
    free(full_cmd);
#else
    FILE* proc = popen(full_cmd, "w");
    free(full_cmd);

    if (proc && input && *input) {
        fputs(input, proc);
    }
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
typedef void (*CertoLineHandler)(void* ctx, certo_text_t line);

int64_t certo_process_lines(certo_text_t cmd, CertoList* args, certo_fn_t handler) {
    CertoLineHandler fn = (CertoLineHandler)handler.fn;
    void* ctx = handler.env;
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
    HANDLE hproc = NULL;   /* BACKLOG item 428: hidden, not _popen */
    FILE* proc = certo_popen_hidden_r(full_cmd, &hproc);
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
        fn(ctx, (certo_text_t)line_buf);
    }

    int rc = 0;
#ifdef _WIN32
    rc = certo_pclose_hidden(proc, hproc);
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
    HANDLE hproc = NULL;   /* BACKLOG item 428: hidden, not _popen */
    FILE* proc = certo_popen_hidden_r(full_cmd, &hproc);
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
    rc = certo_pclose_hidden(proc, hproc);
    return (int64_t)rc;
#else
    rc = pclose(proc);
    return WIFEXITED(rc) ? (int64_t)WEXITSTATUS(rc) : -1;
#endif
}

/* ------------------------------------------------------------------ *
 * Process.run — BACKLOG item 330. The one working-directory-aware call
 * (spawnDetached) is fire-and-forget with no output capture, and none of
 * the output-capturing exec family accepts a working directory or a
 * timeout. This is the missing combination: a captured, cancellable,
 * cwd-aware exec. `workingDir`/`timeoutMs` follow the exact same
 * sentinel-value convention `spawnDetached`'s own `working_dir` already
 * uses (an empty string means "don't change directory") — `timeoutMs <= 0`
 * means "no timeout, wait forever", matching every other exec variant's
 * existing blocking behavior exactly when the feature isn't used. A timed-
 * out process is killed and reported as exit code -1 — the same sentinel
 * `WIFEXITED(...) ? ... : -1` already reports for a signal-terminated
 * child elsewhere in this file, so callers don't need a third case.
 * ------------------------------------------------------------------ */
#ifndef _WIN32
static int64_t monotonic_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}
#endif

CertoProcessResult* certo_process_run(certo_text_t cmd, CertoList* args,
                                       certo_text_t working_dir, int64_t timeout_ms) {
    char out_tmp[64], err_tmp[64];
    bool timed_out = false;
    int64_t exit_code;

#ifdef _WIN32
    snprintf(out_tmp, sizeof(out_tmp), "%s\\certo_out_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount());
    snprintf(err_tmp, sizeof(err_tmp), "%s\\certo_err_%u.tmp", getenv("TEMP") ? getenv("TEMP") : ".", (unsigned)GetTickCount() + 1);

    size_t cap = 256, pos = 0;
    char* cbuf = (char*)malloc(cap);
    if (!cbuf) certo_panic("out of memory");
    append_arg(&cbuf, &cap, &pos, cmd);
    if (args) {
        for (int64_t i = 0; i < args->len; i++) {
            cbuf[pos++] = ' ';
            if (pos + 4 > cap) { cap *= 2; char* nb = (char*)realloc(cbuf, cap); if (!nb) certo_panic("out of memory"); cbuf = nb; }
            append_arg(&cbuf, &cap, &pos, (certo_text_t)args->data[i]);
        }
    }
    cbuf[pos] = '\0';

    SECURITY_ATTRIBUTES sa;
    memset(&sa, 0, sizeof(sa));
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;
    HANDLE hOut = CreateFileA(out_tmp, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    HANDLE hErr = CreateFileA(err_tmp, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    memset(&pi, 0, sizeof(pi));
    si.cb = sizeof(si);
    si.dwFlags    = STARTF_USESTDHANDLES;
    si.hStdInput  = GetStdHandle(STD_INPUT_HANDLE);
    si.hStdOutput = hOut;
    si.hStdError  = hErr;

    BOOL ok = CreateProcessA(
        NULL, cbuf, NULL, NULL,
        TRUE /* inherit hOut/hErr */,
        CREATE_NO_WINDOW /* BACKLOG item 427: no console flash from a GUI parent */, NULL,
        (working_dir && *working_dir) ? working_dir : NULL,
        &si, &pi
    );
    free(cbuf);
    if (hOut) CloseHandle(hOut);
    if (hErr) CloseHandle(hErr);

    if (!ok) {
        exit_code = -1;
    } else {
        CloseHandle(pi.hThread);
        DWORD wait_ms = (timeout_ms > 0) ? (DWORD)timeout_ms : INFINITE;
        DWORD wr = WaitForSingleObject(pi.hProcess, wait_ms);
        if (wr == WAIT_TIMEOUT) {
            timed_out = true;
            TerminateProcess(pi.hProcess, (UINT)-1);
            WaitForSingleObject(pi.hProcess, INFINITE);
        }
        DWORD code = 0;
        GetExitCodeProcess(pi.hProcess, &code);
        CloseHandle(pi.hProcess);
        exit_code = timed_out ? -1 : (int64_t)code;
    }
#else
    snprintf(out_tmp, sizeof(out_tmp), "/tmp/certo_run_out_%d.tmp", (int)getpid());
    snprintf(err_tmp, sizeof(err_tmp), "/tmp/certo_run_err_%d.tmp", (int)getpid());

    int64_t argc = 1 + (args ? args->len : 0);
    char** argv = (char**)malloc(sizeof(char*) * (size_t)(argc + 1));
    if (!argv) certo_panic("out of memory");
    argv[0] = (char*)cmd;
    for (int64_t i = 0; i < (args ? args->len : 0); i++) argv[1 + i] = (char*)args->data[i];
    argv[argc] = NULL;

    pid_t pid = fork();
    if (pid < 0) {
        free(argv);
        exit_code = -1;
        goto done;
    }
    if (pid == 0) {
        if (working_dir && *working_dir && chdir(working_dir) != 0) _exit(127);
        FILE* fo = fopen(out_tmp, "wb");
        FILE* fe = fopen(err_tmp, "wb");
        if (fo) dup2(fileno(fo), STDOUT_FILENO);
        if (fe) dup2(fileno(fe), STDERR_FILENO);
        execvp(cmd, argv);
        _exit(127);
    }
    free(argv);

    int status = 0;
    if (timeout_ms > 0) {
        int64_t deadline = monotonic_ms() + timeout_ms;
        for (;;) {
            pid_t r = waitpid(pid, &status, WNOHANG);
            if (r == pid) break;
            if (monotonic_ms() >= deadline) {
                kill(pid, SIGKILL);
                waitpid(pid, &status, 0);
                timed_out = true;
                break;
            }
            struct timespec nap = { 0, 5 * 1000 * 1000 }; /* 5ms */
            nanosleep(&nap, NULL);
        }
    } else {
        waitpid(pid, &status, 0);
    }
    exit_code = timed_out ? -1 : (WIFEXITED(status) ? (int64_t)WEXITSTATUS(status) : -1);
done:
    ;
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
    res->exit_code = exit_code;
    res->out = out_str;
    res->err = err_str;
    return res;
}
"#;
