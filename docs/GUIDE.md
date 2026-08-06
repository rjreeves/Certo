# Certo — Developer Guide

> How to write, build, test, and ship Certo programs.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Installing Certo](#2-installing-certo)
3. [Your First Program](#3-your-first-program)
4. [Language Basics](#4-language-basics)
5. [Modules](#5-modules)
6. [Standard Library](#6-standard-library)
7. [Testing](#7-testing)
8. [Linting, Benchmarking, Docs, and the REPL](#8-linting-benchmarking-docs-and-the-repl)
9. [Project Layout & certo new](#9-project-layout--certo-new)
10. [Database Migrations](#10-database-migrations)
11. [Compiling to a Shared Library (DLL)](#11-compiling-to-a-shared-library-dll)
12. [Code Formatting](#12-code-formatting)
13. [LLVM IR & WebAssembly](#13-llvm-ir--webassembly)
14. [FFI — C Headers & REST Clients](#14-ffi--c-headers--rest-clients)
15. [UI Views & Forms](#15-ui-views--forms)
16. [Language Server (LSP)](#16-language-server-lsp)
17. [Building the Compiler from Source](#17-building-the-compiler-from-source)
18. [Release & Distribution Scripts](#18-release--distribution-scripts)
19. [Compiler Pipeline Reference](#19-compiler-pipeline-reference)

---

## 1. Prerequisites

| Tool | Minimum | Purpose |
|------|---------|---------|
| [Clang / LLVM](https://github.com/llvm/llvm-project/releases) | 15+ | C backend compilation |
| [Rust & Cargo](https://rustup.rs) | 1.77+ | Building the compiler from source |

**Windows quick-install:**
```powershell
winget install LLVM.LLVM
winget install Rustlang.Rustup
```

**macOS:**
```bash
brew install llvm
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clang must be on `PATH` **or** installed at `C:\Program Files\LLVM\bin\clang.exe`
(the compiler probes both automatically).

---

## 2. Installing Certo

### Option A — pre-built binaries (recommended)

Download a release archive from the releases page and extract it.
The archive contains these executables:

| Binary | Purpose |
|--------|---------|
| `certo` | Main compiler — Certo → native executable or DLL |
| `certo-test` | Test runner |
| `certo-fmt` | Code formatter |
| `certo-lsp` | Language server (for editor integration) |
| `certo-llvm` | Emit LLVM IR |
| `certo-wasm` | Emit WebAssembly IR |
| `certo-ffi` | C header / REST client generator |
| `certo-ui` | Compile `view` / `form` declarations to Htmx HTML |

Add the extracted folder to `PATH`.

### Option B — build from source

```powershell
git clone https://github.com/you/certo
cd certo
.\scripts\release.ps1          # builds & copies all binaries to dist/
```

See [§17](#17-building-the-compiler-from-source) for details.

---

## 3. Your First Program

Create `hello.cto`:

```
module Hello

fn main(): Unit [io] = println("Hello, world!")
```

Compile and run:

```powershell
certo hello.cto -o hello.exe
.\hello.exe
# Hello, world!
```

Pass `--watch` / `-w` to rebuild automatically whenever the source file changes:

```powershell
certo hello.cto -o hello.exe --watch
```

### Anatomy of a program

- Every file starts with `module <Name>`.
- `fn main(): Unit [io]` is the entry point.  The `[io]` effect annotation
  declares that this function performs I/O.
- `println` is from `Stdlib.Core` and is always in scope.

---

## 4. Language Basics

### Types

| Certo type | C equivalent | Notes |
|------------|-------------|-------|
| `Int` | `int64_t` | 64-bit signed integer |
| `Float` | `double` | 64-bit IEEE 754 |
| `Bool` | `bool` | `true` / `false` |
| `Text` | `char*` | UTF-8 string (heap-allocated) |
| `Unit` | `void` / `int64_t 0` | No meaningful value |
| `T?` | `T*` (nullable) | Option type — `None` is null |
| `List<T>` | length-prefixed heap array | Persistent / copy-on-write |
| `Map<K,V>` | hash map | Key-value store |
| `DateTime` | `struct tm` | Calendar date-time |
| `Decimal` | scaled int64 | Exact decimal arithmetic |
| `Result<T,E>` | `void*` → `certo_result_t*` | Either `Ok(T)` or `Err(E)`; `?` propagates errors |

### Functions

```
fn add(a: Int, b: Int): Int = a + b

fn greet(name: Text): Text = "Hello, " ++ name ++ "!"
```

Single-expression functions use `=`.  Block functions use `{ }`:

```
fn describe(n: Int): Unit [io] = {
    println("The number is: " ++ intToText(n))
}
```

### `pub` visibility

```
pub fn square(n: Int): Int = n * n   // exported from the module
fn helper(): Int = 42                // private
```

### If / else

```
fn sign(n: Int): Text =
    if n > 0 then "positive"
    else if n < 0 then "negative"
    else "zero"
```

### Val / var bindings

`let` is an alias for `val` — both declare an immutable binding:

```
fn hypotenuse(a: Float, b: Float): Float = {
    let a2 = a * a
    val b2 = b * b   // val and let are interchangeable
    sqrt(a2 + b2)
}
```

Use `val`/`let` for immutable bindings and `var` for mutable ones:

```
fn countDown(n: Int): Unit = {
    var i = n
    while i > 0 {
        println(intToText(i))
        i = i - 1
    }
}
```

### Operators

| Operator | Meaning |
|----------|---------|
| `+` `-` `*` `/` `%` | Arithmetic (Int or Float) |
| `**` | Integer exponentiation |
| `++` | Text concatenation |
| `==` `!=` `<` `<=` `>` `>=` | Comparison |
| `and` `or` `not` / `!` | Boolean logic |
| `??` | Null coalesce — `a ?? b` returns `a` if non-null, else `b` |
| `?` | Result propagation — `expr?` unwraps `Ok` or returns `Err` early |
| `\|>` | Pipe — `x \|> f` is `f(x)` |
| `..` `...` | Range (inclusive / exclusive) |

### Effects

Effects are declared in square brackets on function signatures.
The compiler tracks them and prevents calling `[io]` functions from
pure contexts.

```
fn readName(): Text [io] = readLine() ?? "anonymous"
```

Common effects: `[io]`, `[db]`, `[fallible]`.

### Loops and iteration

Certo has `for` loops over lists and `while` loops:

```
fn printAll(items: List<Text>): Unit = {
    for item in items {
        println(item)
    }
}

fn countdown(n: Int): Unit = {
    var i = n
    while i > 0 {
        println(intToText(i))
        i = i - 1
    }
}
```

Higher-order functions are also available for functional style:

```
fn sumList(xs: List<Int>): Int =
    List.fold(xs, 0, (acc, x) => acc + x)
```

### Lambdas

Lambdas can be passed inline as `(params) => body` or written with the `fn` expression form:

```
fn double(xs: List<Int>): List<Int> =
    List.map(xs, (x) => x * 2)

fn double(xs: List<Int>): List<Int> =
    List.map(xs, fn(x: Int): Int = x * 2)
```

**Trailing lambda syntax** — when the last argument is a lambda, it can be
written outside the parentheses (or the parentheses omitted entirely for
single-argument calls):

```
// equivalent to List.map(xs, (x) => x * 2)
List.map(xs) { x => x * 2 }

// two-parameter lambda in trailing position
List.fold(xs, 0) { acc, x => acc + x }

// single-argument call — no parens needed
List.forEach(xs) { x => println(x) }
```

### Error handling with Result and ?

Functions that can fail return `Result<T, E>`. Use `Ok` and `Err` to construct
results, and `?` to propagate errors early:

```
fn divide(a: Int, b: Int): Result<Int, Text> =
    if b == 0 then Err("division by zero")
    else Ok(a / b)

fn compute(a: Int, b: Int): Result<Int, Text> = {
    val x = divide(a, b)?   // returns Err early if divide fails
    Ok(x * 2)
}
```

### Guard clauses

`guard` validates a condition and returns early if it fails — useful for
precondition checks at the top of a function:

```
fn processRefund(amount: Int, max: Int): Result<Int, Text> = {
    guard amount > 0 else Err("amount must be positive")
    guard amount <= max else Err("amount exceeds maximum")
    Ok(amount)
}
```

---

## 5. Modules

Each `.cto` file declares exactly one module at the top:

```
module MyApp.Utils
```

Nested module names use `.` as a separator.  The module name is
independent of the file path — it is purely a namespace.

### Importing

```
module MyApp

import Stdlib.Text
import MyApp.Utils
```

All names from `Stdlib.Core` are available automatically without an
explicit import.

---

## 6. Standard Library

All standard library functions are in scope once imported.
`Stdlib.Core` is always available without an import.

### Stdlib.Core

```
// Output
println(s: Text): Unit [io]
print(s: Text): Unit [io]
eprintln(s: Text): Unit [io]

// Conversions
intToText(n: Int): Text
floatToText(f: Float): Text
boolToText(b: Bool): Text
floatToInt(f: Float): Int
intToFloat(n: Int): Float
parseInt(s: Text): Int?
parseFloat(s: Text): Float?

// Arithmetic helpers
pow(base: Int, exp: Int): Int
absInt(n: Int): Int          absFloat(f: Float): Float
minInt(a: Int, b: Int): Int  maxInt(a: Int, b: Int): Int
floor(f: Float): Float       ceil(f: Float): Float
round(f: Float): Float       sqrt(f: Float): Float

// Ranges
range(start: Int, end: Int): List<Int>         // [start, end)
rangeInclusive(start: Int, end: Int): List<Int>

// I/O
readLine(): Text? [io]
readAll(): Text [io]
argCount(): Int
arg(i: Int): Text?

// Assertions
assert(cond: Bool, msg: Text): Unit
```

### Stdlib.Text

```
Text.len(s)           Text.concat(a, b)
Text.contains(s, sub) Text.startsWith(s, prefix)
Text.endsWith(s, suf) Text.toUpper(s) / Text.toLower(s)
Text.trim(s)          Text.trimStart(s) / Text.trimEnd(s)
Text.slice(s, i, j)   Text.indexOf(s, sub): Int?
Text.replace(s, from, to)
Text.split(s, sep): List<Text>
Text.join(parts, sep): Text
Text.repeat(s, n): Text
```

### Stdlib.Collections

```
// List
List.empty<T>(): List<T>
List.len(list)         List.get(list, i): T?
List.push(list, item)  List.concat(a, b)
List.first(list): T?   List.last(list): T?
List.slice(list, i, j) List.reverse(list)
List.map(list, f)      List.filter(list, pred)
List.fold(list, init, f)
List.contains(list, item): Bool

// Map
Map.empty<K,V>(): Map<K,V>
Map.insert(map, key, value)
Map.get(map, key): V?
Map.contains(map, key): Bool
Map.remove(map, key)
Map.len(map): Int
Map.keys(map): List<K>
Map.values(map): List<V>
```

### Stdlib.DateTime

```
DateTime.now(): DateTime [io]
Date.today(): Date [io]
DateTime.fromUnix(secs: Int): DateTime
DateTime.toUnix(dt): Int
DateTime.format(dt, fmt: Text): Text   // strftime format
DateTime.toIso(dt): Text               // ISO 8601
DateTime.parseIso(s): DateTime [fallible]
DateTime.addDays(dt, d) / addHours / addMinutes / addSeconds
DateTime.diffDays(a, b): Int
DateTime.before(a, b): Bool  DateTime.after(a, b): Bool
DateTime.year(dt) / month / day / hour / minute / second
```

### Stdlib.Money

```
Decimal.add(a, b)  Decimal.sub(a, b)
Decimal.mul(a, b)  Decimal.div(a, b) [fallible]
Decimal.round(d, places)
Decimal.toText(d): Text
Money.fromCents(cents: Int): Decimal
Money.toCents(m): Int
```

### Stdlib.Db

```
dbConnect(url: Text): Int [io]          // returns connection handle; 0 = failed
dbClose(conn: Int): Unit [io]
dbError(conn: Int): Text

dbExec(conn, sql, params: List<Text>): Int [io]      // rows affected; -1 on error
dbQuery(conn, sql, params): List<List<Text?>> [io]   // rows × columns; NULL → None
dbQueryTyped<T: DbRow>(conn, sql, params, mapper: fn(List<Text?>): T): List<T> [io]
dbQueryRow(conn, sql, params): List<Text?>? [io]
dbQueryOne(conn, sql): Text? [io]                    // None when no rows or SQL NULL
dbColumns(conn, sql): List<Text> [io]
dbNull(): Text                          // sentinel for SQL NULL parameters
dbStream(conn, sql, params, handler: fn(List<Text?>): Unit): Int [io]  // row count

dbBegin(conn): Int [io]
dbCommit(conn): Int [io]
dbRollback(conn): Int [io]

// Higher-level wrappers — prefer these over manual resource management
withConnection(url: Text, body: fn(Int): Result<T, E>): Result<T, E> [io]
withTransaction(conn: Int, body: fn(): Result<T, E>): Result<T, E> [io]
```

#### `DbRow` — the schema-derived type trait

`dbQueryTyped` is bounded by `trait DbRow`. Only types generated by
`certo db pull` implement `DbRow` automatically (via `impl DbRow for TypeName {}`).
Passing a hand-rolled type without an explicit `impl DbRow for T {}` is a compile
error (E0305). This catches mapper / schema mismatches at compile time rather than
at runtime.

```
// Generated by `certo db pull`:
type User { id: UUID  name: Text  email: Text? }
impl DbRow for User {}

// At the call site — compiler verifies User implements DbRow:
val users = dbQueryTyped(conn, "SELECT id, name, email FROM users", [], userFromRow)
```

#### `dbStream` — constant-memory streaming for large result sets

`dbQuery` materialises the full result set. For tables too large to fit in memory,
use `dbStream`: it declares a server-side PostgreSQL cursor and delivers rows 100 at
a time via a callback, keeping memory usage constant.

```
// Export a million-row table as CSV without loading it all at once:
dbStream(conn, "SELECT id, name, amount FROM orders ORDER BY id", []) { row ->
    val id     = List.getOrPanic(row, 0) ?? ""
    val name   = List.getOrPanic(row, 1) ?? ""
    val amount = List.getOrPanic(row, 2) ?? "0"
    println(id ++ "," ++ name ++ "," ++ amount)
}
```

`dbStream` returns the total row count, or -1 on error. If `conn` is not already
in a transaction, one is opened automatically and committed after the cursor
closes (PostgreSQL cursors require an active transaction). If called inside
`withTransaction`, the cursor runs within the existing transaction.

Rows are `List<Text?>` — the same nullable cell type as `dbQuery`.

### `defer` — guaranteed cleanup on scope exit

`defer { expr }` schedules `expr` to run when the enclosing **function** returns,
regardless of how it exits: normal return, early `?` propagation, or `guard`.
Multiple defers in one function run in LIFO order (last registered, first executed).

```
fn readAndProcess(): Result<Text, Text> [io] = {
    val conn = dbConnect(DATABASE_URL)
    defer { dbClose(conn) }          // runs on every exit path below

    val rows = dbQuery(conn, "SELECT body FROM messages LIMIT 1", [])?
    val cell = List.getOrPanic(List.getOrPanic(rows, 0), 0) ?? ""
    Ok(cell)
}
```

Even if the `dbQuery` `?` propagates an `Err`, `dbClose(conn)` is called first.

`defer` is function-scoped (Go-style), not block-scoped. A `defer` inside an `if`
or `for` body still runs at **function** exit, not at the end of the block.

#### `withConnection` and `withTransaction` — safe, composable resource management

`withConnection` opens a connection, passes it to the body, and **always closes it**
on exit — whether the body succeeds or fails.

`withTransaction` wraps a body in `BEGIN` / `COMMIT`. If the body returns `Err`
(including via `?` propagation), it issues `ROLLBACK`. When called inside an existing
transaction it automatically uses a savepoint instead, so nesting is safe.

Compose them for full resource safety with no manual cleanup:

```
fn transfer(fromId: Text, toId: Text, amount: Int): Result<Unit, Text> [io] =
    withConnection(DATABASE_URL) { conn =>
        withTransaction(conn) {
            val s = intToText(amount)
            dbExec(conn, "UPDATE accounts SET balance = balance - $1 WHERE id = $2",
                   [s, fromId])?
            dbExec(conn, "UPDATE accounts SET balance = balance + $1 WHERE id = $2",
                   [s, toId])?
            Ok(Unit)
        }
    }
```

If either `dbExec` returns `Err`, `?` short-circuits the block, `withTransaction` rolls
back, `withConnection` closes the connection, and the error reaches the caller — zero
manual resource management.

---

## 7. Testing

Write test blocks in any `.cto` file:

```
module MathTest

fn factorial(n: Int): Int = if n <= 1 then 1 else n * factorial(n - 1)

test "factorial(5) = 120" {
    {
        assert(factorial(5) == 120, "factorial(5)")
    }
}

test "5 is prime" {
    {
        assert(isPrime(5), "isPrime(5)")
    }
}
```

Run all tests:

```powershell
certo test math_test.cto
certo test math_test.cto --timeout=10000   # per-test timeout in ms (default 5000)
```

Output:

```
PASS  factorial(5) = 120
PASS  5 is prime
2 passed, 0 failed
```

`assert(cond, message)` aborts with the message if `cond` is false — the
test runner captures the non-zero exit and marks it as `FAIL`.

### dbTest and property

```
dbTest "user exists after insert" {
    { /* runs with a real DB connection */ }
}
```

`property "name"(params...) { body }` runs `body` against many randomly
generated values for each declared parameter — not just once. Generation is
supported for `Int`, `Float`, `Bool`, and `Text` today:

```certo
property "addition commutes"(x: Int, y: Int) {
    assert(x + y == y + x, "commutativity failed")
}
```

```
$ certo test props.cto
PASS  addition commutes [property] (12ms)
All 1 tests passed (0.01s)
```

100 cases run by default, each against a fresh generated `(x, y)`. When a
case fails, the runner doesn't just report the first random input it found —
it shrinks: it re-runs the property against smaller and simpler candidates
(toward `0`, `false`, and `""`) until it can't shrink any further without the
failure disappearing, then reports that minimal counterexample:

```certo
property "every int is less than 50"(x: Int) {
    assert(x < 50, "counterexample found")
}
```

```
$ certo test props.cto
FAIL  every int is less than 50 [property] (18ms)
  counterexample (case 54 of 100, seed 1234567890):
    x = 50
  reproduce with: CERTO_TEST_SEED=1234567890 CERTO_TEST_CASES=100
```

That's the exact boundary the assertion was checking for — not some
arbitrary large number the generator happened to pick. Set `CERTO_TEST_SEED`
(and `CERTO_TEST_CASES` if you changed the count) to replay a failure
deterministically, e.g. in CI logs.

A `property "name" { body }` with no parameters still runs exactly once,
same as before — there's nothing to generate.

Generation for `List<T>`, records, and sum types isn't implemented yet;
declaring a property parameter of an unsupported type is a build-time error
naming the offending parameter, not a silent no-op.

---

## 8. Linting, Benchmarking, Docs, and the REPL

### `certo lint` — static analysis

`certo lint` runs a dataflow pass over the HIR and reports:

- **L001** unused parameter — parameter never read in the function body
- **L002** unused variable — `val`/`var` declared but never read
- **L003** assigned but never read — variable written but the value discarded before the next write
- **L004** unreachable statement — code after a call to `panic`/`todo`/`unreachable` or after a guard that always fires
- **L005** guard condition is a literal bool — `guard true else ...` can never fire; `guard false else ...` always fires

```powershell
certo lint myapp.cto
```

Names prefixed with `_` suppress all lint warnings for that binding.

### `certo bench` — micro-benchmarks

Declare functions named `bench_*` with no parameters; `certo bench` compiles and
runs each one N times (default 1000) and reports the median time in ns/iter:

```
module MyBench

fn bench_concat(): Text =
    "Hello" ++ ", " ++ "world!"
```

```powershell
certo bench myapp.cto                   # 1000 iterations (default)
certo bench myapp.cto --iterations=5000 # override iteration count
# bench_concat   42 ns/iter
```

### `certo doc` — HTML documentation

Generate HTML documentation from `///` doc comments:

```
module Math

/// Raise `base` to the power of `exp`. Both must be non-negative.
pub fn pow(base: Int, exp: Int): Int = ...
```

```powershell
certo doc mylib.cto           # writes to docs/ next to the source file
certo doc mylib.cto -o out/   # write to a specific directory
```

Each `pub fn` with a `///` comment gets its own HTML page with its signature,
description, and parameter list.

### `certo repl` — interactive session

`certo repl` starts an interactive session. Expressions are evaluated and their
values printed automatically. Declarations (`fn`, `val`, `type`) accumulate in
the session.

```
Certo v1.0.2.260610.52 (c) SyntrA 2026
> 1 + 1
2 : Int
> val name = "Alice"
name
> f"Hello, {name}!"
"Hello, Alice!" : Text
> :help      -- show meta-commands
> :quit
```

---

## 9. Project Layout & certo new

### Scaffolding a new project

```powershell
certo new my-project
certo new my-api --template api
certo new my-lib --template lib
```

Available templates: `cli` (default), `api`, `lib`.

This creates the canonical project structure:

```
my-project/
  certo.toml          project manifest
  src/
    main.cto          entry point
  tests/
    unit/             unit test files
    integration/      integration test files
  db/
    migrations/       numbered SQL migration files
```

### certo.toml manifest

```toml
[project]
name    = "my-project"
version = "0.1.0"
entry   = "src/main.cto"
authors = ["Your Name"]

[build]
target  = "native"   # native | wasm | llvm-ir
opt     = "release"  # debug | release

[database]
url     = "postgres://localhost/mydb"

[dependencies]
# future use — package dependencies go here
```

Run `certo build` (reads `certo.toml`) instead of passing a file directly once
`certo.toml` is present.

---

## 10. Database Migrations

Certo ships a migration runner that applies numbered SQL files tracked in a
`migrations/` directory.

### Commands

```powershell
certo db migrate              # apply all pending migrations
certo db migrate --dry-run    # preview SQL without executing
certo db rollback             # roll back the most recent migration
certo db rollback 3           # roll back the last 3 migrations
certo db status               # show applied vs pending migrations
certo db create add_users_table        # scaffold a new migration file
certo db pull                 # introspect live DB → db/schema.cto
certo db pull -o schema.cto  # write to a specific path
certo db diff schema.cto      # compare type declarations to live DB
```

`certo migrate` is an alias for `certo db` and accepts the same subcommands.

### `certo db pull` — schema introspection

Connects to the live database and writes a Certo type declaration for every
table in the schema:

```powershell
certo db pull                        # writes db/schema.cto
certo db pull -o src/schema.cto      # custom output path
certo db pull --schema myschema      # non-public PostgreSQL schema
```

Requires `psql` on `PATH` and `DATABASE_URL` in the environment or `.env`.

Example — a `users` table with `id uuid PK`, `email text NOT NULL`, `name text NULL`:

```
module DbSchema

// Generated by `certo db pull` — do not edit manually.

type User {
    id: UUID  // PK
    email: Text
    name: Text?
}

fn userFromRow(row: List<Text?>): User = ...
fn userFindAll(conn: Int): List<User> [io] = ...
fn userFindById(conn: Int, id: UUID): User? [io] = ...

impl DbRow for User {}
```

Column names are converted to camelCase; table names to PascalCase.
Nullable columns get `?` appended. Primary key columns are annotated `// PK`.
Each type gets `impl DbRow for TypeName {}` so it can be used with `dbQueryTyped`.

PostgreSQL type mapping:

| PostgreSQL | Certo |
|---|---|
| `integer`, `bigint`, `serial` | `Int` |
| `text`, `varchar`, `char` | `Text` |
| `boolean` | `Bool` |
| `double precision`, `real` | `Float` |
| `numeric`, `decimal`, `money` | `Decimal` |
| `uuid` | `UUID` |
| `timestamp`, `date` | `DateTime` |
| `json`, `jsonb` | `Text` |

### `certo db diff` — detect schema drift

`certo db diff` compares your Certo `type` declarations against the live PostgreSQL
database and reports any discrepancies. Run it in CI or before a deploy to catch
drift before it causes runtime errors.

```powershell
certo db diff schema.cto                    # compare against public schema
certo db diff schema.cto --schema myschema  # non-public schema
```

Requires `psql` on `PATH` and `DATABASE_URL` in the environment or `.env`.
Exits 0 when in sync, 1 when drift is detected (CI-friendly).

Example output when drift is found:

```
schema drift detected:

  MISSING TABLE  orders
  TABLE  users
    MISSING COLUMN  users.email: Text
    TYPE MISMATCH   users.age — code: `Int`, db: `Text`
    NULLABLE DRIFT  users.bio — code: NOT NULL, db: nullable
    EXTRA COLUMN    users.legacy_id: Int?
  EXTRA TABLE     _archive (not declared as a type)

4 issue(s) found, 2 table(s) ok
```

When everything matches:

```
schema in sync — 5 table(s) match the live database
```

`EXTRA TABLE` lines are informational — tables in the live DB with no matching
`type` declaration do not cause a non-zero exit, since system tables and
migration-tracking tables are expected.

### Migration schema validation

`certo build` and `certo check` validate migration declarations against your
`type` declarations automatically. Errors are reported with source locations:

| Code | What it catches |
|---|---|
| E0501 | Column type in migration doesn't match the `type` declaration |
| E0502 | Foreign key targets a table not declared as a `type` |
| E0503 | Duplicate migration name |
| E0504 | Migration has no `down` block |
| E0505 | `CreateTable` for a table with no matching `type` declaration |
| E0506 | Column in migration doesn't exist on the declared type |
| E0507 | `AlterTable` or `DropTable` before the table was created |

Example — column type mismatch:

```
error[E0501]: column `User.age` type mismatch
  --> schema.cto:12:5
   |
12 |     age: text,
   |     ^^^^^^^^^
   |     migration uses `text`, type declaration has `Int`
   |
   = note: update the migration column type to match the `type` declaration
```

### Migration files

`certo migrate create <name>` scaffolds a `.cto` file in `migrations/`:

```
migration "add_users_table" {
    up {
        // TODO: add operations
    }
    down {
        // TODO: add rollback operations
    }
}
```

Files are applied in alphabetical order — prefix names with a sequence number
(e.g. `001_create_users.cto`) to control the order.

### Database connection

Set the `DATABASE_URL` environment variable (or put it in `.env`):

```
DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret
```

---

## 11. Compiling to a Shared Library (DLL)

```powershell
certo math.cto --emit-dll -o math.dll
```

This produces:
- `math.dll` — the shared library
- `math.lib` — Windows import library (link against this from C/C++)

All `pub fn` declarations are exported with `CERTO_EXPORT`
(`__declspec(dllexport)` on Windows, `visibility("default")` elsewhere).

Generate a C header for the exported symbols:

```powershell
certo-ffi --header math.cto -o math.h
```

---

## 12. Code Formatting

Format a source file in-place:

```powershell
certo fmt myfile.cto
```

The formatter is idempotent and enforces the canonical style:
four-space indentation, operators spaced, one blank line between
top-level declarations.

Use `--check` to verify formatting without writing (useful in CI):

```powershell
certo fmt --check myfile.cto   # exits 1 if the file would be reformatted
```

---

## 13. LLVM IR & WebAssembly

### LLVM IR

```powershell
certo-llvm math.cto -o math.ll
certo-llvm math.cto -o math.ll --annotate    # adds type comments
```

You can then compile the IR with:

```powershell
clang math.ll -o math.exe
```

### WebAssembly

```powershell
certo-wasm math.cto --emit-ir -o math.wasm.ll
```

Emits LLVM IR with the `wasm32-unknown-unknown` target triple.
Compile to `.wasm` with:

```bash
clang --target=wasm32 --no-standard-libraries -Wl,--export-all \
      -Wl,--no-entry -o math.wasm math.wasm.ll
```

---

## 14. FFI — C Headers & REST Clients

### C header from a Certo module

```powershell
certo-ffi --header math.cto -o math.h
```

Generates a C header with `extern` declarations for all `pub fn` symbols.
Include it from C to call into a compiled Certo DLL.

### REST client from an OpenAPI schema

```powershell
certo-ffi --rest-client api_schema.json -o TaskApi.cto
```

Reads a JSON schema (`api_schema.json`) and emits a Certo source file
with typed function stubs for every endpoint.

---

## 15. UI Views & Forms

`view` declarations describe server-rendered UI pages.
`form` declarations describe HTML forms bound to a module endpoint.

```
module TasksUI

view Dashboard {
    layout = VStack(children: [
        Heading("Task Dashboard"),
        Button("New Task", "/tasks/new"),
    ])
}

form NewTask -> Tasks {
    title: Text
    dueDate: DateTime
    onSubmit: submitTask
}
```

Compile to Htmx HTML:

```powershell
certo-ui views.cto -o out\
```

Produces one `.html` file per `view` declaration in `out/`.

---

## 16. Language Server (LSP)

`certo-lsp` implements the Language Server Protocol.

### VS Code

Add to `.vscode/settings.json`:

```json
{
  "certo.languageServerPath": "C:/path/to/dist/certo-lsp.exe"
}
```

### Neovim (nvim-lspconfig)

Copy `editors/neovim/certo.lua` from the repo into your Neovim config and
`require("certo")` from `init.lua`. It registers `cto_lsp` for `.cto` files
with go-to-definition, hover, completion, and diagnostics keybindings.

The LSP provides: hover documentation, go-to-definition, diagnostics,
completion (including position-aware local variable completions), and formatting.

---

## 17. Building the Compiler from Source

### Clone and build

```powershell
git clone https://github.com/you/certo
cd certo
cargo build --release --bins
```

Binaries land in `target\release\`.

### One-step build (recommended)

```powershell
.\build.ps1             # release build — compiles Rust + copies binaries + compiles all examples
.\build.ps1 -Debug      # debug build (faster compile, larger binaries)
.\build.ps1 -NoExamples # skip example compilation
```

`build.ps1` in the repo root does everything in one shot: `cargo build`, copies
the 8 binaries to `dist\`, and compiles all example `.cto` files to verify the
build is healthy.

### Copy to dist/ only

```powershell
.\scripts\release.ps1
```

This runs `cargo build --release --bins` and copies the 8 executables
to `dist\`.  See [§18](#18-release--distribution-scripts).

### Running tests

```powershell
cargo test --workspace
```

### Workspace layout

```
crates/
  ast/         AST data types
  lexer/       Tokeniser (Logos)
  parser/      Recursive-descent parser
  resolve/     Name resolution
  typeck/      Type inference (Algorithm W / HM)
  traits/      Trait system
  effects/     Effect type checker
  dbschema/    DB schema validation
  hir/         High-level IR + lowering
  mir/         Mid-level IR
  codegen/     C code generator
  stdlib/      Standard library (C + Certo declarations)
  testrunner/  certo-test harness
  llvm/        LLVM IR emitter
  wasm/        WebAssembly target
  ffi/         C header + REST client generator
  fmt/         Code formatter
  lsp/         Language server
  ui/          View/form → Htmx HTML compiler
  cli/         certo main binary
dist/          Pre-built release binaries
docs/          This guide and the language specification
examples/      Sample .cto files
scripts/       Build and release automation
```

---

## 18. Release & Distribution Scripts

All scripts live in `scripts\` and are run from the **repo root**.

### `scripts\release.ps1` — build & stage

```powershell
.\scripts\release.ps1
```

Runs `cargo build --release --bins`, then copies all 8 binaries to
`dist\`.  Pass `-verbose` to see each copy operation.

### `scripts\dist-package.ps1` — create a release archive

```powershell
.\scripts\dist-package.ps1 -version "0.1.0"
```

Creates `dist\certo-0.1.0-windows-x64.zip` (Windows) or
`certo-0.1.0-linux-x64.tar.gz` (Linux/macOS) containing:

- All 8 executables from `dist\`
- `docs\GUIDE.md`
- `examples\` (source files only, no `out\`)
- `README.md` (if present)

Pass `-out <dir>` to change the output directory.

---

## 19. Compiler Pipeline Reference

```
Source (.cto)
    │
    ▼  crates/lexer      Tokenise
    │
    ▼  crates/parser     Parse → AST
    │
    ▼  crates/resolve    Name resolution
    │
    ▼  crates/typeck     Type inference (HM / Algorithm W)
    │
    ▼  crates/traits     Trait conformance checking
    │
    ▼  crates/effects    Effect tracking
    │
    ▼  crates/hir        Lower AST → HIR
    │
    ▼  crates/mir        Lower HIR → MIR (3-address code)
    │
    ▼  crates/codegen    Emit C source
    │
    ▼  clang             Compile C → native binary / DLL
```

The `--emit-c` flag stops the pipeline after codegen and prints the
generated C to stdout:

```powershell
certo math.cto --emit-c
```

This is useful for debugging and for understanding how Certo maps to C.

---

## Quick-reference cheat sheet

```
# Scaffold a new project
certo new my-project

# Compile to executable
certo myapp.cto -o myapp.exe

# Compile and run immediately
certo run myapp.cto

# Compile to DLL + import lib
certo mylib.cto --emit-dll -o mylib.dll

# Type-check only (no compilation)
certo check myapp.cto

# Type-check + fail on lint warnings (unused vars, dead code, ...)
certo check myapp.cto --strict

# Type-check + print full explanations for each diagnostic code
certo check myapp.cto --explain

# Print generated C (no compilation)
certo myapp.cto --emit-c

# Verbose (shows clang invocation)
certo myapp.cto -o myapp.exe -v

# Watch mode — rebuild on file change
certo myapp.cto -o myapp.exe --watch

# Compile, run, and re-run on every file change
certo run myapp.cto --watch

# Interactive REPL
certo repl

# Run unit tests
certo test myapp_test.cto
certo test myapp_test.cto --timeout=10000
certo test myapp_test.cto --filter "addition"   # only tests whose name contains this

# Lint for unused variables and dead code
certo lint myapp.cto

# Run bench_ functions and report ns/iter
certo bench myapp.cto
certo bench myapp.cto --iterations=5000

# Format source file (in place)
certo fmt myfile.cto

# Check formatting without writing (for CI)
certo fmt --check myfile.cto

# Generate HTML documentation from /// comments
certo doc mylib.cto -o docs/

# Database migrations
certo db status
certo db migrate
certo db migrate --dry-run
certo db rollback
certo db create add_users_table

# Generate C header
certo-ffi --header mylib.cto -o mylib.h

# Generate REST client
certo-ffi --rest-client schema.json -o Client.cto

# Emit LLVM IR
certo-llvm mylib.cto -o mylib.ll

# Emit WASM IR
certo-wasm mylib.cto --emit-ir -o mylib.wasm.ll

# Compile UI views to HTML
certo-ui views.cto -o out/

# Build compiler + copy binaries + compile all examples
.\build.ps1
.\build.ps1 -Debug        # debug build
.\build.ps1 -NoExamples   # skip example compilation

# Build and stage binaries only (no examples)
.\scripts\release.ps1

# Package a release archive
.\scripts\dist-package.ps1 -version "0.1.0"
```
