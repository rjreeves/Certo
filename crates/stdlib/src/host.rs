/// C implementation for `Stdlib.Host`.
///
/// This is intentionally a small, in-process application host. Plugins are
/// ordinary Certo function values linked into the program; dynamic libraries
/// and dependency injection belong to later layers.
pub const HOST_C: &str = r#"
/* ================================================================
   Stdlib.Host — ordered in-process plugin lifecycle
   ================================================================ */

#include <signal.h>
#include <ctype.h>

struct CertoHost;
struct CertoHostWorker;

typedef struct CertoRestartPolicy {
    int mode;
    int64_t max_retries;
    int64_t initial_delay_ms;
    int64_t max_delay_ms;
} CertoRestartPolicy;

typedef struct CertoHostContext {
    volatile sig_atomic_t stopping;
    int64_t plugin_count;
    CertoList* services;
    CertoList* config;
    struct CertoHost* host;
    struct CertoHostWorker* worker;
    certo_text_t plugin_name;
} CertoHostContext;

typedef struct CertoServiceKey {
    certo_text_t name;
} CertoServiceKey;

typedef struct CertoHostService {
    CertoServiceKey* key;
    void* value;
} CertoHostService;

typedef struct CertoHostConfig {
    certo_text_t key;
    certo_text_t value;
} CertoHostConfig;

typedef struct CertoHostPlugin {
    certo_text_t name;
    certo_fn_t start;
    certo_fn_t quiesce;
    certo_fn_t stop;
    CertoList* provides;
    CertoList* requires;
    CertoList* workers;
} CertoHostPlugin;

typedef struct CertoHostWorker {
    __certo_task_hdr_t hdr;
    certo_text_t name;
    certo_text_t plugin_name;
    certo_fn_t callback;
    CertoHostContext* context;
    struct CertoHost* host;
    void* result;
    bool launched;
    bool joined;
    volatile int ready;
    CertoRestartPolicy* restart;
    int64_t restart_count;
    certo_text_t last_error;
    volatile int health;
} CertoHostWorker;

typedef struct CertoHostMetric {
    certo_text_t name;
    int64_t value;
    bool gauge;
} CertoHostMetric;

typedef struct CertoHost {
    CertoList* plugins;
    CertoHostContext* context;
    int64_t started_count;
    bool running;
    volatile sig_atomic_t stop_requested;
    volatile int state;
    volatile int startup_complete;
    volatile int shutdown_complete;
    certo_text_t shutdown_error;
    int64_t shutdown_timeout_ms;
    int64_t readiness_timeout_ms;
    int64_t quiesce_timeout_ms;
    int64_t stop_timeout_ms;
    volatile int64_t ready_workers;
    int64_t worker_count;
    certo_text_t worker_error;
    CertoList* metrics;
#if defined(_WIN32)
    CRITICAL_SECTION wait_lock;
    CONDITION_VARIABLE wait_changed;
#else
    pthread_mutex_t wait_lock;
    pthread_cond_t wait_changed;
#endif
} CertoHost;

typedef void* (*CertoHostCallback)(void* env, void* context);

static CertoHost* __certo_active_host = NULL;

enum {
    CERTO_HOST_NEW = 0,
    CERTO_HOST_STARTING = 1,
    CERTO_HOST_HEALTHY = 2,
    CERTO_HOST_STOPPING = 3,
    CERTO_HOST_STOPPED = 4,
    CERTO_HOST_FAILED = 5
};

enum {
    CERTO_WORKER_STARTING = 0,
    CERTO_WORKER_HEALTHY = 1,
    CERTO_WORKER_RESTARTING = 2,
    CERTO_WORKER_STOPPED = 3,
    CERTO_WORKER_FAILED = 4
};

enum {
    CERTO_RESTART_NEVER = 0,
    CERTO_RESTART_ON_FAILURE = 1,
    CERTO_RESTART_ALWAYS = 2
};

#define CERTO_ATOMIC_LOAD(p) __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define CERTO_ATOMIC_STORE(p, value) __atomic_store_n((p), (value), __ATOMIC_RELEASE)

static void __certo_host_signal(int sig) {
    (void)sig;
    if (__certo_active_host && __certo_active_host->context) {
        CERTO_ATOMIC_STORE(&__certo_active_host->stop_requested, 1);
    }
}

static certo_text_t __certo_host_error(certo_text_t phase, certo_text_t name,
                                        certo_text_t detail) {
    if (!phase) phase = "host";
    if (!name) name = "<unnamed>";
    if (!detail) detail = "unknown error";
    size_t n = strlen(phase) + strlen(name) + strlen(detail) + 6;
    char* out = (char*)malloc(n);
    if (!out) certo_panic("out of memory");
    snprintf(out, n, "%s %s: %s", phase, name, detail);
    return out;
}

static certo_text_t __certo_host_append_error(certo_text_t errors,
                                               certo_text_t error) {
    if (!error) return errors;
    if (!errors) return error;
    size_t n = strlen(errors) + strlen(error) + 2;
    char* out = (char*)malloc(n);
    if (!out) certo_panic("out of memory");
    snprintf(out, n, "%s\n%s", errors, error);
    return out;
}

static certo_text_t __certo_host_state_text(int state) {
    switch (state) {
        case CERTO_HOST_NEW: return "New";
        case CERTO_HOST_HEALTHY: return "Healthy";
        case CERTO_HOST_STOPPING: return "Stopping";
        case CERTO_HOST_STOPPED: return "Stopped";
        case CERTO_HOST_FAILED: return "Failed";
        default: return "Starting";
    }
}

static certo_text_t __certo_worker_state_text(int state) {
    switch (state) {
        case CERTO_WORKER_HEALTHY: return "Healthy";
        case CERTO_WORKER_RESTARTING: return "Restarting";
        case CERTO_WORKER_STOPPED: return "Stopped";
        case CERTO_WORKER_FAILED: return "Failed";
        default: return "Starting";
    }
}

CertoHost* certo_host_new(void) {
    CertoHost* host = (CertoHost*)calloc(1, sizeof(CertoHost));
    CertoHostContext* context = (CertoHostContext*)calloc(1, sizeof(CertoHostContext));
    if (!host || !context) certo_panic("out of memory");
    host->plugins = certo_list_new_empty();
    host->context = context;
    host->shutdown_timeout_ms = 10000;
    host->readiness_timeout_ms = 10000;
    host->quiesce_timeout_ms = 10000;
    host->stop_timeout_ms = 10000;
    CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_NEW);
    CERTO_ATOMIC_STORE(&host->startup_complete, 0);
    context->host = host;
#if defined(_WIN32)
    InitializeCriticalSection(&host->wait_lock);
    InitializeConditionVariable(&host->wait_changed);
#else
    pthread_mutex_init(&host->wait_lock, NULL);
    pthread_cond_init(&host->wait_changed, NULL);
#endif
    context->services = certo_list_new_empty();
    context->config = certo_list_new_empty();
    host->metrics = certo_list_new_empty();
    return host;
}

CertoServiceKey* certo_host_service_key(certo_text_t name) {
    if (!name || !*name) certo_panic("service key name cannot be empty");
    CertoServiceKey* key = (CertoServiceKey*)malloc(sizeof(CertoServiceKey));
    if (!key) certo_panic("out of memory");
    key->name = name;
    return key;
}

CertoHost* certo_host_provide(CertoHost* host, CertoServiceKey* key, void* value) {
    if (!host || !key || !key->name) certo_panic("Host.provide requires a host and service key");
    if (host->running || host->started_count > 0) {
        certo_panic("services cannot be added after the host has started");
    }
    for (int64_t i = 0; i < host->context->services->len; i++) {
        CertoHostService* existing = (CertoHostService*)host->context->services->data[i];
        if (strcmp(existing->key->name, key->name) == 0) {
            certo_panic("a service with this name is already registered");
        }
    }
    CertoHostService* service = (CertoHostService*)malloc(sizeof(CertoHostService));
    if (!service) certo_panic("out of memory");
    service->key = key;
    service->value = value;
    host->context->services = certo_list_push_mut(host->context->services, service);
    return host;
}

void* certo_host_context_service(CertoHostContext* context, CertoServiceKey* key) {
    if (!context || !key || !key->name) return NULL;
    for (int64_t i = 0; i < context->services->len; i++) {
        CertoHostService* service = (CertoHostService*)context->services->data[i];
        if (strcmp(service->key->name, key->name) == 0) {
            return __certo_opt_box((int64_t)(intptr_t)service->value);
        }
    }
    return NULL;
}

CertoHost* certo_host_configure(CertoHost* host, certo_text_t key, certo_text_t value) {
    if (!host || !key) certo_panic("Host.configure requires a host and key");
    if (host->running || host->started_count > 0) {
        certo_panic("configuration cannot be changed after the host has started");
    }
    for (int64_t i = 0; i < host->context->config->len; i++) {
        CertoHostConfig* item = (CertoHostConfig*)host->context->config->data[i];
        if (strcmp(item->key, key) == 0) {
            item->value = value;
            return host;
        }
    }
    CertoHostConfig* item = (CertoHostConfig*)malloc(sizeof(CertoHostConfig));
    if (!item) certo_panic("out of memory");
    item->key = key;
    item->value = value;
    host->context->config = certo_list_push_mut(host->context->config, item);
    return host;
}

void* certo_host_context_config(CertoHostContext* context, certo_text_t key) {
    if (!context || !key) return NULL;
    for (int64_t i = context->config->len; i > 0; i--) {
        CertoHostConfig* item = (CertoHostConfig*)context->config->data[i - 1];
        if (strcmp(item->key, key) == 0) {
            return __certo_opt_box((int64_t)(intptr_t)item->value);
        }
    }
    return NULL;
}

certo_text_t certo_host_context_config_or(CertoHostContext* context,
                                           certo_text_t key,
                                           certo_text_t fallback) {
    void* found = certo_host_context_config(context, key);
    return found ? (certo_text_t)(intptr_t)(*(int64_t*)found) : fallback;
}

CertoHostPlugin* certo_host_plugin(certo_text_t name, certo_fn_t start,
                                    certo_fn_t stop) {
    if (!name || !*name) certo_panic("plugin name cannot be empty");
    CertoHostPlugin* plugin = (CertoHostPlugin*)malloc(sizeof(CertoHostPlugin));
    if (!plugin) certo_panic("out of memory");
    plugin->name = name;
    plugin->start = start;
    plugin->quiesce = (certo_fn_t){ NULL, NULL };
    plugin->stop = stop;
    plugin->provides = certo_list_new_empty();
    plugin->requires = certo_list_new_empty();
    plugin->workers = certo_list_new_empty();
    return plugin;
}

CertoHostPlugin* certo_host_plugin_quiesce(CertoHostPlugin* plugin,
                                            certo_fn_t callback) {
    if (!plugin || !callback.fn)
        certo_panic("HostPlugin.quiesce requires a plugin and callback");
    plugin->quiesce = callback;
    return plugin;
}

CertoHostPlugin* certo_host_plugin_worker(CertoHostPlugin* plugin,
                                           certo_text_t name,
                                           certo_fn_t callback) {
    if (!plugin || !name || !callback.fn)
        certo_panic("HostPlugin.worker requires a plugin, name, and callback");
    if (!*name) certo_panic("worker name cannot be empty");
    for (int64_t i = 0; i < plugin->workers->len; i++) {
        CertoHostWorker* existing = (CertoHostWorker*)plugin->workers->data[i];
        if (strcmp(existing->name, name) == 0)
            certo_panic("duplicate worker name in plugin");
    }
    CertoHostWorker* worker = (CertoHostWorker*)calloc(1, sizeof(CertoHostWorker));
    if (!worker) certo_panic("out of memory");
    worker->name = name;
    worker->plugin_name = plugin->name;
    worker->callback = callback;
    plugin->workers = certo_list_push_mut(plugin->workers, worker);
    return plugin;
}

static CertoRestartPolicy* __certo_restart_policy(int mode,
                                                   int64_t max_retries,
                                                   int64_t initial_delay_ms,
                                                   int64_t max_delay_ms) {
    if (max_retries < 0) certo_panic("restart maxRetries cannot be negative");
    if (initial_delay_ms < 0 || max_delay_ms < 0)
        certo_panic("restart delays cannot be negative");
    if (max_delay_ms < initial_delay_ms)
        certo_panic("restart maxDelay cannot be less than initialDelay");
    CertoRestartPolicy* policy =
        (CertoRestartPolicy*)malloc(sizeof(CertoRestartPolicy));
    if (!policy) certo_panic("out of memory");
    policy->mode = mode;
    policy->max_retries = max_retries;
    policy->initial_delay_ms = initial_delay_ms;
    policy->max_delay_ms = max_delay_ms;
    return policy;
}

CertoRestartPolicy* certo_restart_policy_never(void) {
    return __certo_restart_policy(CERTO_RESTART_NEVER, 0, 0, 0);
}

CertoRestartPolicy* certo_restart_policy_on_failure(int64_t max_retries,
                                                     int64_t initial_delay_ms,
                                                     int64_t max_delay_ms) {
    return __certo_restart_policy(CERTO_RESTART_ON_FAILURE, max_retries,
                                  initial_delay_ms, max_delay_ms);
}

CertoRestartPolicy* certo_restart_policy_always(int64_t max_retries,
                                                 int64_t initial_delay_ms,
                                                 int64_t max_delay_ms) {
    return __certo_restart_policy(CERTO_RESTART_ALWAYS, max_retries,
                                  initial_delay_ms, max_delay_ms);
}

CertoHostPlugin* certo_host_plugin_restart(CertoHostPlugin* plugin,
                                            CertoRestartPolicy* policy) {
    if (!plugin || !policy || plugin->workers->len == 0)
        certo_panic("HostPlugin.restart requires a preceding worker and policy");
    CertoHostWorker* worker =
        (CertoHostWorker*)plugin->workers->data[plugin->workers->len - 1];
    worker->restart = policy;
    return plugin;
}

CertoHostPlugin* certo_host_plugin_provides(CertoHostPlugin* plugin,
                                             CertoServiceKey* key) {
    if (!plugin || !key) certo_panic("HostPlugin.provides requires a plugin and service key");
    plugin->provides = certo_list_push_mut(plugin->provides, key);
    return plugin;
}

CertoHostPlugin* certo_host_plugin_requires(CertoHostPlugin* plugin,
                                             CertoServiceKey* key) {
    if (!plugin || !key) certo_panic("HostPlugin.requires requires a plugin and service key");
    plugin->requires = certo_list_push_mut(plugin->requires, key);
    return plugin;
}

CertoHost* certo_host_add(CertoHost* host, CertoHostPlugin* plugin) {
    if (!host) certo_panic("Host.add called with a null host");
    if (!plugin) certo_panic("Host.add called with a null plugin");
    if (host->running || host->started_count > 0) {
        certo_panic("plugins cannot be added after the host has started");
    }
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* existing = (CertoHostPlugin*)host->plugins->data[i];
        if (strcmp(existing->name, plugin->name) == 0)
            certo_panic("duplicate plugin name");
        for (int64_t w = 0; w < existing->workers->len; w++) {
            CertoHostWorker* existing_worker =
                (CertoHostWorker*)existing->workers->data[w];
            for (int64_t p = 0; p < plugin->workers->len; p++) {
                CertoHostWorker* new_worker =
                    (CertoHostWorker*)plugin->workers->data[p];
                if (strcmp(existing_worker->name, new_worker->name) == 0)
                    certo_panic("duplicate worker name in host");
            }
        }
    }
    host->plugins = certo_list_push_mut(host->plugins, plugin);
    host->context->plugin_count = host->plugins->len;
    return host;
}

CertoHost* certo_host_shutdown_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.shutdownTimeout called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("shutdown timeout cannot be changed after the host has started");
    host->shutdown_timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
    return host;
}

CertoHost* certo_host_readiness_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.readinessTimeout called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("readiness timeout cannot be changed after the host has started");
    host->readiness_timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
    return host;
}

CertoHost* certo_host_quiesce_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.quiesceTimeout called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("quiesce timeout cannot be changed after the host has started");
    host->quiesce_timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
    return host;
}

CertoHost* certo_host_drain_timeout(CertoHost* host, int64_t timeout_ms) {
    return certo_host_shutdown_timeout(host, timeout_ms);
}

CertoHost* certo_host_stop_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.stopTimeout called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("stop timeout cannot be changed after the host has started");
    host->stop_timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
    return host;
}

static void* __certo_host_call(certo_fn_t callback, CertoHostContext* context) {
    CertoHostCallback fn = (CertoHostCallback)callback.fn;
    if (!fn) return certo_err((intptr_t)"plugin callback is missing");
    return fn(callback.env, context);
}

typedef struct CertoHostLifecycleCall {
    __certo_task_hdr_t hdr;
    certo_fn_t callback;
    CertoHostContext* context;
    void* result;
} CertoHostLifecycleCall;

static void* __certo_host_lifecycle_main(void* raw) {
    CertoHostLifecycleCall* call = (CertoHostLifecycleCall*)raw;
    call->result = __certo_host_call(call->callback, call->context);
    __certo_task_signal_done(&call->hdr);
    return NULL;
}

static void* __certo_host_call_timed(certo_fn_t callback,
                                     CertoHostContext* context,
                                     int64_t timeout_ms,
                                     bool* timed_out) {
    CertoHostLifecycleCall* call =
        (CertoHostLifecycleCall*)calloc(1, sizeof(CertoHostLifecycleCall));
    if (!call) certo_panic("out of memory");
    call->callback = callback;
    call->context = context;
    __certo_task_hdr_init(&call->hdr);
    call->hdr.thread = __certo_thread_spawn(__certo_host_lifecycle_main, call);
    if (!__certo_thread_join_timed(&call->hdr, timeout_ms)) {
        *timed_out = true;
        return NULL;
    }
    *timed_out = false;
    void* result = call->result;
    free(call);
    return result;
}

static void __certo_host_wake_waiters(CertoHost* host) {
    if (!host) return;
#if defined(_WIN32)
    EnterCriticalSection(&host->wait_lock);
    WakeAllConditionVariable(&host->wait_changed);
    LeaveCriticalSection(&host->wait_lock);
#else
    pthread_mutex_lock(&host->wait_lock);
    pthread_cond_broadcast(&host->wait_changed);
    pthread_mutex_unlock(&host->wait_lock);
#endif
}

/* Wait for the requested interval or until shutdown. True means the full
   interval elapsed; false means cancellation won. The deadline loop handles
   spurious condition-variable wakeups without extending the requested wait. */
static bool __certo_host_interruptible_sleep(CertoHost* host, int64_t timeout_ms) {
    if (!host || CERTO_ATOMIC_LOAD(&host->context->stopping)) return false;
    if (timeout_ms <= 0) return !CERTO_ATOMIC_LOAD(&host->context->stopping);
    int64_t deadline = certo_monotonic_millis() + timeout_ms;
#if defined(_WIN32)
    EnterCriticalSection(&host->wait_lock);
    while (!CERTO_ATOMIC_LOAD(&host->context->stopping)) {
        int64_t remaining = deadline - certo_monotonic_millis();
        if (remaining <= 0) break;
        if (remaining > 50) remaining = 50;
        SleepConditionVariableCS(&host->wait_changed, &host->wait_lock, (DWORD)remaining);
    }
    bool completed = !CERTO_ATOMIC_LOAD(&host->context->stopping) && certo_monotonic_millis() >= deadline;
    LeaveCriticalSection(&host->wait_lock);
#else
    pthread_mutex_lock(&host->wait_lock);
    while (!CERTO_ATOMIC_LOAD(&host->context->stopping)) {
        int64_t remaining = deadline - certo_monotonic_millis();
        if (remaining <= 0) break;
        if (remaining > 50) remaining = 50;
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_sec += remaining / 1000;
        ts.tv_nsec += (long)(remaining % 1000) * 1000000L;
        if (ts.tv_nsec >= 1000000000L) { ts.tv_sec++; ts.tv_nsec -= 1000000000L; }
        pthread_cond_timedwait(&host->wait_changed, &host->wait_lock, &ts);
    }
    bool completed = !CERTO_ATOMIC_LOAD(&host->context->stopping) && certo_monotonic_millis() >= deadline;
    pthread_mutex_unlock(&host->wait_lock);
#endif
    return completed;
}

static void __certo_host_lock(CertoHost* host) {
#if defined(_WIN32)
    EnterCriticalSection(&host->wait_lock);
#else
    pthread_mutex_lock(&host->wait_lock);
#endif
}

static void __certo_host_unlock(CertoHost* host) {
#if defined(_WIN32)
    LeaveCriticalSection(&host->wait_lock);
#else
    pthread_mutex_unlock(&host->wait_lock);
#endif
}

static CertoHostMetric* __certo_host_metric(CertoHost* host,
                                            certo_text_t name,
                                            bool gauge) {
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        if (metric->gauge == gauge && strcmp(metric->name, name) == 0) return metric;
    }
    CertoHostMetric* metric = (CertoHostMetric*)calloc(1, sizeof(CertoHostMetric));
    if (!metric) certo_panic("out of memory");
    metric->name = name;
    metric->gauge = gauge;
    host->metrics = certo_list_push_mut(host->metrics, metric);
    return metric;
}

static bool __certo_host_valid_metric_name(certo_text_t name) {
    if (!name || !*name) return false;
    unsigned char first = (unsigned char)*name;
    if (!(isalpha(first) || first == '_' || first == ':')) return false;
    for (const unsigned char* p = (const unsigned char*)name + 1; *p; p++) {
        if (!(isalnum(*p) || *p == '_' || *p == '.' || *p == '-' || *p == ':'))
            return false;
    }
    return true;
}

static void __certo_host_metric_add(CertoHost* host,
                                    certo_text_t name,
                                    int64_t amount) {
    if (!host || !name) return;
    if (!__certo_host_valid_metric_name(name)) certo_panic("invalid metric name");
    __certo_host_lock(host);
    __certo_host_metric(host, name, false)->value += amount;
    __certo_host_unlock(host);
}

static void __certo_host_write_json_string(FILE* stream, certo_text_t text) {
    fputc('"', stream);
    const unsigned char* p = (const unsigned char*)(text ? text : "");
    for (; *p; p++) {
        switch (*p) {
            case '"': fputs("\\\"", stream); break;
            case '\\': fputs("\\\\", stream); break;
            case '\n': fputs("\\n", stream); break;
            case '\r': fputs("\\r", stream); break;
            case '\t': fputs("\\t", stream); break;
            default:
                if (*p < 0x20) fprintf(stream, "\\u%04x", (unsigned)*p);
                else fputc(*p, stream);
        }
    }
    fputc('"', stream);
}

int64_t certo_host_context_log(CertoHostContext* context,
                               certo_text_t level,
                               certo_text_t event,
                               certo_text_t message) {
    if (!context || !context->host) return 0;
    if (!level || !*level) certo_panic("log level cannot be empty");
    if (!event || !*event) certo_panic("log event cannot be empty");
    CertoHost* host = context->host;
    certo_text_t plugin = context->worker
        ? context->worker->plugin_name : context->plugin_name;
    certo_text_t worker = context->worker ? context->worker->name : NULL;
    __certo_host_lock(host);
    fputs("{\"timestamp_ms\":", stderr);
    fprintf(stderr, "%" PRId64, certo_monotonic_millis());
    fputs(",\"level\":", stderr); __certo_host_write_json_string(stderr, level);
    fputs(",\"event\":", stderr); __certo_host_write_json_string(stderr, event);
    fputs(",\"message\":", stderr); __certo_host_write_json_string(stderr, message);
    fputs(",\"plugin\":", stderr); __certo_host_write_json_string(stderr, plugin);
    if (worker) {
        fputs(",\"worker\":", stderr); __certo_host_write_json_string(stderr, worker);
    }
    fputs("}\n", stderr);
    fflush(stderr);
    __certo_host_unlock(host);
    return 0;
}

int64_t certo_host_context_counter(CertoHostContext* context,
                                   certo_text_t name,
                                   int64_t amount) {
    if (context && context->host) __certo_host_metric_add(context->host, name, amount);
    return 0;
}

int64_t certo_host_context_gauge(CertoHostContext* context,
                                 certo_text_t name,
                                 int64_t value) {
    if (!context || !context->host || !name) return 0;
    if (!__certo_host_valid_metric_name(name)) certo_panic("invalid metric name");
    __certo_host_lock(context->host);
    __certo_host_metric(context->host, name, true)->value = value;
    __certo_host_unlock(context->host);
    return 0;
}

static void __certo_host_worker_clear_ready(CertoHostWorker* worker) {
    if (CERTO_ATOMIC_LOAD(&worker->ready) && __sync_bool_compare_and_swap(&worker->ready, 1, 0)) {
        int64_t ready = __sync_sub_and_fetch(&worker->host->ready_workers, 1);
        certo_host_context_gauge(worker->context, "host.worker.ready", ready);
    }
}

static void* __certo_host_worker_main(void* raw) {
    CertoHostWorker* worker = (CertoHostWorker*)raw;
    CertoRestartPolicy fallback = { CERTO_RESTART_NEVER, 0, 0, 0 };
    CertoRestartPolicy* policy = worker->restart ? worker->restart : &fallback;
    while (!CERTO_ATOMIC_LOAD(&worker->host->context->stopping)) {
        CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STARTING);
        worker->result = __certo_host_call(worker->callback, worker->context);
        bool failed = !__result_is_ok(worker->result) || !CERTO_ATOMIC_LOAD(&worker->ready);
        if (!__result_is_ok(worker->result)) {
            CERTO_ATOMIC_STORE(&worker->last_error,
                (certo_text_t)__result_unwrap(worker->result));
            __certo_host_metric_add(worker->host, "host.worker.failures", 1);
        } else if (!CERTO_ATOMIC_LOAD(&worker->ready)) {
            CERTO_ATOMIC_STORE(&worker->last_error,
                "worker exited before reporting ready");
            __certo_host_metric_add(worker->host, "host.worker.failures", 1);
        } else {
            CERTO_ATOMIC_STORE(&worker->last_error, NULL);
        }

        if (CERTO_ATOMIC_LOAD(&worker->host->context->stopping)) {
            __certo_host_worker_clear_ready(worker);
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STOPPED);
            break;
        }

        bool restart = policy->mode == CERTO_RESTART_ALWAYS ||
            (policy->mode == CERTO_RESTART_ON_FAILURE && failed);
        if (!restart) {
            if (failed) CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_FAILED);
            else CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STOPPED);
            if (!failed) break;
        }

        if (!restart || CERTO_ATOMIC_LOAD(&worker->restart_count) >= policy->max_retries) {
            __certo_host_worker_clear_ready(worker);
            certo_text_t last_error = CERTO_ATOMIC_LOAD(&worker->last_error);
            if (!last_error) {
                last_error = "restart limit exhausted";
                CERTO_ATOMIC_STORE(&worker->last_error, last_error);
            }
            certo_text_t detail = last_error;
            certo_text_t error = __certo_host_error("worker failed", worker->name, detail);
            __sync_bool_compare_and_swap(&worker->host->worker_error, NULL, error);
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_FAILED);
            CERTO_ATOMIC_STORE(&worker->host->state, CERTO_HOST_FAILED);
            CERTO_ATOMIC_STORE(&worker->host->stop_requested, 1);
            CERTO_ATOMIC_STORE(&worker->host->context->stopping, 1);
            __certo_host_metric_add(worker->host, "host.worker.exhausted", 1);
            certo_host_context_log(worker->context, "error", "worker.failed", detail);
            __certo_host_wake_waiters(worker->host);
            break;
        }

        __certo_host_worker_clear_ready(worker);
        int64_t restart_count =
            __atomic_add_fetch(&worker->restart_count, 1, __ATOMIC_ACQ_REL);
        CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_RESTARTING);
        CERTO_ATOMIC_STORE(&worker->host->state, CERTO_HOST_STARTING);
        __certo_host_metric_add(worker->host, "host.worker.restarts", 1);
        certo_text_t restart_reason = CERTO_ATOMIC_LOAD(&worker->last_error);
        certo_host_context_log(worker->context, "warn", "worker.restarting",
            restart_reason ? restart_reason : "worker completed");

        int64_t delay = policy->initial_delay_ms;
        for (int64_t i = 1; i < restart_count && delay < policy->max_delay_ms; i++) {
            delay = delay > policy->max_delay_ms / 2
                ? policy->max_delay_ms : delay * 2;
        }
        if (delay > policy->max_delay_ms) delay = policy->max_delay_ms;
        if (!__certo_host_interruptible_sleep(worker->host, delay)) {
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STOPPED);
            break;
        }
    }
    __certo_task_signal_done(&worker->hdr);
    return NULL;
}

static void __certo_host_launch_workers(CertoHost* host) {
    CERTO_ATOMIC_STORE(&host->worker_error, NULL);
    CERTO_ATOMIC_STORE(&host->ready_workers, 0);
    host->worker_count = 0;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t w = 0; w < plugin->workers->len; w++) {
            CertoHostWorker* worker = (CertoHostWorker*)plugin->workers->data[w];
            worker->context = (CertoHostContext*)malloc(sizeof(CertoHostContext));
            if (!worker->context) certo_panic("out of memory");
            *worker->context = *host->context;
            worker->context->worker = worker;
            worker->host = host;
            worker->result = NULL;
            worker->joined = false;
            CERTO_ATOMIC_STORE(&worker->ready, 0);
            CERTO_ATOMIC_STORE(&worker->restart_count, 0);
            CERTO_ATOMIC_STORE(&worker->last_error, NULL);
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STARTING);
            host->worker_count++;
            __certo_host_metric_add(host, "host.worker.starts", 1);
            __certo_task_hdr_init(&worker->hdr);
            worker->hdr.thread = __certo_thread_spawn(__certo_host_worker_main, worker);
            worker->launched = true;
        }
    }
}

static certo_text_t __certo_host_wait_until_ready(CertoHost* host,
                                                   int64_t timeout_ms) {
    int64_t deadline = certo_monotonic_millis() + (timeout_ms < 0 ? 0 : timeout_ms);
    while (CERTO_ATOMIC_LOAD(&host->ready_workers) < host->worker_count) {
        certo_text_t worker_error = CERTO_ATOMIC_LOAD(&host->worker_error);
        if (worker_error) return worker_error;
        if (certo_monotonic_millis() >= deadline) {
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            __certo_host_wake_waiters(host);
            return __certo_host_error("host readiness", "timed out",
                                      "not all workers reported ready");
        }
#ifdef _WIN32
        Sleep(10);
#else
        struct timespec delay = { 0, 10000000L };
        nanosleep(&delay, NULL);
#endif
    }
    return CERTO_ATOMIC_LOAD(&host->worker_error);
}

static certo_text_t __certo_host_join_workers(CertoHost* host) {
    int64_t deadline = certo_monotonic_millis() + host->shutdown_timeout_ms;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t w = 0; w < plugin->workers->len; w++) {
            CertoHostWorker* worker = (CertoHostWorker*)plugin->workers->data[w];
            if (!worker->launched || worker->joined) continue;
            int64_t remaining = deadline - certo_monotonic_millis();
            if (remaining < 0) remaining = 0;
            if (!__certo_thread_join_timed(&worker->hdr, remaining)) {
                return __certo_host_error("worker did not stop", worker->name,
                                          "shutdown timeout elapsed");
            }
            worker->joined = true;
        }
    }
    return CERTO_ATOMIC_LOAD(&host->worker_error);
}

static bool __certo_host_has_service(CertoHost* host, certo_text_t name) {
    for (int64_t i = 0; i < host->context->services->len; i++) {
        CertoHostService* service = (CertoHostService*)host->context->services->data[i];
        if (strcmp(service->key->name, name) == 0) return true;
    }
    return false;
}

static int64_t __certo_host_provider(CertoHost* host, certo_text_t name,
                                     int64_t* count) {
    int64_t found = -1;
    *count = 0;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t j = 0; j < plugin->provides->len; j++) {
            CertoServiceKey* key = (CertoServiceKey*)plugin->provides->data[j];
            if (strcmp(key->name, name) == 0) {
                found = i;
                (*count)++;
            }
        }
    }
    return found;
}

/* Validate dependencies and replace the plugin list with a stable topological
   ordering. Original registration order breaks ties between ready plugins. */
static certo_text_t __certo_host_order_plugins(CertoHost* host) {
    int64_t n = host->plugins->len;
    for (int64_t i = 0; i < n; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t p = 0; p < plugin->provides->len; p++) {
            CertoServiceKey* key = (CertoServiceKey*)plugin->provides->data[p];
            int64_t provider_count = 0;
            __certo_host_provider(host, key->name, &provider_count);
            if (provider_count > 1) {
                return __certo_host_error("multiple providers for", key->name,
                                          "service claimed by more than one plugin");
            }
        }
    }
    if (n < 2) {
        if (n == 1) {
            CertoHostPlugin* only = (CertoHostPlugin*)host->plugins->data[0];
            for (int64_t r = 0; r < only->requires->len; r++) {
                CertoServiceKey* key = (CertoServiceKey*)only->requires->data[r];
                if (!__certo_host_has_service(host, key->name))
                    return __certo_host_error("missing service for", only->name, key->name);
            }
        }
        return NULL;
    }

    int64_t* indegree = (int64_t*)calloc((size_t)n, sizeof(int64_t));
    bool* emitted = (bool*)calloc((size_t)n, sizeof(bool));
    bool* edges = (bool*)calloc((size_t)(n * n), sizeof(bool));
    if (!indegree || !emitted || !edges) certo_panic("out of memory");

    for (int64_t consumer = 0; consumer < n; consumer++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[consumer];
        for (int64_t r = 0; r < plugin->requires->len; r++) {
            CertoServiceKey* key = (CertoServiceKey*)plugin->requires->data[r];
            if (!__certo_host_has_service(host, key->name)) {
                free(indegree); free(emitted); free(edges);
                return __certo_host_error("missing service for", plugin->name, key->name);
            }
            int64_t provider_count = 0;
            int64_t provider = __certo_host_provider(host, key->name, &provider_count);
            if (provider_count > 1) {
                free(indegree); free(emitted); free(edges);
                return __certo_host_error("multiple providers for", plugin->name, key->name);
            }
            if (provider >= 0 && provider != consumer && !edges[provider * n + consumer]) {
                edges[provider * n + consumer] = true;
                indegree[consumer]++;
            }
        }
    }

    CertoList* ordered = certo_list_new_empty();
    for (int64_t step = 0; step < n; step++) {
        int64_t next = -1;
        for (int64_t i = 0; i < n; i++) {
            if (!emitted[i] && indegree[i] == 0) { next = i; break; }
        }
        if (next < 0) {
            CertoHostPlugin* blocked = NULL;
            for (int64_t i = 0; i < n; i++) {
                if (!emitted[i]) { blocked = (CertoHostPlugin*)host->plugins->data[i]; break; }
            }
            free(indegree); free(emitted); free(edges);
            return __certo_host_error("dependency cycle at", blocked ? blocked->name : "<unknown>",
                                      "plugin requirements form a cycle");
        }
        emitted[next] = true;
        ordered = certo_list_push_mut(ordered, host->plugins->data[next]);
        for (int64_t dependent = 0; dependent < n; dependent++) {
            if (edges[next * n + dependent]) indegree[dependent]--;
        }
    }

    free(indegree); free(emitted); free(edges);
    host->plugins = ordered;
    return NULL;
}

static certo_text_t __certo_host_quiesce_started(CertoHost* host) {
    certo_text_t errors = NULL;
    for (int64_t i = host->started_count; i > 0; i--) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i - 1];
        if (!plugin->quiesce.fn) continue;
        host->context->plugin_name = plugin->name;
        bool timed_out = false;
        void* result = __certo_host_call_timed(
            plugin->quiesce, host->context, host->quiesce_timeout_ms, &timed_out);
        if (timed_out) {
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to quiesce", plugin->name, "timeout elapsed"));
        } else if (!__result_is_ok(result)) {
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to quiesce", plugin->name,
                    (certo_text_t)__result_unwrap(result)));
        }
    }
    return errors;
}

static certo_text_t __certo_host_stop_started(CertoHost* host) {
    certo_text_t errors = NULL;
    while (host->started_count > 0) {
        int64_t index = --host->started_count;
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[index];
        host->context->plugin_name = plugin->name;
        bool timed_out = false;
        void* result = __certo_host_call_timed(
            plugin->stop, host->context, host->stop_timeout_ms, &timed_out);
        if (timed_out) {
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to stop", plugin->name, "timeout elapsed"));
        } else if (!__result_is_ok(result)) {
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to stop", plugin->name,
                    (certo_text_t)__result_unwrap(result)));
        }
    }
    host->running = false;
    return errors;
}

void* certo_host_start(CertoHost* host) {
    if (!host) return certo_err((intptr_t)"host is null");
    if (!__sync_bool_compare_and_swap(
            &host->state, CERTO_HOST_NEW, CERTO_HOST_STARTING)) {
        return certo_err((intptr_t)"host has already started");
    }
    CERTO_ATOMIC_STORE(&host->context->stopping, 0);
    CERTO_ATOMIC_STORE(&host->stop_requested, 0);
    certo_text_t dependency_error = __certo_host_order_plugins(host);
    if (dependency_error) {
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        return certo_err((intptr_t)dependency_error);
    }
    host->running = true;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        host->context->plugin_name = plugin->name;
        void* result = __certo_host_call(plugin->start, host->context);
        if (!__result_is_ok(result)) {
            certo_text_t error = __certo_host_error(
                "failed to start", plugin->name,
                (certo_text_t)__result_unwrap(result));
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
            __certo_host_stop_started(host);
            return certo_err((intptr_t)error);
        }
        host->started_count++;
    }
    __certo_host_launch_workers(host);
    certo_text_t readiness_error = __certo_host_wait_until_ready(
        host, host->readiness_timeout_ms);
    if (readiness_error) {
        CERTO_ATOMIC_STORE(&host->context->stopping, 1);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_join_workers(host);
        __certo_host_stop_started(host);
        return certo_err((intptr_t)readiness_error);
    }
    CERTO_ATOMIC_STORE(&host->startup_complete, 1);
    CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_HEALTHY);
    return certo_ok(0);
}

void* certo_host_wait_until_ready(CertoHost* host, int64_t timeout_ms) {
    if (!host) return certo_err((intptr_t)"host is null");
    if (!host->running) return certo_err((intptr_t)"host has not started");
    certo_text_t error = __certo_host_wait_until_ready(host, timeout_ms);
    return error ? certo_err((intptr_t)error) : certo_ok(0);
}

void* certo_host_stop(CertoHost* host) {
    if (!host) return certo_err((intptr_t)"host is null");
    if (CERTO_ATOMIC_LOAD(&host->shutdown_complete)) {
        certo_text_t shutdown_error = CERTO_ATOMIC_LOAD(&host->shutdown_error);
        return shutdown_error
            ? certo_err((intptr_t)shutdown_error) : certo_ok(0);
    }
    if (!__sync_bool_compare_and_swap(
            &host->state, CERTO_HOST_HEALTHY, CERTO_HOST_STOPPING)) {
        if (CERTO_ATOMIC_LOAD(&host->state) == CERTO_HOST_STOPPING) {
#if defined(_WIN32)
            EnterCriticalSection(&host->wait_lock);
            while (!CERTO_ATOMIC_LOAD(&host->shutdown_complete))
                SleepConditionVariableCS(&host->wait_changed, &host->wait_lock, INFINITE);
            LeaveCriticalSection(&host->wait_lock);
#else
            pthread_mutex_lock(&host->wait_lock);
            while (!CERTO_ATOMIC_LOAD(&host->shutdown_complete))
                pthread_cond_wait(&host->wait_changed, &host->wait_lock);
            pthread_mutex_unlock(&host->wait_lock);
#endif
            certo_text_t shutdown_error = CERTO_ATOMIC_LOAD(&host->shutdown_error);
            return shutdown_error
                ? certo_err((intptr_t)shutdown_error) : certo_ok(0);
        }
        return certo_err((intptr_t)"host is not running");
    }
    certo_text_t errors = __certo_host_quiesce_started(host);
    CERTO_ATOMIC_STORE(&host->context->stopping, 1);
    __certo_host_wake_waiters(host);
    certo_text_t worker_error = __certo_host_join_workers(host);
    certo_text_t stop_error = __certo_host_stop_started(host);
    errors = __certo_host_append_error(errors, worker_error);
    errors = __certo_host_append_error(errors, stop_error);
    if (errors) __certo_host_metric_add(host, "host.shutdown.failures", 1);
    CERTO_ATOMIC_STORE(&host->shutdown_error, errors);
    CERTO_ATOMIC_STORE(&host->state, errors ? CERTO_HOST_FAILED : CERTO_HOST_STOPPED);
    CERTO_ATOMIC_STORE(&host->shutdown_complete, 1);
    __certo_host_wake_waiters(host);
    return errors ? certo_err((intptr_t)errors) : certo_ok(0);
}

void* certo_host_run(CertoHost* host) {
    void* started = certo_host_start(host);
    if (!__result_is_ok(started)) return started;

    __certo_active_host = host;
    void (*old_sigint)(int) = signal(SIGINT, __certo_host_signal);
#ifdef SIGTERM
    void (*old_sigterm)(int) = signal(SIGTERM, __certo_host_signal);
#endif

    while (!CERTO_ATOMIC_LOAD(&host->stop_requested) &&
           !CERTO_ATOMIC_LOAD(&host->context->stopping)) {
#ifdef _WIN32
        Sleep(50);
#else
        struct timespec delay = { 0, 50000000L };
        nanosleep(&delay, NULL);
#endif
    }

    signal(SIGINT, old_sigint);
#ifdef SIGTERM
    signal(SIGTERM, old_sigterm);
#endif
    __certo_active_host = NULL;
    return certo_host_stop(host);
}

int64_t certo_host_request_stop(CertoHostContext* context) {
    if (context && context->host) {
        CERTO_ATOMIC_STORE(&context->host->stop_requested, 1);
        __certo_host_wake_waiters(context->host);
    }
    return 0;
}

bool certo_host_context_is_stopping(CertoHostContext* context) {
    return context && context->host &&
        CERTO_ATOMIC_LOAD(&context->host->context->stopping);
}

int64_t certo_host_context_ready(CertoHostContext* context) {
    if (!context || !context->host || !context->worker) return 0;
    if (__sync_bool_compare_and_swap(&context->worker->ready, 0, 1)) {
        __sync_add_and_fetch(&context->host->ready_workers, 1);
        int64_t ready_workers = CERTO_ATOMIC_LOAD(&context->host->ready_workers);
        certo_host_context_gauge(context, "host.worker.ready", ready_workers);
        CERTO_ATOMIC_STORE(&context->worker->health, CERTO_WORKER_HEALTHY);
        int host_state = CERTO_ATOMIC_LOAD(&context->host->state);
        if (CERTO_ATOMIC_LOAD(&context->host->startup_complete) &&
            ready_workers >= context->host->worker_count &&
            host_state != CERTO_HOST_STOPPING && host_state != CERTO_HOST_FAILED)
            CERTO_ATOMIC_STORE(&context->host->state, CERTO_HOST_HEALTHY);
    }
    return 0;
}

int64_t certo_host_context_fail(CertoHostContext* context, certo_text_t error) {
    if (!context || !context->host) return 0;
    certo_text_t name = context->worker ? context->worker->name : "host";
    certo_text_t detail = __certo_host_error("worker failed", name, error);
    __sync_bool_compare_and_swap(&context->host->worker_error, NULL, detail);
    CERTO_ATOMIC_STORE(&context->host->state, CERTO_HOST_FAILED);
    CERTO_ATOMIC_STORE(&context->host->stop_requested, 1);
    CERTO_ATOMIC_STORE(&context->host->context->stopping, 1);
    __certo_host_wake_waiters(context->host);
    return 0;
}

bool certo_host_context_sleep(CertoHostContext* context, int64_t duration_ms) {
    return context && context->host &&
        __certo_host_interruptible_sleep(context->host, duration_ms);
}

typedef bool (*CertoHostPredicate)(void* env, void* context);

bool certo_host_context_wait_until(CertoHostContext* context,
                                    certo_fn_t predicate,
                                    int64_t interval_ms) {
    if (!context || !context->host || !predicate.fn) return false;
    CertoHostPredicate test = (CertoHostPredicate)predicate.fn;
    while (!CERTO_ATOMIC_LOAD(&context->host->context->stopping)) {
        if (test(predicate.env, context)) return true;
        if (!__certo_host_interruptible_sleep(context->host, interval_ms)) return false;
    }
    return false;
}

certo_text_t certo_host_health(CertoHost* host) {
    if (!host) return "Failed";
    return __certo_host_state_text(CERTO_ATOMIC_LOAD(&host->state));
}

static CertoHostWorker* __certo_host_find_worker(CertoHost* host,
                                                 certo_text_t name) {
    if (!host || !name) return NULL;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t w = 0; w < plugin->workers->len; w++) {
            CertoHostWorker* worker = (CertoHostWorker*)plugin->workers->data[w];
            if (strcmp(worker->name, name) == 0) return worker;
        }
    }
    return NULL;
}

void* certo_host_worker_health(CertoHost* host, certo_text_t name) {
    CertoHostWorker* worker = __certo_host_find_worker(host, name);
    return worker ? __certo_opt_box((int64_t)(intptr_t)
        __certo_worker_state_text(CERTO_ATOMIC_LOAD(&worker->health))) : NULL;
}

void* certo_host_worker_restarts(CertoHost* host, certo_text_t name) {
    CertoHostWorker* worker = __certo_host_find_worker(host, name);
    return worker
        ? __certo_opt_box(CERTO_ATOMIC_LOAD(&worker->restart_count)) : NULL;
}

void* certo_host_worker_last_error(CertoHost* host, certo_text_t name) {
    CertoHostWorker* worker = __certo_host_find_worker(host, name);
    certo_text_t last_error = worker
        ? CERTO_ATOMIC_LOAD(&worker->last_error) : NULL;
    return last_error
        ? __certo_opt_box((int64_t)(intptr_t)last_error) : NULL;
}

static char* __certo_host_json_quote(certo_text_t text) {
    const unsigned char* p = (const unsigned char*)(text ? text : "");
    size_t cap = strlen((const char*)p) * 6 + 3;
    char* out = (char*)malloc(cap);
    if (!out) certo_panic("out of memory");
    size_t n = 0;
    out[n++] = '"';
    for (; *p; p++) {
        if (*p == '"' || *p == '\\') { out[n++] = '\\'; out[n++] = (char)*p; }
        else if (*p == '\n') { out[n++] = '\\'; out[n++] = 'n'; }
        else if (*p == '\r') { out[n++] = '\\'; out[n++] = 'r'; }
        else if (*p == '\t') { out[n++] = '\\'; out[n++] = 't'; }
        else if (*p < 0x20) n += (size_t)snprintf(out + n, cap - n, "\\u%04x", (unsigned)*p);
        else out[n++] = (char)*p;
    }
    out[n++] = '"'; out[n] = '\0';
    return out;
}

certo_text_t certo_host_metrics(CertoHost* host) {
    if (!host) return "{\"health\":\"Failed\",\"counters\":{},\"gauges\":{}}";
    __certo_host_lock(host);
    size_t cap = 128;
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        cap += strlen(metric->name) * 6 + 48;
    }
    char* out = (char*)malloc(cap);
    if (!out) certo_panic("out of memory");
    size_t n = (size_t)snprintf(out, cap,
        "{\"health\":\"%s\",\"counters\":{",
        __certo_host_state_text(CERTO_ATOMIC_LOAD(&host->state)));
    bool first = true;
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        if (metric->gauge) continue;
        char* name = __certo_host_json_quote(metric->name);
        n += (size_t)snprintf(out + n, cap - n, "%s%s:%" PRId64,
                              first ? "" : ",", name, metric->value);
        free(name); first = false;
    }
    n += (size_t)snprintf(out + n, cap - n, "},\"gauges\":{");
    first = true;
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        if (!metric->gauge) continue;
        char* name = __certo_host_json_quote(metric->name);
        n += (size_t)snprintf(out + n, cap - n, "%s%s:%" PRId64,
                              first ? "" : ",", name, metric->value);
        free(name); first = false;
    }
    snprintf(out + n, cap - n, "}}");
    __certo_host_unlock(host);
    return out;
}

int64_t certo_host_context_plugin_count(CertoHostContext* context) {
    return context ? context->plugin_count : 0;
}
"#;
