# Certo Stdlib Quick Reference

Signatures only — one line per function. For narrative usage see
[HOW-TO-PROGRAM.md](HOW-TO-PROGRAM.md). For full prose descriptions see
[REFERENCE.md](REFERENCE.md).

---

## Contents

| Module | import |
|---|---|
| [Core](#core) | *(auto-imported)* |
| [Result](#result) | `import Stdlib.Result` |
| [Text](#text) | `import Stdlib.Text` |
| [Char](#char) | `import Stdlib.Text` |
| [Collections](#collections) | `import Stdlib.Collections` |
| [Channel\<T\>](#channelt) | `import Stdlib.Collections` |
| [Money / Decimal](#money--decimal) | `import Stdlib.Money` |
| [DateTime](#datetime) | `import Stdlib.DateTime` |
| [Timestamp](#timestamp) | `import Stdlib.DateTime` |
| [Duration](#duration) | `import Stdlib.DateTime` |
| [Math](#math) | `import Stdlib.Math` |
| [Json](#json) | `import Stdlib.Json` |
| [Http](#http) | `import Stdlib.Http` |
| [Db](#db) | `import Stdlib.Db` |
| [File](#file) | `import Stdlib.File` |
| [Path](#path) | `import Stdlib.Path` |
| [Env](#env) | `import Stdlib.Env` |
| [Process](#process) | `import Stdlib.Process` |
| [Host](#host) | `import Stdlib.Host` |
| [Regex](#regex) | `import Stdlib.Regex` |
| [Csv](#csv) | `import Stdlib.Csv` |
| [Crypto](#crypto) | `import Stdlib.Crypto` |

---

## Core

Auto-imported. No `import` required.

### Output

```
print(s: Text): Unit [io]           // stdout, no newline
println(s: Text): Unit [io]         // stdout + newline
eprint(s: Text): Unit [io]          // stderr, no newline
eprintln(s: Text): Unit [io]        // stderr + newline
sleep(ms: Int): Unit [io]           // block the current thread for ms milliseconds
```

### Conversions

```
intToText(n: Int): Text
floatToText(f: Float): Text
boolToText(b: Bool): Text           // "true" | "false"
floatToInt(f: Float): Int           // truncates toward zero
intToFloat(n: Int): Float
parseInt(s: Text): Int?             // None if not a valid integer
parseFloat(s: Text): Float?         // None if not a valid float
parseDecimal(s: Text): Decimal?     // None if not a valid decimal (strict — no trailing garbage)

// Float32 — a distinct type from Float (32-bit vs 64-bit, does not unify)
float32ToText(f: Float32): Text
float32ToInt(f: Float32): Int       // truncates toward zero
intToFloat32(n: Int): Float32
float32ToFloat(f: Float32): Float   // widen
floatToFloat32(f: Float): Float32   // narrow, may lose precision
```

### Arithmetic helpers

```
pow(base: Int, exp: Int): Int       // integer exponentiation
absInt(n: Int): Int
absFloat(f: Float): Float
minInt(a: Int, b: Int): Int
maxInt(a: Int, b: Int): Int
minFloat(a: Float, b: Float): Float
maxFloat(a: Float, b: Float): Float
floor(f: Float): Float
ceil(f: Float): Float
round(f: Float): Float              // half-up
sqrt(f: Float): Float
```

### Ranges

```
range(start: Int, end: Int): List<Int>          // [start, end)  exclusive
rangeInclusive(start: Int, end: Int): List<Int> // [start, end]  inclusive
```

### I/O and arguments

```
readLine(): Text? [io]              // one line from stdin; None on EOF
readAll(): Text [io]                // all of stdin as one Text
argCount(): Int                     // total argv length (including program name)
arg(i: Int): Text?                  // argv[i], or None if out of range
```

### Control

```
assert(cond: Bool, msg: Text): Unit  // panics if cond is false
panic(msg: Text): Nothing            // unconditional abort
```

### Assertions — `expect(x).toBeXxx(...)`

`expect<T>(x: T): T` is a real, honest identity function — it exists for
readability at the call site, not as a syntactic gate; `x.toBe(y)` and
`expect(x).toBe(y)` are equivalent. Every matcher desugars to the same
`assert` above.

```
expect(x).toBe(y)       // x == y — same equality assert(a == b, ...) uses; a
                         // Decimal/record x hits assert's own struct-compare
                         // limitation, not a new one
expect(x).toBeTrue()    // x
expect(x).toBeFalse()   // !x
expect(x).toBeSome()    // x: Option<T> — Option.isSome(x)
expect(x).toBeNone()    // x: Option<T> — Option.isNone(x)
expect(x).toBeOk()      // x: Result<T,E> — Result.isOk(x)
expect(x).toBeErr()     // x: Result<T,E> — Result.isErr(x)
```

`Option.isSome`/`Option.isNone`/`Result.isOk`/`Result.isErr` are also real,
independently callable functions, not just plumbing for `expect(...)`.

---

## Result

`import Stdlib.Result`

`flatMap`/`mapErr`/`getOrElse`/`recover` are bare functions, used via the
pipe operator (`r |> flatMap(f)` desugars to `flatMap(r, f)`) — Certo has
no `value.method()` dispatch, only `Type.method(...)` static calls, so
these aren't called as `r.flatMap(f)`. `Result.all`/`Result.allSettled`
are qualified, like `List.map`.

```
flatMap<T, U, E>(r: Result<T, E>, f: fn(T): Result<U, E>): Result<U, E>
    // Ok(v) -> f(v); Err(e) -> Err(e) unchanged (short-circuits)

mapErr<T, E, F>(r: Result<T, E>, f: fn(E): F): Result<T, F>
    // transforms the error side only; Ok is untouched

getOrElse<T, E>(r: Result<T, E>, default: T): T
    // unwraps Ok, else returns default

recover<T, E>(r: Result<T, E>, f: fn(E): T): T
    // unwraps Ok, else computes a fallback value from the error

Result.all<T, E>(results: List<Result<T, E>>): Result<List<T>, E>
    // Ok(every payload) if all succeeded, else the first Err (short-circuits)

Result.allSettled<T, E>(results: List<Result<T, E>>): List<Result<T, E>>
    // every outcome unchanged; never short-circuits
```

---

## Text

`import Stdlib.Text`

```
Text.len(s: Text): Int                     // byte count
Text.byteLength(s: Text): Int              // byte count (explicit alias — see Text.len)
Text.concat(a: Text, b: Text): Text        // same as a ++ b
Text.eq(a: Text, b: Text): Bool
Text.contains(s: Text, sub: Text): Bool
Text.startsWith(s: Text, prefix: Text): Bool
Text.endsWith(s: Text, suffix: Text): Bool
Text.toUpper(s: Text): Text                // real Unicode case conversion on Windows (e.g. "straße" -> "STRASSE"); ASCII-only on POSIX
Text.toLower(s: Text): Text                // same platform note as toUpper
Text.toUpperLocale(s: Text, locale: Text): Text // locale-conditional casing (e.g. "i" -> "İ" for "tr"); Windows only, panics on POSIX
Text.toLowerLocale(s: Text, locale: Text): Text // same platform note as toUpperLocale
Text.trim(s: Text): Text                   // both ends
Text.trimStart(s: Text): Text              // leading whitespace
Text.trimEnd(s: Text): Text                // trailing whitespace
Text.slice(s: Text, start: Int, end: Int): Text   // [start, end)
Text.indexOf(s: Text, sub: Text): Int?     // byte offset, or None
Text.replace(s: Text, from: Text, to: Text): Text // all occurrences
Text.split(s: Text, sep: Text): List<Text> // empty sep → char list
Text.join(parts: List<Text>, sep: Text): Text
Text.repeat(s: Text, n: Int): Text
Text.charAt(s: Text, index: Int): Char?    // bounds-checked; None if out of range
```

---

## Char

`import Stdlib.Text` — `Char` is a single ASCII byte, the same byte-oriented
convention as the rest of `Text` (`toUpper`/`toLower`/`len` are all
byte-based, not real Unicode). There's no `Char` literal syntax yet —
construct one via `Text.charAt` or `Char.fromInt`.

```
Char.toText(c: Char): Text
Char.toInt(c: Char): Int                   // 0-255
Char.fromInt(n: Int): Char                 // truncates to a byte (n & 0xFF)
Char.isDigit(c: Char): Bool
Char.isAlpha(c: Char): Bool
Char.isUpperCase(c: Char): Bool
Char.isLowerCase(c: Char): Bool
Char.isWhitespace(c: Char): Bool
Char.toUpperCase(c: Char): Char
Char.toLowerCase(c: Char): Char
```

```certo
fn countDigits(s: Text): Int = {
  var n = 0
  var i = 0
  while i < Text.len(s) {
    match Text.charAt(s, i) {
      Some(c) => { if Char.isDigit(c) then n = n + 1 }
      None => {}
    }
    i = i + 1
  }
  n
}
```

---

## Collections

`import Stdlib.Collections`

### List\<T\>

```
List.empty<T>(): List<T>
List.len<T>(list: List<T>): Int
List.get<T>(list: List<T>, i: Int): T?          // None if out of bounds
List.getOrPanic<T>(list: List<T>, i: Int): T    // panics if out of bounds
List.first<T>(list: List<T>): T?
List.last<T>(list: List<T>): T?
List.push<T>(list: List<T>, item: T): List<T>   // appends; returns new list
List.concat<T>(a: List<T>, b: List<T>): List<T>
List.slice<T>(list: List<T>, start: Int, end: Int): List<T>  // [start, end)
List.reverse<T>(list: List<T>): List<T>
List.map<A, B>(list: List<A>, f: A => B): List<B>
List.filter<T>(list: List<T>, pred: T => Bool): List<T>
List.fold<T, A>(list: List<T>, init: A, f: (A, T) => A): A
List.find<T>(list: List<T>, pred: T => Bool): T?
List.contains<T>(list: List<T>, item: T): Bool
List.any<T>(list: List<T>, pred: T => Bool): Bool
List.all<T>(list: List<T>, pred: T => Bool): Bool
List.sort<T>(list: List<T>, cmp: (T, T) => Int): List<T>  // negative/0/positive
List.zip<A, B>(a: List<A>, b: List<B>): List<(A, B)>      // length = min(len a, len b)
List.distinct<T>(list: List<T>): List<T>                 // dedup, pointer equality (see Map note below)
List.partition<T>(list: List<T>, pred: T => Bool): (List<T>, List<T>)  // (matches, non-matches)
List.chunked<T>(list: List<T>, size: Int): List<List<T>>
List.groupBy<T, K>(list: List<T>, key: T => K): Map<K, List<T>>  // K uses Map's pointer-equality keys — see below
List.flatMap<A, B>(list: List<A>, f: A => List<B>): List<B>  // map then flatten one level
List.reduce<T, A>(list: List<T>, init: A, f: (A, T) => A): A  // same as List.fold, spec's own name
List.sortBy<T, K>(list: List<T>, key: T => K): List<T>   // ascending by projected key
List.minBy<T, K>(list: List<T>, key: T => K): T?         // None on an empty list
List.maxBy<T, K>(list: List<T>, key: T => K): T?         // None on an empty list
List.sumBy<T, N>(list: List<T>, key: T => N): N          // sums the projected numeric field
```

`sortBy`/`minBy`/`maxBy`/`sumBy`'s key/numeric projection (`K`/`N` above)
is restricted to `Int`/`Int8`/`Int16`/`Int32`/`UInt`/`Float`/`Float32` — a
deliberate scope decision, not a missing case: this codebase's `<`/`>`/`+`
C operators are only correct for those types (`Text`'s `<` is raw pointer
comparison, not lexicographic, and `Decimal` has no generic comparison/
addition operator to synthesize a call to). Passing an unsupported
projection type is a compile error, `E0710` (see
[ERROR-REFERENCE.md](ERROR-REFERENCE.md)). The list element type `T` itself
is unrestricted, including struct/record types — only the *projected*
key/sum type is limited.

### Map\<K, V\>

Map uses pointer equality for keys. Use `Text` keys carefully — intern
identical strings or compare via `Map.get` semantics.

```
Map.empty<K, V>(): Map<K, V>
Map.insert<K, V>(map: Map<K, V>, key: K, value: V): Map<K, V>  // returns new map
Map.get<K, V>(map: Map<K, V>, key: K): V?
Map.contains<K, V>(map: Map<K, V>, key: K): Bool
Map.remove<K, V>(map: Map<K, V>, key: K): Map<K, V>             // returns new map
Map.len<K, V>(map: Map<K, V>): Int
Map.keys<K, V>(map: Map<K, V>): List<K>
Map.values<K, V>(map: Map<K, V>): List<V>
Map.fromList<K, V>(pairs: List<(K, V)>): Map<K, V>
```

### Channel\<T\>

A thread-safe, capacity-bounded queue for communicating between `spawn`ed
tasks. `send` blocks while the channel is full; `receive` blocks while it's
empty. Sending on a closed channel panics. `receive` keeps delivering
already-buffered items after `close` — it only returns `None` once the
channel is both closed *and* drained, so "no more items" is a real `Option`
value, not a special sentinel.

```
Channel.new<T>(capacity: Int): Channel<T>
Channel.send<T>(channel: Channel<T>, item: T): Unit          // blocks if full; panics if closed
Channel.receive<T>(channel: Channel<T>): T?                  // blocks if empty; None once closed+drained
Channel.tryReceive<T>(channel: Channel<T>): T?                // never blocks; None if empty right now
Channel.close<T>(channel: Channel<T>): Unit                  // idempotent
Channel.isClosed<T>(channel: Channel<T>): Bool
```

```certo
module Producer

fn producer(ch: Channel<Int>): Unit [io] = {
  var i = 0
  while i < 5 {
    Channel.send(ch, i)
    i = i + 1
  }
  Channel.close(ch)
}

fn consumer(ch: Channel<Int>): Int [io] = {
  var sum = 0
  var running = true
  while running {
    match Channel.receive(ch) {
      Some(item) => { sum = sum + item }
      None => { running = false }
    }
  }
  sum
}

pub async fn main(): Unit [io] = {
  val ch = Channel.new(capacity: 2)
  val p = spawn producer(ch)
  val c = spawn consumer(ch)
  await p
  println(intToText(await c))   // 10
}
```

---

## Money / Decimal

`import Stdlib.Money`

`Decimal` is stored as `{ value: Int, scale: Int }` where the true value is
`value / 10^scale`. Never use `Float` for money.

### `Decimal(p, s)` — precision/scale

`Decimal(19, 4)` is the same runtime type as bare `Decimal` — the
`(precision, scale)` is a compile-time-only refinement, not a new runtime
representation. It exists specifically to check fidelity against a
`NUMERIC(p,s)` database column: `certo db pull` generates it directly from a
column's real precision/scale, and `certo db diff`/`[features] schema-sync`
flag it as drift if your code's declared precision/scale doesn't match the
live column's.

A bare `Decimal` unifies with *any* `Decimal(p, s)` in both directions — it's
always safe to pass a `Decimal(19, 4)` value anywhere a plain `Decimal` is
expected, and vice versa. Every arithmetic/comparison/conversion function
below is typed with bare `Decimal`, so none of them need a parameterized
overload:

```
val price: Decimal(19, 4) = d"199.99"
val tax: Decimal          = Decimal.fromInt(10)
val total = Decimal.add(price, tax)   // Decimal — bare Decimal params accept price fine
```

Two *different* parameterizations don't unify with each other —
`Decimal(10, 2)` and `Decimal(19, 4)` are a real type error if used
interchangeably, since that would silently misrepresent one column's real
constraint as another's.

### Arithmetic

```
Decimal.add(a: Decimal, b: Decimal): Decimal
Decimal.sub(a: Decimal, b: Decimal): Decimal
Decimal.mul(a: Decimal, b: Decimal): Decimal
Decimal.div(a: Decimal, b: Decimal): Decimal [fallible]   // panics on zero
```

### Comparison

```
Decimal.eq(a: Decimal, b: Decimal): Bool
Decimal.lt(a: Decimal, b: Decimal): Bool
Decimal.gt(a: Decimal, b: Decimal): Bool
Decimal.lte(a: Decimal, b: Decimal): Bool
Decimal.gte(a: Decimal, b: Decimal): Bool
```

### Unary

```
Decimal.abs(d: Decimal): Decimal
Decimal.negate(d: Decimal): Decimal
Decimal.round(d: Decimal, places: Int): Decimal   // half-up
```

### Conversion

```
Decimal.fromInt(n: Int): Decimal
Decimal.toInt(d: Decimal): Int          // truncates fractional part
Decimal.toText(d: Decimal): Text        // e.g. "19.99"
```

### Money helpers

```
Money.fromCents(cents: Int): Decimal    // cents / 100, scale=2
Money.toCents(m: Decimal): Int          // rounds to 2dp first
Money.fromDecimal(d: Decimal): Decimal  // round to 2dp
```

---

## DateTime

`import Stdlib.DateTime`

`DateTime` and `Date` are both `Int` (Unix seconds). `Date` values use
midnight UTC.

### Constructors

```
DateTime.now(): DateTime [io]
Date.today(): Date [io]
DateTime.fromUnix(secs: Int): DateTime
DateTime.toUnix(dt: DateTime): Int
```

### Formatting

```
DateTime.toIso(dt: DateTime): Text          // "2025-06-22T14:30:00Z"
DateTime.format(dt: DateTime, fmt: Text): Text   // strftime format
Date.format(d: Date, fmt: Text): Text
DateTime.parseIso(s: Text): DateTime [fallible]  // returns error if unparseable
```

### Arithmetic

```
DateTime.addSeconds(dt: DateTime, s: Int): DateTime
DateTime.addMinutes(dt: DateTime, m: Int): DateTime
DateTime.addHours(dt: DateTime, h: Int): DateTime
DateTime.addDays(dt: DateTime, d: Int): DateTime
DateTime.diffSeconds(a: DateTime, b: DateTime): Int   // a - b
DateTime.diffDays(a: DateTime, b: DateTime): Int      // (a - b) / 86400

DateTime.addDuration(dt: DateTime, duration: Duration): DateTime
DateTime.diff(a: DateTime, b: DateTime): Duration     // a - b
Date.addDuration(date: Date, duration: Duration): Date
```

### Comparison

```
DateTime.before(a: DateTime, b: DateTime): Bool
DateTime.after(a: DateTime, b: DateTime): Bool
DateTime.eq(a: DateTime, b: DateTime): Bool
```

### Components (UTC)

```
DateTime.year(dt: DateTime): Int
DateTime.month(dt: DateTime): Int    // 1–12
DateTime.day(dt: DateTime): Int      // 1–31
DateTime.hour(dt: DateTime): Int     // 0–23
DateTime.minute(dt: DateTime): Int   // 0–59
DateTime.second(dt: DateTime): Int   // 0–60
```

---

## Timestamp

`import Stdlib.DateTime`

`Timestamp` is a distinct nominal type from `DateTime` — a `DateTime` value
does not type-check where a `Timestamp` is expected, or vice versa — but
both compile to the identical representation (`Int` Unix seconds), so
constructing one and reading it back is exact either way. `Timestamp` is
also the only type `expr.age` accepts (see `docs/CERTO-SPEC.md`'s `.age`
entry). Timezone-aware operations take a `Timezone` (see `Timezone(name:
Text): Timezone?`, constructed elsewhere via `DateTime`'s own `Timezone(...)`
— same type, shared across both names).

```
Timestamp.now(): Timestamp [io]
Timestamp.of(year: Int, month: Int, day: Int, hour: Int, minute: Int, second: Int, tz: Timezone): Timestamp
Timestamp.parse(s: Text): Timestamp [fallible]     // ISO 8601, panics if unparseable
Timestamp.inTimezone(ts: Timestamp, tz: Timezone): Text   // e.g. "2026-07-15T13:30:00+01:00"
Timestamp.formatTz(ts: Timestamp, fmt: Text, tz: Timezone): Text   // strftime format, tz's wall clock

Date.of(year: Int, month: Int, day: Int): Date     // UTC midnight, same convention as Date.today()
```

`Timestamp.of`'s `year`/`month`/.../`second` are the wall-clock time *in*
`tz`, not UTC — `Timestamp.of(2026, 7, 15, 13, 30, 0, london)` where
`london` is `Europe/London` (BST, UTC+1 in July) produces the same instant
as `13:30:00 - 01:00` UTC.

---

## Duration

`import Stdlib.DateTime`

`Duration` is an `Int` (signed seconds) representing a span of time —
not a point in time. No `Timezone` type exists yet; see BACKLOG.

### Constructors

```
Duration.seconds(n: Int): Duration
Duration.minutes(n: Int): Duration
Duration.hours(n: Int): Duration
Duration.days(n: Int): Duration
```

### Accessors (truncating)

```
Duration.toSeconds(d: Duration): Int
Duration.toMinutes(d: Duration): Int
Duration.toHours(d: Duration): Int
Duration.toDays(d: Duration): Int
```

### Arithmetic and comparison

```
Duration.add(a: Duration, b: Duration): Duration
Duration.sub(a: Duration, b: Duration): Duration
Duration.negate(d: Duration): Duration
Duration.eq(a: Duration, b: Duration): Bool
Duration.lt(a: Duration, b: Duration): Bool
Duration.gt(a: Duration, b: Duration): Bool
```

---

## Math

`import Stdlib.Math`

### Constants

```
Math.pi(): Float    // 3.14159…
Math.e(): Float     // 2.71828…
```

### Trigonometry (radians)

```
Math.sin(x: Float): Float
Math.cos(x: Float): Float
Math.tan(x: Float): Float
Math.asin(x: Float): Float
Math.acos(x: Float): Float
Math.atan(x: Float): Float
Math.atan2(y: Float, x: Float): Float
```

### Logarithm and exponential

```
Math.log(x: Float): Float       // natural log
Math.log2(x: Float): Float
Math.log10(x: Float): Float
Math.exp(x: Float): Float       // e^x
Math.pow(x: Float, y: Float): Float
```

### Geometry

```
Math.hypot(a: Float, b: Float): Float   // sqrt(a² + b²)
```

### Clamp, sign, round

```
Math.clamp(x: Float, lo: Float, hi: Float): Float
Math.clampInt(x: Int, lo: Int, hi: Int): Int
Math.sign(x: Float): Float        // -1.0, 0.0, or 1.0
Math.signInt(x: Int): Int         // -1, 0, or 1
Math.trunc(x: Float): Float       // round toward zero
```

### Random

```
Math.random(): Float    // [0.0, 1.0); seeded from system time on first call
```

---

## Json

`import Stdlib.Json`

### Constructors

```
Json.null(): JsonValue
Json.bool(b: Bool): JsonValue
Json.int(i: Int): JsonValue
Json.float(f: Float): JsonValue
Json.string(s: Text): JsonValue
Json.array(): JsonValue     // empty mutable array
Json.object(): JsonValue    // empty mutable object
```

### Parse and serialise

```
Json.parse(text: Text): JsonValue       // panics on invalid JSON
Json.stringify(value: JsonValue): Text
```

### Type tests

```
JsonValue.isNull(v: JsonValue): Bool
JsonValue.isBool(v: JsonValue): Bool
JsonValue.isInt(v: JsonValue): Bool
JsonValue.isFloat(v: JsonValue): Bool
JsonValue.isString(v: JsonValue): Bool
JsonValue.isArray(v: JsonValue): Bool
JsonValue.isObject(v: JsonValue): Bool
```

### Extraction

```
JsonValue.asBool(v: JsonValue): Bool        // false if wrong type
JsonValue.asInt(v: JsonValue): Int          // 0 if wrong type
JsonValue.asFloat(v: JsonValue): Float      // 0.0 if wrong type
JsonValue.asText(v: JsonValue): Text        // "" if wrong type
```

### Array access

```
JsonValue.length(v: JsonValue): Int
JsonValue.at(v: JsonValue, index: Int): JsonValue
JsonValue.push(arr: JsonValue, item: JsonValue): Unit [io]   // mutates arr
```

### Object access

```
JsonValue.get(v: JsonValue, key: Text): JsonValue    // alias: field
JsonValue.field(v: JsonValue, key: Text): JsonValue
JsonValue.keys(v: JsonValue): List<Text>
JsonValue.set(obj: JsonValue, key: Text, value: JsonValue): Unit [io]  // mutates obj
```

> `push` and `set` mutate in place and also return the object, so they can
> be chained with `|>`.

---

## Http

`import Stdlib.Http`

### Client

```
Http.get(url: Text): HttpResponse [io]
Http.post(url: Text, body: Text, contentType: Text): HttpResponse [io]
Http.put(url: Text, body: Text, contentType: Text): HttpResponse [io]
Http.delete(url: Text): HttpResponse [io]
```

### Reading responses

```
HttpResponse.status(r: HttpResponse): Int
HttpResponse.body(r: HttpResponse): Text
HttpResponse.bodyLength(r: HttpResponse): Int // binary-safe response byte count
HttpResponse.contentType(r: HttpResponse): Text
HttpResponse.ok(r: HttpResponse): Bool       // status in 200–299
```

### Server

```
Http.serve(port: Int, handler: fn(HttpRequest): HttpResponse): Unit [io]
```

### Building responses

```
Http.respond(status: Int, body: Text, contentType: Text): HttpResponse
Http.ok(body: Text, contentType: Text): HttpResponse          // 200
Http.notFound(body: Text): HttpResponse                       // 404
Http.badRequest(body: Text): HttpResponse                     // 400
Http.serverError(body: Text): HttpResponse                    // 500
```

### Reading requests

```
HttpRequest.method(r: HttpRequest): Text          // "GET", "POST", …
HttpRequest.path(r: HttpRequest): Text            // "/api/users"
HttpRequest.query(r: HttpRequest): Text           // "?key=val" raw string
HttpRequest.body(r: HttpRequest): Text
HttpRequest.header(r: HttpRequest, name: Text): Text   // "" if absent
HttpRequest.headers(r: HttpRequest): List<List<Text>>  // [[name, value], …]
```

---

## Db

`import Stdlib.Db`

All parameters are passed as `Text` (positional `$1`, `$2`, … in SQL).

### Connection

```
dbConnect(connstr: Text): Int [io]    // returns handle; 0 on failure
dbClose(conn: Int): Unit [io]
dbError(conn: Int): Text              // last error message
dbServerVersion(conn: Int): Int [io]  // e.g. 160000 for PG 16
dbVersionString(conn: Int): Text [io]
```

### Execution

```
dbExec(conn: Int, sql: Text, params: List<Text>): Int [io]
    // Returns rows-affected (≥ 0) or -1 on error
```

### Queries

```
dbQuery(conn: Int, sql: Text, params: List<Text>): List<List<Text?>> [io]
    // Returns list of rows; each row is a list of nullable Text values

dbQueryTyped<T>(conn: Int, sql: Text, params: List<Text>,
                mapper: fn(List<Text?>): T): List<T> [io]
    // Applies mapper to each row; returns typed list

dbQueryRow(conn: Int, sql: Text, params: List<Text>): List<Text?>? [io]
    // First row only, or None if no rows

dbQueryOne(conn: Int, sql: Text): Text? [io]
    // First column of first row, or None

dbColumns(conn: Int, sql: Text): List<Text> [io]
    // Column names of the result set

dbStream(conn: Int, sql: Text, params: List<Text>,
         handler: fn(List<Text?>): Unit): Int [io]
    // Calls handler for each row; returns row count
```

### Transactions

```
dbBegin(conn: Int): Int [io]
dbCommit(conn: Int): Int [io]
dbRollback(conn: Int): Int [io]

withTransaction(conn: Int, body: fn(): Result<T, E>): Result<T, E> [io]
    // Commits if body returns Ok; rolls back on Err

withConnection(url: Text, body: fn(Int): Result<T, E>): Result<T, E> [io]
    // Opens connection, runs body, closes connection
```

### Null sentinel

```
dbNull(): Text    // returns the null sentinel value for use in params
```

---

## File

`import Stdlib.File`

```
readFile(path: Text): Text? [io]              // None if not found
writeFile(path: Text, content: Text): Bool [io]   // true on success
appendFile(path: Text, content: Text): Bool [io]
fileExists(path: Text): Bool [io]
deleteFile(path: Text): Bool [io]
listDir(path: Text): List<Text>? [io]         // None if not a directory
```

---

## Path

`import Stdlib.Path`

```
Path.join(a: Text, b: Text): Text       // OS-correct path separator
Path.basename(path: Text): Text         // "foo.txt" from "/dir/foo.txt"
Path.dirname(path: Text): Text          // "/dir" from "/dir/foo.txt"
Path.extension(path: Text): Text?       // "txt", or None if no extension
Path.stem(path: Text): Text             // "foo" from "foo.txt"
```

---

## Env

`import Stdlib.Env`

```
getEnv(key: Text): Text?                     // None if not set
setEnv(key: Text, val: Text): Unit [io]
unsetEnv(key: Text): Unit [io]
```

---

## Process

`import Stdlib.Process`

```
Process.exec(cmd: Text, args: List<Text>): ProcessResult [io]
    // Runs command synchronously; collects stdout and stderr

Process.execWithInput(cmd: Text, args: List<Text>, input: Text): ProcessResult [io]
    // Same but writes input to the process's stdin

Process.lines(cmd: Text, args: List<Text>, handler: fn(Text): Unit): Int [io]
    // Streams stdout line by line; returns exit code

ProcessResult.exitCode(r: ProcessResult): Int
ProcessResult.stdout(r: ProcessResult): Text
ProcessResult.stderr(r: ProcessResult): Text
```

---

## Host

`import Stdlib.Host`

An in-process host for statically linked plugins. Plugins start in registration
order and stop in reverse order. If startup fails, the host stops every plugin
that already started before returning the original startup error.

```
Host.new(): Host
Host.plugin(
    name: Text,
    start: fn(HostContext): Result<Unit, Text>,
    stop: fn(HostContext): Result<Unit, Text>
): HostPlugin
HostPlugin.provides<T>(plugin: HostPlugin, key: ServiceKey<T>): HostPlugin
HostPlugin.requires<T>(plugin: HostPlugin, key: ServiceKey<T>): HostPlugin
HostPlugin.worker(
    plugin: HostPlugin,
    name: Text,
    run: fn(HostContext): Result<Unit, Text>
): HostPlugin
HostPlugin.quiesce(
    plugin: HostPlugin,
    callback: fn(HostContext): Result<Unit, Text>
): HostPlugin
RestartPolicy.never(): RestartPolicy
RestartPolicy.onFailure(
    maxRetries: Int,
    initialDelay: Duration,
    maxDelay: Duration
): RestartPolicy
RestartPolicy.always(
    maxRetries: Int,
    initialDelay: Duration,
    maxDelay: Duration
): RestartPolicy
HostPlugin.restart(plugin: HostPlugin, policy: RestartPolicy): HostPlugin
HostPlugin.provideFactory<T>(
    plugin: HostPlugin,
    key: ServiceKey<T>,
    factory: fn(HostContext): Result<T, Text>,
    dispose: fn(HostContext): Result<Unit, Text>
): HostPlugin
HostPlugin.factoryDependsOn<T, U>(
    plugin: HostPlugin,
    factoryKey: ServiceKey<T>,
    dependencyKey: ServiceKey<U>
): HostPlugin
Host.add(host: Host, plugin: HostPlugin): Host
Host.configure(host: Host, key: Text, value: Text): Host
Host.configKey<T>(name: Text, parse: fn(Text): Result<T, Text>): ConfigKey<T>
Host.requireConfig<T>(host: Host, key: ConfigKey<T>): Host
Host.defaultConfig<T>(host: Host, key: ConfigKey<T>, value: T): Host
Host.validateConfig<T>(host: Host, key: ConfigKey<T>, validator: fn(T): Result<Unit, Text>): Host
Host.shutdownTimeout(host: Host, timeout: Duration): Host
Host.readinessTimeout(host: Host, timeout: Duration): Host
Host.quiesceTimeout(host: Host, timeout: Duration): Host
Host.drainTimeout(host: Host, timeout: Duration): Host
Host.stopTimeout(host: Host, timeout: Duration): Host
Host.disposalTimeout(host: Host, timeout: Duration): Host

Host.serviceKey<T>(name: Text): ServiceKey<T>
Host.provide<T>(host: Host, key: ServiceKey<T>, service: T): Host
Host.provideFactory<T>(
    host: Host,
    key: ServiceKey<T>,
    factory: fn(HostContext): Result<T, Text>,
    dispose: fn(HostContext): Result<Unit, Text>
): Host
Host.factoryDependsOn<T, U>(
    host: Host,
    factoryKey: ServiceKey<T>,
    dependencyKey: ServiceKey<U>
): Host

Host.start(host: Host): Result<Unit, Text> [io]
Host.stop(host: Host): Result<Unit, Text> [io]
Host.run(host: Host): Result<Unit, Text> [io]
Host.waitUntilReady(host: Host, timeout: Duration): Result<Unit, Text> [io]
Host.health(host: Host): Text
Host.metrics(host: Host): Text [io]
Host.workerHealth(host: Host, name: Text): Text?
Host.workerRestarts(host: Host, name: Text): Int?
Host.workerLastError(host: Host, name: Text): Text?

Host.startTyped(host: Host): Result<Unit, HostLifecycleError> [io]
Host.stopTyped(host: Host): Result<Unit, HostLifecycleError> [io]
Host.runTyped(host: Host): Result<Unit, HostLifecycleError> [io]
Host.waitUntilReadyTyped(
    host: Host,
    timeout: Duration
): Result<Unit, HostLifecycleError> [io]
Host.status(host: Host): HostStatusSnapshot [io]

HostStatusSnapshot.state(snapshot: HostStatusSnapshot): HostState
HostStatusSnapshot.isReady(snapshot: HostStatusSnapshot): Bool
HostStatusSnapshot.isLive(snapshot: HostStatusSnapshot): Bool
HostStatusSnapshot.workerCount(snapshot: HostStatusSnapshot): Int
HostStatusSnapshot.readyWorkers(snapshot: HostStatusSnapshot): Int
HostStatusSnapshot.workers(snapshot: HostStatusSnapshot): List<HostWorkerStatus>
HostStatusSnapshot.counters(snapshot: HostStatusSnapshot): List<HostMetricSnapshot>
HostStatusSnapshot.gauges(snapshot: HostStatusSnapshot): List<HostMetricSnapshot>
HostStatusSnapshot.lastFailure(snapshot: HostStatusSnapshot): HostLifecycleError?

HostWorkerStatus.name(worker: HostWorkerStatus): Text
HostWorkerStatus.plugin(worker: HostWorkerStatus): Text
HostWorkerStatus.state(worker: HostWorkerStatus): HostWorkerState
HostWorkerStatus.isReady(worker: HostWorkerStatus): Bool
HostWorkerStatus.isLive(worker: HostWorkerStatus): Bool
HostWorkerStatus.restarts(worker: HostWorkerStatus): Int
HostWorkerStatus.lastError(worker: HostWorkerStatus): Text?
HostMetricSnapshot.name(metric: HostMetricSnapshot): Text
HostMetricSnapshot.value(metric: HostMetricSnapshot): Int

HostState.name(state): Text
HostWorkerState.name(state): Text
HostLifecycleError.kind(error): HostFailureKind
HostLifecycleError.phase(error): Text
HostLifecycleError.subject(error): Text
HostLifecycleError.message(error): Text
HostLifecycleError.isTimeout(error): Bool
HostLifecycleError.configurationErrors(error): List<HostConfigurationError>
HostConfigurationError.key(error): Text
HostConfigurationError.source(error): Text
HostConfigurationError.location(error): Text
HostConfigurationError.category(error): Text
HostConfigurationError.message(error): Text
HostFailureKind.name(kind): Text

Host.requestStop(context: HostContext): Unit [io]
HostContext.ready(context: HostContext): Unit [io]
HostContext.fail(context: HostContext, error: Text): Unit [io]
HostContext.sleep(context: HostContext, duration: Duration): Bool [io]
HostContext.waitUntil(
    context: HostContext,
    predicate: fn(HostContext): Bool,
    interval: Duration
): Bool [io]
HostContext.log(
    context: HostContext,
    level: Text,
    event: Text,
    message: Text
): Unit [io]
HostLogSeverity.trace/debug/info/warn/error/fatal(): HostLogSeverity
HostLogSeverity.name(severity: HostLogSeverity): Text
HostLogField.text(name: Text, value: Text): HostLogField
HostLogField.int(name: Text, value: Int): HostLogField
HostLogField.float(name: Text, value: Float): HostLogField
HostLogField.bool(name: Text, value: Bool): HostLogField
HostLogEvent.create(
    severity: HostLogSeverity,
    event: Text,
    message: Text,
    fields: List<HostLogField>
): HostLogEvent
HostContext.logEvent(context: HostContext, event: HostLogEvent): Unit [io]
HostLogEvent.schema/sequence/timestampUnixMs/severity/event/message/hostId(...)
HostLogEvent.plugin/worker/correlationId(...)
HostLogEvent.fields(event: HostLogEvent): List<HostLogField>
HostLogOverflowPolicy.dropNewest/dropOldest(): HostLogOverflowPolicy
HostLogOverflowPolicy.wait(duration: Duration): HostLogOverflowPolicy
HostLogFailurePolicy.ignore/disable/failHost(): HostLogFailurePolicy
Host.logSink(
    host: Host,
    name: Text,
    capacity: Int,
    overflow: HostLogOverflowPolicy,
    failure: HostLogFailurePolicy,
    write: fn(HostLogEvent): Result<Unit, Text>,
    flush: fn(): Result<Unit, Text>,
    dispose: fn(): Result<Unit, Text>
): Host
Host.disableStderrLog(host: Host): Host
Host.telemetryTimeout(host: Host, timeout: Duration): Host
HostContext.counter(context: HostContext, name: Text, amount: Int): Unit [io]
HostContext.gauge(context: HostContext, name: Text, value: Int): Unit [io]
HostContext.isStopping(context: HostContext): Bool
HostContext.pluginCount(context: HostContext): Int
HostContext.service<T>(context: HostContext, key: ServiceKey<T>): T?
HostContext.config(context: HostContext, key: Text): Text?
HostContext.configOr(context: HostContext, key: Text, fallback: Text): Text
HostContext.configValue<T>(context: HostContext, key: ConfigKey<T>): T
```

Log sinks must be registered while the host is new. Each sink owns an
independent bounded queue and serial callback worker. Overflow can drop the
newest event, replace the oldest queued event, or wait for a bounded duration.
Callback failures can be ignored, disable only that sink, or request orderly
host shutdown. Shutdown closes admission, drains accepted events, then flushes
and disposes sinks in reverse registration order within the per-sink telemetry
timeout. Supply a callback returning `Ok(())` when no flush work is needed.

Typed host configuration is merged at startup in this order: typed defaults,
the `[host]` subtree of `certo.toml`, `CERTO__...` environment variables,
repeatable `--config key=value` arguments, then `Host.configure` overrides.
Later entries win within one source. Parsing and validation complete before any
service factory or plugin callback runs. `HostConfigurationError` reports the
winning source and location without including its raw value.

Keys whose parsed type structurally contains `Secret<_>` redact parser and
validator messages. Their source and location remain visible, but raw access
through `HostContext.config` or `configOr` is rejected; use `configValue`.

Configuration is snapshotted once during startup and is not reloaded. Restart
the process to apply changes to files, environment variables, or arguments.

`Host.run` starts the host, waits until Ctrl+C, SIGTERM, or `requestStop`, then
performs an orderly shutdown. Use `start` and `stop` separately for applications
that already own their main loop. A host cannot be started twice or modified
after startup. Calling `Host.stop` while initial startup is in progress requests
startup cancellation and waits for rollback to finish. `Host.start` then returns
`Err("host startup cancelled")`, while the waiting `Host.stop` returns `Ok(())`
after every successfully started plugin has been stopped. Calling `Host.stop`
while a worker is restarting performs an ordinary graceful shutdown.

The `*Typed` lifecycle methods preserve those contracts but return structured
errors. `HostFailureKind.name` is one of `StartupFailure`, `StartupCancelled`,
`WorkerFailure`, `ShutdownFailure`, `Timeout`, or `ForcedTermination` (the last
is reserved until forced termination is implemented). `phase` identifies the
lifecycle boundary, `subject` identifies the host, plugin, or worker, and
`message` retains the original diagnostic. Existing lifecycle methods remain
compatibility adapters returning the same `Text` errors as before.

`Host.status` captures an immutable point-in-time operational snapshot. A host
is ready only when its state is `Healthy` and all launched workers are ready. It
is live in `New`, `Starting`, `Healthy`, or `Stopping`, and not live after
`Stopped` or `Failed`. A worker is live while `Starting`, `Healthy`, or
`Restarting`; it is not live after `Stopped` or `Failed`. Worker entries include
their plugin, typed state, readiness, restart count, and last callback error.
Counters and gauges are copied into typed metric entries. `Host.metrics`,
`Host.health`, and the individual `worker*` methods remain compatibility views.

Host lifecycle transitions are deterministic:

| Current state | Event | Result | Next state |
|---|---|---|---|
| `New` | `start*` / `run*` | plugins and workers begin starting | `Starting` |
| `New` | `stop*` | `Err` (`host is not running`) | `New` |
| initial `Starting` | startup succeeds and all workers are ready | `Ok` | `Healthy` |
| initial `Starting` | plugin/worker/readiness failure | typed failure | `Failed` |
| initial `Starting` | `stop*` | requests cancellation, waits for rollback, then returns `Ok` | `Failed` |
| restart `Starting` | restarted workers become ready | — | `Healthy` |
| restart `Starting` | `stop*` | owns or joins graceful shutdown | `Stopping` |
| `Healthy` | first `stop*` | owns graceful shutdown | `Stopping` |
| `Stopping` | another `stop*` | waits and returns the owner's shared result | `Stopped` or `Failed` |
| `Stopped` | `stop*` | returns the cached `Ok` result | `Stopped` |
| any non-`New` state | `start*` / `run*` | `Err` (`host has already started`) | unchanged |
| `Failed` | `stop*` after a completed failed shutdown | returns the cached failure | `Failed` |

Here, `start*`, `stop*`, and `run*` mean both typed and text-compatible forms.
A readiness timeout cancels and drains workers, rolls back plugins, records a
typed `Timeout`, and leaves the host `Failed`. Quiesce, drain, or stop timeout
records `Timeout`; shutdown continues through later phases and finishes
`Failed`. Callback timeouts are cooperative: the host does not forcibly kill a
native callback thread. `ForcedTermination` is therefore never emitted by this
version.

Callback execution and API rules:

| Callback | Ordering and concurrency | Safe operations |
|---|---|---|
| host service factory | serial, stable dependency order, before plugins | resolve declared host dependencies, context config reads, logging, metrics |
| scoped service factory | serial, stable local dependency order, immediately before its plugin | resolve that plugin's scoped services and host services, context reads, logging, metrics |
| plugin `start` | serial, dependency order, on the `start*` caller | context service/config reads, status, logging, metrics, interruptible waits |
| worker | one host-owned thread per worker; workers run concurrently | all `HostContext` worker methods, status, logging, metrics |
| plugin `quiesce` | serial, reverse dependency order; may overlap a callback that exceeded its timeout | context reads, status, logging, metrics, bounded cleanup |
| plugin `stop` | serial, reverse dependency order; runs after worker drain | context reads, status, logging, metrics, bounded cleanup |
| host service disposer | serial, reverse construction order, after all plugins stop; a timeout retains the rest of the scope | resolve the service being disposed and dependencies that outlive it, context reads, logging, metrics |
| scoped service disposer | serial within its scope, reverse local construction order, immediately after its plugin stops; a timeout may overlap cleanup of independent plugin scopes | resolve only that plugin's scoped services and host services, context reads, logging, metrics |

Callbacks must not call `Host.start*`, `Host.run*`, or `Host.stop*` on their own
host. Those operations own or wait for the callback's lifecycle phase and a
reentrant call can wait on itself. `Host.requestStop(context)` is the safe way for
a callback to request eventual shutdown. Registration, service provisioning,
configuration, and timeout setters are pre-start operations and must not be
called from lifecycle callbacks.

Services use typed keys rather than casts or string-based result types:

```
type Logger = { prefix: Text }

fn loggerKey(): ServiceKey<Logger> = Host.serviceKey("logger")

val host = Host.new()
    .configure("worker.queue", "orders")
    .provide(loggerKey(), Logger { prefix: "[app]" })
    .add(workerPlugin().requires(loggerKey()))
    .add(loggerPlugin().provides(loggerKey()))
```

Host-owned singleton services use `provideFactory`. Declare dependencies with
`factoryDependsOn`; registration order is only the stable tie-breaker. The host
validates missing dependencies and cycles before invoking any factory, then
constructs factories in topological order. A disposer retrieves its typed value
through `HostContext.service` and runs in reverse construction order. Factory or
plugin startup failure disposes every service that was already constructed.
Values registered with `provide` remain caller-owned and are never disposed by
the host.

Plugin-local services use `HostPlugin.provideFactory` and declare dependencies
with `HostPlugin.factoryDependsOn`. They are constructed immediately before the
owning plugin starts and disposed immediately after it stops. A plugin callback
or worker can resolve its own scoped services before host services, but scoped
services from other plugins are invisible. Cross-plugin scoped dependencies,
missing dependencies, and local dependency cycles fail during preflight before
any factory or plugin callback runs. A scoped factory failure disposes the
current plugin's completed services, then rolls back previously started plugins
and their scopes. If a plugin stop callback exceeds its timeout, its scoped
services are retained because the callback may still be using them.

Service disposers have an independent `disposalTimeout`, defaulting to 10
seconds. A timed-out disposer reports a typed `Timeout` in phase `dispose`; its
subject is `host/<service>` or `<plugin>/<service>`. Because callback timeouts
are cooperative, the host retains the timed-out service and its remaining
dependencies. Cleanup continues with independent plugin scopes. If a scoped
disposer times out, host-owned services are also retained because that callback
can still resolve them. This deliberate retention lasts until process teardown
and prevents use-after-disposal; the timed-out callback is never retried.

Looking up `loggerKey()` returns `Logger?`; using a key with the wrong service
type is rejected during type checking. Duplicate service names are rejected.
Before startup, the host validates every requirement and performs a stable
topological sort. Providers start before their consumers even when registered
later; unrelated plugins retain registration order. Missing services, duplicate
provider claims, and dependency cycles return an error without starting anything.

Workers start only after every plugin has initialized. Each worker must call
`HostContext.ready` after its own initialization; `Host.start` waits for all
workers and fails if the readiness timeout elapses. The default is 10 seconds.
`Host.waitUntilReady` supports an explicit deadline for callers that need to
recheck readiness, and `Host.health` reports `Starting`, `Healthy`, `Stopping`,
`Stopped`, or `Failed`. A worker may call `HostContext.fail` to record a fatal health error
and request shutdown immediately.

`HostContext.sleep` waits for the duration and returns `true`, or wakes early
and returns `false` when shutdown begins. `HostContext.waitUntil` evaluates a
predicate immediately and then at the requested interval; it returns `true`
when the predicate succeeds or `false` when shutdown interrupts the wait. Both
operations share the host's wake signal, so workers do not need polling loops.

Shutdown invokes plugin `quiesce` callbacks in reverse dependency order before
requesting worker cancellation. It then wakes and drains workers before invoking
plugin `stop` callbacks in reverse order. Quiesce, drain, stop, and service
disposal have separate timeouts; `shutdownTimeout` remains as a compatibility
alias for `drainTimeout`.
Errors from every shutdown phase are aggregated into the returned error instead
of discarding later failures. A worker error
requests host shutdown and is returned by `Host.run`; a worker that misses the
deadline produces a shutdown-timeout error. The default timeout is 10 seconds.

`HostContext.log` writes one JSON object per line to stderr. Every record includes
`timestamp_ms`, `level`, `event`, `message`, and plugin context; worker logs also
include the worker name. Output is serialized across workers so records never
interleave. `counter` atomically adds to a named counter, while `gauge` replaces
a named integer gauge. `Host.metrics` returns a JSON snapshot containing host
health, counters, and gauges. The host automatically records worker starts,
ready workers, worker failures, and shutdown failures.

The worker most recently added with `HostPlugin.worker` can be configured with
`.restart(policy)`. `onFailure` restarts only failed workers; `always` also
restarts workers that return successfully; `never` makes failure fatal
immediately. Delays use exponential backoff capped by `maxDelay`. A restarting
worker leaves readiness until it calls `HostContext.ready` again. Restarts are
disabled during quiescing, and exhausting `maxRetries` fails the host and begins
graceful shutdown. Restart events are logged and counted automatically.
`workerHealth`, `workerRestarts`, and `workerLastError` expose supervisor state by
worker name.

Plugin names and worker names must be non-empty and unique within a host.
Service-key names cannot be empty. Restart counts and delays cannot be negative,
and `maxDelay` must be at least `initialDelay`. Metric names use the portable
`[A-Za-z_:][A-Za-z0-9_.:-]*` form; empty log levels and event names are rejected.
These checks run during host construction, before any plugin is started.

See `examples/host.cto` for a complete lifecycle example.

---

## Cli

`import Stdlib.Cli`

Declarative command-line parsing inspired by clap. Empty `short`, `long`, or
`version` strings omit that part of a definition. `Cli.parse` reads the current
process arguments and never exits; the application decides how to present an
error, help, or version response.

```
Cli.command(name: Text, about: Text, version: Text): CliCommand
Cli.option(cmd: CliCommand, name: Text, short: Text, long: Text,
           valueName: Text, help: Text): CliCommand
Cli.flag(cmd: CliCommand, name: Text, short: Text, long: Text,
         help: Text): CliCommand
Cli.positional(cmd: CliCommand, name: Text, valueName: Text,
               help: Text): CliCommand
Cli.required(cmd: CliCommand, name: Text): CliCommand
Cli.defaultValue(cmd: CliCommand, name: Text, value: Text): CliCommand
Cli.subcommand(cmd: CliCommand, child: CliCommand): CliCommand

Cli.parse(cmd: CliCommand): CliMatches [io]
Cli.help(cmd: CliCommand): Text
Cli.version(cmd: CliCommand): Text

CliMatches.ok(matches: CliMatches): Bool
CliMatches.error(matches: CliMatches): Text
CliMatches.helpRequested(matches: CliMatches): Bool
CliMatches.versionRequested(matches: CliMatches): Bool
CliMatches.help(matches: CliMatches): Text       // help for active command
CliMatches.version(matches: CliMatches): Text    // version for active command
CliMatches.has(matches: CliMatches, name: Text): Bool
CliMatches.get(matches: CliMatches, name: Text): Text?
CliMatches.getOr(matches: CliMatches, name: Text, fallback: Text): Text
CliMatches.flag(matches: CliMatches, name: Text): Bool
CliMatches.subcommand(matches: CliMatches): CliMatches?
CliMatches.subcommandName(matches: CliMatches): Text
```

Long options accept `--output value` and `--output=value`. Short options accept
`-o value` and attached values such as `-ofile`; flags may be grouped (`-vq`).
`--` ends option parsing. `-h`/`--help` and `-V`/`--version` are built in.
Errors and help/version requests in a subcommand are visible from the root
matches; `matches.help()` and `matches.version()` select the active command.

---

## Regex

`import Stdlib.Regex`

Patterns use POSIX extended regular expression syntax.

```
Regex.match(pattern: Text, input: Text): Bool
    // true if pattern matches anywhere in input

Regex.find(pattern: Text, input: Text): Text
    // first match substring, or "" if none

Regex.captures(pattern: Text, input: Text): List<Text>
    // all capture groups of the first match

Regex.replace(pattern: Text, input: Text, replacement: Text): Text
    // replace first match with replacement

Regex.split(pattern: Text, input: Text): List<Text>
    // split input at each match of pattern
```

---

## Csv

`import Stdlib.Csv`

```
Csv.parse(text: Text): List<List<Text>>
    // Parses RFC-4180 CSV; returns list of rows (first row = headers if present)

Csv.serialize(rows: List<List<Text>>): Text
    // Serializes rows back to CSV text

Csv.header(rows: List<List<Text>>): List<Text>
    // Returns first row (the header row)

Csv.rows(rows: List<List<Text>>): List<List<Text>>
    // Returns all rows after the first
```

---

## Crypto

`import Stdlib.Crypto`

```
Crypto.sha256(s: Text): Text          // hex-encoded SHA-256 digest
Crypto.md5(s: Text): Text             // hex-encoded MD5 digest
Crypto.base64Encode(s: Text): Text    // standard Base64
Crypto.base64Decode(s: Text): Text    // panics on invalid input
```

---

## Effect annotations

Functions annotated `[io]` perform I/O and may not be used in pure contexts.
Functions annotated `[fallible]` can panic at runtime on invalid input.

---

## Operator quick reference

| Operator | Meaning |
|---|---|
| `++` | Text concatenation |
| `??` | Null-coalesce: `a ?? b` returns `a` if `Some(a)`, else `b` |
| `\|>` | Pipeline: `x \|> f` is `f(x)` |
| `?` | Error propagation: unwraps `Ok(v)` or returns `Err` early |
| `and` / `or` / `not` | Boolean operators |
| `==` `!=` `<` `>` `<=` `>=` | Comparison (structural for records) |
