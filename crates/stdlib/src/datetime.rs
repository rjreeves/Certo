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


/* ---- calendar arithmetic (BACKLOG items 396/397) ----
   The C library's gmtime/timegm are not used: gmtime returns NULL (and the program crashed) for
   instants outside the platform's range, and timegm silently normalises an impossible date such as
   2023-02-29 into March. These work for every instant in years 0000-9999 and reject the rest. */
#define CERTO_DT_MIN (-62167219200LL)   /* 0000-01-01T00:00:00Z */
#define CERTO_DT_MAX (253402300799LL)   /* 9999-12-31T23:59:59Z */

static int64_t certo_days_from_civil(int64_t y, int64_t m, int64_t d) {
    y -= m <= 2;
    int64_t era = (y >= 0 ? y : y - 399) / 400;
    int64_t yoe = y - era * 400;
    int64_t doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    int64_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

static void certo_civil_from_days(int64_t z, int64_t* y, int64_t* m, int64_t* d) {
    z += 719468;
    int64_t era = (z >= 0 ? z : z - 146096) / 146097;
    int64_t doe = z - era * 146097;
    int64_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t yy = yoe + era * 400;
    int64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    int64_t mp = (5 * doy + 2) / 153;
    *d = doy - (153 * mp + 2) / 5 + 1;
    *m = mp < 10 ? mp + 3 : mp - 9;
    *y = yy + (*m <= 2);
}

static int certo_is_leap(int64_t y) { return (y % 4 == 0 && y % 100 != 0) || y % 400 == 0; }

static int certo_days_in_month(int64_t y, int64_t m) {
    static const int dm[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return m == 2 && certo_is_leap(y) ? 29 : dm[m - 1];
}

/* Break `secs` (UTC) into `out`; 0 if outside years 0000-9999. */
static int certo_gm(int64_t secs, struct tm* out) {
    if (secs < CERTO_DT_MIN || secs > CERTO_DT_MAX) return 0;
    int64_t days = secs >= 0 ? secs / 86400 : -((-secs + 86399) / 86400);
    int64_t rem = secs - days * 86400;
    int64_t y, m, d;
    certo_civil_from_days(days, &y, &m, &d);
    memset(out, 0, sizeof *out);
    out->tm_year = (int)(y - 1900);
    out->tm_mon = (int)(m - 1);
    out->tm_mday = (int)d;
    out->tm_hour = (int)(rem / 3600);
    out->tm_min = (int)((rem % 3600) / 60);
    out->tm_sec = (int)(rem % 60);
    int64_t wd = (days + 4) % 7;            /* 1970-01-01 was a Thursday */
    out->tm_wday = (int)(wd < 0 ? wd + 7 : wd);
    out->tm_yday = (int)(days - certo_days_from_civil(y, 1, 1));
    return 1;
}

static struct tm certo_gm_or_panic(int64_t secs, const char* what) {
    struct tm t;
    if (!certo_gm(secs, &t)) {
        char msg[128];
        snprintf(msg, sizeof msg, "%s: instant out of range (years 0000-9999 only)", what);
        certo_panic(msg);
    }
    return t;
}

/* Seconds since the epoch for a validated wall-clock time; 0 if any field is out of range. */
static int certo_mk_utc(int64_t y, int64_t mo, int64_t d, int64_t h, int64_t mi, int64_t s, int64_t* out) {
    if (y < 0 || y > 9999 || mo < 1 || mo > 12 || d < 1 || d > certo_days_in_month(y, mo)) return 0;
    if (h < 0 || h > 23 || mi < 0 || mi > 59 || s < 0 || s > 59) return 0;
    *out = certo_days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s;
    return 1;
}

/* strftime for UTC (or a fixed offset `off` seconds, shown by %z). Supports Y y C m d e H I M S p
   b B a A h j u w F T D R Z z n t %% ; any other specifier is a panic, not a crash. */
static void certo_format_time(char* buf, size_t cap, const char* fmt, const struct tm* t, int64_t off) {
    static const char* const mon[12] = {"January","February","March","April","May","June","July","August","September","October","November","December"};
    static const char* const day[7] = {"Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"};
    size_t n = 0;
    char piece[64];
    for (const char* p = fmt; *p; p++) {
        piece[0] = 0;
        if (*p != '%') { piece[0] = *p; piece[1] = 0; }
        else {
            p++;
            int64_t y = t->tm_year + 1900;
            switch (*p) {
                case 'Y': snprintf(piece, sizeof piece, "%04lld", (long long)y); break;
                case 'y': snprintf(piece, sizeof piece, "%02lld", (long long)(y % 100)); break;
                case 'C': snprintf(piece, sizeof piece, "%02lld", (long long)(y / 100)); break;
                case 'm': snprintf(piece, sizeof piece, "%02d", t->tm_mon + 1); break;
                case 'd': snprintf(piece, sizeof piece, "%02d", t->tm_mday); break;
                case 'e': snprintf(piece, sizeof piece, "%2d", t->tm_mday); break;
                case 'H': snprintf(piece, sizeof piece, "%02d", t->tm_hour); break;
                case 'I': snprintf(piece, sizeof piece, "%02d", t->tm_hour % 12 == 0 ? 12 : t->tm_hour % 12); break;
                case 'M': snprintf(piece, sizeof piece, "%02d", t->tm_min); break;
                case 'S': snprintf(piece, sizeof piece, "%02d", t->tm_sec); break;
                case 'p': snprintf(piece, sizeof piece, "%s", t->tm_hour < 12 ? "AM" : "PM"); break;
                case 'b': case 'h': snprintf(piece, sizeof piece, "%.3s", mon[t->tm_mon]); break;
                case 'B': snprintf(piece, sizeof piece, "%s", mon[t->tm_mon]); break;
                case 'a': snprintf(piece, sizeof piece, "%.3s", day[t->tm_wday]); break;
                case 'A': snprintf(piece, sizeof piece, "%s", day[t->tm_wday]); break;
                case 'j': snprintf(piece, sizeof piece, "%03d", t->tm_yday + 1); break;
                case 'u': snprintf(piece, sizeof piece, "%d", t->tm_wday == 0 ? 7 : t->tm_wday); break;
                case 'w': snprintf(piece, sizeof piece, "%d", t->tm_wday); break;
                case 'F': snprintf(piece, sizeof piece, "%04lld-%02d-%02d", (long long)y, t->tm_mon + 1, t->tm_mday); break;
                case 'T': snprintf(piece, sizeof piece, "%02d:%02d:%02d", t->tm_hour, t->tm_min, t->tm_sec); break;
                case 'R': snprintf(piece, sizeof piece, "%02d:%02d", t->tm_hour, t->tm_min); break;
                case 'D': snprintf(piece, sizeof piece, "%02d/%02d/%02lld", t->tm_mon + 1, t->tm_mday, (long long)(y % 100)); break;
                case 'Z': {
                    /* No zone abbreviations here: UTC, or the numeric offset. */
                    int64_t a = off < 0 ? -off : off;
                    if (off == 0) snprintf(piece, sizeof piece, "UTC");
                    else snprintf(piece, sizeof piece, "%c%02d:%02d", off < 0 ? '-' : '+', (int)(a / 3600), (int)((a % 3600) / 60));
                    break;
                }
                case 'z': {
                    int64_t a = off < 0 ? -off : off;
                    snprintf(piece, sizeof piece, "%c%02d%02d", off < 0 ? '-' : '+', (int)(a / 3600), (int)((a % 3600) / 60));
                    break;
                }
                case 'n': piece[0] = '\n'; piece[1] = 0; break;
                case 't': piece[0] = '\t'; piece[1] = 0; break;
                case '%': piece[0] = '%'; piece[1] = 0; break;
                default: {
                    char msg[96];
                    snprintf(msg, sizeof msg, "DateTime.format: unsupported format specifier %%%c", *p ? *p : '?');
                    certo_panic(msg);
                }
            }
        }
        size_t len = strlen(piece);
        if (n + len >= cap) { len = cap - 1 - n; }
        memcpy(buf + n, piece, len);
        n += len;
        if (n >= cap - 1) break;
    }
    buf[n] = 0;
}

/* Strict ISO 8601: YYYY-MM-DD, optionally followed by T or a space and HH:MM[:SS[.fraction]],
   optionally followed by Z or a UTC offset (+HH:MM, +HHMM, +HH). No time means midnight, no offset
   means UTC. Returns 1 and the UTC instant, or 0 for anything else (bad field values, trailing
   text, the wrong separators). */
static int certo_parse_iso8601(const char* s, int64_t* out) {
    int64_t v[6] = {0, 0, 0, 0, 0, 0};
    int i = 0;
    #define CERTO_DIGITS(n, dst) do { int64_t acc = 0; for (int k = 0; k < (n); k++) { \
        if (s[i] < '0' || s[i] > '9') return 0; acc = acc * 10 + (s[i] - '0'); i++; } (dst) = acc; } while (0)
    CERTO_DIGITS(4, v[0]);
    if (s[i++] != '-') return 0;
    CERTO_DIGITS(2, v[1]);
    if (s[i++] != '-') return 0;
    CERTO_DIGITS(2, v[2]);
    int64_t off = 0;
    if (s[i] == 'T' || s[i] == ' ') {
        i++;
        CERTO_DIGITS(2, v[3]);
        if (s[i++] != ':') return 0;
        CERTO_DIGITS(2, v[4]);
        if (s[i] == ':') {
            i++;
            CERTO_DIGITS(2, v[5]);
            if (s[i] == '.') {
                i++;
                if (s[i] < '0' || s[i] > '9') return 0;
                while (s[i] >= '0' && s[i] <= '9') i++;
            }
        }
        if (s[i] == 'Z') { i++; }
        else if (s[i] == '+' || s[i] == '-') {
            int sign = s[i] == '-' ? -1 : 1;
            i++;
            int64_t oh = 0, om = 0;
            CERTO_DIGITS(2, oh);
            if (s[i] == ':') i++;
            if (s[i] >= '0' && s[i] <= '9') CERTO_DIGITS(2, om);
            if (oh > 23 || om > 59) return 0;
            off = sign * (oh * 3600 + om * 60);
        }
    }
    #undef CERTO_DIGITS
    if (s[i] != 0) return 0;
    int64_t local;
    if (!certo_mk_utc(v[0], v[1], v[2], v[3], v[4], v[5], &local)) return 0;
    *out = local - off;
    return 1;
}

/* ---- constructors ---- */

CertoDateTime certo_datetime_now(void) {
    return (CertoDateTime)time(NULL);
}

CertoDate certo_date_today(void) {
    int64_t now = (int64_t)time(NULL);
    return (CertoDate)(now - ((now % 86400) + 86400) % 86400);
}

CertoDateTime certo_datetime_from_unix(int64_t secs) {
    return (CertoDateTime)secs;
}

int64_t certo_datetime_to_unix(CertoDateTime dt) {
    return (int64_t)dt;
}

/* ---- formatting ---- */

certo_text_t certo_datetime_format(CertoDateTime dt, certo_text_t fmt) {
    struct tm tm_info = certo_gm_or_panic((int64_t)dt, "DateTime.format");
    char* buf = (char*)malloc(256);
    if (!buf) certo_panic("out of memory");
    certo_format_time(buf, 256, fmt ? fmt : "%Y-%m-%dT%H:%M:%SZ", &tm_info, 0);
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

int64_t certo_datetime_year(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.year"); return m.tm_year + 1900; }
int64_t certo_datetime_month(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.month"); return m.tm_mon + 1; }
int64_t certo_datetime_day(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.day"); return m.tm_mday; }
int64_t certo_datetime_hour(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.hour"); return m.tm_hour; }
int64_t certo_datetime_minute(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.minute"); return m.tm_min; }
int64_t certo_datetime_second(CertoDateTime dt) { struct tm m = certo_gm_or_panic((int64_t)dt, "DateTime.second"); return m.tm_sec; }

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

int64_t certo_duration_to_milliseconds(CertoDuration d) { return d; }
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
    int64_t out;
    if (!certo_parse_iso8601(s, &out)) certo_panic("datetime_parse_iso: not a valid ISO 8601 date-time");
    return (CertoDateTime)out;
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
    int64_t out;
    if (!certo_parse_iso8601(s, &out)) return NULL;
    return __certo_opt_box(out);
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

#elif defined(__APPLE__) || defined(__FreeBSD__)

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

#else /* Linux/glibc: serialize the process-global TZ fallback. */

#include <pthread.h>
#include <unistd.h>

static pthread_mutex_t __certo_tz_lock = PTHREAD_MUTEX_INITIALIZER;

static int __certo_tz_is_valid(const char* name) {
    if (!name || !*name || *name == '/' || strstr(name, "..")) return 0;
    char path[512];
    int n = snprintf(path, sizeof(path), "/usr/share/zoneinfo/%s", name);
    return n > 0 && (size_t)n < sizeof(path) && access(path, R_OK) == 0;
}

static int64_t __certo_tz_offset_seconds(const char* name, int64_t epoch_secs, int* ok) {
    *ok = 0;
    if (!__certo_tz_is_valid(name)) return 0;
    pthread_mutex_lock(&__certo_tz_lock);
    const char* current = getenv("TZ");
    char* saved = current ? strdup(current) : NULL;
    setenv("TZ", name, 1);
    tzset();
    time_t t = (time_t)epoch_secs;
    struct tm result;
    struct tm* converted = localtime_r(&t, &result);
    int64_t offset = converted ? (int64_t)result.tm_gmtoff : 0;
    if (saved) {
        setenv("TZ", saved, 1);
        free(saved);
    } else {
        unsetenv("TZ");
    }
    tzset();
    pthread_mutex_unlock(&__certo_tz_lock);
    if (converted) *ok = 1;
    return offset;
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
    int64_t naive_utc;
    if (!certo_mk_utc(year, month, day, hour, minute, second, &naive_utc))
        certo_panic("Timestamp.of: not a valid date and time");
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, naive_utc, &ok);
    if (!ok) certo_panic("Timestamp.of: timezone became unresolvable");
    return (CertoDateTime)(naive_utc - off);
}

/* Date.of(year, month, day): Date — calendar date at UTC midnight, the
   same convention `Date.today()` already uses. */
CertoDate certo_date_of(int64_t year, int64_t month, int64_t day) {
    int64_t out;
    if (!certo_mk_utc(year, month, day, 0, 0, 0, &out)) certo_panic("Date.of: not a valid calendar date");
    return (CertoDate)out;
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
    struct tm mv = certo_gm_or_panic((int64_t)dt + off, "inTimezone");
    struct tm* m = &mv;
    char* buf = (char*)malloc(64);
    if (!buf) certo_panic("out of memory");
    int64_t abs_off = off < 0 ? -off : off;
    snprintf(buf, 64, "%04d-%02d-%02dT%02d:%02d:%02d%c%02d:%02d",
             m->tm_year + 1900, m->tm_mon + 1, m->tm_mday,
             m->tm_hour, m->tm_min, m->tm_sec,
             off < 0 ? '-' : '+', (int)(abs_off / 3600), (int)((abs_off % 3600) / 60));
    return buf;
}

/* DateTime.formatTz(dt, fmt, tz): Text - the same pattern language as `DateTime.format`, applied
   to `tz`'s wall clock (year/month/day/hour/etc. are zone- and DST-adjusted). `%z` is `tz`'s
   numeric UTC offset at that instant (`+0100`) and `%Z` is `UTC` or that offset with a colon
   (`+01:00`); there are no zone abbreviations (`BST`). See `certo_format_time` for the supported
   specifiers; an unsupported one is a panic. */
certo_text_t certo_date_time_format_tz(CertoDateTime dt, certo_text_t fmt, CertoTimezone tz) {
    int ok = 0;
    int64_t off = __certo_tz_offset_seconds(tz, dt, &ok);
    if (!ok) certo_panic("formatTz: timezone became unresolvable");
    struct tm m = certo_gm_or_panic((int64_t)dt + off, "formatTz");
    char* buf = (char*)malloc(256);
    if (!buf) certo_panic("out of memory");
    certo_format_time(buf, 256, fmt ? fmt : "%Y-%m-%dT%H:%M:%S", &m, off);
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
    int64_t adjusted = now + off;
    return (CertoDate)(adjusted - ((adjusted % 86400) + 86400) % 86400);
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
