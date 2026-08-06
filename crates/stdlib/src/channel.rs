/// C implementation for `Stdlib.Channel<T>` — a thread-safe bounded queue
/// for communicating between spawned tasks.
pub const CHANNEL_C: &str = r#"
/* ================================================================
   Stdlib.Channel<T> — bounded, thread-safe queue
   ================================================================
   A blocking multi-producer/multi-consumer queue backed by a mutex and
   two condition variables (not-full / not-empty) — the same cross-platform
   split (Win32 CRITICAL_SECTION+CONDITION_VARIABLE vs POSIX pthread
   mutex+cond) as the `__certo_thread_*` shim in RUNTIME_HEADER, which is
   emitted earlier in the generated file so windows.h/pthread.h are already
   included by the time this runs.

   `T` is stored as an untyped `void*` element slot — the same convention
   as List<T>/Map<K,V>: pointer-sized scalars (Int, Bool, Text, records)
   round-trip through the slot as-is; this inherits List/Map's existing,
   pre-existing, documented limitation for non-pointer-sized structs
   (Decimal) rather than introducing a new one.
   ================================================================ */

typedef struct {
    void**  buf;
    int64_t cap;
    int64_t head;
    int64_t len;
    bool    closed;
#if defined(_WIN32)
    CRITICAL_SECTION lock;
    CONDITION_VARIABLE not_full;
    CONDITION_VARIABLE not_empty;
#else
    pthread_mutex_t lock;
    pthread_cond_t  not_full;
    pthread_cond_t  not_empty;
#endif
} CertoChannel;

static void __certo_channel_lock(CertoChannel* c) {
#if defined(_WIN32)
    EnterCriticalSection(&c->lock);
#else
    pthread_mutex_lock(&c->lock);
#endif
}

static void __certo_channel_unlock(CertoChannel* c) {
#if defined(_WIN32)
    LeaveCriticalSection(&c->lock);
#else
    pthread_mutex_unlock(&c->lock);
#endif
}

CertoChannel* certo_channel_new(int64_t capacity) {
    if (capacity < 1) capacity = 1;
    CertoChannel* c = (CertoChannel*)malloc(sizeof(CertoChannel));
    if (!c) certo_panic("out of memory");
    c->buf = (void**)malloc((size_t)capacity * sizeof(void*));
    if (!c->buf) certo_panic("out of memory");
    c->cap    = capacity;
    c->head   = 0;
    c->len    = 0;
    c->closed = false;
#if defined(_WIN32)
    InitializeCriticalSection(&c->lock);
    InitializeConditionVariable(&c->not_full);
    InitializeConditionVariable(&c->not_empty);
#else
    pthread_mutex_init(&c->lock, NULL);
    pthread_cond_init(&c->not_full, NULL);
    pthread_cond_init(&c->not_empty, NULL);
#endif
    return c;
}

/* Blocks until there is room, then enqueues. Panics if the channel is
   already closed — sending on a closed channel is a programmer error
   (same convention as Go's channels). */
int64_t certo_channel_send(CertoChannel* c, void* item) {
    if (!c) certo_panic("send on a null channel");
    __certo_channel_lock(c);
    while (c->len == c->cap && !c->closed) {
#if defined(_WIN32)
        SleepConditionVariableCS(&c->not_full, &c->lock, INFINITE);
#else
        pthread_cond_wait(&c->not_full, &c->lock);
#endif
    }
    if (c->closed) {
        __certo_channel_unlock(c);
        certo_panic("send on closed channel");
    }
    int64_t tail = (c->head + c->len) % c->cap;
    c->buf[tail] = item;
    c->len++;
#if defined(_WIN32)
    WakeConditionVariable(&c->not_empty);
#else
    pthread_cond_signal(&c->not_empty);
#endif
    __certo_channel_unlock(c);
    return 0;
}

/* Blocks until an item is available or the channel is closed and fully
   drained. Returns Option<T>: Some(item) normally, None once closed with
   nothing left buffered — "channel closed" is unrepresentable as anything
   else, the same illegal-states-unrepresentable idiom the rest of the
   stdlib uses (Map.get, List.first, ...) rather than a sentinel value or
   a second out-param. Buffered items are still delivered after close —
   close only means no *more* sends will happen. */
void* certo_channel_receive(CertoChannel* c) {
    if (!c) return NULL;
    __certo_channel_lock(c);
    while (c->len == 0 && !c->closed) {
#if defined(_WIN32)
        SleepConditionVariableCS(&c->not_empty, &c->lock, INFINITE);
#else
        pthread_cond_wait(&c->not_empty, &c->lock);
#endif
    }
    if (c->len == 0) {
        __certo_channel_unlock(c);
        return NULL;   /* closed and drained: None */
    }
    void* item = c->buf[c->head];
    c->head = (c->head + 1) % c->cap;
    c->len--;
#if defined(_WIN32)
    WakeConditionVariable(&c->not_full);
#else
    pthread_cond_signal(&c->not_full);
#endif
    __certo_channel_unlock(c);
    return __certo_opt_box((int64_t)item);   /* Some(item) */
}

/* Non-blocking: None immediately if nothing is buffered right now
   (whether or not the channel is closed). */
void* certo_channel_try_receive(CertoChannel* c) {
    if (!c) return NULL;
    __certo_channel_lock(c);
    if (c->len == 0) {
        __certo_channel_unlock(c);
        return NULL;
    }
    void* item = c->buf[c->head];
    c->head = (c->head + 1) % c->cap;
    c->len--;
#if defined(_WIN32)
    WakeConditionVariable(&c->not_full);
#else
    pthread_cond_signal(&c->not_full);
#endif
    __certo_channel_unlock(c);
    return __certo_opt_box((int64_t)item);
}

/* Idempotent. Wakes every blocked sender/receiver so they can re-check
   `closed` and unblock instead of waiting forever. */
int64_t certo_channel_close(CertoChannel* c) {
    if (!c) return 0;
    __certo_channel_lock(c);
    if (!c->closed) {
        c->closed = true;
#if defined(_WIN32)
        WakeAllConditionVariable(&c->not_full);
        WakeAllConditionVariable(&c->not_empty);
#else
        pthread_cond_broadcast(&c->not_full);
        pthread_cond_broadcast(&c->not_empty);
#endif
    }
    __certo_channel_unlock(c);
    return 0;
}

bool certo_channel_is_closed(CertoChannel* c) {
    if (!c) return true;
    __certo_channel_lock(c);
    bool closed = c->closed;
    __certo_channel_unlock(c);
    return closed;
}
"#;
