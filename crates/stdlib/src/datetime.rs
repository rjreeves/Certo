/// C implementations for `Stdlib.DateTime`.
pub const DATETIME_C: &str = r#"
/* ================================================================
   Stdlib.DateTime
   Dates are stored as int64_t Unix timestamps (seconds since epoch).
   Date-only values use midnight UTC.
   ================================================================ */

#include <time.h>

/* timegm is POSIX; on Windows use _mkgmtime */
#ifdef _WIN32
#  define timegm _mkgmtime
#endif

typedef int64_t CertoDateTime;   /* Unix seconds */
typedef int64_t CertoDate;       /* Unix seconds at midnight UTC */
typedef int64_t CertoDuration;   /* signed span, in MILLISECONDS (BACKLOG
                                     item 188) — DateTime/Date remain whole-
                                     second Unix timestamps (unchanged, a
                                     separate and much larger gap not
                                     attempted here), so a Duration used in
                                     DateTime/Date arithmetic still only
                                     affects/reflects whole seconds; see the
                                     DateTime/Date <-> Duration bridge below
                                     for the /1000 and x1000 conversions
                                     that keep that arithmetic correct now
                                     that Duration's own internal unit has
                                     changed. Duration-to-Duration operations
                                     (add, sub, negate, eq, lt, gt) and the
                                     accessors and constructors below are
                                     unaffected by DateTime/Date's coarser
                                     grain and are now genuinely
                                     millisecond-precise. */

/* ---- constructors ---- */

CertoDateTime certo_datetime_now(void) {
    return (CertoDateTime)time(NULL);
}

CertoDate certo_date_today(void) {
    time_t now = time(NULL);
    struct tm* t = gmtime(&now);
    t->tm_hour = 0; t->tm_min = 0; t->tm_sec = 0;
    return (CertoDate)timegm(t);
}

CertoDateTime certo_datetime_from_unix(int64_t secs) {
    return (CertoDateTime)secs;
}

int64_t certo_datetime_to_unix(CertoDateTime dt) {
    return (int64_t)dt;
}

/* ---- formatting ---- */

certo_text_t certo_datetime_format(CertoDateTime dt, certo_text_t fmt) {
    time_t t = (time_t)dt;
    struct tm* tm_info = gmtime(&t);
    char* buf = (char*)malloc(256);
    if (!buf) certo_panic("out of memory");
    strftime(buf, 256, fmt ? fmt : "%Y-%m-%dT%H:%M:%SZ", tm_info);
    return buf;
}

certo_text_t certo_date_format(CertoDate d, certo_text_t fmt) {
    return certo_datetime_format((CertoDateTime)d, fmt ? fmt : "%Y-%m-%d");
}

certo_text_t certo_datetime_to_iso(CertoDateTime dt) {
    return certo_datetime_format(dt, "%Y-%m-%dT%H:%M:%SZ");
}

/* ---- arithmetic ---- */

CertoDateTime certo_datetime_add_seconds(CertoDateTime dt, int64_t s) { return dt + s; }
CertoDateTime certo_datetime_add_minutes(CertoDateTime dt, int64_t m) { return dt + m * 60; }
CertoDateTime certo_datetime_add_hours  (CertoDateTime dt, int64_t h) { return dt + h * 3600; }
CertoDateTime certo_datetime_add_days   (CertoDateTime dt, int64_t d) { return dt + d * 86400; }

int64_t certo_datetime_diff_seconds(CertoDateTime a, CertoDateTime b) { return a - b; }
int64_t certo_datetime_diff_days   (CertoDateTime a, CertoDateTime b) { return (a - b) / 86400; }

/* ---- comparison ---- */

bool certo_datetime_before(CertoDateTime a, CertoDateTime b) { return a < b; }
bool certo_datetime_after (CertoDateTime a, CertoDateTime b) { return a > b; }
bool certo_datetime_eq    (CertoDateTime a, CertoDateTime b) { return a == b; }

/* ---- components ---- */

int64_t certo_datetime_year  (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_year + 1900; }
int64_t certo_datetime_month (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_mon + 1; }
int64_t certo_datetime_day   (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_mday; }
int64_t certo_datetime_hour  (CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_hour; }
int64_t certo_datetime_minute(CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_min; }
int64_t certo_datetime_second(CertoDateTime dt) { time_t t = (time_t)dt; struct tm* m = gmtime(&t); return m->tm_sec; }

/* ---- Duration: constructors ---- */

/* `CertoDuration` (see its own typedef above) now stores milliseconds —
   BACKLOG item 188, widened from the previous whole-seconds-only
   representation (item 187's own honest limitation). `Duration.milliseconds`
   is exact now, not truncated to zero. */
CertoDuration certo_duration_milliseconds(int64_t n) { return (CertoDuration)n; }
CertoDuration certo_duration_seconds(int64_t n) { return (CertoDuration)(n * 1000); }
CertoDuration certo_duration_minutes(int64_t n) { return (CertoDuration)(n * 60000); }
CertoDuration certo_duration_hours  (int64_t n) { return (CertoDuration)(n * 3600000); }
CertoDuration certo_duration_days   (int64_t n) { return (CertoDuration)(n * 86400000); }
/* `Duration.months` (BACKLOG item 271, section-16-validators.md §16.9's own
   `Duration.months(12)` example) — a fixed 30-day approximation, the same
   "fixed-length, not calendar-aware" convention every other Duration unit
   already uses (`.days`/`.hours` don't account for a real month's 28-31 day
   variance or a DST-shifted day either); there is no real calendar-month
   concept anywhere in this Duration representation to derive an exact value
   from. */
CertoDuration certo_duration_months (int64_t n) { return (CertoDuration)(n * 2592000000LL); }

/* ---- Duration: accessors (truncating to the coarser unit) ---- */

int64_t certo_duration_to_seconds(CertoDuration d) { return d / 1000; }
int64_t certo_duration_to_minutes(CertoDuration d) { return d / 60000; }
int64_t certo_duration_to_hours  (CertoDuration d) { return d / 3600000; }
int64_t certo_duration_to_days   (CertoDuration d) { return d / 86400000; }

/* ---- Duration: arithmetic ---- */

CertoDuration certo_duration_add   (CertoDuration a, CertoDuration b) { return a + b; }
CertoDuration certo_duration_sub   (CertoDuration a, CertoDuration b) { return a - b; }
CertoDuration certo_duration_negate(CertoDuration d) { return -d; }

/* ---- Duration: comparison ---- */

bool certo_duration_eq (CertoDuration a, CertoDuration b) { return a == b; }
bool certo_duration_lt (CertoDuration a, CertoDuration b) { return a < b; }
bool certo_duration_gt (CertoDuration a, CertoDuration b) { return a > b; }

/* ---- DateTime/Date <-> Duration ----
   DateTime/Date are whole-second Unix timestamps (unchanged by item 188);
   Duration is now milliseconds — these three functions are the only place
   the two representations meet, so they convert explicitly rather than
   silently mixing units (adding a raw millisecond count as if it were
   seconds would misplace a `Duration.days(30)` deadline by a factor of
   1000). A sub-second Duration (e.g. `Duration.milliseconds(500)`) still
   contributes zero whole seconds here — DateTime/Date's own coarser grain,
   not this conversion, is what truncates it; see the `CertoDuration`
   typedef comment above. */
CertoDateTime certo_datetime_add_duration(CertoDateTime dt, CertoDuration d) { return dt + d / 1000; }
CertoDuration certo_datetime_diff        (CertoDateTime a, CertoDateTime b)  { return (a - b) * 1000; }
CertoDate     certo_date_add_duration    (CertoDate d, CertoDuration dur)    { return d + dur / 1000; }

/* ---- parse ISO 8601 ---- */
CertoDateTime certo_datetime_parse_iso(certo_text_t s) {
    if (!s) certo_panic("datetime_parse_iso: null input");
    struct tm t = {0};
    /* Minimal parser: YYYY-MM-DDTHH:MM:SSZ */
    if (sscanf(s, "%d-%d-%dT%d:%d:%d",
               &t.tm_year, &t.tm_mon, &t.tm_mday,
               &t.tm_hour, &t.tm_min, &t.tm_sec) < 3)
        certo_panic("datetime_parse_iso: invalid format");
    t.tm_year -= 1900;
    t.tm_mon  -= 1;
    return (CertoDateTime)timegm(&t);
}

/* `Timestamp.parse` (spec §9.4, BACKLOG item 289) — the spec documents this
   as `Result<Timestamp, ParseError>`, but `Result`'s own error side needs a
   real `ParseError` type this stdlib doesn't have; `Option<Timestamp>`
   (None on any unparseable input) is the same "don't panic the whole
   program on bad user input" contract `parseInt`/`parseDecimal`/`parseBool`
   above already establish, just without a payload on the failure side.
   Deliberately a *separate* function from `certo_datetime_parse_iso`
   (`DateTime.parseIso`'s own real underlying implementation, which the
   spec never documents as fallible) rather than changing that one to also
   return an Option — this fix only touches `Timestamp.parse`'s own
   declared behavior, not `DateTime.parseIso`'s. Reuses the identical parse
   logic (same minimal `YYYY-MM-DDTHH:MM:SSZ` scanner) rather than calling
   `certo_datetime_parse_iso` and catching its panic — there is no
   catchable-panic mechanism in this C runtime to catch. */
int64_t* certo_timestamp_parse_opt(certo_text_t s) {
    if (!s) return NULL;
    struct tm t = {0};
    if (sscanf(s, "%d-%d-%dT%d:%d:%d",
               &t.tm_year, &t.tm_mon, &t.tm_mday,
               &t.tm_hour, &t.tm_min, &t.tm_sec) < 3)
        return NULL;
    t.tm_year -= 1900;
    t.tm_mon  -= 1;
    return __certo_opt_box((int64_t)(CertoDateTime)timegm(&t));
}

/* ================================================================
   Stdlib.Timezone — real IANA timezone support (BACKLOG item 118).
   `CertoTimezone` is just `certo_text_t` — the IANA zone name itself; no
   wrapper struct needed since a C string pointer is already pointer-sized.

   Windows: dynamically loads the OS-bundled ICU (`icuin.dll`, present
   since Windows 10 1903) and calls its C API via `GetProcAddress` — no ICU
   SDK/headers needed. Confirmed by direct inspection of a real Windows 11
   install's `icuin.dll` export table (`llvm-readobj --coff-exports`) that
   this redistributed package exports `ucal_open`/`ucal_close`/
   `ucal_setMillis`/`ucal_get`/`ucal_getCanonicalTimeZoneID` UNVERSIONED —
   classic ICU4C's *versioned* symbol convention (`ucal_open_70`) does not
   apply here, so no version-suffix probing is needed. Verified end-to-end
   with a standalone test program before wiring this in: Europe/London
   correctly resolves to UTC+1 (BST) in July 2026 and UTC+0 (GMT) in
   January 2026; `ucal_getCanonicalTimeZoneID` correctly distinguishes a
   real IANA name from a bogus one (`ucal_open` itself does NOT fail on an
   unknown zone name — it silently falls back to a default — so validation
   must go through `getCanonicalTimeZoneID`, not `ucal_open`'s own status).

   POSIX (Linux/macOS): `tzalloc`/`localtime_rz`/`tzfree` — real, per-call
   reentrant handles into the system's own zoneinfo database (a GNU/BSD
   extension present on both glibc and macOS/BSD libc, not strict ISO C).
   Deliberately NOT using `tzset`+the process-global `TZ` environment
   variable: mutating `TZ` is not thread-safe to do concurrently with
   another thread's own `localtime`/`gmtime` calls, and this codebase's own
   `spawn`/`parallel` make concurrent use routine.
   ================================================================ */

typedef certo_text_t CertoTimezone;

#ifdef _WIN32

typedef void* CertoUCal;
typedef uint16_t CertoUChar;
typedef int32_t CertoUErrorCode;
typedef char CertoUBool;

typedef CertoUCal (*certo_ucal_open_fn)(const CertoUChar*, int32_t, const char*, int32_t, CertoUErrorCode*);
typedef void (*certo_ucal_close_fn)(CertoUCal);
typedef void (*certo_ucal_setMillis_fn)(CertoUCal, double, CertoUErrorCode*);
typedef int32_t (*certo_ucal_get_fn)(const CertoUCal, int32_t, CertoUErrorCode*);
typedef int32_t (*certo_ucal_getCanonicalTimeZoneID_fn)(const CertoUChar*, int32_t, CertoUChar*, int32_t, CertoUBool*, CertoUErrorCode*);

/* UCalendarDateFields — stable, public ICU4C enum values since its
   earliest version; hardcoded here since we have no ICU headers. */
#define CERTO_UCAL_ZONE_OFFSET 15
#define CERTO_UCAL_DST_OFFSET  16

static certo_ucal_open_fn __certo_ucal_open;
static certo_ucal_close_fn __certo_ucal_close;
static certo_ucal_setMillis_fn __certo_ucal_setMillis;
static certo_ucal_get_fn __certo_ucal_get;
static certo_ucal_getCanonicalTimeZoneID_fn __certo_ucal_getCanonicalTimeZoneID;
static INIT_ONCE __certo_icu_init_once = INIT_ONCE_STATIC_INIT;

static BOOL CALLBACK __certo_icu_init(PINIT_ONCE once, PVOID param, PVOID* ctx) {
    (void)once; (void)param; (void)ctx;
    HMODULE h = LoadLibraryA("icuin.dll");
    if (!h) return TRUE; /* function pointers stay NULL; callers check before use */
    __certo_ucal_open = (certo_ucal_open_fn)GetProcAddress(h, "ucal_open");
    __certo_ucal_close = (certo_ucal_close_fn)GetProcAddress(h, "ucal_close");
    __certo_ucal_setMillis = (certo_ucal_setMillis_fn)GetProcAddress(h, "ucal_setMillis");
    __certo_ucal_get = (certo_ucal_get_fn)GetProcAddress(h, "ucal_get");
    __certo_ucal_getCanonicalTimeZoneID = (certo_ucal_getCanonicalTimeZoneID_fn)GetProcAddress(h, "ucal_getCanonicalTimeZoneID");
    return TRUE;
}

/* Thread-safe lazy init (Win32 INIT_ONCE) — ICU is loaded at most once per process. */
static void __certo_tz_ensure_icu(void) {
    InitOnceExecuteOnce(&__certo_icu_init_once, __certo_icu_init, NULL, NULL);
}

static void __certo_tz_utf8_to_utf16(const char* s, CertoUChar* out, int outlen) {
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0 || n > outlen) n = outlen;
    MultiByteToWideChar(CP_UTF8, 0, s, -1, (wchar_t*)out, n);
}

/* 1 = `name` is a real IANA zone, 0 = unknown or ICU unavailable. */
static int __certo_tz_is_valid(const char* name) {
    __certo_tz_ensure_icu();
    if (!__certo_ucal_getCanonicalTimeZoneID) return 0;
    CertoUChar zone[128];
    __certo_tz_utf8_to_utf16(name, zone, 128);
    CertoUChar result[128];
    CertoUBool is_system = 0;
    CertoUErrorCode status = 0;
    __certo_ucal_getCanonicalTimeZoneID(zone, -1, result, 128, &is_system, &status);
    return status == 0;
}

/* Total UTC offset (zone + DST) in seconds for `name` at `epoch_secs`.
   *ok is set to 0 (offset undefined, do not use) if resolution fails. */
static int64_t __certo_tz_offset_seconds(const char* name, int64_t epoch_secs, int* ok) {
    __certo_tz_ensure_icu();
    *ok = 0;
    if (!__certo_ucal_open || !__certo_ucal_close || !__certo_ucal_setMillis || !__certo_ucal_get) return 0;
    CertoUChar zone[128];
    __certo_tz_utf8_to_utf16(name, zone, 128);
    CertoUErrorCode status = 0;
    CertoUCal cal = __certo_ucal_open(zone, -1, "en_US", 0 /* UCAL_DEFAULT */, &status);
    if (status != 0 || !cal) return 0;
    __certo_ucal_setMillis(cal, (double)epoch_secs * 1000.0, &status);
    int32_t zone_off = __certo_ucal_get(cal, CERTO_UCAL_ZONE_OFFSET, &status);
    int32_t dst_off  = __certo_ucal_get(cal, CERTO_UCAL_DST_OFFSET, &status);
    __certo_ucal_close(cal);
    if (status != 0) return 0;
    *ok = 1;
    return (int64_t)(zone_off + dst_off) / 1000;
}

#else /* POSIX */

/* 1 = `name` is a real IANA zone, 0 = unknown. */
static int __certo_tz_is_valid(const char* name) {
    timezone_t tz = tzalloc(name);
    if (!tz) return 0;
    tzfree(tz);
    return 1;
}

static int64_t __certo_tz_offset_seconds(const char* name, int64_t epoch_secs, int* ok) {
    *ok = 0;
    timezone_t tz = tzalloc(name);
    if (!tz) return 0;
    time_t t = (time_t)epoch_secs;
    struct tm result;
    if (!localtime_rz(tz, &t, &result)) { tzfree(tz); return 0; }
    tzfree(tz);
    *ok = 1;
    return (int64_t)result.tm_gmtoff;
}

#endif

/* ---- Timestamp: component-based constructor (BACKLOG item 164b) ----
   `Timestamp` has no runtime type of its own — same as `.age`'s existing
   treatment (see the codegen comment on the `Timestamp` ty_to_c arm) — it
   reuses `CertoDateTime` exactly. Everything else `Timestamp.*` needs
   (`.now`, `.parse`, `.inTimezone`, `.formatTz`) is a real, already-
   working `DateTime`/`Timezone` implementation under a different name, so
   those are bridged via #define below rather than reimplemented; `.of` has
   no `DateTime` equivalent to bridge to (component-based construction was
   missing for both names — this is the one genuinely new function). */

/* Timestamp.of(year, month, day, hour, minute, second, tz): Timestamp —
   y/m/d/h/min/s are the WALL-CLOCK time *in* `tz`, converted to a real UTC
   epoch by looking up `tz`'s offset at the naive (as-if-UTC) instant first.
   Same single-lookup approximation `certo_date_today_in` above already
   uses for its own local-to-UTC conversion — correct at almost every real
   instant, but not provably correct in the ~1-hour window spanning a DST
   transition (the offset can differ on either side of the transition, and
   this uses only the "before" reading) — an existing, documented
   limitation of this timezone code, not a new one introduced here. */
CertoDateTime certo_timestamp_of(int64_t year, int64_t month, int64_t day,
                                  int64_t hour, int64_t minute, int64_t second,
                                  CertoTimezone tz) {
    struct tm t = {0};
    t.tm_year = (int)(year - 1900);
    t.tm_mon  = (int)(month - 1);
    t.tm_mday = (int)day;
    t.tm_hour = (int)hour;
    t.tm_min  = (int)minute;
    t.tm_sec  = (int)second;
    int64_t naive_utc = (int64_t)timegm(&t);
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, naive_utc, &ok);
    if (!ok) certo_panic("Timestamp.of: timezone became unresolvable");
    return (CertoDateTime)(naive_utc - off);
}

/* Date.of(year, month, day): Date — calendar date at UTC midnight, the
   same convention `Date.today()` already uses. */
CertoDate certo_date_of(int64_t year, int64_t month, int64_t day) {
    struct tm t = {0};
    t.tm_year = (int)(year - 1900);
    t.tm_mon  = (int)(month - 1);
    t.tm_mday = (int)day;
    return (CertoDate)timegm(&t);
}

/* Timezone(name: Text): Timezone? — None if `name` isn't a real IANA zone. */
void* certo_timezone(certo_text_t name) {
    if (!name || !__certo_tz_is_valid(name)) return NULL;
    size_t len = strlen(name) + 1;
    char* copy = (char*)malloc(len);
    if (!copy) certo_panic("out of memory");
    memcpy(copy, name, len);
    return __certo_opt_box((int64_t)copy);   /* Some(tz) */
}

/* Timezone.name(tz: Timezone): Text */
certo_text_t certo_timezone_name(CertoTimezone tz) { return tz; }

/* DateTime.inTimezone(dt, tz): Text — ISO 8601 with the zone's own UTC
   offset suffix (not "Z"), e.g. "2026-07-15T13:00:00+01:00". `tz` was only
   ever constructed via `Timezone(name)`, which already validated `name`,
   so a resolution failure here means the underlying platform timezone
   database became unavailable after construction — panics rather than
   silently mis-rendering the wrong instant. */
certo_text_t certo_date_time_in_timezone(CertoDateTime dt, CertoTimezone tz) {
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, dt, &ok);
    if (!ok) certo_panic("inTimezone: timezone became unresolvable");
    time_t adjusted = (time_t)(dt + off);
    struct tm* m = gmtime(&adjusted);
    char* buf = (char*)malloc(64);
    if (!buf) certo_panic("out of memory");
    int64_t abs_off = off < 0 ? -off : off;
    snprintf(buf, 64, "%04d-%02d-%02dT%02d:%02d:%02d%c%02d:%02d",
             m->tm_year + 1900, m->tm_mon + 1, m->tm_mday,
             m->tm_hour, m->tm_min, m->tm_sec,
             off < 0 ? '-' : '+', (int)(abs_off / 3600), (int)((abs_off % 3600) / 60));
    return buf;
}

/* DateTime.formatTz(dt, fmt, tz): Text — same `strftime` pattern language
   as the existing UTC-only `DateTime.format`, applied to `tz`'s wall clock
   (year/month/day/hour/etc. are all correctly zone- and DST-adjusted).
   KNOWN LIMITATION, confirmed by direct testing, not just suspected: `%Z`/
   `%z` are NOT reliable here and must not be relied on — `strftime` reads
   those two conversions from `struct tm`'s own zone-name/gmtoff fields
   (or, on some platforms, falls back to the C library's *process-wide*
   locale timezone), neither of which this function ever sets to `tz` —
   there is no portable, standard way to hand `strftime` an arbitrary
   zone's display name/abbreviation directly. Confirmed on a real Windows
   machine: formatting a `Europe/London` instant with `%Z` printed the
   *host machine's own* configured Windows timezone name (unrelated to
   London) instead of anything London-specific. Use `DateTime.inTimezone`
   for a numeric UTC-offset suffix (`+01:00`) computed correctly from `tz`
   itself, or `Timezone.name(tz)` for the IANA name, instead of `%Z`/`%z`. */
certo_text_t certo_date_time_format_tz(CertoDateTime dt, certo_text_t fmt, CertoTimezone tz) {
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, dt, &ok);
    if (!ok) certo_panic("formatTz: timezone became unresolvable");
    time_t adjusted = (time_t)(dt + off);
    struct tm* m = gmtime(&adjusted);
    char* buf = (char*)malloc(256);
    if (!buf) certo_panic("out of memory");
    strftime(buf, 256, fmt ? fmt : "%Y-%m-%dT%H:%M:%S", m);
    return buf;
}

/* Date.todayIn(tz): Date — midnight in `tz`'s own wall clock, not UTC
   midnight, so e.g. "today" in Tokyo can legitimately differ from "today"
   in Los Angeles at the same instant. */
CertoDate certo_date_today_in(CertoTimezone tz) {
    int64_t now = (int64_t)time(NULL);
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, now, &ok);
    if (!ok) certo_panic("todayIn: timezone became unresolvable");
    time_t adjusted = (time_t)(now + off);
    struct tm* m = gmtime(&adjusted);
    m->tm_hour = 0; m->tm_min = 0; m->tm_sec = 0;
    return (CertoDate)timegm(m);
}

/* Bridge codegen's DateTime.* names (certo_date_time_*) to the certo_datetime_*
   implementations above. Placed after all definitions so only call sites rewrite. */
#define certo_date_time_now          certo_datetime_now
#define certo_date_time_format       certo_datetime_format
#define certo_date_time_from_unix    certo_datetime_from_unix
#define certo_date_time_to_unix      certo_datetime_to_unix
#define certo_date_time_to_iso       certo_datetime_to_iso
#define certo_date_time_parse_iso    certo_datetime_parse_iso
#define certo_date_time_add_seconds  certo_datetime_add_seconds
#define certo_date_time_add_minutes  certo_datetime_add_minutes
#define certo_date_time_add_hours    certo_datetime_add_hours
#define certo_date_time_add_days     certo_datetime_add_days
#define certo_date_time_diff_seconds certo_datetime_diff_seconds
#define certo_date_time_diff_days    certo_datetime_diff_days
#define certo_date_time_before       certo_datetime_before
#define certo_date_time_after        certo_datetime_after
#define certo_date_time_eq           certo_datetime_eq
#define certo_date_time_year         certo_datetime_year
#define certo_date_time_month        certo_datetime_month
#define certo_date_time_day          certo_datetime_day
#define certo_date_time_hour         certo_datetime_hour
#define certo_date_time_minute       certo_datetime_minute
#define certo_date_time_second       certo_datetime_second
#define certo_date_time_add_duration certo_datetime_add_duration
#define certo_date_time_diff         certo_datetime_diff

/* Bridge codegen's Timestamp.* names (certo_timestamp_*) to the real
   DateTime/Timezone implementations above — BACKLOG item 164b. `Timestamp`
   and `DateTime` are distinct nominal types at the Certo level (a value of
   one cannot be passed where the other is expected — confirmed by direct
   testing), but both compile to the identical `CertoDateTime` C
   representation, so reusing the implementation is exact, not approximate.
   `certo_timestamp_of` is defined directly above (no DateTime equivalent
   to bridge to) and needs no macro here. */
#define certo_timestamp_now         certo_datetime_now
/* BACKLOG item 289 — bridges to the real, Option-returning implementation
   above, not `certo_datetime_parse_iso` (which panics on bad input and
   backs the separate, never-documented-as-fallible `DateTime.parseIso`). */
#define certo_timestamp_parse       certo_timestamp_parse_opt
#define certo_timestamp_in_timezone certo_date_time_in_timezone
#define certo_timestamp_format_tz   certo_date_time_format_tz
/* BACKLOG item 315 — Timestamp.format, a genuine alias for Timestamp.formatTz
   (spec §9.4's own documented name), same bridge pattern as the others here. */
#define certo_timestamp_format      certo_date_time_format_tz
/* BACKLOG item 214 — Timestamp.diff, same bridge pattern as the four above. */
#define certo_timestamp_diff        certo_datetime_diff
"#;
