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
| [Duration](#duration) | `import Stdlib.DateTime` |
| [Math](#math) | `import Stdlib.Math` |
| [Json](#json) | `import Stdlib.Json` |
| [Http](#http) | `import Stdlib.Http` |
| [Db](#db) | `import Stdlib.Db` |
| [File](#file) | `import Stdlib.File` |
| [Path](#path) | `import Stdlib.Path` |
| [Env](#env) | `import Stdlib.Env` |
| [Process](#process) | `import Stdlib.Process` |
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
Text.toUpper(s: Text): Text
Text.toLower(s: Text): Text
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
```

`sumBy`/`minBy`/`maxBy` (numeric/ordered projections) are not implemented yet —
see BACKLOG item 112: passing a Float-returning lambda into any of these
higher-order stdlib functions is unsound today (a deeper, pre-existing
codegen bug, not specific to these three).

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
