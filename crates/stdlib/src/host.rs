/// C implementation for `Stdlib.Host`.
///
/// This is intentionally a small, in-process application host. Plugins are
/// ordinary Certo function values linked into the program; dynamic libraries
/// and dependency injection belong to later layers.
pub const HOST_C: &str = r##"
/* ================================================================
   Stdlib.Host — ordered in-process plugin lifecycle
   ================================================================ */

#include <signal.h>
#include <ctype.h>
#include <stdarg.h>

struct CertoHost;
struct CertoHostWorker;
struct CertoHostLifecycleError;
struct CertoConfigKey;
struct CertoHost;
certo_text_t __certo_host_http_start_listeners(struct CertoHost* host);
void __certo_host_http_close_listeners(struct CertoHost* host);
certo_text_t __certo_host_http_drain_listeners(struct CertoHost* host);

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
    certo_text_t correlation_id;
} CertoHostContext;

typedef struct CertoServiceKey {
    certo_text_t name;
} CertoServiceKey;

typedef struct CertoHostService {
    CertoServiceKey* key;
    void* value;
    certo_fn_t factory;
    certo_fn_t dispose;
    CertoList* dependencies;
    bool constructed;
    bool owned;
} CertoHostService;

typedef struct CertoHostConfig {
    certo_text_t key;
    certo_text_t value;
    certo_text_t source;
    certo_text_t location;
    int precedence;
    int64_t sequence;
} CertoHostConfig;

typedef struct CertoConfigKey {
    certo_text_t name;
    certo_fn_t parse;
    bool parsed_value_boxed;
    bool secret_bearing;
} CertoConfigKey;

typedef struct CertoHostConfigBinding {
    CertoConfigKey* key;
    bool required;
    bool has_default;
    void* default_value;
    void* value;
    bool bound;
    CertoList* validators;
} CertoHostConfigBinding;

typedef struct CertoHostConfigurationError {
    certo_text_t key;
    certo_text_t source;
    certo_text_t location;
    certo_text_t category;
    certo_text_t message;
} CertoHostConfigurationError;

typedef struct CertoHostPlugin {
    certo_text_t name;
    certo_fn_t start;
    certo_fn_t quiesce;
    certo_fn_t stop;
    CertoList* provides;
    CertoList* requires;
    CertoList* workers;
    CertoList* scoped_services;
    CertoList* scoped_construction_order;
    CertoHostContext* context;
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
    volatile int enabled;
    volatile int inactive;
    volatile int control_busy;
} CertoHostWorker;

typedef struct CertoHostMetric {
    certo_text_t name;
    int64_t value;
    bool gauge;
} CertoHostMetric;

typedef struct CertoHostMetricSeries {
    CertoList* labels;
    int64_t value;
    int64_t count;
    int64_t sum;
    int64_t* bucket_counts;
} CertoHostMetricSeries;

typedef struct CertoHostMetricDescriptor {
    struct CertoHost* host;
    certo_text_t name;
    certo_text_t help;
    certo_text_t unit;
    int kind;
    CertoList* label_names;
    CertoList* buckets;
    int64_t max_series;
    CertoList* series;
} CertoHostMetricDescriptor;

typedef struct CertoHostLifecycleError {
    certo_text_t kind;
    certo_text_t phase;
    certo_text_t subject;
    certo_text_t message;
    bool timed_out;
    CertoList* configuration_errors;
} CertoHostLifecycleError;

typedef struct CertoHostWorkerStatus {
    certo_text_t name;
    certo_text_t plugin;
    certo_text_t state;
    bool ready;
    bool live;
    int64_t restarts;
    certo_text_t last_error;
    bool enabled;
} CertoHostWorkerStatus;

typedef struct CertoHostMetricSnapshot {
    certo_text_t name;
    int64_t value;
} CertoHostMetricSnapshot;

typedef struct CertoHostLogField {
    certo_text_t name;
    int kind;
    certo_text_t text_value;
    int64_t int_value;
    double float_value;
    bool bool_value;
} CertoHostLogField;

typedef struct CertoHostLogEvent {
    certo_text_t schema;
    int64_t sequence;
    int64_t timestamp_unix_ms;
    certo_text_t severity;
    certo_text_t event;
    certo_text_t message;
    certo_text_t host_id;
    certo_text_t plugin;
    certo_text_t worker;
    certo_text_t correlation_id;
    CertoList* fields;
    volatile int references;
} CertoHostLogEvent;

typedef struct CertoHostLogOverflowPolicy {
    int mode;
    int64_t wait_ms;
} CertoHostLogOverflowPolicy;

typedef struct CertoHostLogFailurePolicy {
    int mode;
} CertoHostLogFailurePolicy;

typedef struct CertoHostLogSink {
    __certo_task_hdr_t hdr;
    struct CertoHost* host;
    certo_text_t name;
    int64_t capacity;
    CertoHostLogOverflowPolicy overflow;
    CertoHostLogFailurePolicy failure;
    certo_fn_t write;
    certo_fn_t flush;
    certo_fn_t dispose;
    CertoHostLogEvent** queue;
    int64_t head;
    int64_t length;
    bool accepting;
    bool disabled;
    bool started;
#if defined(_WIN32)
    CRITICAL_SECTION lock;
    CONDITION_VARIABLE changed;
#else
    pthread_mutex_t lock;
    pthread_cond_t changed;
#endif
} CertoHostLogSink;

typedef struct CertoHostStatusSnapshot {
    certo_text_t state;
    bool ready;
    bool live;
    volatile int64_t worker_count;
    int64_t ready_workers;
    CertoList* workers;
    CertoList* counters;
    CertoList* gauges;
    CertoHostLifecycleError* last_failure;
} CertoHostStatusSnapshot;

typedef struct CertoHostOperationalStatus {
    certo_text_t condition;
    certo_text_t state;
    bool ready;
    bool live;
    certo_text_t failure_kind;
    certo_text_t start_reason;
    certo_text_t stop_reason;
} CertoHostOperationalStatus;

typedef struct CertoHost {
    CertoList* plugins;
    CertoHostContext* context;
    int64_t started_count;
    bool running;
    volatile sig_atomic_t stop_requested;
    volatile int state;
    volatile int startup_complete;
    volatile int startup_cancel_requested;
    volatile int shutdown_started;
    volatile int shutdown_complete;
    certo_text_t shutdown_error;
    int64_t shutdown_timeout_ms;
    int64_t readiness_timeout_ms;
    int64_t quiesce_timeout_ms;
    int64_t stop_timeout_ms;
    int64_t disposal_timeout_ms;
    int64_t telemetry_timeout_ms;
    volatile int retain_host_services;
    volatile int64_t ready_workers;
    int64_t worker_count;
    certo_text_t worker_error;
    CertoHostLifecycleError* last_failure;
    CertoList* metrics;
    CertoList* metric_descriptors;
    CertoList* service_construction_order;
    CertoList* config_bindings;
    CertoList* configuration_errors;
    int64_t config_sequence;
    certo_text_t host_id;
    int64_t event_sequence;
    bool stderr_log_enabled;
    CertoList* log_sinks;
    CertoList* http_listeners;
    certo_text_t start_reason;
    certo_text_t stop_reason;
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
static bool __certo_host_valid_metric_name(certo_text_t name);

static char* __certo_host_replace_secret(
        const char* input, const char* secret) {
    if (!input) return strdup("");
    if (!secret || !*secret || !strstr(input, secret)) return strdup(input);
    const char* replacement = "[REDACTED]";
    size_t secret_len = strlen(secret), replacement_len = strlen(replacement);
    size_t count = 0;
    for (const char* p = input; (p = strstr(p, secret)); p += secret_len) count++;
    size_t output_len = strlen(input) - count * secret_len +
        count * replacement_len + 1;
    char* output = (char*)malloc(output_len);
    if (!output) certo_panic("out of memory");
    char* out = output;
    const char* cursor = input;
    const char* found;
    while ((found = strstr(cursor, secret))) {
        size_t prefix = (size_t)(found - cursor);
        memcpy(out, cursor, prefix); out += prefix;
        memcpy(out, replacement, replacement_len); out += replacement_len;
        cursor = found + secret_len;
    }
    strcpy(out, cursor);
    return output;
}

static certo_text_t __certo_host_sanitize(CertoHost* host, certo_text_t text) {
    char* sanitized = strdup(text ? text : "");
    if (!sanitized) certo_panic("out of memory");
    if (!host || !host->config_bindings || !host->context || !host->context->config)
        return sanitized;
    for (int64_t i = 0; i < host->config_bindings->len; i++) {
        CertoHostConfigBinding* binding =
            (CertoHostConfigBinding*)host->config_bindings->data[i];
        if (!binding->key->secret_bearing) continue;
        CertoHostConfig* best = NULL;
        for (int64_t c = 0; c < host->context->config->len; c++) {
            CertoHostConfig* item =
                (CertoHostConfig*)host->context->config->data[c];
            if (strcmp(item->key, binding->key->name) != 0) continue;
            if (!best || item->precedence > best->precedence ||
                (item->precedence == best->precedence &&
                 item->sequence > best->sequence)) best = item;
        }
        if (!best || !best->value || !*best->value) continue;
        char* next = __certo_host_replace_secret(sanitized, best->value);
        free(sanitized);
        sanitized = next;
    }
    return sanitized;
}

static void __certo_host_reject_secret_identity(
        CertoHost* host, certo_text_t value, const char* category) {
    certo_text_t sanitized = __certo_host_sanitize(host, value);
    bool changed = strcmp(value ? value : "", sanitized) != 0;
    free((void*)sanitized);
    if (changed) {
        char message[128];
        snprintf(message, sizeof(message), "secret configuration cannot be used as %s", category);
        certo_panic(message);
    }
}
static void __certo_host_metric_add(CertoHost* host, certo_text_t name,
                                    int64_t amount);

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
    CERTO_WORKER_FAILED = 4,
    CERTO_WORKER_DISABLED = 5
};

enum {
    CERTO_RESTART_NEVER = 0,
    CERTO_RESTART_ON_FAILURE = 1,
    CERTO_RESTART_ALWAYS = 2
};

enum {
    CERTO_LOG_DROP_NEWEST = 0,
    CERTO_LOG_DROP_OLDEST = 1,
    CERTO_LOG_WAIT = 2
};

enum {
    CERTO_LOG_FAILURE_IGNORE = 0,
    CERTO_LOG_FAILURE_DISABLE = 1,
    CERTO_LOG_FAILURE_FAIL_HOST = 2
};

enum {
    CERTO_METRIC_COUNTER = 0,
    CERTO_METRIC_GAUGE = 1,
    CERTO_METRIC_HISTOGRAM = 2
};

#define CERTO_ATOMIC_LOAD(p) __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define CERTO_ATOMIC_STORE(p, value) __atomic_store_n((p), (value), __ATOMIC_RELEASE)

static void __certo_host_signal(int sig) {
    (void)sig;
    if (__certo_active_host && __certo_active_host->context) {
        CERTO_ATOMIC_STORE(&__certo_active_host->stop_requested, 1);
        __sync_bool_compare_and_swap(
            &__certo_active_host->stop_reason, NULL, "Signal");
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
        case CERTO_WORKER_DISABLED: return "Disabled";
        default: return "Starting";
    }
}

static CertoHostLifecycleError* __certo_host_record_failure(
        CertoHost* host, certo_text_t kind, certo_text_t phase,
        certo_text_t subject, certo_text_t message, bool timed_out) {
    CertoHostLifecycleError* error =
        (CertoHostLifecycleError*)malloc(sizeof(CertoHostLifecycleError));
    if (!error) certo_panic("out of memory");
    error->kind = kind ? kind : "StartupFailure";
    error->phase = phase ? phase : "host";
    error->subject = subject ? subject : "host";
    error->message = __certo_host_sanitize(
        host, message ? message : "unknown error");
    error->timed_out = timed_out;
    error->configuration_errors =
        host && strcmp(error->kind, "ConfigurationFailure") == 0
            ? host->configuration_errors : certo_list_new_empty();
    if (host) CERTO_ATOMIC_STORE(&host->last_failure, error);
    if (host && (strcmp(kind, "StartupFailure") == 0 ||
                 strcmp(kind, "ConfigurationFailure") == 0 ||
                 strcmp(kind, "StartupCancelled") == 0))
        __sync_bool_compare_and_swap(&host->stop_reason, NULL, "StartupFailure");
    if (host && strcmp(kind, "WorkerFailure") == 0)
        __sync_bool_compare_and_swap(&host->stop_reason, NULL, "WorkerFailure");
    return error;
}

static CertoHostLifecycleError* __certo_host_failure_or_fallback(
        CertoHost* host, certo_text_t kind, certo_text_t phase,
        certo_text_t message) {
    CertoHostLifecycleError* error = host
        ? CERTO_ATOMIC_LOAD(&host->last_failure) : NULL;
    return error ? error : __certo_host_record_failure(
        host, kind, phase, "host", message, false);
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
    host->disposal_timeout_ms = 10000;
    host->telemetry_timeout_ms = 10000;
    host->stderr_log_enabled = true;
    CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_NEW);
    CERTO_ATOMIC_STORE(&host->startup_complete, 0);
    CERTO_ATOMIC_STORE(&host->startup_cancel_requested, 0);
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
    host->metric_descriptors = certo_list_new_empty();
    host->service_construction_order = certo_list_new_empty();
    host->config_bindings = certo_list_new_empty();
    host->configuration_errors = certo_list_new_empty();
    host->log_sinks = certo_list_new_empty();
    host->http_listeners = certo_list_new_empty();
    host->start_reason = "Application";
    host->stop_reason = NULL;
    char* host_id = (char*)malloc(48);
    if (!host_id) certo_panic("out of memory");
    snprintf(host_id, 48, "host-%p", (void*)host);
    host->host_id = host_id;
    return host;
}

/* The compiler replaces this identity operation when [host-plugins] is configured. */
CertoHost* certo_host_discovered(CertoHost* host);

#ifndef CERTO_HOST_DISCOVERED_DEFINED
CertoHost* certo_host_discovered(CertoHost* host) {
    return host;
}
#endif

CertoHost* certo_host_correlation(CertoHost* host, certo_text_t correlation_id) {
    if (!host) certo_panic("Host.correlation called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("host correlation cannot be changed after startup");
    if (!correlation_id || !*correlation_id)
        certo_panic("host correlation must not be empty");
    host->context->correlation_id = correlation_id;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        if (plugin->context) plugin->context->correlation_id = correlation_id;
    }
    return host;
}

certo_text_t certo_host_start_reason_application(void) { return "Application"; }
certo_text_t certo_host_start_reason_service_manager(void) { return "ServiceManager"; }
certo_text_t certo_host_start_reason_restart(void) { return "Restart"; }
certo_text_t certo_host_start_reason_test_run(void) { return "Test"; }
certo_text_t certo_host_start_reason_name(certo_text_t reason) {
    return reason ? reason : "Application";
}
certo_text_t certo_host_stop_reason_name(certo_text_t reason) {
    return reason ? reason : "ApplicationRequest";
}
CertoHost* certo_host_start_reason(CertoHost* host, certo_text_t reason) {
    if (!host) certo_panic("Host.startReason called with a null host");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("start reason cannot be changed after startup");
    host->start_reason = reason ? reason : "Application";
    return host;
}

CertoHostContext* certo_host_context_with_correlation(
        CertoHostContext* context, certo_text_t correlation_id) {
    if (!context) certo_panic("HostContext.withCorrelation called with a null context");
    if (!correlation_id || !*correlation_id)
        certo_panic("correlation identity must not be empty");
    CertoHostContext* derived =
        (CertoHostContext*)malloc(sizeof(CertoHostContext));
    if (!derived) certo_panic("out of memory");
    *derived = *context;
    derived->correlation_id = correlation_id;
    return derived;
}

certo_text_t certo_host_context_host_id(CertoHostContext* context) {
    return context && context->host ? context->host->host_id : "";
}

void* certo_host_context_plugin(CertoHostContext* context) {
    certo_text_t plugin = context && context->worker
        ? context->worker->plugin_name : (context ? context->plugin_name : NULL);
    return plugin ? __certo_opt_box((int64_t)(intptr_t)plugin) : NULL;
}

void* certo_host_context_worker(CertoHostContext* context) {
    certo_text_t worker = context && context->worker ? context->worker->name : NULL;
    return worker ? __certo_opt_box((int64_t)(intptr_t)worker) : NULL;
}

void* certo_host_context_correlation_id(CertoHostContext* context) {
    return context && context->correlation_id
        ? __certo_opt_box((int64_t)(intptr_t)context->correlation_id) : NULL;
}

CertoHostLogOverflowPolicy* certo_host_log_overflow_policy_drop_newest(void) {
    CertoHostLogOverflowPolicy* policy =
        (CertoHostLogOverflowPolicy*)calloc(1, sizeof(CertoHostLogOverflowPolicy));
    if (!policy) certo_panic("out of memory");
    policy->mode = CERTO_LOG_DROP_NEWEST;
    return policy;
}

CertoHostLogOverflowPolicy* certo_host_log_overflow_policy_drop_oldest(void) {
    CertoHostLogOverflowPolicy* policy = certo_host_log_overflow_policy_drop_newest();
    policy->mode = CERTO_LOG_DROP_OLDEST;
    return policy;
}

CertoHostLogOverflowPolicy* certo_host_log_overflow_policy_wait(int64_t wait_ms) {
    if (wait_ms < 0) certo_panic("log sink overflow wait cannot be negative");
    CertoHostLogOverflowPolicy* policy = certo_host_log_overflow_policy_drop_newest();
    policy->mode = CERTO_LOG_WAIT;
    policy->wait_ms = wait_ms;
    return policy;
}

static CertoHostLogFailurePolicy* __certo_host_log_failure(int mode) {
    CertoHostLogFailurePolicy* policy =
        (CertoHostLogFailurePolicy*)malloc(sizeof(CertoHostLogFailurePolicy));
    if (!policy) certo_panic("out of memory");
    policy->mode = mode;
    return policy;
}

CertoHostLogFailurePolicy* certo_host_log_failure_policy_ignore(void) {
    return __certo_host_log_failure(CERTO_LOG_FAILURE_IGNORE);
}
CertoHostLogFailurePolicy* certo_host_log_failure_policy_disable(void) {
    return __certo_host_log_failure(CERTO_LOG_FAILURE_DISABLE);
}
CertoHostLogFailurePolicy* certo_host_log_failure_policy_fail_host(void) {
    return __certo_host_log_failure(CERTO_LOG_FAILURE_FAIL_HOST);
}

CertoHost* certo_host_log_sink(CertoHost* host, certo_text_t name,
        int64_t capacity, CertoHostLogOverflowPolicy* overflow,
        CertoHostLogFailurePolicy* failure, certo_fn_t write,
        certo_fn_t flush, certo_fn_t dispose) {
    if (!host || !name || !*name || !overflow || !failure ||
        !write.fn || !dispose.fn)
        certo_panic("Host.logSink requires a host, name, policies, writer, and disposer");
    if (!__certo_host_valid_metric_name(name))
        certo_panic("invalid host log sink name");
    if (capacity <= 0) certo_panic("host log sink capacity must be positive");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("log sinks cannot be registered after startup begins");
    for (int64_t i = 0; i < host->log_sinks->len; i++) {
        CertoHostLogSink* existing = (CertoHostLogSink*)host->log_sinks->data[i];
        if (strcmp(existing->name, name) == 0)
            certo_panic("a log sink with this name is already registered");
    }
    CertoHostLogSink* sink = (CertoHostLogSink*)calloc(1, sizeof(CertoHostLogSink));
    if (!sink) certo_panic("out of memory");
    sink->queue = (CertoHostLogEvent**)calloc((size_t)capacity, sizeof(CertoHostLogEvent*));
    if (!sink->queue) certo_panic("out of memory");
    sink->host = host;
    sink->name = name;
    sink->capacity = capacity;
    sink->overflow = *overflow;
    sink->failure = *failure;
    sink->write = write;
    sink->flush = flush;
    sink->dispose = dispose;
    sink->accepting = true;
#if defined(_WIN32)
    InitializeCriticalSection(&sink->lock);
    InitializeConditionVariable(&sink->changed);
#else
    pthread_mutex_init(&sink->lock, NULL);
    pthread_cond_init(&sink->changed, NULL);
#endif
    host->log_sinks = certo_list_push_mut(host->log_sinks, sink);
    return host;
}

CertoHost* certo_host_disable_stderr_log(CertoHost* host) {
    if (!host) certo_panic("Host.disableStderrLog called with a null host");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("stderr logging cannot be changed after startup begins");
    host->stderr_log_enabled = false;
    return host;
}

CertoHost* certo_host_telemetry_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.telemetryTimeout called with a null host");
    if (timeout_ms < 0) certo_panic("telemetry timeout cannot be negative");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("telemetry timeout cannot be changed after startup begins");
    host->telemetry_timeout_ms = timeout_ms;
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
    CertoHostService* service = (CertoHostService*)calloc(1, sizeof(CertoHostService));
    if (!service) certo_panic("out of memory");
    service->key = key;
    service->value = value;
    service->dependencies = certo_list_new_empty();
    service->constructed = true;
    host->context->services = certo_list_push_mut(host->context->services, service);
    return host;
}

CertoHost* certo_host_provide_factory(CertoHost* host, CertoServiceKey* key,
                                      certo_fn_t factory, certo_fn_t dispose) {
    if (!host || !key || !key->name || !factory.fn || !dispose.fn)
        certo_panic("Host.provideFactory requires a host, service key, factory, and disposer");
    if (host->running || host->started_count > 0)
        certo_panic("services cannot be added after the host has started");
    for (int64_t i = 0; i < host->context->services->len; i++) {
        CertoHostService* existing = (CertoHostService*)host->context->services->data[i];
        if (strcmp(existing->key->name, key->name) == 0)
            certo_panic("a service with this name is already registered");
    }
    CertoHostService* service = (CertoHostService*)calloc(1, sizeof(CertoHostService));
    if (!service) certo_panic("out of memory");
    service->key = key;
    service->factory = factory;
    service->dispose = dispose;
    service->dependencies = certo_list_new_empty();
    service->owned = true;
    host->context->services = certo_list_push_mut(host->context->services, service);
    return host;
}

CertoHost* certo_host_factory_depends_on(CertoHost* host,
                                         CertoServiceKey* factory_key,
                                         CertoServiceKey* dependency_key) {
    if (!host || !factory_key || !dependency_key)
        certo_panic("Host.factoryDependsOn requires a host and two service keys");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("factory dependencies cannot be changed after startup begins");
    CertoHostService* factory = NULL;
    for (int64_t i = 0; i < host->context->services->len; i++) {
        CertoHostService* service =
            (CertoHostService*)host->context->services->data[i];
        if (strcmp(service->key->name, factory_key->name) == 0) {
            factory = service;
            break;
        }
    }
    if (!factory || !factory->owned)
        certo_panic("factory dependency target is not a registered factory");
    for (int64_t i = 0; i < factory->dependencies->len; i++) {
        CertoServiceKey* existing =
            (CertoServiceKey*)factory->dependencies->data[i];
        if (strcmp(existing->name, dependency_key->name) == 0)
            certo_panic("duplicate service factory dependency");
    }
    factory->dependencies = certo_list_push_mut(
        factory->dependencies, dependency_key);
    return host;
}

void* certo_host_context_service(CertoHostContext* context, CertoServiceKey* key) {
    if (!context || !key || !key->name) return NULL;
    if (context->host && context->plugin_name) {
        for (int64_t p = 0; p < context->host->plugins->len; p++) {
            CertoHostPlugin* plugin =
                (CertoHostPlugin*)context->host->plugins->data[p];
            if (strcmp(plugin->name, context->plugin_name) != 0) continue;
            for (int64_t i = 0; i < plugin->scoped_services->len; i++) {
                CertoHostService* service =
                    (CertoHostService*)plugin->scoped_services->data[i];
                if (service->constructed &&
                    strcmp(service->key->name, key->name) == 0)
                    return __certo_opt_box((int64_t)(intptr_t)service->value);
            }
            break;
        }
    }
    for (int64_t i = 0; i < context->services->len; i++) {
        CertoHostService* service = (CertoHostService*)context->services->data[i];
        if (service->constructed && strcmp(service->key->name, key->name) == 0) {
            return __certo_opt_box((int64_t)(intptr_t)service->value);
        }
    }
    return NULL;
}

CertoHost* certo_host_configure(CertoHost* host, certo_text_t key, certo_text_t value) {
    if (!host || !key) certo_panic("Host.configure requires a host and key");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW) {
        certo_panic("configuration cannot be changed after the host has started");
    }
    CertoHostConfig* item = (CertoHostConfig*)malloc(sizeof(CertoHostConfig));
    if (!item) certo_panic("out of memory");
    item->key = key;
    item->value = value;
    item->source = "Programmatic";
    item->location = "Host.configure";
    item->precedence = 5;
    item->sequence = host->config_sequence++;
    host->context->config = certo_list_push_mut(host->context->config, item);
    return host;
}

void* certo_host_context_config(CertoHostContext* context, certo_text_t key) {
    if (!context || !key) return NULL;
    if (context->host) {
        for (int64_t i = 0; i < context->host->config_bindings->len; i++) {
            CertoHostConfigBinding* binding =
                (CertoHostConfigBinding*)context->host->config_bindings->data[i];
            if (strcmp(binding->key->name, key) == 0 &&
                binding->key->secret_bearing)
                certo_panic("raw lookup is not allowed for secret configuration; use HostContext.configValue");
        }
    }
    CertoHostConfig* best = NULL;
    for (int64_t i = 0; i < context->config->len; i++) {
        CertoHostConfig* item = (CertoHostConfig*)context->config->data[i];
        if (strcmp(item->key, key) == 0 && (!best ||
            item->precedence > best->precedence ||
            (item->precedence == best->precedence && item->sequence > best->sequence)))
            best = item;
    }
    return best ? __certo_opt_box((int64_t)(intptr_t)best->value) : NULL;
}

certo_text_t certo_host_context_config_or(CertoHostContext* context,
                                           certo_text_t key,
                                           certo_text_t fallback) {
    void* found = certo_host_context_config(context, key);
    return found ? (certo_text_t)(intptr_t)(*(int64_t*)found) : fallback;
}

static CertoHostConfigBinding* __certo_host_config_binding(
        CertoHost* host, CertoConfigKey* key) {
    if (!host || !key) return NULL;
    for (int64_t i = 0; i < host->config_bindings->len; i++) {
        CertoHostConfigBinding* binding =
            (CertoHostConfigBinding*)host->config_bindings->data[i];
        if (strcmp(binding->key->name, key->name) == 0) return binding;
    }
    return NULL;
}

CertoConfigKey* certo_host_config_key(certo_text_t name, certo_fn_t parse,
                                      bool parsed_value_boxed,
                                      bool secret_bearing) {
    if (!name || !*name || !parse.fn)
        certo_panic("Host.configKey requires a name and parser");
    CertoConfigKey* key = (CertoConfigKey*)malloc(sizeof(CertoConfigKey));
    if (!key) certo_panic("out of memory");
    key->name = name;
    key->parse = parse;
    key->parsed_value_boxed = parsed_value_boxed;
    key->secret_bearing = secret_bearing;
    return key;
}

static CertoHostConfigBinding* __certo_host_register_config(
        CertoHost* host, CertoConfigKey* key) {
    if (!host || !key)
        certo_panic("typed configuration registration requires a host and key");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("typed configuration cannot be changed after startup begins");
    if (__certo_host_config_binding(host, key))
        certo_panic("a typed configuration key with this name is already registered");
    CertoHostConfigBinding* binding =
        (CertoHostConfigBinding*)calloc(1, sizeof(CertoHostConfigBinding));
    if (!binding) certo_panic("out of memory");
    binding->key = key;
    binding->validators = certo_list_new_empty();
    host->config_bindings = certo_list_push_mut(host->config_bindings, binding);
    return binding;
}

CertoHost* certo_host_require_config(CertoHost* host, CertoConfigKey* key) {
    CertoHostConfigBinding* binding = __certo_host_register_config(host, key);
    binding->required = true;
    return host;
}

CertoHost* certo_host_default_config(CertoHost* host, CertoConfigKey* key,
                                      void* value) {
    CertoHostConfigBinding* binding = __certo_host_register_config(host, key);
    binding->has_default = true;
    intptr_t* boxed = (intptr_t*)malloc(sizeof(intptr_t));
    if (!boxed) certo_panic("out of memory");
    *boxed = (intptr_t)value;
    binding->default_value = boxed;
    return host;
}

CertoHost* certo_host_validate_config(CertoHost* host, CertoConfigKey* key,
                                       certo_fn_t validator) {
    if (!host || !key || !validator.fn)
        certo_panic("Host.validateConfig requires a host, key, and validator");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("configuration validators cannot be changed after startup begins");
    CertoHostConfigBinding* binding = __certo_host_config_binding(host, key);
    if (!binding)
        certo_panic("configuration key must be required or defaulted before validation");
    certo_fn_t* stored = (certo_fn_t*)malloc(sizeof(certo_fn_t));
    if (!stored) certo_panic("out of memory");
    *stored = validator;
    binding->validators = certo_list_push_mut(binding->validators, stored);
    return host;
}

intptr_t certo_host_context_config_value(CertoHostContext* context,
                                          CertoConfigKey* key) {
    CertoHostConfigBinding* binding = context && context->host
        ? __certo_host_config_binding(context->host, key) : NULL;
    if (!binding || !binding->bound)
        certo_panic("typed configuration value is unavailable before successful binding");
    return *(intptr_t*)binding->value;
}

typedef void* (*CertoHostConfigParser)(void* env, certo_text_t value);
typedef void* (*CertoHostConfigValidator)(void* env, void* value);

static void __certo_host_add_raw_config(CertoHost* host, certo_text_t key,
        certo_text_t value, certo_text_t source, certo_text_t location,
        int precedence) {
    CertoHostConfig* item = (CertoHostConfig*)malloc(sizeof(CertoHostConfig));
    if (!item) certo_panic("out of memory");
    item->key = key;
    item->value = value;
    item->source = source;
    item->location = location;
    item->precedence = precedence;
    item->sequence = host->config_sequence++;
    host->context->config = certo_list_push_mut(host->context->config, item);
}

static bool __certo_host_has_config_key(CertoHost* host, certo_text_t name) {
    for (int64_t i = 0; i < host->config_bindings->len; i++) {
        CertoHostConfigBinding* binding =
            (CertoHostConfigBinding*)host->config_bindings->data[i];
        if (strcmp(binding->key->name, name) == 0) return true;
    }
    return false;
}

static CertoHostConfig* __certo_host_winning_config(
        CertoHost* host, certo_text_t key) {
    CertoHostConfig* best = NULL;
    for (int64_t i = 0; i < host->context->config->len; i++) {
        CertoHostConfig* item =
            (CertoHostConfig*)host->context->config->data[i];
        if (strcmp(item->key, key) != 0) continue;
        if (!best || item->precedence > best->precedence ||
            (item->precedence == best->precedence &&
             item->sequence > best->sequence)) best = item;
    }
    return best;
}

static char* __certo_host_trim(char* text) {
    while (*text && isspace((unsigned char)*text)) text++;
    char* end = text + strlen(text);
    while (end > text && isspace((unsigned char)end[-1])) end--;
    *end = '\0';
    return text;
}

static certo_text_t __certo_host_load_toml(CertoHost* host) {
    FILE* file = fopen("certo.toml", "rb");
    if (!file) return NULL;
    char line[4096];
    char section[1024] = "";
    int64_t line_number = 0;
    while (fgets(line, sizeof(line), file)) {
        line_number++;
        char* text = __certo_host_trim(line);
        if (!*text || *text == '#') continue;
        char* comment = strchr(text, '#');
        if (comment) { *comment = '\0'; text = __certo_host_trim(text); }
        size_t len = strlen(text);
        if (text[0] == '[' && len > 2 && text[len - 1] == ']') {
            text[len - 1] = '\0';
            snprintf(section, sizeof(section), "%s", __certo_host_trim(text + 1));
            continue;
        }
        char* equals = strchr(text, '=');
        if (!equals) { fclose(file); return "malformed certo.toml assignment"; }
        *equals = '\0';
        char* leaf = __certo_host_trim(text);
        char* value = __certo_host_trim(equals + 1);
        char key[2048];
        if (strcmp(section, "host") == 0)
            snprintf(key, sizeof(key), "%s", leaf);
        else if (strncmp(section, "host.", 5) == 0)
            snprintf(key, sizeof(key), "%s.%s", section + 5, leaf);
        else continue;
        if (!__certo_host_has_config_key(host, key)) continue;
        len = strlen(value);
        if (len >= 2 && value[0] == '"' && value[len - 1] == '"') {
            value[len - 1] = '\0'; value++;
        }
        char* stored_key = strdup(key);
        char* stored_value = strdup(value);
        char* location = (char*)malloc(64);
        if (!stored_key || !stored_value || !location) certo_panic("out of memory");
        snprintf(location, 64, "certo.toml:%lld", (long long)line_number);
        __certo_host_add_raw_config(host, stored_key, stored_value,
                                    "Toml", location, 2);
    }
    fclose(file);
    return NULL;
}

static void __certo_host_load_environment(CertoHost* host) {
    for (int64_t i = 0; i < host->config_bindings->len; i++) {
        CertoHostConfigBinding* binding =
            (CertoHostConfigBinding*)host->config_bindings->data[i];
        const char* key = binding->key->name;
        size_t n = strlen(key);
        char* variable = (char*)malloc(n * 2 + 8);
        if (!variable) certo_panic("out of memory");
        strcpy(variable, "CERTO__");
        size_t out = 7;
        for (size_t p = 0; p < n; p++) {
            unsigned char ch = (unsigned char)key[p];
            if (ch == '.') { variable[out++] = '_'; variable[out++] = '_'; }
            else if (isupper(ch)) { variable[out++] = '_'; variable[out++] = (char)ch; }
            else variable[out++] = (char)toupper(ch);
        }
        variable[out] = '\0';
        const char* value = getenv(variable);
        if (value) __certo_host_add_raw_config(
            host, binding->key->name, strdup(value), "Environment", variable, 3);
        else free(variable);
    }
}

static certo_text_t __certo_host_load_arguments(CertoHost* host) {
    for (int i = 1; i < __certo_argc; i++) {
        const char* value = NULL;
        if (strcmp(__certo_argv[i], "--config") == 0) {
            if (++i >= __certo_argc) return "--config requires key=value";
            value = __certo_argv[i];
        } else if (strncmp(__certo_argv[i], "--config=", 9) == 0) {
            value = __certo_argv[i] + 9;
        } else continue;
        const char* equals = strchr(value, '=');
        if (!equals || equals == value) return "--config requires key=value";
        size_t key_len = (size_t)(equals - value);
        char* key = (char*)malloc(key_len + 1);
        char* location = (char*)malloc(64);
        if (!key || !location) certo_panic("out of memory");
        memcpy(key, value, key_len); key[key_len] = '\0';
        snprintf(location, 64, "argument %d", i);
        __certo_host_add_raw_config(host, key, strdup(equals + 1),
                                    "CommandLine", location, 4);
    }
    return NULL;
}

static certo_text_t __certo_host_load_configuration_sources(CertoHost* host) {
    certo_text_t error = __certo_host_load_toml(host);
    if (error) return error;
    __certo_host_load_environment(host);
    return __certo_host_load_arguments(host);
}

static certo_text_t __certo_host_add_configuration_error(
        CertoHost* host, certo_text_t errors, certo_text_t key,
        certo_text_t source, certo_text_t location,
        certo_text_t category, certo_text_t message) {
    CertoHostConfigurationError* error =
        (CertoHostConfigurationError*)malloc(sizeof(CertoHostConfigurationError));
    if (!error) certo_panic("out of memory");
    error->key = key;
    error->source = source;
    error->location = location;
    error->category = category;
    error->message = message;
    host->configuration_errors =
        certo_list_push_mut(host->configuration_errors, error);
    return __certo_host_append_error(errors,
        __certo_host_error("configuration", key, message));
}

static certo_text_t __certo_host_bind_configuration(CertoHost* host) {
    certo_text_t errors = NULL;
    host->configuration_errors = certo_list_new_empty();
    for (int64_t i = 0; i < host->config_bindings->len; i++) {
        CertoHostConfigBinding* binding =
            (CertoHostConfigBinding*)host->config_bindings->data[i];
        CertoHostConfig* raw =
            __certo_host_winning_config(host, binding->key->name);
        if (raw) {
            CertoHostConfigParser parse =
                (CertoHostConfigParser)binding->key->parse.fn;
            void* result = parse(binding->key->parse.env, raw->value);
            if (!__result_is_ok(result)) {
                errors = __certo_host_add_configuration_error(
                    host, errors, binding->key->name, raw->source,
                    raw->location, "Parse",
                    binding->key->secret_bearing
                        ? "secret configuration value could not be parsed"
                        : (certo_text_t)__result_unwrap(result));
                continue;
            }
            intptr_t parsed = __result_unwrap(result);
            if (binding->key->parsed_value_boxed) {
                binding->value = (void*)parsed;
            } else {
                intptr_t* boxed = (intptr_t*)malloc(sizeof(intptr_t));
                if (!boxed) certo_panic("out of memory");
                *boxed = parsed;
                binding->value = boxed;
            }
            binding->bound = true;
        } else if (binding->has_default) {
            binding->value = binding->default_value;
            binding->bound = true;
        } else if (binding->required) {
            errors = __certo_host_add_configuration_error(
                host, errors, binding->key->name, "None", "", "Missing",
                "required configuration value is missing");
            continue;
        }
        if (!binding->bound) continue;
        certo_text_t source = raw ? raw->source : "Default";
        for (int64_t v = 0; v < binding->validators->len; v++) {
            certo_fn_t* callback =
                (certo_fn_t*)binding->validators->data[v];
            CertoHostConfigValidator validate =
                (CertoHostConfigValidator)callback->fn;
            void* result = validate(
                callback->env, (void*)(intptr_t)(*(intptr_t*)binding->value));
            if (!__result_is_ok(result))
                errors = __certo_host_add_configuration_error(
                    host, errors, binding->key->name, source,
                    raw ? raw->location : "typed default", "Validation",
                    binding->key->secret_bearing
                        ? "secret configuration value failed validation"
                        : (certo_text_t)__result_unwrap(result));
        }
    }
    if (errors) {
        CertoHostConfigurationError* first =
            (CertoHostConfigurationError*)host->configuration_errors->data[0];
        __certo_host_record_failure(host, "ConfigurationFailure", "configuration",
                                    first->key, first->message, false);
    }
    return errors;
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
    plugin->scoped_services = certo_list_new_empty();
    plugin->scoped_construction_order = certo_list_new_empty();
    return plugin;
}

CertoHostPlugin* certo_host_plugin_provide_factory(
        CertoHostPlugin* plugin, CertoServiceKey* key,
        certo_fn_t factory, certo_fn_t dispose) {
    if (!plugin || !key || !key->name || !factory.fn || !dispose.fn)
        certo_panic("HostPlugin.provideFactory requires a plugin, service key, factory, and disposer");
    for (int64_t i = 0; i < plugin->scoped_services->len; i++) {
        CertoHostService* existing =
            (CertoHostService*)plugin->scoped_services->data[i];
        if (strcmp(existing->key->name, key->name) == 0)
            certo_panic("a scoped service with this name is already registered");
    }
    CertoHostService* service = (CertoHostService*)calloc(1, sizeof(CertoHostService));
    if (!service) certo_panic("out of memory");
    service->key = key;
    service->factory = factory;
    service->dispose = dispose;
    service->dependencies = certo_list_new_empty();
    service->owned = true;
    plugin->scoped_services = certo_list_push_mut(plugin->scoped_services, service);
    return plugin;
}

CertoHostPlugin* certo_host_plugin_factory_depends_on(
        CertoHostPlugin* plugin, CertoServiceKey* factory_key,
        CertoServiceKey* dependency_key) {
    if (!plugin || !factory_key || !dependency_key)
        certo_panic("HostPlugin.factoryDependsOn requires a plugin and two service keys");
    CertoHostService* factory = NULL;
    for (int64_t i = 0; i < plugin->scoped_services->len; i++) {
        CertoHostService* service =
            (CertoHostService*)plugin->scoped_services->data[i];
        if (strcmp(service->key->name, factory_key->name) == 0) {
            factory = service;
            break;
        }
    }
    if (!factory)
        certo_panic("scoped dependency target is not a registered factory");
    for (int64_t i = 0; i < factory->dependencies->len; i++) {
        CertoServiceKey* existing =
            (CertoServiceKey*)factory->dependencies->data[i];
        if (strcmp(existing->name, dependency_key->name) == 0)
            certo_panic("duplicate scoped service factory dependency");
    }
    factory->dependencies = certo_list_push_mut(
        factory->dependencies, dependency_key);
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
    CERTO_ATOMIC_STORE(&worker->enabled, 1);
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
    plugin->context = (CertoHostContext*)calloc(1, sizeof(CertoHostContext));
    if (!plugin->context) certo_panic("out of memory");
    plugin->context->plugin_count = host->plugins->len;
    plugin->context->services = host->context->services;
    plugin->context->config = host->context->config;
    plugin->context->host = host;
    plugin->context->plugin_name = plugin->name;
    plugin->context->correlation_id = host->context->correlation_id;
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

CertoHost* certo_host_disposal_timeout(CertoHost* host, int64_t timeout_ms) {
    if (!host) certo_panic("Host.disposalTimeout called with a null host");
    if (host->running || host->started_count > 0)
        certo_panic("disposal timeout cannot be changed after the host has started");
    host->disposal_timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
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

static void __certo_host_wait_for_startup(CertoHost* host) {
#if defined(_WIN32)
    EnterCriticalSection(&host->wait_lock);
    while (CERTO_ATOMIC_LOAD(&host->state) == CERTO_HOST_STARTING &&
           !CERTO_ATOMIC_LOAD(&host->startup_complete))
        SleepConditionVariableCS(&host->wait_changed, &host->wait_lock, INFINITE);
    LeaveCriticalSection(&host->wait_lock);
#else
    pthread_mutex_lock(&host->wait_lock);
    while (CERTO_ATOMIC_LOAD(&host->state) == CERTO_HOST_STARTING &&
           !CERTO_ATOMIC_LOAD(&host->startup_complete))
        pthread_cond_wait(&host->wait_changed, &host->wait_lock);
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
    __certo_host_reject_secret_identity(host, name, "a metric name");
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        if (metric->gauge == gauge && strcmp(metric->name, name) == 0) return metric;
        if (metric->gauge != gauge && strcmp(metric->name, name) == 0)
            certo_panic("metric name is already registered with another kind");
    }
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* descriptor =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        if (strcmp(descriptor->name, name) == 0)
            certo_panic("metric name is already registered as a typed descriptor");
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

static bool __certo_host_text_list_equal(CertoList* left, CertoList* right) {
    if (!left || !right || left->len != right->len) return false;
    for (int64_t i = 0; i < left->len; i++)
        if (strcmp((certo_text_t)left->data[i], (certo_text_t)right->data[i]) != 0)
            return false;
    return true;
}

static bool __certo_host_int_list_equal(CertoList* left, CertoList* right) {
    if (!left || !right || left->len != right->len) return false;
    for (int64_t i = 0; i < left->len; i++)
        if ((int64_t)(intptr_t)left->data[i] !=
            (int64_t)(intptr_t)right->data[i]) return false;
    return true;
}

static char* __certo_host_prometheus_name(certo_text_t name, int kind) {
    size_t len = strlen(name ? name : "");
    bool add_total = kind == CERTO_METRIC_COUNTER &&
        (len < 6 || strcmp(name + len - 6, "_total") != 0);
    char* normalized = (char*)malloc(len + (add_total ? 7 : 1));
    if (!normalized) certo_panic("out of memory");
    for (size_t i = 0; i < len; i++) {
        unsigned char c = (unsigned char)name[i];
        bool valid = i == 0 ? (isalpha(c) || c == '_' || c == ':')
                            : (isalnum(c) || c == '_' || c == ':');
        normalized[i] = valid ? (char)c : '_';
    }
    normalized[len] = '\0';
    if (add_total) strcat(normalized, "_total");
    return normalized;
}

static bool __certo_host_prometheus_histogram_component(
        const char* histogram_name, const char* candidate) {
    size_t len = strlen(histogram_name);
    if (strncmp(histogram_name, candidate, len) != 0) return false;
    const char* suffix = candidate + len;
    return strcmp(suffix, "_bucket") == 0 || strcmp(suffix, "_sum") == 0 ||
           strcmp(suffix, "_count") == 0;
}

static bool __certo_host_prometheus_family_collision(
        const char* left, int left_kind, const char* right, int right_kind) {
    if (strcmp(left, right) == 0) return true;
    if (left_kind == CERTO_METRIC_HISTOGRAM &&
        __certo_host_prometheus_histogram_component(left, right)) return true;
    if (right_kind == CERTO_METRIC_HISTOGRAM &&
        __certo_host_prometheus_histogram_component(right, left)) return true;
    return false;
}

static void __certo_host_validate_prometheus_labels(
        CertoList* label_names, int kind) {
    for (int64_t i = 0; i < label_names->len; i++) {
        char* current = __certo_host_prometheus_name(
            (certo_text_t)label_names->data[i], CERTO_METRIC_GAUGE);
        if (kind == CERTO_METRIC_HISTOGRAM && strcmp(current, "le") == 0) {
            free(current);
            certo_panic("histogram label name collides with Prometheus le label");
        }
        for (int64_t j = 0; j < i; j++) {
            char* previous = __certo_host_prometheus_name(
                (certo_text_t)label_names->data[j], CERTO_METRIC_GAUGE);
            bool collision = strcmp(current, previous) == 0;
            free(previous);
            if (collision) {
                free(current);
                certo_panic("label names collide after Prometheus normalization");
            }
        }
        free(current);
    }
}

static CertoHostMetricDescriptor* __certo_host_metric_descriptor(
        CertoHost* host, certo_text_t name, certo_text_t help, certo_text_t unit,
        int kind, CertoList* label_names, CertoList* buckets, int64_t max_series) {
    if (!host || !__certo_host_valid_metric_name(name))
        certo_panic("invalid metric descriptor name");
    if (CERTO_ATOMIC_LOAD(&host->state) != CERTO_HOST_NEW)
        certo_panic("metric descriptors cannot be registered after startup begins");
    if (max_series <= 0) certo_panic("metric series limit must be positive");
    label_names = label_names ? label_names : certo_list_new_empty();
    buckets = buckets ? buckets : certo_list_new_empty();
    __certo_host_reject_secret_identity(host, name, "a metric name");
    __certo_host_reject_secret_identity(host, help, "metric help text");
    __certo_host_reject_secret_identity(host, unit, "a metric unit");
    for (int64_t i = 0; i < label_names->len; i++) {
        certo_text_t label = (certo_text_t)label_names->data[i];
        __certo_host_reject_secret_identity(host, label, "a metric label name");
        if (!__certo_host_valid_metric_name(label)) certo_panic("invalid metric label name");
        for (int64_t j = 0; j < i; j++)
            if (strcmp(label, (certo_text_t)label_names->data[j]) == 0)
                certo_panic("duplicate metric label name");
    }
    int64_t previous = -1;
    for (int64_t i = 0; i < buckets->len; i++) {
        int64_t boundary = (int64_t)(intptr_t)buckets->data[i];
        if (boundary < 0 || (i > 0 && boundary <= previous))
            certo_panic("histogram buckets must be non-negative and strictly increasing");
        previous = boundary;
    }
    for (int64_t i = 0; i < host->metrics->len; i++)
        if (strcmp(((CertoHostMetric*)host->metrics->data[i])->name, name) == 0)
            certo_panic("metric name is already used by a compatibility metric");
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* existing =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        if (strcmp(existing->name, name) != 0) continue;
        if (existing->kind == kind && strcmp(existing->help, help ? help : "") == 0 &&
            strcmp(existing->unit, unit ? unit : "") == 0 &&
            existing->max_series == max_series &&
            __certo_host_text_list_equal(existing->label_names, label_names) &&
            __certo_host_int_list_equal(existing->buckets, buckets)) return existing;
        certo_panic("metric descriptor conflicts with an existing registration");
    }
    char* exported_name = __certo_host_prometheus_name(name, kind);
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* existing =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        char* existing_name = __certo_host_prometheus_name(existing->name, existing->kind);
        bool collision = __certo_host_prometheus_family_collision(
            exported_name, kind, existing_name, existing->kind);
        free(existing_name);
        if (collision) {
            free(exported_name);
            certo_panic("metric names collide after Prometheus normalization");
        }
    }
    free(exported_name);
    __certo_host_validate_prometheus_labels(label_names, kind);
    if (kind != CERTO_METRIC_HISTOGRAM && buckets->len != 0)
        certo_panic("only histograms may declare buckets");
    CertoHostMetricDescriptor* descriptor =
        (CertoHostMetricDescriptor*)calloc(1, sizeof(CertoHostMetricDescriptor));
    if (!descriptor) certo_panic("out of memory");
    descriptor->host = host; descriptor->name = name;
    descriptor->help = help ? help : ""; descriptor->unit = unit ? unit : "";
    descriptor->kind = kind; descriptor->label_names = label_names;
    descriptor->buckets = buckets; descriptor->max_series = max_series;
    descriptor->series = certo_list_new_empty();
    host->metric_descriptors = certo_list_push_mut(host->metric_descriptors, descriptor);
    return descriptor;
}

CertoHostMetricDescriptor* certo_host_counter_metric(CertoHost* host,
        certo_text_t name, certo_text_t help, certo_text_t unit,
        CertoList* labels, int64_t max_series) {
    return __certo_host_metric_descriptor(host, name, help, unit,
        CERTO_METRIC_COUNTER, labels, certo_list_new_empty(), max_series);
}
CertoHostMetricDescriptor* certo_host_gauge_metric(CertoHost* host,
        certo_text_t name, certo_text_t help, certo_text_t unit,
        CertoList* labels, int64_t max_series) {
    return __certo_host_metric_descriptor(host, name, help, unit,
        CERTO_METRIC_GAUGE, labels, certo_list_new_empty(), max_series);
}
CertoHostMetricDescriptor* certo_host_histogram_metric(CertoHost* host,
        certo_text_t name, certo_text_t help, certo_text_t unit,
        CertoList* labels, CertoList* buckets, int64_t max_series) {
    return __certo_host_metric_descriptor(host, name, help, unit,
        CERTO_METRIC_HISTOGRAM, labels, buckets, max_series);
}

static int __certo_host_compare_label_values(CertoList* left, CertoList* right) {
    for (int64_t i = 0; i < left->len; i++) {
        int compared = strcmp((certo_text_t)left->data[i], (certo_text_t)right->data[i]);
        if (compared) return compared;
    }
    return 0;
}

static CertoHostMetricSeries* __certo_host_metric_series(
        CertoHostMetricDescriptor* descriptor, CertoList* labels) {
    if (!descriptor || !labels || labels->len != descriptor->label_names->len)
        certo_panic("metric label values must exactly match the descriptor labels");
    for (int64_t i = 0; i < labels->len; i++)
        __certo_host_reject_secret_identity(
            descriptor->host, (certo_text_t)labels->data[i], "a metric label value");
    for (int64_t i = 0; i < descriptor->series->len; i++) {
        CertoHostMetricSeries* series =
            (CertoHostMetricSeries*)descriptor->series->data[i];
        if (__certo_host_text_list_equal(series->labels, labels)) return series;
    }
    if (descriptor->series->len >= descriptor->max_series) return NULL;
    CertoHostMetricSeries* series =
        (CertoHostMetricSeries*)calloc(1, sizeof(CertoHostMetricSeries));
    if (!series) certo_panic("out of memory");
    series->labels = labels;
    if (descriptor->kind == CERTO_METRIC_HISTOGRAM) {
        series->bucket_counts = (int64_t*)calloc(
            (size_t)descriptor->buckets->len, sizeof(int64_t));
        if (descriptor->buckets->len && !series->bucket_counts) certo_panic("out of memory");
    }
    descriptor->series = certo_list_push_mut(descriptor->series, series);
    for (int64_t i = descriptor->series->len - 1; i > 0; i--) {
        CertoHostMetricSeries* before =
            (CertoHostMetricSeries*)descriptor->series->data[i - 1];
        if (__certo_host_compare_label_values(before->labels, labels) <= 0) break;
        descriptor->series->data[i] = descriptor->series->data[i - 1];
        descriptor->series->data[i - 1] = series;
    }
    return series;
}

static int64_t __certo_host_metric_update(CertoHostMetricDescriptor* descriptor,
        CertoList* labels, int64_t value, int operation) {
    if (!descriptor) certo_panic("metric descriptor is null");
    if ((operation == CERTO_METRIC_COUNTER || operation == CERTO_METRIC_HISTOGRAM) && value < 0)
        certo_panic("counter increments and histogram observations cannot be negative");
    CertoHost* host = descriptor->host;
    __certo_host_lock(host);
    if (descriptor->kind != operation) {
        __certo_host_unlock(host);
        certo_panic("metric operation does not match descriptor kind");
    }
    CertoHostMetricSeries* series = __certo_host_metric_series(descriptor, labels);
    if (!series) {
        __certo_host_unlock(host);
        __certo_host_metric_add(host, "host.telemetry.metric_series_dropped_total", 1);
        return 0;
    }
    if (operation == CERTO_METRIC_COUNTER) series->value += value;
    else if (operation == CERTO_METRIC_GAUGE) series->value = value;
    else {
        series->count++; series->sum += value;
        for (int64_t i = 0; i < descriptor->buckets->len; i++)
            if (value <= (int64_t)(intptr_t)descriptor->buckets->data[i])
                series->bucket_counts[i]++;
    }
    __certo_host_unlock(host);
    return 0;
}

int64_t certo_host_metric_counter_add(CertoHostMetricDescriptor* descriptor,
                                      CertoList* labels, int64_t amount) {
    return __certo_host_metric_update(descriptor, labels, amount, CERTO_METRIC_COUNTER);
}
int64_t certo_host_metric_gauge_set(CertoHostMetricDescriptor* descriptor,
                                    CertoList* labels, int64_t value) {
    return __certo_host_metric_update(descriptor, labels, value, CERTO_METRIC_GAUGE);
}
int64_t certo_host_metric_histogram_observe(CertoHostMetricDescriptor* descriptor,
                                            CertoList* labels, int64_t value) {
    return __certo_host_metric_update(descriptor, labels, value, CERTO_METRIC_HISTOGRAM);
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

static bool __certo_host_text_equal_ascii_ci(certo_text_t left,
                                              certo_text_t right) {
    if (!left || !right) return false;
    while (*left && *right) {
        if (tolower((unsigned char)*left) != tolower((unsigned char)*right))
            return false;
        left++; right++;
    }
    return *left == '\0' && *right == '\0';
}

static certo_text_t __certo_host_log_severity(certo_text_t level) {
    static const char* levels[] = { "Trace", "Debug", "Info", "Warn", "Error", "Fatal" };
    for (int i = 0; i < 6; i++)
        if (__certo_host_text_equal_ascii_ci(level, levels[i])) return levels[i];
    certo_panic("invalid host log severity");
}

certo_text_t certo_host_log_severity_trace(void) { return "Trace"; }
certo_text_t certo_host_log_severity_debug(void) { return "Debug"; }
certo_text_t certo_host_log_severity_info(void) { return "Info"; }
certo_text_t certo_host_log_severity_warn(void) { return "Warn"; }
certo_text_t certo_host_log_severity_error(void) { return "Error"; }
certo_text_t certo_host_log_severity_fatal(void) { return "Fatal"; }
certo_text_t certo_host_log_severity_name(certo_text_t severity) {
    return __certo_host_log_severity(severity);
}

static CertoHostLogField* __certo_host_log_field(certo_text_t name, int kind) {
    if (!__certo_host_valid_metric_name(name)) certo_panic("invalid host log field name");
    CertoHostLogField* field = (CertoHostLogField*)calloc(1, sizeof(CertoHostLogField));
    if (!field) certo_panic("out of memory");
    field->name = name;
    field->kind = kind;
    return field;
}

CertoHostLogField* certo_host_log_field_text(certo_text_t name, certo_text_t value) {
    CertoHostLogField* field = __certo_host_log_field(name, 0);
    field->text_value = value;
    return field;
}
CertoHostLogField* certo_host_log_field_int(certo_text_t name, int64_t value) {
    CertoHostLogField* field = __certo_host_log_field(name, 1);
    field->int_value = value;
    return field;
}
CertoHostLogField* certo_host_log_field_float(certo_text_t name, double value) {
    CertoHostLogField* field = __certo_host_log_field(name, 2);
    field->float_value = value;
    return field;
}
CertoHostLogField* certo_host_log_field_bool(certo_text_t name, bool value) {
    CertoHostLogField* field = __certo_host_log_field(name, 3);
    field->bool_value = value;
    return field;
}
certo_text_t certo_host_log_field_name(CertoHostLogField* field) {
    return field ? field->name : "";
}
certo_text_t certo_host_log_field_kind(CertoHostLogField* field) {
    if (!field) return "Text";
    switch (field->kind) {
        case 1: return "Int";
        case 2: return "Float";
        case 3: return "Bool";
        default: return "Text";
    }
}

CertoHostLogEvent* certo_host_log_event_create(certo_text_t severity,
                                                certo_text_t event,
                                                certo_text_t message,
                                                CertoList* fields) {
    if (!event || !*event || !__certo_host_valid_metric_name(event))
        certo_panic("invalid host log event name");
    CertoHostLogEvent* item = (CertoHostLogEvent*)calloc(1, sizeof(CertoHostLogEvent));
    if (!item) certo_panic("out of memory");
    item->schema = "certo.host.event/v1";
    item->severity = __certo_host_log_severity(severity);
    item->event = event;
    item->message = message ? message : "";
    item->fields = fields ? fields : certo_list_new_empty();
    for (int64_t i = 0; i < item->fields->len; i++) {
        CertoHostLogField* field = (CertoHostLogField*)item->fields->data[i];
        if (!field) certo_panic("host log fields cannot contain null values");
        for (int64_t j = 0; j < i; j++) {
            CertoHostLogField* earlier = (CertoHostLogField*)item->fields->data[j];
            if (strcmp(earlier->name, field->name) == 0)
                certo_panic("duplicate host log field name");
        }
    }
    return item;
}

static void __certo_host_write_log_field(CertoHostLogField* field) {
    __certo_host_write_json_string(stderr, field->name);
    fputc(':', stderr);
    switch (field->kind) {
        case 1: fprintf(stderr, "%" PRId64, field->int_value); break;
        case 2: fprintf(stderr, "%.17g", field->float_value); break;
        case 3: fputs(field->bool_value ? "true" : "false", stderr); break;
        default: __certo_host_write_json_string(stderr, field->text_value); break;
    }
}

typedef void* (*CertoHostLogWriteCallback)(void* env, void* event);
typedef void* (*CertoHostLogFinalizerCallback)(void* env);

static void __certo_host_log_event_retain(CertoHostLogEvent* event) {
    __sync_add_and_fetch(&event->references, 1);
}

static void __certo_host_log_event_release(CertoHostLogEvent* event) {
    if (__sync_sub_and_fetch(&event->references, 1) == 0) free(event);
}

static void __certo_host_sink_lock(CertoHostLogSink* sink) {
#if defined(_WIN32)
    EnterCriticalSection(&sink->lock);
#else
    pthread_mutex_lock(&sink->lock);
#endif
}

static void __certo_host_sink_unlock(CertoHostLogSink* sink) {
#if defined(_WIN32)
    LeaveCriticalSection(&sink->lock);
#else
    pthread_mutex_unlock(&sink->lock);
#endif
}

static void __certo_host_sink_wake(CertoHostLogSink* sink) {
#if defined(_WIN32)
    WakeAllConditionVariable(&sink->changed);
#else
    pthread_cond_broadcast(&sink->changed);
#endif
}

static bool __certo_host_sink_wait(CertoHostLogSink* sink, int64_t timeout_ms) {
#if defined(_WIN32)
    return SleepConditionVariableCS(&sink->changed, &sink->lock,
        timeout_ms < 0 ? INFINITE : (DWORD)timeout_ms) != 0;
#else
    if (timeout_ms < 0) return pthread_cond_wait(&sink->changed, &sink->lock) == 0;
    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline);
    deadline.tv_sec += timeout_ms / 1000;
    deadline.tv_nsec += (timeout_ms % 1000) * 1000000L;
    if (deadline.tv_nsec >= 1000000000L) {
        deadline.tv_sec++;
        deadline.tv_nsec -= 1000000000L;
    }
    return pthread_cond_timedwait(&sink->changed, &sink->lock, &deadline) == 0;
#endif
}

static void __certo_host_sink_failure(CertoHostLogSink* sink) {
    CertoHost* host = sink->host;
    __certo_host_metric_add(host, "host.telemetry.sink_failures_total", 1);
    if (sink->failure.mode == CERTO_LOG_FAILURE_DISABLE) {
        __certo_host_sink_lock(sink);
        sink->disabled = true;
        while (sink->length > 0) {
            CertoHostLogEvent* event = sink->queue[sink->head];
            sink->head = (sink->head + 1) % sink->capacity;
            sink->length--;
            __certo_host_log_event_release(event);
        }
        __certo_host_sink_wake(sink);
        __certo_host_sink_unlock(sink);
    } else if (sink->failure.mode == CERTO_LOG_FAILURE_FAIL_HOST) {
        __certo_host_record_failure(host, "TelemetryFailure", "telemetry",
            sink->name, "sink callback failed", false);
        CERTO_ATOMIC_STORE(&host->stop_requested, 1);
        __certo_host_wake_waiters(host);
    }
}

static void* __certo_host_log_sink_main(void* raw) {
    CertoHostLogSink* sink = (CertoHostLogSink*)raw;
    for (;;) {
        __certo_host_sink_lock(sink);
        while (sink->length == 0 && sink->accepting)
            __certo_host_sink_wait(sink, -1);
        if (sink->length == 0 && !sink->accepting) {
            __certo_host_sink_unlock(sink);
            break;
        }
        CertoHostLogEvent* event = sink->queue[sink->head];
        sink->head = (sink->head + 1) % sink->capacity;
        sink->length--;
        __certo_host_sink_wake(sink);
        bool disabled = sink->disabled;
        __certo_host_sink_unlock(sink);
        if (!disabled) {
            CertoHostLogWriteCallback write = (CertoHostLogWriteCallback)sink->write.fn;
            void* result = write(sink->write.env, event);
            if (!__result_is_ok(result)) __certo_host_sink_failure(sink);
        }
        __certo_host_log_event_release(event);
    }
    CertoHostLogFinalizerCallback finalizer;
    if (sink->flush.fn) {
        finalizer = (CertoHostLogFinalizerCallback)sink->flush.fn;
        if (!__result_is_ok(finalizer(sink->flush.env))) __certo_host_sink_failure(sink);
    }
    finalizer = (CertoHostLogFinalizerCallback)sink->dispose.fn;
    if (!__result_is_ok(finalizer(sink->dispose.env))) __certo_host_sink_failure(sink);
    __certo_task_signal_done(&sink->hdr);
    return NULL;
}

static bool __certo_host_sink_admit(CertoHostLogSink* sink,
                                    CertoHostLogEvent* event) {
    __certo_host_sink_lock(sink);
    if (!sink->accepting || sink->disabled) {
        __certo_host_sink_unlock(sink);
        return false;
    }
    while (sink->length == sink->capacity) {
        if (sink->overflow.mode == CERTO_LOG_DROP_NEWEST) {
            __certo_host_sink_unlock(sink);
            return false;
        }
        if (sink->overflow.mode == CERTO_LOG_DROP_OLDEST) {
            CertoHostLogEvent* oldest = sink->queue[sink->head];
            sink->head = (sink->head + 1) % sink->capacity;
            sink->length--;
            __certo_host_log_event_release(oldest);
            break;
        }
        if (!__certo_host_sink_wait(sink, sink->overflow.wait_ms) ||
            !sink->accepting) {
            __certo_host_sink_unlock(sink);
            return false;
        }
    }
    int64_t tail = (sink->head + sink->length) % sink->capacity;
    __certo_host_log_event_retain(event);
    sink->queue[tail] = event;
    sink->length++;
    __certo_host_sink_wake(sink);
    __certo_host_sink_unlock(sink);
    return true;
}

static int64_t __certo_host_emit_log_event(CertoHostContext* context,
                                           CertoHostLogEvent* event) {
    if (!context || !context->host || !event) return 0;
    CertoHost* host = context->host;
    CertoHostLogEvent* accepted =
        (CertoHostLogEvent*)malloc(sizeof(CertoHostLogEvent));
    if (!accepted) certo_panic("out of memory");
    *accepted = *event;
    accepted->references = 1;
    certo_text_t plugin = context->worker
        ? context->worker->plugin_name : context->plugin_name;
    certo_text_t worker = context->worker ? context->worker->name : NULL;
    __certo_host_reject_secret_identity(host, accepted->event, "an event name");
    __certo_host_reject_secret_identity(host, plugin, "a plugin identity");
    __certo_host_reject_secret_identity(host, worker, "a worker identity");
    __certo_host_reject_secret_identity(
        host, context->correlation_id, "a correlation identity");
    accepted->message = __certo_host_sanitize(host, accepted->message);
    CertoList* safe_fields = certo_list_new_empty();
    for (int64_t i = 0; i < accepted->fields->len; i++) {
        CertoHostLogField* source =
            (CertoHostLogField*)accepted->fields->data[i];
        __certo_host_reject_secret_identity(host, source->name, "an event field name");
        CertoHostLogField* field =
            (CertoHostLogField*)malloc(sizeof(CertoHostLogField));
        if (!field) certo_panic("out of memory");
        *field = *source;
        if (field->kind == 0)
            field->text_value = __certo_host_sanitize(host, field->text_value);
        safe_fields = certo_list_push_mut(safe_fields, field);
    }
    accepted->fields = safe_fields;
    __certo_host_lock(host);
    accepted->sequence = ++host->event_sequence;
    accepted->timestamp_unix_ms = (int64_t)time(NULL) * 1000;
    accepted->host_id = host->host_id;
    accepted->plugin = plugin;
    accepted->worker = worker;
    accepted->correlation_id = context->correlation_id;
    bool stderrEnabled = host->stderr_log_enabled;
    if (stderrEnabled) {
    fputs("{\"schema\":", stderr); __certo_host_write_json_string(stderr, accepted->schema);
    fprintf(stderr, ",\"sequence\":%" PRId64, accepted->sequence);
    fprintf(stderr, ",\"timestamp_unix_ms\":%" PRId64, accepted->timestamp_unix_ms);
    fputs(",\"severity\":", stderr); __certo_host_write_json_string(stderr, accepted->severity);
    fputs(",\"event\":", stderr); __certo_host_write_json_string(stderr, accepted->event);
    fputs(",\"message\":", stderr); __certo_host_write_json_string(stderr, accepted->message);
    fputs(",\"host_id\":", stderr); __certo_host_write_json_string(stderr, accepted->host_id);
    fputs(",\"plugin\":", stderr);
    if (accepted->plugin) __certo_host_write_json_string(stderr, accepted->plugin); else fputs("null", stderr);
    fputs(",\"worker\":", stderr);
    if (accepted->worker) __certo_host_write_json_string(stderr, accepted->worker); else fputs("null", stderr);
    fputs(",\"correlation_id\":", stderr);
    if (accepted->correlation_id) __certo_host_write_json_string(stderr, accepted->correlation_id); else fputs("null", stderr);
    fputs(",\"fields\":{", stderr);
    for (int64_t i = 0; i < accepted->fields->len; i++) {
        if (i) fputc(',', stderr);
        __certo_host_write_log_field((CertoHostLogField*)accepted->fields->data[i]);
    }
    fputs("}}\n", stderr);
    fflush(stderr);
    }
    int64_t dropped = 0;
    for (int64_t i = 0; i < host->log_sinks->len; i++) {
        CertoHostLogSink* sink = (CertoHostLogSink*)host->log_sinks->data[i];
        if (!__certo_host_sink_admit(sink, accepted))
            dropped++;
    }
    __certo_host_unlock(host);
    if (dropped) __certo_host_metric_add(
        host, "host.telemetry.events_dropped_total", dropped);
    __certo_host_log_event_release(accepted);
    return 0;
}

int64_t certo_host_context_log(CertoHostContext* context,
                               certo_text_t level,
                               certo_text_t event,
                               certo_text_t message) {
    CertoHostLogEvent* item = certo_host_log_event_create(
        level, event, message, certo_list_new_empty());
    return __certo_host_emit_log_event(context, item);
}

int64_t certo_host_context_log_event(CertoHostContext* context,
                                     CertoHostLogEvent* event) {
    return __certo_host_emit_log_event(context, event);
}

certo_text_t certo_host_log_event_schema(CertoHostLogEvent* event) { return event ? event->schema : ""; }
int64_t certo_host_log_event_sequence(CertoHostLogEvent* event) { return event ? event->sequence : 0; }
int64_t certo_host_log_event_timestamp_unix_ms(CertoHostLogEvent* event) { return event ? event->timestamp_unix_ms : 0; }
certo_text_t certo_host_log_event_severity(CertoHostLogEvent* event) { return event ? event->severity : "Info"; }
certo_text_t certo_host_log_event_event(CertoHostLogEvent* item) { return item ? item->event : ""; }
certo_text_t certo_host_log_event_message(CertoHostLogEvent* event) { return event ? event->message : ""; }
certo_text_t certo_host_log_event_host_id(CertoHostLogEvent* event) { return event ? event->host_id : ""; }
void* certo_host_log_event_plugin(CertoHostLogEvent* event) {
    return event && event->plugin ? __certo_opt_box((int64_t)(intptr_t)event->plugin) : NULL;
}
void* certo_host_log_event_worker(CertoHostLogEvent* event) {
    return event && event->worker ? __certo_opt_box((int64_t)(intptr_t)event->worker) : NULL;
}
void* certo_host_log_event_correlation_id(CertoHostLogEvent* event) {
    return event && event->correlation_id ? __certo_opt_box((int64_t)(intptr_t)event->correlation_id) : NULL;
}
CertoList* certo_host_log_event_fields(CertoHostLogEvent* event) {
    return event ? event->fields : certo_list_new_empty();
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
        if (!CERTO_ATOMIC_LOAD(&worker->enabled)) {
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_DISABLED);
            CERTO_ATOMIC_STORE(&worker->inactive, 1);
            while (!CERTO_ATOMIC_LOAD(&worker->host->context->stopping) &&
                   !CERTO_ATOMIC_LOAD(&worker->enabled))
                __certo_host_interruptible_sleep(worker->host, 25);
            CERTO_ATOMIC_STORE(&worker->inactive, 0);
            continue;
        }
        CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STARTING);
        worker->result = __certo_host_call(worker->callback, worker->context);
        if (!CERTO_ATOMIC_LOAD(&worker->enabled)) {
            __certo_host_worker_clear_ready(worker);
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_DISABLED);
            continue;
        }
        bool failed = !__result_is_ok(worker->result) || !CERTO_ATOMIC_LOAD(&worker->ready);
        if (!__result_is_ok(worker->result)) {
            CERTO_ATOMIC_STORE(&worker->last_error, __certo_host_sanitize(
                worker->host, (certo_text_t)__result_unwrap(worker->result)));
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
            __certo_host_record_failure(worker->host, "WorkerFailure", "worker",
                                        worker->name, detail, false);
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
        __sync_bool_compare_and_swap(
            &worker->host->state, CERTO_HOST_HEALTHY, CERTO_HOST_STARTING);
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
    CERTO_ATOMIC_STORE(&host->worker_count, 0);
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t w = 0; w < plugin->workers->len; w++) {
            CertoHostWorker* worker = (CertoHostWorker*)plugin->workers->data[w];
            worker->context = (CertoHostContext*)calloc(1, sizeof(CertoHostContext));
            if (!worker->context) certo_panic("out of memory");
            worker->context->plugin_count = host->context->plugin_count;
            worker->context->services = host->context->services;
            worker->context->config = host->context->config;
            worker->context->host = host;
            worker->context->worker = worker;
            worker->context->plugin_name = plugin->name;
            worker->context->correlation_id = plugin->context->correlation_id;
            worker->host = host;
            worker->result = NULL;
            worker->joined = false;
            CERTO_ATOMIC_STORE(&worker->enabled, 1);
            CERTO_ATOMIC_STORE(&worker->inactive, 0);
            CERTO_ATOMIC_STORE(&worker->control_busy, 0);
            CERTO_ATOMIC_STORE(&worker->ready, 0);
            CERTO_ATOMIC_STORE(&worker->restart_count, 0);
            CERTO_ATOMIC_STORE(&worker->last_error, NULL);
            CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STARTING);
            __sync_add_and_fetch(&host->worker_count, 1);
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
    while (CERTO_ATOMIC_LOAD(&host->ready_workers) <
           CERTO_ATOMIC_LOAD(&host->worker_count)) {
        if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested))
            return "host startup cancelled";
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
    if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested))
        return "host startup cancelled";
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
                __certo_host_record_failure(host, "Timeout", "drain",
                                            worker->name, "shutdown timeout elapsed", true);
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

static bool __certo_host_has_registered_service(CertoList* services,
                                                certo_text_t name) {
    for (int64_t i = 0; i < services->len; i++) {
        CertoHostService* service = (CertoHostService*)services->data[i];
        if (strcmp(service->key->name, name) == 0) return true;
    }
    return false;
}

static certo_text_t __certo_host_order_scoped_services(
        CertoHost* host, CertoHostPlugin* plugin, int64_t* order) {
    int64_t n = plugin->scoped_services->len;
    if (n == 0) return NULL;
    int64_t* indegree = (int64_t*)calloc((size_t)n, sizeof(int64_t));
    bool* emitted = (bool*)calloc((size_t)n, sizeof(bool));
    bool* edges = (bool*)calloc((size_t)(n * n), sizeof(bool));
    if (!indegree || !emitted || !edges) certo_panic("out of memory");
    for (int64_t dependent = 0; dependent < n; dependent++) {
        CertoHostService* service =
            (CertoHostService*)plugin->scoped_services->data[dependent];
        for (int64_t d = 0; d < service->dependencies->len; d++) {
            CertoServiceKey* dependency =
                (CertoServiceKey*)service->dependencies->data[d];
            int64_t provider = -1;
            for (int64_t candidate = 0; candidate < n; candidate++) {
                CertoHostService* possible =
                    (CertoHostService*)plugin->scoped_services->data[candidate];
                if (strcmp(possible->key->name, dependency->name) == 0) {
                    provider = candidate;
                    break;
                }
            }
            if (provider < 0) {
                if (__certo_host_has_registered_service(
                        host->context->services, dependency->name)) continue;
                free(indegree); free(emitted); free(edges);
                return __certo_host_error("missing scoped dependency for",
                                          plugin->name, dependency->name);
            }
            if (!edges[provider * n + dependent]) {
                edges[provider * n + dependent] = true;
                indegree[dependent]++;
            }
        }
    }
    for (int64_t position = 0; position < n; position++) {
        int64_t next = -1;
        for (int64_t candidate = 0; candidate < n; candidate++) {
            if (!emitted[candidate] && indegree[candidate] == 0) {
                next = candidate;
                break;
            }
        }
        if (next < 0) {
            free(indegree); free(emitted); free(edges);
            return __certo_host_error("scoped service dependencies form a cycle in",
                                      plugin->name, "factory graph");
        }
        emitted[next] = true;
        order[position] = next;
        for (int64_t dependent = 0; dependent < n; dependent++)
            if (edges[next * n + dependent]) indegree[dependent]--;
    }
    free(indegree); free(emitted); free(edges);
    return NULL;
}

static certo_text_t __certo_host_validate_scoped_services(CertoHost* host) {
    for (int64_t p = 0; p < host->plugins->len; p++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[p];
        int64_t n = plugin->scoped_services->len;
        int64_t* order = n > 0
            ? (int64_t*)calloc((size_t)n, sizeof(int64_t)) : NULL;
        if (n > 0 && !order) certo_panic("out of memory");
        certo_text_t error = __certo_host_order_scoped_services(host, plugin, order);
        free(order);
        if (error) return error;
    }
    return NULL;
}

static certo_text_t __certo_host_dispose_service_order(
        CertoHost* host, CertoHostContext* context,
        CertoList* construction_order, certo_text_t scope) {
    certo_text_t errors = NULL;
    for (int64_t i = construction_order->len; i > 0; i--) {
        CertoHostService* service =
            (CertoHostService*)construction_order->data[i - 1];
        if (!service->owned || !service->constructed) continue;
        bool timed_out = false;
        void* result = __certo_host_call_timed(
            service->dispose, context, host->disposal_timeout_ms, &timed_out);
        if (timed_out) {
            size_t subject_len = strlen(scope) + strlen(service->key->name) + 2;
            char* subject = (char*)malloc(subject_len);
            if (!subject) certo_panic("out of memory");
            snprintf(subject, subject_len, "%s/%s", scope, service->key->name);
            __certo_host_record_failure(host, "Timeout", "dispose",
                                        subject, "timeout elapsed", true);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to dispose", subject, "timeout elapsed"));
            /* The callback remains cooperative and may still resolve its own
               service and dependencies. Retain the timed-out service and all
               remaining services in this scope. A scoped callback can also
               resolve host services, so retain those until process teardown. */
            if (strcmp(scope, "host") != 0)
                CERTO_ATOMIC_STORE(&host->retain_host_services, 1);
            break;
        } else if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            __certo_host_record_failure(host, "ShutdownFailure", "dispose",
                                        service->key->name, detail, false);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to dispose", service->key->name, detail));
        }
        service->constructed = false;
        service->value = NULL;
    }
    return errors;
}

static certo_text_t __certo_host_dispose_services(CertoHost* host) {
    if (CERTO_ATOMIC_LOAD(&host->retain_host_services)) return NULL;
    return __certo_host_dispose_service_order(
        host, host->context, host->service_construction_order, "host");
}

static certo_text_t __certo_host_construct_scoped_services(
        CertoHost* host, CertoHostPlugin* plugin) {
    int64_t n = plugin->scoped_services->len;
    if (n == 0) return NULL;
    int64_t* order = (int64_t*)calloc((size_t)n, sizeof(int64_t));
    if (!order) certo_panic("out of memory");
    certo_text_t graph_error =
        __certo_host_order_scoped_services(host, plugin, order);
    if (graph_error) { free(order); return graph_error; }
    for (int64_t position = 0; position < n; position++) {
        CertoHostService* service =
            (CertoHostService*)plugin->scoped_services->data[order[position]];
        void* result = __certo_host_call(service->factory, plugin->context);
        if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            certo_text_t error = __certo_host_error(
                "failed to construct scoped service", service->key->name, detail);
            certo_text_t rollback_error = __certo_host_dispose_service_order(
                host, plugin->context, plugin->scoped_construction_order,
                plugin->name);
            __certo_host_record_failure(host, "StartupFailure", "service",
                                        service->key->name, detail, false);
            free(order);
            return __certo_host_append_error(error, rollback_error);
        }
        service->value = (void*)(intptr_t)__result_unwrap(result);
        service->constructed = true;
        plugin->scoped_construction_order = certo_list_push_mut(
            plugin->scoped_construction_order, service);
    }
    free(order);
    return NULL;
}

static certo_text_t __certo_host_construct_services(CertoHost* host) {
    int64_t n = host->context->services->len;
    if (n == 0) return NULL;
    int64_t* indegree = (int64_t*)calloc((size_t)n, sizeof(int64_t));
    bool* emitted = (bool*)calloc((size_t)n, sizeof(bool));
    bool* edges = (bool*)calloc((size_t)(n * n), sizeof(bool));
    int64_t* order = (int64_t*)calloc((size_t)n, sizeof(int64_t));
    if (!indegree || !emitted || !edges || !order) certo_panic("out of memory");

    for (int64_t dependent = 0; dependent < n; dependent++) {
        CertoHostService* service =
            (CertoHostService*)host->context->services->data[dependent];
        if (!service->owned) continue;
        for (int64_t d = 0; d < service->dependencies->len; d++) {
            CertoServiceKey* dependency =
                (CertoServiceKey*)service->dependencies->data[d];
            int64_t provider = -1;
            for (int64_t candidate = 0; candidate < n; candidate++) {
                CertoHostService* possible =
                    (CertoHostService*)host->context->services->data[candidate];
                if (strcmp(possible->key->name, dependency->name) == 0) {
                    provider = candidate;
                    break;
                }
            }
            if (provider < 0) {
                free(indegree); free(emitted); free(edges); free(order);
                return __certo_host_error("missing factory dependency for",
                                          service->key->name, dependency->name);
            }
            if (!edges[provider * n + dependent]) {
                edges[provider * n + dependent] = true;
                indegree[dependent]++;
            }
        }
    }

    for (int64_t position = 0; position < n; position++) {
        int64_t next = -1;
        for (int64_t candidate = 0; candidate < n; candidate++) {
            if (!emitted[candidate] && indegree[candidate] == 0) {
                next = candidate;
                break;
            }
        }
        if (next < 0) {
            free(indegree); free(emitted); free(edges); free(order);
            return "service factory dependencies form a cycle";
        }
        emitted[next] = true;
        order[position] = next;
        for (int64_t dependent = 0; dependent < n; dependent++) {
            if (edges[next * n + dependent]) indegree[dependent]--;
        }
    }
    free(indegree); free(emitted); free(edges);

    for (int64_t position = 0; position < n; position++) {
        CertoHostService* service =
            (CertoHostService*)host->context->services->data[order[position]];
        if (!service->owned) continue;
        host->context->plugin_name = "host";
        void* result = __certo_host_call(service->factory, host->context);
        if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            certo_text_t error = __certo_host_error(
                "failed to construct service", service->key->name, detail);
            certo_text_t rollback_error = __certo_host_dispose_services(host);
            __certo_host_record_failure(host, "StartupFailure", "service",
                                        service->key->name, detail, false);
            free(order);
            return __certo_host_append_error(error, rollback_error);
        }
        service->value = (void*)(intptr_t)__result_unwrap(result);
        service->constructed = true;
        host->service_construction_order = certo_list_push_mut(
            host->service_construction_order, service);
    }
    free(order);
    return NULL;
}

static certo_text_t __certo_host_quiesce_started(CertoHost* host) {
    certo_text_t errors = NULL;
    for (int64_t i = host->started_count; i > 0; i--) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i - 1];
        if (!plugin->quiesce.fn) continue;
        bool timed_out = false;
        void* result = __certo_host_call_timed(
            plugin->quiesce, plugin->context, host->quiesce_timeout_ms, &timed_out);
        if (timed_out) {
            __certo_host_record_failure(host, "Timeout", "quiesce",
                                        plugin->name, "timeout elapsed", true);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to quiesce", plugin->name, "timeout elapsed"));
        } else if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            __certo_host_record_failure(host, "ShutdownFailure", "quiesce",
                                        plugin->name, detail, false);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to quiesce", plugin->name, detail));
        }
    }
    return errors;
}

static certo_text_t __certo_host_stop_started(CertoHost* host) {
    certo_text_t errors = NULL;
    while (host->started_count > 0) {
        int64_t index = --host->started_count;
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[index];
        bool timed_out = false;
        void* result = __certo_host_call_timed(
            plugin->stop, plugin->context, host->stop_timeout_ms, &timed_out);
        if (timed_out) {
            __certo_host_record_failure(host, "Timeout", "stop",
                                        plugin->name, "timeout elapsed", true);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to stop", plugin->name, "timeout elapsed"));
        } else if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            __certo_host_record_failure(host, "ShutdownFailure", "stop",
                                        plugin->name, detail, false);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to stop", plugin->name, detail));
        }
        if (!timed_out) {
            certo_text_t scoped_error = __certo_host_dispose_service_order(
                host, plugin->context, plugin->scoped_construction_order,
                plugin->name);
            errors = __certo_host_append_error(errors, scoped_error);
        }
    }
    host->running = false;
    return errors;
}

static void __certo_host_start_log_sinks(CertoHost* host) {
    for (int64_t i = 0; i < host->log_sinks->len; i++) {
        CertoHostLogSink* sink = (CertoHostLogSink*)host->log_sinks->data[i];
        __certo_task_hdr_init(&sink->hdr);
        sink->started = true;
        sink->hdr.thread = __certo_thread_spawn(__certo_host_log_sink_main, sink);
    }
}

static certo_text_t __certo_host_stop_log_sinks(CertoHost* host) {
    certo_text_t errors = NULL;
    for (int64_t i = 0; i < host->log_sinks->len; i++) {
        CertoHostLogSink* sink = (CertoHostLogSink*)host->log_sinks->data[i];
        __certo_host_sink_lock(sink);
        sink->accepting = false;
        __certo_host_sink_wake(sink);
        __certo_host_sink_unlock(sink);
    }
    for (int64_t i = host->log_sinks->len; i > 0; i--) {
        CertoHostLogSink* sink = (CertoHostLogSink*)host->log_sinks->data[i - 1];
        if (!sink->started) continue;
        if (!__certo_thread_join_timed(&sink->hdr, host->telemetry_timeout_ms)) {
            __certo_host_record_failure(host, "Timeout", "telemetry",
                                        sink->name, "timeout elapsed", true);
            errors = __certo_host_append_error(errors,
                __certo_host_error("failed to finalize telemetry sink",
                                   sink->name, "timeout elapsed"));
        }
    }
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
    CERTO_ATOMIC_STORE(&host->last_failure, NULL);
    host->configuration_errors = certo_list_new_empty();
    certo_text_t source_error = __certo_host_load_configuration_sources(host);
    if (source_error) {
        bool arguments = strncmp(source_error, "--config", 8) == 0;
        certo_text_t errors = __certo_host_add_configuration_error(
            host, NULL, arguments ? "<arguments>" : "<document>",
            arguments ? "CommandLine" : "Toml",
            arguments ? "arguments" : "certo.toml", "Syntax", source_error);
        __certo_host_record_failure(host, "ConfigurationFailure", "configuration",
                                    arguments ? "<arguments>" : "<document>",
                                    source_error, false);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)errors);
    }
    certo_text_t configuration_error = __certo_host_bind_configuration(host);
    if (configuration_error) {
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)configuration_error);
    }
    certo_text_t dependency_error = __certo_host_order_plugins(host);
    if (dependency_error) {
        __certo_host_record_failure(host, "StartupFailure", "dependencies",
                                    "host", dependency_error, false);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)dependency_error);
    }
    dependency_error = __certo_host_validate_scoped_services(host);
    if (dependency_error) {
        __certo_host_record_failure(host, "StartupFailure", "dependencies",
                                    "host", dependency_error, false);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)dependency_error);
    }
    if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)) {
        __certo_host_record_failure(host, "StartupCancelled", "startup",
                                    "host", "host startup cancelled", false);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)"host startup cancelled");
    }
    certo_text_t service_error = __certo_host_construct_services(host);
    if (service_error) {
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)service_error);
    }
    if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)) {
        __certo_host_dispose_services(host);
        __certo_host_record_failure(host, "StartupCancelled", "startup",
                                    "host", "host startup cancelled", false);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)"host startup cancelled");
    }
    __certo_host_start_log_sinks(host);
    host->running = true;
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        plugin->context->plugin_count = host->plugins->len;
        plugin->context->services = host->context->services;
        plugin->context->config = host->context->config;
        certo_text_t scoped_error =
            __certo_host_construct_scoped_services(host, plugin);
        if (scoped_error) {
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
            __certo_host_stop_started(host);
            __certo_host_dispose_services(host);
            __certo_host_stop_log_sinks(host);
            __certo_host_wake_waiters(host);
            return certo_err((intptr_t)scoped_error);
        }
        if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)) {
            __certo_host_dispose_service_order(
                host, plugin->context, plugin->scoped_construction_order,
                plugin->name);
            __certo_host_record_failure(host, "StartupCancelled", "startup",
                                        plugin->name, "host startup cancelled", false);
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
            __certo_host_stop_started(host);
            __certo_host_dispose_services(host);
            __certo_host_stop_log_sinks(host);
            __certo_host_wake_waiters(host);
            return certo_err((intptr_t)"host startup cancelled");
        }
        void* result = __certo_host_call(plugin->start, plugin->context);
        if (!__result_is_ok(result)) {
            certo_text_t detail = __certo_host_sanitize(
                host, (certo_text_t)__result_unwrap(result));
            certo_text_t error = __certo_host_error(
                "failed to start", plugin->name,
                detail);
            __certo_host_record_failure(host, "StartupFailure", "start",
                                        plugin->name, detail, false);
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
            certo_text_t scoped_dispose_error =
                __certo_host_dispose_service_order(
                    host, plugin->context, plugin->scoped_construction_order,
                    plugin->name);
            __certo_host_stop_started(host);
            __certo_host_dispose_services(host);
            __certo_host_stop_log_sinks(host);
            __certo_host_wake_waiters(host);
            return certo_err((intptr_t)__certo_host_append_error(
                error, scoped_dispose_error));
        }
        host->started_count++;
        if (CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)) {
            __certo_host_record_failure(host, "StartupCancelled", "startup",
                                        plugin->name, "host startup cancelled", false);
            CERTO_ATOMIC_STORE(&host->context->stopping, 1);
            CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
            __certo_host_stop_started(host);
            __certo_host_dispose_services(host);
            __certo_host_stop_log_sinks(host);
            __certo_host_wake_waiters(host);
            return certo_err((intptr_t)"host startup cancelled");
        }
    }
    __certo_host_launch_workers(host);
    certo_text_t readiness_error = __certo_host_wait_until_ready(
        host, host->readiness_timeout_ms);
    if (readiness_error) {
        certo_text_t kind = CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)
            ? "StartupCancelled" : (CERTO_ATOMIC_LOAD(&host->worker_error)
                ? "WorkerFailure" : "Timeout");
        if (strcmp(kind, "WorkerFailure") != 0)
            __certo_host_record_failure(host, kind, "readiness", "host",
                                        readiness_error,
                                        strcmp(kind, "Timeout") == 0);
        CERTO_ATOMIC_STORE(&host->context->stopping, 1);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_join_workers(host);
        __certo_host_stop_started(host);
        __certo_host_dispose_services(host);
        __certo_host_stop_log_sinks(host);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)readiness_error);
    }
    certo_text_t listener_error = __certo_host_http_start_listeners(host);
    if (listener_error) {
        CERTO_ATOMIC_STORE(&host->context->stopping, 1);
        CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_FAILED);
        __certo_host_join_workers(host);
        __certo_host_stop_started(host);
        __certo_host_dispose_services(host);
        __certo_host_stop_log_sinks(host);
        __certo_host_record_failure(host, "StartupFailure", "http", "listener",
                                    listener_error, false);
        __certo_host_wake_waiters(host);
        return certo_err((intptr_t)listener_error);
    }
    CERTO_ATOMIC_STORE(&host->startup_complete, 1);
    __sync_bool_compare_and_swap(
        &host->state, CERTO_HOST_STARTING, CERTO_HOST_HEALTHY);
    __certo_host_wake_waiters(host);
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
    if (CERTO_ATOMIC_LOAD(&host->state) == CERTO_HOST_STARTING &&
        !CERTO_ATOMIC_LOAD(&host->startup_complete)) {
        __sync_bool_compare_and_swap(
            &host->stop_reason, NULL, "ApplicationRequest");
        CERTO_ATOMIC_STORE(&host->startup_cancel_requested, 1);
        CERTO_ATOMIC_STORE(&host->stop_requested, 1);
        CERTO_ATOMIC_STORE(&host->context->stopping, 1);
        __certo_host_wake_waiters(host);
        __certo_host_wait_for_startup(host);
        if (CERTO_ATOMIC_LOAD(&host->state) == CERTO_HOST_FAILED &&
            !CERTO_ATOMIC_LOAD(&host->worker_error))
            return certo_ok(0);
    }
    int state = CERTO_ATOMIC_LOAD(&host->state);
    bool can_shutdown = state == CERTO_HOST_HEALTHY ||
        (state == CERTO_HOST_STARTING &&
         CERTO_ATOMIC_LOAD(&host->startup_complete)) ||
        (state == CERTO_HOST_FAILED && CERTO_ATOMIC_LOAD(&host->worker_error));
    if (!can_shutdown && !CERTO_ATOMIC_LOAD(&host->shutdown_started))
        return certo_err((intptr_t)"host is not running");

    if (can_shutdown)
        __sync_bool_compare_and_swap(
            &host->stop_reason, NULL, "ApplicationRequest");

    bool owns_shutdown = can_shutdown && __sync_bool_compare_and_swap(
        &host->shutdown_started, 0, 1);
    if (!owns_shutdown) {
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
    CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_STOPPING);
    __certo_host_http_close_listeners(host);
    certo_text_t errors = __certo_host_quiesce_started(host);
    certo_text_t http_error = __certo_host_http_drain_listeners(host);
    CERTO_ATOMIC_STORE(&host->context->stopping, 1);
    __certo_host_wake_waiters(host);
    certo_text_t worker_error = __certo_host_join_workers(host);
    certo_text_t stop_error = __certo_host_stop_started(host);
    certo_text_t dispose_error = __certo_host_dispose_services(host);
    certo_text_t telemetry_error = __certo_host_stop_log_sinks(host);
    errors = __certo_host_append_error(errors, worker_error);
    errors = __certo_host_append_error(errors, http_error);
    errors = __certo_host_append_error(errors, stop_error);
    errors = __certo_host_append_error(errors, telemetry_error);
    errors = __certo_host_append_error(errors, dispose_error);
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
        __sync_bool_compare_and_swap(
            &context->host->stop_reason, NULL, "ApplicationRequest");
        CERTO_ATOMIC_STORE(&context->host->stop_requested, 1);
        __certo_host_wake_waiters(context->host);
    }
    return 0;
}

int64_t certo_host_request_shutdown(CertoHost* host) {
    if (!host) return 0;
    __sync_bool_compare_and_swap(&host->stop_reason, NULL, "ApplicationRequest");
    CERTO_ATOMIC_STORE(&host->stop_requested, 1);
    __certo_host_wake_waiters(host);
    return 0;
}

int64_t __certo_host_request_shutdown_reason(CertoHost* host, certo_text_t reason) {
    if (!host) return 0;
    __sync_bool_compare_and_swap(&host->stop_reason, NULL, reason);
    CERTO_ATOMIC_STORE(&host->stop_requested, 1);
    __certo_host_wake_waiters(host);
    return 0;
}

bool certo_host_context_is_stopping(CertoHostContext* context) {
    return context && context->host &&
        (CERTO_ATOMIC_LOAD(&context->host->context->stopping) ||
         (context->worker && !CERTO_ATOMIC_LOAD(&context->worker->enabled)));
}

int64_t certo_host_context_ready(CertoHostContext* context) {
    if (!context || !context->host || !context->worker) return 0;
    if (!CERTO_ATOMIC_LOAD(&context->worker->enabled)) return 0;
    if (__sync_bool_compare_and_swap(&context->worker->ready, 0, 1)) {
        __sync_add_and_fetch(&context->host->ready_workers, 1);
        int64_t ready_workers = CERTO_ATOMIC_LOAD(&context->host->ready_workers);
        certo_host_context_gauge(context, "host.worker.ready", ready_workers);
        CERTO_ATOMIC_STORE(&context->worker->health, CERTO_WORKER_HEALTHY);
        int host_state = CERTO_ATOMIC_LOAD(&context->host->state);
        if (CERTO_ATOMIC_LOAD(&context->host->startup_complete) &&
            ready_workers >= CERTO_ATOMIC_LOAD(&context->host->worker_count) &&
            host_state != CERTO_HOST_STOPPING && host_state != CERTO_HOST_FAILED)
            CERTO_ATOMIC_STORE(&context->host->state, CERTO_HOST_HEALTHY);
    }
    return 0;
}

int64_t certo_host_context_fail(CertoHostContext* context, certo_text_t error) {
    if (!context || !context->host) return 0;
    certo_text_t name = context->worker ? context->worker->name : "host";
    certo_text_t safe_error = __certo_host_sanitize(context->host, error);
    certo_text_t detail = __certo_host_error("worker failed", name, safe_error);
    __certo_host_record_failure(context->host, "WorkerFailure", "worker",
                                name, safe_error, false);
    __sync_bool_compare_and_swap(&context->host->worker_error, NULL, detail);
    CERTO_ATOMIC_STORE(&context->host->state, CERTO_HOST_FAILED);
    CERTO_ATOMIC_STORE(&context->host->stop_requested, 1);
    CERTO_ATOMIC_STORE(&context->host->context->stopping, 1);
    __certo_host_wake_waiters(context->host);
    return 0;
}

bool certo_host_context_sleep(CertoHostContext* context, int64_t duration_ms) {
    if (!context || !context->host || certo_host_context_is_stopping(context))
        return false;
    int64_t deadline = certo_monotonic_millis() + (duration_ms < 0 ? 0 : duration_ms);
    while (!certo_host_context_is_stopping(context)) {
        int64_t remaining = deadline - certo_monotonic_millis();
        if (remaining <= 0) return true;
        if (!__certo_host_interruptible_sleep(
                context->host, remaining > 25 ? 25 : remaining))
            return false;
    }
    return false;
}

typedef bool (*CertoHostPredicate)(void* env, void* context);

bool certo_host_context_wait_until(CertoHostContext* context,
                                    certo_fn_t predicate,
                                    int64_t interval_ms) {
    if (!context || !context->host || !predicate.fn) return false;
    CertoHostPredicate test = (CertoHostPredicate)predicate.fn;
    while (!certo_host_context_is_stopping(context)) {
        if (test(predicate.env, context)) return true;
        if (!certo_host_context_sleep(context, interval_ms)) return false;
    }
    return false;
}

certo_text_t certo_host_health(CertoHost* host) {
    if (!host) return "Failed";
    return __certo_host_state_text(CERTO_ATOMIC_LOAD(&host->state));
}

certo_text_t certo_host_state_name(certo_text_t state) {
    return state ? state : "Failed";
}

certo_text_t certo_host_worker_state_name(certo_text_t state) {
    return state ? state : "Failed";
}

certo_text_t certo_host_failure_kind_name(certo_text_t kind) {
    return kind ? kind : "StartupFailure";
}

certo_text_t certo_host_lifecycle_error_kind(CertoHostLifecycleError* error) {
    return error ? error->kind : "StartupFailure";
}

certo_text_t certo_host_lifecycle_error_phase(CertoHostLifecycleError* error) {
    return error ? error->phase : "host";
}

certo_text_t certo_host_lifecycle_error_subject(CertoHostLifecycleError* error) {
    return error ? error->subject : "host";
}

certo_text_t certo_host_lifecycle_error_message(CertoHostLifecycleError* error) {
    return error ? error->message : "unknown error";
}

bool certo_host_lifecycle_error_is_timeout(CertoHostLifecycleError* error) {
    return error && error->timed_out;
}

CertoList* certo_host_lifecycle_error_configuration_errors(
        CertoHostLifecycleError* error) {
    return error && error->configuration_errors
        ? error->configuration_errors : certo_list_new_empty();
}

certo_text_t certo_host_configuration_error_key(
        CertoHostConfigurationError* error) {
    return error ? error->key : "";
}

certo_text_t certo_host_configuration_error_source(
        CertoHostConfigurationError* error) {
    return error ? error->source : "None";
}

certo_text_t certo_host_configuration_error_category(
        CertoHostConfigurationError* error) {
    return error ? error->category : "Validation";
}

certo_text_t certo_host_configuration_error_location(
        CertoHostConfigurationError* error) {
    return error ? error->location : "";
}

certo_text_t certo_host_configuration_error_message(
        CertoHostConfigurationError* error) {
    return error ? error->message : "unknown configuration error";
}

static void* __certo_host_typed_result(CertoHost* host, void* result,
                                       certo_text_t kind, certo_text_t phase) {
    if (__result_is_ok(result)) return certo_ok(0);
    certo_text_t message = (certo_text_t)__result_unwrap(result);
    CertoHostLifecycleError* error = __certo_host_failure_or_fallback(
        host, kind, phase, message);
    return certo_err((intptr_t)error);
}

void* certo_host_start_typed(CertoHost* host) {
    return __certo_host_typed_result(
        host, certo_host_start(host), "StartupFailure", "startup");
}

void* certo_host_stop_typed(CertoHost* host) {
    return __certo_host_typed_result(
        host, certo_host_stop(host), "ShutdownFailure", "shutdown");
}

void* certo_host_run_typed(CertoHost* host) {
    return __certo_host_typed_result(
        host, certo_host_run(host), "ShutdownFailure", "run");
}

void* certo_host_wait_until_ready_typed(CertoHost* host, int64_t timeout_ms) {
    void* result = certo_host_wait_until_ready(host, timeout_ms);
    if (!__result_is_ok(result)) {
        certo_text_t message = (certo_text_t)__result_unwrap(result);
        certo_text_t kind = host && CERTO_ATOMIC_LOAD(&host->startup_cancel_requested)
            ? "StartupCancelled" : (host && CERTO_ATOMIC_LOAD(&host->worker_error)
                ? "WorkerFailure" : (strstr(message, "timed out")
                    ? "Timeout" : "StartupFailure"));
        __certo_host_record_failure(host, kind, "readiness", "host", message,
                                    strcmp(kind, "Timeout") == 0);
    }
    return __certo_host_typed_result(host, result, "Timeout", "readiness");
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

certo_text_t certo_host_disable_worker(CertoHost* host, certo_text_t name) {
    CertoHostWorker* worker = __certo_host_find_worker(host, name);
    if (!worker) return (void*)"UnknownWorker";
    int state = CERTO_ATOMIC_LOAD(&host->state);
    if (state != CERTO_HOST_HEALTHY && state != CERTO_HOST_STARTING)
        return (void*)"InvalidHostState";
    if (!__sync_bool_compare_and_swap(&worker->control_busy, 0, 1))
        return (void*)"ControlInProgress";
    if (!__sync_bool_compare_and_swap(&worker->enabled, 1, 0)) {
        CERTO_ATOMIC_STORE(&worker->control_busy, 0);
        return (void*)"AlreadyDisabled";
    }
    __certo_host_worker_clear_ready(worker);
    __sync_sub_and_fetch(&host->worker_count, 1);
    __certo_host_wake_waiters(host);
    int64_t deadline = certo_monotonic_millis() + host->shutdown_timeout_ms;
    while (!CERTO_ATOMIC_LOAD(&worker->inactive) &&
           certo_monotonic_millis() < deadline) {
#if defined(_WIN32)
        Sleep(1);
#else
        struct timespec delay = { 0, 1000000L };
        nanosleep(&delay, NULL);
#endif
    }
    if (!CERTO_ATOMIC_LOAD(&worker->inactive)) {
        CERTO_ATOMIC_STORE(&worker->control_busy, 0);
        return (void*)"Timeout";
    }
    CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_DISABLED);
    CERTO_ATOMIC_STORE(&worker->control_busy, 0);
    __certo_host_metric_add(host, "host.worker.disables", 1);
    return (void*)"Disabled";
}

certo_text_t certo_host_enable_worker(CertoHost* host, certo_text_t name) {
    CertoHostWorker* worker = __certo_host_find_worker(host, name);
    if (!worker) return (void*)"UnknownWorker";
    int state = CERTO_ATOMIC_LOAD(&host->state);
    if (CERTO_ATOMIC_LOAD(&worker->enabled) &&
        (state == CERTO_HOST_HEALTHY || state == CERTO_HOST_STARTING))
        return (void*)"AlreadyEnabled";
    if (state != CERTO_HOST_HEALTHY)
        return (void*)"InvalidHostState";
    if (!__sync_bool_compare_and_swap(&worker->control_busy, 0, 1))
        return (void*)"ControlInProgress";
    if (!CERTO_ATOMIC_LOAD(&worker->inactive)) {
        CERTO_ATOMIC_STORE(&worker->control_busy, 0);
        return (void*)"ControlInProgress";
    }
    worker->result = NULL;
    CERTO_ATOMIC_STORE(&worker->ready, 0);
    CERTO_ATOMIC_STORE(&worker->last_error, NULL);
    CERTO_ATOMIC_STORE(&worker->health, CERTO_WORKER_STARTING);
    CERTO_ATOMIC_STORE(&worker->enabled, 1);
    __sync_add_and_fetch(&host->worker_count, 1);
    CERTO_ATOMIC_STORE(&host->state, CERTO_HOST_STARTING);
    __certo_host_wake_waiters(host);
    CERTO_ATOMIC_STORE(&worker->control_busy, 0);
    __certo_host_metric_add(host, "host.worker.enables", 1);
    __certo_host_metric_add(host, "host.worker.starts", 1);
    return (void*)"Enabled";
}

certo_text_t certo_host_worker_control_status_name(certo_text_t status) {
    return status ? status : "InvalidHostState";
}

CertoHostStatusSnapshot* certo_host_status(CertoHost* host) {
    CertoHostStatusSnapshot* snapshot =
        (CertoHostStatusSnapshot*)calloc(1, sizeof(CertoHostStatusSnapshot));
    if (!snapshot) certo_panic("out of memory");
    snapshot->workers = certo_list_new_empty();
    snapshot->counters = certo_list_new_empty();
    snapshot->gauges = certo_list_new_empty();
    if (!host) {
        snapshot->state = "Failed";
        snapshot->last_failure = __certo_host_record_failure(
            NULL, "StartupFailure", "status", "host", "host is null", false);
        return snapshot;
    }
    int state = CERTO_ATOMIC_LOAD(&host->state);
    snapshot->state = __certo_host_state_text(state);
    snapshot->worker_count = CERTO_ATOMIC_LOAD(&host->worker_count);
    snapshot->ready_workers = CERTO_ATOMIC_LOAD(&host->ready_workers);
    snapshot->ready = state == CERTO_HOST_HEALTHY &&
        snapshot->ready_workers >= snapshot->worker_count;
    snapshot->live = state != CERTO_HOST_FAILED && state != CERTO_HOST_STOPPED;
    snapshot->last_failure = CERTO_ATOMIC_LOAD(&host->last_failure);
    for (int64_t i = 0; i < host->plugins->len; i++) {
        CertoHostPlugin* plugin = (CertoHostPlugin*)host->plugins->data[i];
        for (int64_t w = 0; w < plugin->workers->len; w++) {
            CertoHostWorker* worker = (CertoHostWorker*)plugin->workers->data[w];
            CertoHostWorkerStatus* item =
                (CertoHostWorkerStatus*)calloc(1, sizeof(CertoHostWorkerStatus));
            if (!item) certo_panic("out of memory");
            int worker_state = CERTO_ATOMIC_LOAD(&worker->health);
            item->name = worker->name;
            item->plugin = worker->plugin_name;
            item->state = __certo_worker_state_text(worker_state);
            item->ready = CERTO_ATOMIC_LOAD(&worker->ready) != 0;
            item->live = worker_state != CERTO_WORKER_FAILED &&
                         worker_state != CERTO_WORKER_STOPPED;
            item->restarts = CERTO_ATOMIC_LOAD(&worker->restart_count);
            item->last_error = CERTO_ATOMIC_LOAD(&worker->last_error);
            item->enabled = CERTO_ATOMIC_LOAD(&worker->enabled) != 0;
            snapshot->workers = certo_list_push_mut(snapshot->workers, item);
        }
    }
    __certo_host_lock(host);
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        CertoHostMetricSnapshot* item =
            (CertoHostMetricSnapshot*)malloc(sizeof(CertoHostMetricSnapshot));
        if (!item) certo_panic("out of memory");
        item->name = metric->name;
        item->value = metric->value;
        if (metric->gauge)
            snapshot->gauges = certo_list_push_mut(snapshot->gauges, item);
        else
            snapshot->counters = certo_list_push_mut(snapshot->counters, item);
    }
    __certo_host_unlock(host);
    return snapshot;
}

certo_text_t certo_host_status_snapshot_state(CertoHostStatusSnapshot* s) {
    return s ? s->state : "Failed";
}
bool certo_host_status_snapshot_is_ready(CertoHostStatusSnapshot* s) {
    return s && s->ready;
}
bool certo_host_status_snapshot_is_live(CertoHostStatusSnapshot* s) {
    return s && s->live;
}
int64_t certo_host_status_snapshot_worker_count(CertoHostStatusSnapshot* s) {
    return s ? s->worker_count : 0;
}
int64_t certo_host_status_snapshot_ready_workers(CertoHostStatusSnapshot* s) {
    return s ? s->ready_workers : 0;
}
CertoList* certo_host_status_snapshot_workers(CertoHostStatusSnapshot* s) {
    return s ? s->workers : certo_list_new_empty();
}
CertoList* certo_host_status_snapshot_counters(CertoHostStatusSnapshot* s) {
    return s ? s->counters : certo_list_new_empty();
}
CertoList* certo_host_status_snapshot_gauges(CertoHostStatusSnapshot* s) {
    return s ? s->gauges : certo_list_new_empty();
}
void* certo_host_status_snapshot_last_failure(CertoHostStatusSnapshot* s) {
    return s && s->last_failure
        ? __certo_opt_box((int64_t)(intptr_t)s->last_failure) : NULL;
}

CertoHostOperationalStatus* certo_host_operation_status(CertoHost* host) {
    CertoHostOperationalStatus* status =
        (CertoHostOperationalStatus*)calloc(1, sizeof(CertoHostOperationalStatus));
    if (!status) certo_panic("out of memory");
    if (!host) {
        status->condition = "failed";
        status->state = "Failed";
        status->failure_kind = "StartupFailure";
        status->start_reason = "Application";
        status->stop_reason = "StartupFailure";
        return status;
    }
    int state = CERTO_ATOMIC_LOAD(&host->state);
    int64_t workers = CERTO_ATOMIC_LOAD(&host->worker_count);
    int64_t ready_workers = CERTO_ATOMIC_LOAD(&host->ready_workers);
    status->state = __certo_host_state_text(state);
    status->ready = state == CERTO_HOST_HEALTHY && ready_workers >= workers;
    status->live = state != CERTO_HOST_FAILED && state != CERTO_HOST_STOPPED;
    CertoHostLifecycleError* failure = CERTO_ATOMIC_LOAD(&host->last_failure);
    status->failure_kind = failure ? failure->kind : NULL;
    status->start_reason = host->start_reason;
    status->stop_reason = CERTO_ATOMIC_LOAD(&host->stop_reason);
    switch (state) {
        case CERTO_HOST_NEW:
            status->condition = "starting";
            break;
        case CERTO_HOST_STARTING:
            status->condition = CERTO_ATOMIC_LOAD(&host->startup_complete)
                ? "degraded" : "starting";
            break;
        case CERTO_HOST_HEALTHY:
            status->condition = "ready";
            break;
        case CERTO_HOST_STOPPING:
            status->condition = "stopping";
            break;
        case CERTO_HOST_STOPPED:
            status->condition = "stopped";
            break;
        default:
            status->condition = "failed";
            break;
    }
    return status;
}

certo_text_t certo_host_operational_status_condition(CertoHostOperationalStatus* s) {
    return s ? s->condition : "failed";
}
certo_text_t certo_host_operational_status_state(CertoHostOperationalStatus* s) {
    return s ? s->state : "Failed";
}
bool certo_host_operational_status_is_ready(CertoHostOperationalStatus* s) {
    return s && s->ready;
}
bool certo_host_operational_status_is_live(CertoHostOperationalStatus* s) {
    return s && s->live;
}
void* certo_host_operational_status_failure_kind(CertoHostOperationalStatus* s) {
    return s && s->failure_kind
        ? __certo_opt_box((int64_t)(intptr_t)s->failure_kind) : NULL;
}
certo_text_t certo_host_operational_condition_name(certo_text_t condition) {
    return condition ? condition : "failed";
}
certo_text_t certo_host_operational_status_start_reason(CertoHostOperationalStatus* s) {
    return s && s->start_reason ? s->start_reason : "Application";
}
void* certo_host_operational_status_stop_reason(CertoHostOperationalStatus* s) {
    return s && s->stop_reason
        ? __certo_opt_box((int64_t)(intptr_t)s->stop_reason) : NULL;
}

certo_text_t certo_host_worker_status_name(CertoHostWorkerStatus* s) {
    return s ? s->name : "";
}
certo_text_t certo_host_worker_status_plugin(CertoHostWorkerStatus* s) {
    return s ? s->plugin : "";
}
certo_text_t certo_host_worker_status_state(CertoHostWorkerStatus* s) {
    return s ? s->state : "Failed";
}
bool certo_host_worker_status_is_ready(CertoHostWorkerStatus* s) {
    return s && s->ready;
}
bool certo_host_worker_status_is_live(CertoHostWorkerStatus* s) {
    return s && s->live;
}
int64_t certo_host_worker_status_restarts(CertoHostWorkerStatus* s) {
    return s ? s->restarts : 0;
}
void* certo_host_worker_status_last_error(CertoHostWorkerStatus* s) {
    return s && s->last_error
        ? __certo_opt_box((int64_t)(intptr_t)s->last_error) : NULL;
}
bool certo_host_worker_status_is_enabled(CertoHostWorkerStatus* s) {
    return s && s->enabled;
}
certo_text_t certo_host_metric_snapshot_name(CertoHostMetricSnapshot* s) {
    return s ? s->name : "";
}
int64_t certo_host_metric_snapshot_value(CertoHostMetricSnapshot* s) {
    return s ? s->value : 0;
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

certo_text_t certo_host_metric_snapshot(CertoHost* host) {
    if (!host) return "{\"instruments\":[]}";
    __certo_host_lock(host);
    size_t cap = 256;
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* d =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        cap += (strlen(d->name) + strlen(d->help) + strlen(d->unit)) * 6 + 256;
        for (int64_t l = 0; l < d->label_names->len; l++)
            cap += strlen((certo_text_t)d->label_names->data[l]) * 6 + 8;
        for (int64_t s = 0; s < d->series->len; s++) {
            CertoHostMetricSeries* series =
                (CertoHostMetricSeries*)d->series->data[s];
            cap += 256 + (size_t)d->buckets->len * 64;
            for (int64_t l = 0; l < series->labels->len; l++)
                cap += strlen((certo_text_t)series->labels->data[l]) * 6 + 8;
        }
    }
    char* out = (char*)malloc(cap);
    if (!out) certo_panic("out of memory");
    size_t n = (size_t)snprintf(out, cap, "{\"instruments\":[");
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* d =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        char* name = __certo_host_json_quote(d->name);
        char* help = __certo_host_json_quote(d->help);
        char* unit = __certo_host_json_quote(d->unit);
        const char* kind = d->kind == CERTO_METRIC_COUNTER ? "Counter" :
            (d->kind == CERTO_METRIC_GAUGE ? "Gauge" : "Histogram");
        n += (size_t)snprintf(out + n, cap - n,
            "%s{\"name\":%s,\"kind\":\"%s\",\"help\":%s,\"unit\":%s,\"labels\":[",
            i ? "," : "", name, kind, help, unit);
        free(name); free(help); free(unit);
        for (int64_t l = 0; l < d->label_names->len; l++) {
            char* label = __certo_host_json_quote((certo_text_t)d->label_names->data[l]);
            n += (size_t)snprintf(out + n, cap - n, "%s%s", l ? "," : "", label);
            free(label);
        }
        n += (size_t)snprintf(out + n, cap - n, "],\"series\":[");
        for (int64_t s = 0; s < d->series->len; s++) {
            CertoHostMetricSeries* series =
                (CertoHostMetricSeries*)d->series->data[s];
            n += (size_t)snprintf(out + n, cap - n, "%s{\"labels\":[", s ? "," : "");
            for (int64_t l = 0; l < series->labels->len; l++) {
                char* value = __certo_host_json_quote((certo_text_t)series->labels->data[l]);
                n += (size_t)snprintf(out + n, cap - n, "%s%s", l ? "," : "", value);
                free(value);
            }
            if (d->kind != CERTO_METRIC_HISTOGRAM) {
                n += (size_t)snprintf(out + n, cap - n,
                    "],\"value\":%" PRId64 "}", series->value);
            } else {
                n += (size_t)snprintf(out + n, cap - n,
                    "],\"count\":%" PRId64 ",\"sum\":%" PRId64 ",\"buckets\":[",
                    series->count, series->sum);
                for (int64_t b = 0; b < d->buckets->len; b++)
                    n += (size_t)snprintf(out + n, cap - n,
                        "%s{\"le\":%" PRId64 ",\"count\":%" PRId64 "}",
                        b ? "," : "", (int64_t)(intptr_t)d->buckets->data[b],
                        series->bucket_counts[b]);
                n += (size_t)snprintf(out + n, cap - n, "]}");
            }
        }
        n += (size_t)snprintf(out + n, cap - n, "]}");
    }
    snprintf(out + n, cap - n, "]}");
    __certo_host_unlock(host);
    return out;
}

static void __certo_host_prom_append(char** out, size_t* len, size_t* cap,
                                     const char* format, ...) {
    for (;;) {
        va_list args;
        va_start(args, format);
        int needed = vsnprintf(*out + *len, *cap - *len, format, args);
        va_end(args);
        if (needed < 0) certo_panic("failed to format Prometheus metrics");
        if ((size_t)needed < *cap - *len) {
            *len += (size_t)needed;
            return;
        }
        *cap = (*cap * 2) + (size_t)needed + 1;
        char* grown = (char*)realloc(*out, *cap);
        if (!grown) certo_panic("out of memory");
        *out = grown;
    }
}

static void __certo_host_prom_escape(char** out, size_t* len, size_t* cap,
                                     certo_text_t text, bool label) {
    for (const unsigned char* p = (const unsigned char*)(text ? text : ""); *p; p++) {
        if (*p == '\\') __certo_host_prom_append(out, len, cap, "\\\\");
        else if (*p == '\n') __certo_host_prom_append(out, len, cap, "\\n");
        else if (label && *p == '"') __certo_host_prom_append(out, len, cap, "\\\"");
        else __certo_host_prom_append(out, len, cap, "%c", *p);
    }
}

static void __certo_host_prom_labels(char** out, size_t* len, size_t* cap,
        CertoHostMetricDescriptor* descriptor, CertoHostMetricSeries* series,
        bool histogram, certo_text_t boundary) {
    if (descriptor->label_names->len == 0 && !histogram) return;
    __certo_host_prom_append(out, len, cap, "{");
    for (int64_t i = 0; i < descriptor->label_names->len; i++) {
        char* label_name = __certo_host_prometheus_name(
            (certo_text_t)descriptor->label_names->data[i], CERTO_METRIC_GAUGE);
        __certo_host_prom_append(out, len, cap, "%s%s=\"", i ? "," : "", label_name);
        free(label_name);
        __certo_host_prom_escape(out, len, cap,
            (certo_text_t)series->labels->data[i], true);
        __certo_host_prom_append(out, len, cap, "\"");
    }
    if (histogram) {
        __certo_host_prom_append(out, len, cap, "%sle=\"%s\"",
            descriptor->label_names->len ? "," : "", boundary);
    }
    __certo_host_prom_append(out, len, cap, "}");
}

certo_text_t certo_host_metrics_prometheus(CertoHost* host) {
    if (!host) return "";
    size_t cap = 1024, len = 0;
    char* out = (char*)malloc(cap);
    if (!out) certo_panic("out of memory");
    out[0] = '\0';
    __certo_host_lock(host);
    for (int64_t i = 0; i < host->metrics->len; i++) {
        CertoHostMetric* metric = (CertoHostMetric*)host->metrics->data[i];
        char* name = __certo_host_prometheus_name(metric->name,
            metric->gauge ? CERTO_METRIC_GAUGE : CERTO_METRIC_COUNTER);
        __certo_host_prom_append(&out, &len, &cap, "# TYPE %s %s\n%s %" PRId64 "\n",
            name, metric->gauge ? "gauge" : "counter", name, metric->value);
        free(name);
    }
    for (int64_t i = 0; i < host->metric_descriptors->len; i++) {
        CertoHostMetricDescriptor* descriptor =
            (CertoHostMetricDescriptor*)host->metric_descriptors->data[i];
        char* name = __certo_host_prometheus_name(descriptor->name, descriptor->kind);
        __certo_host_prom_append(&out, &len, &cap, "# HELP %s ", name);
        __certo_host_prom_escape(&out, &len, &cap, descriptor->help, false);
        const char* kind = descriptor->kind == CERTO_METRIC_COUNTER ? "counter" :
            (descriptor->kind == CERTO_METRIC_GAUGE ? "gauge" : "histogram");
        __certo_host_prom_append(&out, &len, &cap, "\n# TYPE %s %s\n", name, kind);
        for (int64_t s = 0; s < descriptor->series->len; s++) {
            CertoHostMetricSeries* series =
                (CertoHostMetricSeries*)descriptor->series->data[s];
            if (descriptor->kind != CERTO_METRIC_HISTOGRAM) {
                __certo_host_prom_append(&out, &len, &cap, "%s", name);
                __certo_host_prom_labels(&out, &len, &cap, descriptor, series, false, NULL);
                __certo_host_prom_append(&out, &len, &cap, " %" PRId64 "\n", series->value);
                continue;
            }
            for (int64_t b = 0; b < descriptor->buckets->len; b++) {
                char boundary[32];
                snprintf(boundary, sizeof(boundary), "%" PRId64,
                    (int64_t)(intptr_t)descriptor->buckets->data[b]);
                __certo_host_prom_append(&out, &len, &cap, "%s_bucket", name);
                __certo_host_prom_labels(&out, &len, &cap, descriptor, series, true, boundary);
                __certo_host_prom_append(&out, &len, &cap, " %" PRId64 "\n",
                    series->bucket_counts[b]);
            }
            __certo_host_prom_append(&out, &len, &cap, "%s_bucket", name);
            __certo_host_prom_labels(&out, &len, &cap, descriptor, series, true, "+Inf");
            __certo_host_prom_append(&out, &len, &cap, " %" PRId64 "\n", series->count);
            __certo_host_prom_append(&out, &len, &cap, "%s_sum", name);
            __certo_host_prom_labels(&out, &len, &cap, descriptor, series, false, NULL);
            __certo_host_prom_append(&out, &len, &cap, " %" PRId64 "\n", series->sum);
            __certo_host_prom_append(&out, &len, &cap, "%s_count", name);
            __certo_host_prom_labels(&out, &len, &cap, descriptor, series, false, NULL);
            __certo_host_prom_append(&out, &len, &cap, " %" PRId64 "\n", series->count);
        }
        free(name);
    }
    __certo_host_unlock(host);
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
"##;
