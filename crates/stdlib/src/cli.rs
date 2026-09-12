/// C implementation for `Stdlib.Cli` — a small, declarative command-line
/// parser inspired by clap.  Commands and matches are opaque to Certo code;
/// the builder functions mutate and return the command so calls compose.
pub const CLI_C: &str = r#"
/* ================================================================
   Stdlib.Cli — declarative command-line argument parsing
   ================================================================ */

typedef struct CertoCliCommand CertoCliCommand;

typedef struct {
    char* name;
    char* short_name;
    char* long_name;
    char* value_name;
    char* help;
    char* default_value;
    bool  takes_value;
    bool  required;
    bool  positional;
} CertoCliArg;

struct CertoCliCommand {
    char* name;
    char* about;
    char* version;
    CertoCliArg* args;
    int64_t args_len;
    int64_t args_cap;
    CertoCliCommand** subs;
    int64_t subs_len;
    int64_t subs_cap;
};

typedef struct CertoCliMatches {
    CertoCliCommand* command;
    char** names;
    char** values;
    int64_t values_len;
    int64_t values_cap;
    struct CertoCliMatches* sub;
    char* error;
    bool help_requested;
    bool version_requested;
} CertoCliMatches;

static char* cli_dup(certo_text_t s) {
    if (!s) s = "";
    size_t n = strlen(s);
    char* out = (char*)malloc(n + 1);
    if (!out) certo_panic("out of memory");
    memcpy(out, s, n + 1);
    return out;
}

static CertoCliCommand* cli_grow_args(CertoCliCommand* c) {
    if (c->args_len == c->args_cap) {
        c->args_cap = c->args_cap ? c->args_cap * 2 : 8;
        c->args = (CertoCliArg*)realloc(c->args, (size_t)c->args_cap * sizeof(CertoCliArg));
        if (!c->args) certo_panic("out of memory");
    }
    return c;
}

CertoCliCommand* certo_cli_command(certo_text_t name, certo_text_t about, certo_text_t version) {
    CertoCliCommand* c = (CertoCliCommand*)calloc(1, sizeof(CertoCliCommand));
    if (!c) certo_panic("out of memory");
    c->name = cli_dup(name); c->about = cli_dup(about); c->version = cli_dup(version);
    return c;
}

static CertoCliCommand* cli_add_arg(CertoCliCommand* c, certo_text_t name,
        certo_text_t short_name, certo_text_t long_name, certo_text_t value_name,
        certo_text_t help, bool takes_value, bool positional) {
    if (!c) return c;
    cli_grow_args(c);
    CertoCliArg* a = &c->args[c->args_len++];
    memset(a, 0, sizeof(*a));
    a->name = cli_dup(name); a->short_name = cli_dup(short_name);
    a->long_name = cli_dup(long_name); a->value_name = cli_dup(value_name);
    a->help = cli_dup(help); a->takes_value = takes_value; a->positional = positional;
    return c;
}

CertoCliCommand* certo_cli_option(CertoCliCommand* c, certo_text_t name,
        certo_text_t short_name, certo_text_t long_name, certo_text_t value_name,
        certo_text_t help) {
    return cli_add_arg(c, name, short_name, long_name, value_name, help, true, false);
}

CertoCliCommand* certo_cli_flag(CertoCliCommand* c, certo_text_t name,
        certo_text_t short_name, certo_text_t long_name, certo_text_t help) {
    return cli_add_arg(c, name, short_name, long_name, "", help, false, false);
}

CertoCliCommand* certo_cli_positional(CertoCliCommand* c, certo_text_t name,
        certo_text_t value_name, certo_text_t help) {
    return cli_add_arg(c, name, "", "", value_name, help, true, true);
}

static CertoCliArg* cli_find_name(CertoCliCommand* c, const char* name) {
    if (!c || !name) return NULL;
    for (int64_t i = 0; i < c->args_len; i++) if (!strcmp(c->args[i].name, name)) return &c->args[i];
    return NULL;
}

CertoCliCommand* certo_cli_required(CertoCliCommand* c, certo_text_t name) {
    CertoCliArg* a = cli_find_name(c, name); if (a) a->required = true; return c;
}

CertoCliCommand* certo_cli_default_value(CertoCliCommand* c, certo_text_t name, certo_text_t value) {
    CertoCliArg* a = cli_find_name(c, name);
    if (a) { free(a->default_value); a->default_value = cli_dup(value); }
    return c;
}

CertoCliCommand* certo_cli_subcommand(CertoCliCommand* c, CertoCliCommand* sub) {
    if (!c || !sub) return c;
    if (c->subs_len == c->subs_cap) {
        c->subs_cap = c->subs_cap ? c->subs_cap * 2 : 4;
        c->subs = (CertoCliCommand**)realloc(c->subs, (size_t)c->subs_cap * sizeof(CertoCliCommand*));
        if (!c->subs) certo_panic("out of memory");
    }
    c->subs[c->subs_len++] = sub; return c;
}

static CertoCliMatches* cli_matches_new(CertoCliCommand* c) {
    CertoCliMatches* m = (CertoCliMatches*)calloc(1, sizeof(CertoCliMatches));
    if (!m) certo_panic("out of memory"); m->command = c; return m;
}

static void cli_set_error(CertoCliMatches* m, const char* prefix, const char* value) {
    if (m->error) return;
    size_t n = strlen(prefix) + (value ? strlen(value) : 0) + 1;
    m->error = (char*)malloc(n); if (!m->error) certo_panic("out of memory");
    snprintf(m->error, n, "%s%s", prefix, value ? value : "");
}

static void cli_put(CertoCliMatches* m, const char* name, const char* value) {
    for (int64_t i = 0; i < m->values_len; i++) if (!strcmp(m->names[i], name)) {
        m->values[i] = cli_dup(value); return;
    }
    if (m->values_len == m->values_cap) {
        m->values_cap = m->values_cap ? m->values_cap * 2 : 8;
        m->names = (char**)realloc(m->names, (size_t)m->values_cap * sizeof(char*));
        m->values = (char**)realloc(m->values, (size_t)m->values_cap * sizeof(char*));
        if (!m->names || !m->values) certo_panic("out of memory");
    }
    m->names[m->values_len] = cli_dup(name); m->values[m->values_len] = cli_dup(value); m->values_len++;
}

static CertoCliArg* cli_find_switch(CertoCliCommand* c, const char* token, size_t n, bool is_long) {
    for (int64_t i = 0; i < c->args_len; i++) {
        CertoCliArg* a = &c->args[i]; const char* key = is_long ? a->long_name : a->short_name;
        if (!a->positional && key && strlen(key) == n && !strncmp(key, token, n)) return a;
    }
    return NULL;
}

static CertoCliCommand* cli_find_sub(CertoCliCommand* c, const char* name) {
    for (int64_t i = 0; i < c->subs_len; i++) if (!strcmp(c->subs[i]->name, name)) return c->subs[i];
    return NULL;
}

static CertoCliMatches* cli_parse_range(CertoCliCommand* c, int64_t start, int64_t end) {
    CertoCliMatches* m = cli_matches_new(c); int64_t positional = 0; bool switches = true;
    for (int64_t i = start; i < end && !m->error; i++) {
        const char* tok = __certo_argv[i];
        CertoCliCommand* sub = switches ? cli_find_sub(c, tok) : NULL;
        if (sub) { m->sub = cli_parse_range(sub, i + 1, end); break; }
        if (switches && !strcmp(tok, "--")) { switches = false; continue; }
        if (switches && (!strcmp(tok, "--help") || !strcmp(tok, "-h"))) { m->help_requested = true; continue; }
        if (switches && (!strcmp(tok, "--version") || !strcmp(tok, "-V"))) { m->version_requested = true; continue; }
        if (switches && tok[0] == '-' && tok[1] == '-') {
            const char* key = tok + 2; const char* eq = strchr(key, '='); size_t kn = eq ? (size_t)(eq - key) : strlen(key);
            CertoCliArg* a = cli_find_switch(c, key, kn, true);
            if (!a) { cli_set_error(m, "unknown option: --", key); continue; }
            if (!a->takes_value) { if (eq) cli_set_error(m, "flag does not take a value: --", a->long_name); else cli_put(m, a->name, "true"); }
            else if (eq) cli_put(m, a->name, eq + 1);
            else if (++i < end) cli_put(m, a->name, __certo_argv[i]);
            else cli_set_error(m, "missing value for --", a->long_name);
            continue;
        }
        if (switches && tok[0] == '-' && tok[1] != '\0') {
            const char* p = tok + 1;
            while (*p && !m->error) {
                CertoCliArg* a = cli_find_switch(c, p, 1, false);
                if (!a) { char bad[2] = {*p, 0}; cli_set_error(m, "unknown option: -", bad); break; }
                if (!a->takes_value) { cli_put(m, a->name, "true"); p++; }
                else if (p[1]) { cli_put(m, a->name, p + 1); break; }
                else if (++i < end) { cli_put(m, a->name, __certo_argv[i]); break; }
                else { cli_set_error(m, "missing value for -", a->short_name); break; }
            }
            continue;
        }
        while (positional < c->args_len && !c->args[positional].positional) positional++;
        if (positional >= c->args_len) { cli_set_error(m, "unexpected argument: ", tok); continue; }
        cli_put(m, c->args[positional].name, tok); positional++;
    }
    for (int64_t i = 0; i < c->args_len && !m->error; i++) {
        CertoCliArg* a = &c->args[i]; bool found = false;
        for (int64_t j = 0; j < m->values_len; j++) if (!strcmp(m->names[j], a->name)) found = true;
        if (!found && a->default_value) cli_put(m, a->name, a->default_value);
        else if (!found && a->required && !m->help_requested && !m->version_requested) cli_set_error(m, "missing required argument: ", a->name);
    }
    return m;
}

CertoCliMatches* certo_cli_parse(CertoCliCommand* c) { return cli_parse_range(c, 1, __certo_argc); }
bool certo_cli_matches_ok(CertoCliMatches* m) { return m && !m->error && (!m->sub || certo_cli_matches_ok(m->sub)); }
certo_text_t certo_cli_matches_error(CertoCliMatches* m) {
    if (!m) return ""; if (m->error) return m->error; return m->sub ? certo_cli_matches_error(m->sub) : "";
}
bool certo_cli_matches_help_requested(CertoCliMatches* m) { return m && (m->help_requested || (m->sub && certo_cli_matches_help_requested(m->sub))); }
bool certo_cli_matches_version_requested(CertoCliMatches* m) { return m && (m->version_requested || (m->sub && certo_cli_matches_version_requested(m->sub))); }
bool certo_cli_matches_has(CertoCliMatches* m, certo_text_t name) {
    if (!m) return false; for (int64_t i = 0; i < m->values_len; i++) if (!strcmp(m->names[i], name)) return true; return false;
}
void* certo_cli_matches_get(CertoCliMatches* m, certo_text_t name) {
    if (!m) return NULL; for (int64_t i = 0; i < m->values_len; i++) if (!strcmp(m->names[i], name)) return __certo_opt_box((int64_t)m->values[i]); return NULL;
}
certo_text_t certo_cli_matches_get_or(CertoCliMatches* m, certo_text_t name, certo_text_t fallback) {
    if (!m) return fallback; for (int64_t i = 0; i < m->values_len; i++) if (!strcmp(m->names[i], name)) return m->values[i]; return fallback;
}
bool certo_cli_matches_flag(CertoCliMatches* m, certo_text_t name) { return certo_cli_matches_has(m, name); }
void* certo_cli_matches_subcommand(CertoCliMatches* m) { return (m && m->sub) ? __certo_opt_box((int64_t)m->sub) : NULL; }
certo_text_t certo_cli_matches_subcommand_name(CertoCliMatches* m) { return (m && m->sub && m->sub->command) ? m->sub->command->name : ""; }

static void cli_text_append(char** out, size_t* len, size_t* cap, const char* s) {
    size_t n = strlen(s); if (*len + n + 1 > *cap) { while (*len + n + 1 > *cap) *cap *= 2; *out = (char*)realloc(*out, *cap); if (!*out) certo_panic("out of memory"); }
    memcpy(*out + *len, s, n + 1); *len += n;
}

certo_text_t certo_cli_help(CertoCliCommand* c) {
    if (!c) return ""; size_t len = 0, cap = 512; char* out = (char*)malloc(cap); if (!out) certo_panic("out of memory"); out[0] = 0;
    cli_text_append(&out,&len,&cap,c->name); if (*c->version) { cli_text_append(&out,&len,&cap," "); cli_text_append(&out,&len,&cap,c->version); }
    cli_text_append(&out,&len,&cap,"\n"); if (*c->about) { cli_text_append(&out,&len,&cap,c->about); cli_text_append(&out,&len,&cap,"\n"); }
    cli_text_append(&out,&len,&cap,"\nUsage: "); cli_text_append(&out,&len,&cap,c->name);
    if (c->args_len) cli_text_append(&out,&len,&cap," [OPTIONS]"); if (c->subs_len) cli_text_append(&out,&len,&cap," <COMMAND>"); cli_text_append(&out,&len,&cap,"\n");
    if (c->args_len) { cli_text_append(&out,&len,&cap,"\nArguments and options:\n"); for (int64_t i=0;i<c->args_len;i++) { CertoCliArg* a=&c->args[i]; cli_text_append(&out,&len,&cap,"  ");
        if (a->positional) cli_text_append(&out,&len,&cap,*a->value_name?a->value_name:a->name); else { if (*a->short_name) { cli_text_append(&out,&len,&cap,"-"); cli_text_append(&out,&len,&cap,a->short_name); if (*a->long_name) cli_text_append(&out,&len,&cap,", "); } if (*a->long_name) { cli_text_append(&out,&len,&cap,"--"); cli_text_append(&out,&len,&cap,a->long_name); } if (a->takes_value) { cli_text_append(&out,&len,&cap," <"); cli_text_append(&out,&len,&cap,*a->value_name?a->value_name:a->name); cli_text_append(&out,&len,&cap,">"); } }
        if (*a->help) { cli_text_append(&out,&len,&cap,"\t"); cli_text_append(&out,&len,&cap,a->help); } if (a->required) cli_text_append(&out,&len,&cap," (required)"); cli_text_append(&out,&len,&cap,"\n"); } }
    if (c->subs_len) { cli_text_append(&out,&len,&cap,"\nCommands:\n"); for(int64_t i=0;i<c->subs_len;i++){ cli_text_append(&out,&len,&cap,"  "); cli_text_append(&out,&len,&cap,c->subs[i]->name); if(*c->subs[i]->about){cli_text_append(&out,&len,&cap,"\t");cli_text_append(&out,&len,&cap,c->subs[i]->about);} cli_text_append(&out,&len,&cap,"\n"); } }
    cli_text_append(&out,&len,&cap,"\n  -h, --help\tPrint help\n"); if (*c->version) cli_text_append(&out,&len,&cap,"  -V, --version\tPrint version\n"); return out;
}

certo_text_t certo_cli_version(CertoCliCommand* c) { return c ? c->version : ""; }
certo_text_t certo_cli_matches_help(CertoCliMatches* m) {
    if (!m) return "";
    if (m->sub && (certo_cli_matches_help_requested(m->sub) || !certo_cli_matches_ok(m->sub))) return certo_cli_matches_help(m->sub);
    return certo_cli_help(m->command);
}
certo_text_t certo_cli_matches_version(CertoCliMatches* m) {
    if (!m) return ""; if (m->sub && certo_cli_matches_version_requested(m->sub)) return certo_cli_matches_version(m->sub); return certo_cli_version(m->command);
}

/* Dot-call aliases (`command.option(...)`) use the receiver type as their
   namespace during type checking/codegen. */
#define certo_cli_command_option        certo_cli_option
#define certo_cli_command_flag          certo_cli_flag
#define certo_cli_command_positional    certo_cli_positional
#define certo_cli_command_required      certo_cli_required
#define certo_cli_command_default_value certo_cli_default_value
#define certo_cli_command_subcommand    certo_cli_subcommand
"#;
