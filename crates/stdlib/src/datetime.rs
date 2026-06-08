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
"#;

/// Certo source declaration of `Stdlib.DateTime`.
pub const DATETIME_CERTO: &str = r#"
module Stdlib.DateTime

type DateTime = Int   // Unix timestamp (seconds)
type Date     = Int   // Unix timestamp at midnight UTC

fn DateTime.now(): DateTime [io]
fn Date.today(): Date [io]
fn DateTime.fromUnix(secs: Int): DateTime
fn DateTime.toUnix(dt: DateTime): Int

fn DateTime.format(dt: DateTime, fmt: Text): Text
fn Date.format(d: Date, fmt: Text): Text
fn DateTime.toIso(dt: DateTime): Text
fn DateTime.parseIso(s: Text): DateTime [fallible]

fn DateTime.addSeconds(dt: DateTime, s: Int): DateTime
fn DateTime.addMinutes(dt: DateTime, m: Int): DateTime
fn DateTime.addHours(dt: DateTime, h: Int): DateTime
fn DateTime.addDays(dt: DateTime, d: Int): DateTime

fn DateTime.diffSeconds(a: DateTime, b: DateTime): Int
fn DateTime.diffDays(a: DateTime, b: DateTime): Int

fn DateTime.before(a: DateTime, b: DateTime): Bool
fn DateTime.after(a: DateTime, b: DateTime): Bool
fn DateTime.eq(a: DateTime, b: DateTime): Bool

fn DateTime.year(dt: DateTime): Int
fn DateTime.month(dt: DateTime): Int
fn DateTime.day(dt: DateTime): Int
fn DateTime.hour(dt: DateTime): Int
fn DateTime.minute(dt: DateTime): Int
fn DateTime.second(dt: DateTime): Int
"#;
