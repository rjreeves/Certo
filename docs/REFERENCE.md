# Certo Language Reference

> This document describes what the Certo compiler currently implements.
> It is derived directly from the compiler source and is accurate as of June 2026.

---

## Table of Contents

1. [Source Files and Modules](#1-source-files-and-modules)
2. [Lexical Structure](#2-lexical-structure)
3. [Types](#3-types)
4. [Declarations](#4-declarations)
5. [Expressions](#5-expressions)
6. [Pattern Matching](#6-pattern-matching)
7. [Error Handling](#7-error-handling)
8. [Concurrency](#8-concurrency)
9. [State Machines](#9-state-machines)
10. [Validators](#10-validators)
11. [Migrations](#11-migrations)
12. [UI — Views and Forms](#12-ui--views-and-forms)
13. [Standard Library](#13-standard-library)
14. [CLI Reference](#14-cli-reference)
15. [Database Schema Tools](#15-database-schema-tools)

---

## 1. Source Files and Modules

Every `.cto` file begins with a module declaration, which must be the first non-comment line:

```
module MyApp
module MyApp.Orders
module Stdlib.Collections
```

Module names use `PascalCase`. Dots separate nested namespaces. The file name does not need to match the module name.

### Importing

```
import Stdlib.Text
import Stdlib.Collections
import Stdlib.Http
```

Imports make the named module's exported functions available in the current file. There is no selective import syntax (`{List, Map}` style) — import the whole module.

### Visibility

```
pub fn exported(): Int = 42    // visible to importers
fn internal(): Int = 0         // module-private
```

The `pub` modifier applies to `fn`, `type`, `val`, and `var` declarations.

---

## 2. Lexical Structure

### Comments

```
// Single-line comment
/* Block comment — not nested */
\ Line comment (backslash form, equivalent to //)
```

### Keywords

```
fn       type     val      let      var      pub
if       then     else     match    for      in
while    return   guard    defer    unsafe
async    await    parallel spawn
statemachine  validator  migration  view  form
trait    impl     import   module
true     false
```

`let` and `val` are identical — both declare an immutable binding.

### Literals

| Literal | Syntax | Examples |
|---------|--------|---------|
| Integer | Decimal digits | `42`, `0`, `-7` |
| Float | Digit `.` digit | `3.14`, `1.0`, `-2.5` |
| Bool | `true` / `false` | `true`, `false` |
| Text | Double-quoted | `"hello"`, `"line\n"` |
| UUID | `uuid"..."` | `uuid"550e8400-e29b-41d4-a716-446655440000"` |
| Unit | `()` | `()` |
| List | `[a, b, c]` | `[1, 2, 3]`, `[]` |
| Tuple | `(a, b)` | `(1, "x")`, `(true, 0, "y")` |

### Operators

| Operator | Meaning | Notes |
|----------|---------|-------|
| `+` `-` `*` `/` `%` | Arithmetic | Int or Float |
| `**` | Exponentiation | Int only |
| `++` | Text concatenation | `"a" ++ "b"` → `"ab"` |
| `==` `!=` `<` `<=` `>` `>=` | Comparison | |
| `and` `or` `not` | Boolean logic | Short-circuit |
| `!` | Boolean not (prefix) | Alias for `not` |
| `??` | Null coalesce | `a ?? b` — returns `a` if non-null, else `b` |
| `?` | Error propagation | Unwraps `Ok`, returns `Err` early |
| `\|>` | Pipe | `x \|> f` is `f(x)` |

Precedence (lowest to highest): `\|>`, `or`, `and`, `not`, `== != < > <= >=`, `+ -`, `* / %`, `**`, prefix `-` / `not`, `.` field access, `()` call.

---

## 3. Types

### Primitive Types

| Type | Description | C representation |
|------|-------------|-----------------|
| `Int` | 64-bit signed integer | `int64_t` |
| `Int8` | 8-bit signed integer | `int8_t` |
| `Int16` | 16-bit signed integer | `int16_t` |
| `Int32` | 32-bit signed integer | `int32_t` |
| `UInt` | 64-bit unsigned integer | `uint64_t` |
| `Float` | 64-bit IEEE 754 double | `double` |
| `Decimal` | Exact scaled decimal (no rounding errors) | `certo_decimal_t` |
| `Bool` | `true` or `false` | `bool` |
| `Text` | UTF-8 string | `const char*` |
| `UUID` | 128-bit UUID | `certo_uuid_t` |
| `Unit` | No meaningful value | `int64_t` (always 0) |

There are no implicit coercions between numeric types. Use `intToFloat`, `floatToInt`, etc.

### Compound Types

```
Option<T>          // present or absent — None is the absent value
Result<T, E>       // Ok(T) or Err(E)
List<T>            // ordered sequence
Map<K, V>          // key-value store
(T1, T2, ...)      // tuple
{ field: T, ... }  // anonymous record
T => U             // single-parameter function type
fn(T1, T2): U     // multi-parameter function type
```

### Record Types (Product Types)

```
type Address = {
    street:  Text,
    city:    Text,
    country: Text,
    zip:     Text
}

type Invoice = {
    id:        UUID,
    amount:    Decimal,
    paidAt:    Int?,    // optional field (Int? = Option<Int>)
    notes:     Text?
}
```

Optional fields are written `Type?` (equivalent to `Option<Type>`).

Record types support computed fields, which are not stored and are recalculated on each access:

```
type Invoice = {
    amount:  Decimal,
    paidAt:  Int?,
    computed isPaid: Bool = paidAt != ()
}
```

### Sum Types (Enums)

```
type Shape =
    | Circle(radius: Float)
    | Rectangle(width: Float, height: Float)
    | Triangle(base: Float, height: Float)

type Color =
    | Red
    | Green
    | Blue
```

Unit variants (no fields) are constants. Named-field variants are constructor functions.

### Type Aliases

```
type UserId   = UUID
type Callback = Int => Unit
```

### Generics

Functions and types may be generic:

```
fn identity<T>(x: T): T = x

type Pair<A, B> = { first: A, second: B }
```

Type arguments are inferred at call sites and do not need to be written explicitly.

---

## 4. Declarations

### Functions

```
// Single-expression form
fn add(a: Int, b: Int): Int = a + b

// Block form
fn greet(name: Text): Text = {
    val msg = "Hello, " ++ name ++ "!"
    msg
}

// Public export
pub fn square(n: Int): Int = n * n

// Generic
fn first<T>(list: List<T>): T? = List.first(list)

// Async
async fn fetch(url: Text): Text = {
    val r = Http.get(url)
    HttpResponse.body(r)
}

// With effects annotation (informational — not enforced beyond tracking)
fn readName(): Text [io] = readLine() ?? "anonymous"
```

The return type annotation is required when the compiler cannot infer it (e.g. recursive functions). For non-recursive functions it is optional.

### Val and Var Bindings

Module-level bindings:

```
val PI: Float = 3.14159
val greeting = "Hello"    // type inferred

var counter: Int = 0      // mutable
```

Block-level bindings follow the same syntax and are used inside function bodies.

### Traits and Implementations

```
trait Printable {
    fn display(self): Text
}

impl Printable for Color {
    fn display(self): Text =
        match self {
            Red   => "red"
            Green => "green"
            Blue  => "blue"
        }
}
```

Traits can have multiple methods. Implementations must provide a body for each method declared in the trait.

---

## 5. Expressions

### Block Expressions

Blocks are expressions. The value of a block is its last expression:

```
fn max(a: Int, b: Int): Int = {
    val diff = a - b
    if diff > 0 then a else b
}
```

### If / Else

`if` is an expression. Both branches must be present and must have compatible types:

```
val label = if score >= 50 then "pass" else "fail"

fn classify(n: Int): Text =
    if n > 0 then "positive"
    else if n < 0 then "negative"
    else "zero"
```

### Let / Val / Var Bindings (in blocks)

```
fn example(): Int = {
    val x = 10          // immutable
    let y = 20          // same as val
    var z = 0           // mutable
    z = x + y
    z
}
```

### For Loops

```
fn printAll(items: List<Text>): Unit = {
    for item in items {
        println(item)
    }
}
```

The loop body is executed once per element. `for` always iterates over a `List<T>`.

### While Loops

```
fn countdown(n: Int): Unit = {
    var i = n
    while i > 0 {
        println(intToText(i))
        i = i - 1
    }
}
```

### Guard Clauses

`guard` evaluates a condition and exits the current block early if it fails. The else branch must exit (return a value compatible with the function's return type):

```
fn processAmount(n: Int): Result<Int, Text> = {
    guard n > 0 else Err("must be positive")
    guard n < 1000 else Err("too large")
    Ok(n * 2)
}
```

### Defer

`defer` schedules an expression to run when the enclosing block exits, in LIFO order relative to other defers in the same block:

```
fn withResource(): Unit = {
    val r = open()
    defer close(r)
    use(r)
    // close(r) is called here, even if an early return occurred
}
```

### Pipeline

`|>` pipes the left value as the first argument to the right function:

```
val result =
    items
    |> List.filter((x) => x > 0)
    |> List.map((x) => x * 2)
    |> List.fold(0, (acc, x) => acc + x)
```

### Lambda Expressions

```
val double = (x: Int) => x * 2

List.map(xs, (x) => x * 2)

// Multi-parameter
List.fold(xs, 0, (acc, x) => acc + x)

// Trailing lambda — when the last argument is a lambda,
// it may be written outside the parentheses:
List.map(xs) { x => x * 2 }
List.fold(xs, 0) { acc, x => acc + x }
List.forEach(xs) { x => println(x) }
```

### Record Construction and Field Access

```
val addr = Address { street: "1 Main St", city: "Springfield", country: "US", zip: "12345" }

val city = addr.city

// Record update (creates a new record, original unchanged)
val updated = { addr with city: "Shelbyville" }
```

### Unsafe Blocks

Bypasses certain compiler restrictions. Marked explicitly so they are easy to audit:

```
unsafe {
    rawOperation()
}
```

---

## 6. Pattern Matching

`match` is exhaustive — the compiler reports an error if any variant is unhandled.

### Sum Type Matching

```
fn area(shape: Shape): Float =
    match shape {
        Circle(r)       => Math.pi() * r * r
        Rectangle(w, h) => w * h
        Triangle(b, h)  => 0.5 * b * h
    }
```

### Wildcard and Bind

```
fn describe(x: Int): Text =
    match x {
        0 => "zero"
        n => "nonzero: " ++ intToText(n)
    }
```

### Guards

```
fn classify(score: Int): Text =
    match score {
        s if s >= 90 => "A"
        s if s >= 70 => "B"
        s if s >= 50 => "C"
        _            => "F"
    }
```

### Option and Result Patterns

```
match List.find(xs, pred) {
    Some(v) => process(v)
    None    => handleMissing()
}

match divide(a, b) {
    Ok(result) => println(intToText(result))
    Err(msg)   => eprintln("Error: " ++ msg)
}
```

### Nested Patterns

```
match result {
    Ok(Some(user)) => greet(user)
    Ok(None)       => println("not found")
    Err(e)         => eprintln(e)
}
```

---

## 7. Error Handling

### Result Type

All fallible operations return `Result<T, E>`. There are no exceptions.

```
fn divide(a: Int, b: Int): Result<Int, Text> =
    if b == 0 then Err("division by zero")
    else Ok(a / b)
```

### Error Propagation with `?`

The `?` operator unwraps `Ok`, or returns the `Err` early from the enclosing function:

```
fn compute(a: Int, b: Int, c: Int): Result<Int, Text> = {
    val x = divide(a, b)?
    val y = divide(x, c)?
    Ok(y + 1)
}
```

The enclosing function's return type must be `Result<_, E>` where `E` is compatible with the error being propagated.

### Defining Error Types

Error types are ordinary sum types:

```
type OrderError =
    | NotFound(id: UUID)
    | AlreadyProcessed
    | InsufficientFunds(available: Decimal, required: Decimal)
```

---

## 8. Concurrency

### spawn and await

`spawn` runs a function call on a new OS thread and returns an opaque task handle. `await` joins the thread and returns its result.

```
fn fetchUser(id: Int): Text = Http.get("http://api/users/" ++ intToText(id)) |> HttpResponse.body

fn example(): Text = {
    val task = spawn fetchUser(42)
    val result = await task
    result
}
```

`spawn` only works when the spawned expression is a direct function call. The thread is created via `pthread_create` on POSIX; on Windows, `CreateThread` is used. Link with `-lpthread` on Linux/macOS.

### parallel {}

`parallel { expr1, expr2, ... }` spawns all expressions concurrently and awaits all of them, returning a tuple of their results. The expressions are started in order; results arrive in order regardless of completion order.

```
fn fetchBoth(userId: Int, orderId: Int): (Text, Text) = {
    val results = parallel {
        Http.get("http://api/users/" ++ intToText(userId)) |> HttpResponse.body,
        Http.get("http://api/orders/" ++ intToText(orderId)) |> HttpResponse.body
    }
    results
}
```

Each branch must be a direct function call. Branches that capture variables from the enclosing scope pass them as `int64_t`-sized arguments to the thread worker — struct-by-value types (such as `Decimal`) are not supported across spawn boundaries in the current implementation.

---

## 9. State Machines

`statemachine` declares a business entity whose lifecycle is enforced at compile time. Each state is a distinct phantom type, so transition functions that require a specific source state are rejected by the type checker if called on a value in the wrong state.

### Declaration Syntax

```
statemachine Order {
    states: Draft, Submitted, Fulfilled, Cancelled

    transitions:
        Draft -> Submitted  : submit(note: Text)
        Submitted -> Fulfilled : fulfil(shippedAt: Int)
        [Draft, Submitted] -> Cancelled : cancel(reason: Text)
}
```

`[S1, S2] -> T : event` is a multi-from transition — the same event applies from more than one source state.

### Generated API

For a machine named `Order` with states `Draft`, `Submitted`, `Fulfilled`, `Cancelled`:

| Function | Signature | Description |
|----------|-----------|-------------|
| `Order_new` | `(): Order<Draft>` | Constructor — returns first state |
| `Order_submit` | `(Order<Draft>, Text): Order<Submitted>` | Transition function |
| `Order_fulfil` | `(Order<Submitted>, Int): Order<Fulfilled>` | Transition function |
| `Order_cancel` | `(Order<Draft\|Submitted>, Text): Order<Cancelled>` | Multi-from transition |
| `Order_assertDraft` | `(Order<S>): Order<Draft>?` | Downcast from unknown state |
| `Order_assertSubmitted` | `(Order<S>): Order<Submitted>?` | Downcast from unknown state |
| `Order_isDraft` | `(Order<S>): Bool` | State predicate |
| `Order_isSubmitted` | `(Order<S>): Bool` | State predicate |
| `Order_state` | `(Order<S>): OrderState` | Return current state as enum |

The `assertX` functions are for values loaded from a database where the state is not known at compile time. They return `None` if the value is not in that state.

### Compile-time enforcement

```
val draft = Order_new()
val submitted = Order_submit(draft, "ready")

// This is a compile-time type error:
val bad = Order_fulfil(draft, 12345)   // Error: expected Order<Submitted>, got Order<Draft>
```

---

## 10. Validators

Validators express business rules against an entity. They compile to both a runtime validation function (for use in application code) and optionally to a PostgreSQL trigger (for database-level enforcement).

### Declaration Syntax

```
validator PriceValidator for Product errors PriceError {
    rule nonNegative {
        require product.price >= Decimal.fromInt(0)
        else PriceError.NegativePrice(product.price)
    }

    rule reasonablePrice {
        after nonNegative
        require product.price <= Decimal.fromInt(10000)
        else PriceError.PriceTooHigh(product.price)
    }
}
```

### Context Fields

Additional data needed for validation that must be fetched separately:

```
validator OrderValidator for Order errors OrderError {
    context {
        customer: Customer loaded by db.customers.find(order.customerId)
    }

    rule creditCheck {
        require customer.creditLimit >= order.total
        else OrderError.InsufficientCredit(customer.creditLimit, order.total)
    }
}
```

Context fields marked `loaded by expr` are fetched automatically when the validator is invoked through the `validateWithDb` path (see generated API below).

### Triggers

Validators can generate PostgreSQL trigger SQL:

```
validator StockValidator for LineItem errors StockError {
    trigger on insert
    trigger on update when status != "cancelled"

    rule positiveQty {
        require lineItem.quantity > 0
        else StockError.ZeroQuantity
    }
}
```

`trigger on insert` and `trigger on update` cause `certo build` to emit `CREATE TRIGGER` SQL alongside the normal C output.

### Rule Ordering

- `after ruleName` — this rule runs only after `ruleName` has passed.
- Rules with no `after` dependency run first (in declaration order among equals).
- The compiler topologically sorts rules and reports cycles as errors.

### Generated API

For a validator named `V` validating type `T` with error type `E`:

```
V_validate(entity: T, context: V_context): List<E>
V_validateWithDb(conn: Int, entity: T): List<E>  // only when context has loaded_by fields
V_isValid(entity: T, context: V_context): Bool
```

---

## 11. Migrations

Migrations describe schema changes in Certo source rather than raw SQL. The compiler validates them against your `type` declarations.

### Syntax

```
migration "create orders table" {
    up {
        createTable orders {
            id:          UUID        primary key
            customerId:  UUID
            total:       Decimal
            status:      Text
            createdAt:   Int
            notes:       Text?
        }
        createIndex idx_orders_customer on orders (customerId)
    }
    down {
        dropTable orders
    }
}
```

### Operations

| Operation | Syntax |
|-----------|--------|
| Create table | `createTable name { columns... }` |
| Drop table | `dropTable name` |
| Alter table | `alterTable name { addColumn ..., dropColumn name }` |
| Create index | `createIndex name on table (col1, col2)` |
| Drop index | `dropIndex name` |
| Raw SQL | `rawSql "SELECT 1"` |

### Column Modifiers

```
id:         UUID      primary key
email:      Text      unique
parentId:   UUID?                    // nullable
score:      Int       default 0
userId:     UUID      references users(id) on delete cascade
```

Foreign key actions: `cascade`, `setNull`, `restrict`, `noAction`.

### CLI

```sh
certo migrate apply          # apply all pending migrations
certo migrate rollback       # roll back the most recent migration
certo migrate status         # list applied / pending migrations
certo migrate generate       # scaffold a new migration file
```

---

## 12. UI — Views and Forms

### Views

A `view` declares a read-only page layout:

```
view UserList {
    layout = VStack([
        Heading("Users"),
        Text("All registered users")
    ])
}
```

### Forms

A `form` declares an HTML form that submits to a database table:

```
form CreateUser for users {
    fields {
        name        label: "Full Name" placeholder: "Alice Smith"
        email       label: "Email Address"
        bio         label: "Bio" rows: 4
    }
}
```

`for users` names the target database table. If `pk` is set, the form generates an `UPDATE` statement rather than `INSERT`.

### Layout Primitives

`VStack`, `HStack`, `Text`, `Heading`, `Button`, `Link`, `Image`, `Input`, `For`, `If`.

Unknown layout calls emit an HTML comment in the output rather than failing the build.

### Output

`certo-ui` compiles `view` and `form` declarations to a single `server.cto` file (the default mode). The legacy per-page HTML mode is available via `certo-ui --html`.

---

## 13. Standard Library

All stdlib modules must be explicitly imported. `Stdlib.Core` functions (`println`, `intToText`, etc.) are always in scope without an import.

### Stdlib.Core — always in scope

```
print(s: Text): Unit [io]
println(s: Text): Unit [io]
eprint(s: Text): Unit [io]
eprintln(s: Text): Unit [io]

intToText(n: Int): Text
floatToText(f: Float): Text
boolToText(b: Bool): Text
floatToInt(f: Float): Int
intToFloat(n: Int): Float
parseInt(s: Text): Int?
parseFloat(s: Text): Float?

assert(cond: Bool, msg: Text): Unit

pow(base: Int, exp: Int): Int
absInt(n: Int): Int
absFloat(f: Float): Float
minInt(a: Int, b: Int): Int
maxInt(a: Int, b: Int): Int
minFloat(a: Float, b: Float): Float
maxFloat(a: Float, b: Float): Float
floor(f: Float): Float
ceil(f: Float): Float
round(f: Float): Float
sqrt(f: Float): Float

range(start: Int, end: Int): List<Int>          // [start, end)
rangeInclusive(start: Int, end: Int): List<Int> // [start, end]

readLine(): Text? [io]
readAll(): Text [io]
argCount(): Int
arg(i: Int): Text?
```

### Stdlib.Text

```
import Stdlib.Text

Text.len(s: Text): Int
Text.concat(a: Text, b: Text): Text
Text.eq(a: Text, b: Text): Bool
Text.contains(s: Text, sub: Text): Bool
Text.startsWith(s: Text, prefix: Text): Bool
Text.endsWith(s: Text, suffix: Text): Bool
Text.toUpper(s: Text): Text
Text.toLower(s: Text): Text
Text.trim(s: Text): Text
Text.trimStart(s: Text): Text
Text.trimEnd(s: Text): Text
Text.slice(s: Text, start: Int, end: Int): Text
Text.indexOf(s: Text, sub: Text): Int?
Text.replace(s: Text, from: Text, to: Text): Text
Text.split(s: Text, sep: Text): List<Text>
Text.join(parts: List<Text>, sep: Text): Text
Text.repeat(s: Text, n: Int): Text
```

### Stdlib.Collections

```
import Stdlib.Collections

// List<T>
List.empty<T>(): List<T>
List.len<T>(list: List<T>): Int
List.get<T>(list: List<T>, i: Int): T?
List.getOrPanic<T>(list: List<T>, i: Int): T
List.push<T>(list: List<T>, item: T): List<T>
List.concat<T>(a: List<T>, b: List<T>): List<T>
List.first<T>(list: List<T>): T?
List.last<T>(list: List<T>): T?
List.slice<T>(list: List<T>, start: Int, end: Int): List<T>
List.reverse<T>(list: List<T>): List<T>
List.map<A, B>(list: List<A>, f: A => B): List<B>
List.filter<T>(list: List<T>, pred: T => Bool): List<T>
List.fold<T, A>(list: List<T>, init: A, f: (A, T) => A): A
List.contains<T>(list: List<T>, item: T): Bool
List.find<T>(list: List<T>, pred: T => Bool): T?
List.any<T>(list: List<T>, pred: T => Bool): Bool
List.all<T>(list: List<T>, pred: T => Bool): Bool
List.sort<T>(list: List<T>, cmp: (T, T) => Int): List<T>
List.zip<A, B>(a: List<A>, b: List<B>): List<(A, B)>

// Map<K, V>
Map.empty<K, V>(): Map<K, V>
Map.insert<K, V>(map: Map<K, V>, key: K, value: V): Map<K, V>
Map.get<K, V>(map: Map<K, V>, key: K): V?
Map.contains<K, V>(map: Map<K, V>, key: K): Bool
Map.remove<K, V>(map: Map<K, V>, key: K): Map<K, V>
Map.len<K, V>(map: Map<K, V>): Int
Map.keys<K, V>(map: Map<K, V>): List<K>
Map.values<K, V>(map: Map<K, V>): List<V>
Map.fromList<K, V>(pairs: List<(K, V)>): Map<K, V>
```

### Stdlib.DateTime

```
import Stdlib.DateTime

// DateTime = Int (Unix seconds); Date = Int (Unix seconds at midnight UTC)
DateTime.now(): DateTime [io]
Date.today(): Date [io]
DateTime.fromUnix(secs: Int): DateTime
DateTime.toUnix(dt: DateTime): Int
DateTime.format(dt: DateTime, fmt: Text): Text   // strftime format
Date.format(d: Date, fmt: Text): Text
DateTime.toIso(dt: DateTime): Text               // ISO 8601
DateTime.parseIso(s: Text): DateTime [fallible]

DateTime.addSeconds(dt: DateTime, s: Int): DateTime
DateTime.addMinutes(dt: DateTime, m: Int): DateTime
DateTime.addHours(dt: DateTime, h: Int): DateTime
DateTime.addDays(dt: DateTime, d: Int): DateTime
DateTime.diffSeconds(a: DateTime, b: DateTime): Int
DateTime.diffDays(a: DateTime, b: DateTime): Int

DateTime.before(a: DateTime, b: DateTime): Bool
DateTime.after(a: DateTime, b: DateTime): Bool
DateTime.eq(a: DateTime, b: DateTime): Bool

DateTime.year(dt: DateTime): Int
DateTime.month(dt: DateTime): Int   // 1–12
DateTime.day(dt: DateTime): Int     // 1–31
DateTime.hour(dt: DateTime): Int
DateTime.minute(dt: DateTime): Int
DateTime.second(dt: DateTime): Int
```

### Stdlib.Money

```
import Stdlib.Money

// Rounding policy — pass to every rounding operation (no silent defaults)
type RoundingMode =
    | HalfUp        // round half away from zero (1.5 → 2, -1.5 → -2)
    | HalfDown      // round half toward zero    (1.5 → 1, -1.5 → -1)
    | HalfEven      // banker's rounding          (2.5 → 2, 3.5 → 4)
    | Up            // round away from zero (always)
    | Down          // round toward zero (truncate)
    | Ceiling       // round toward +∞
    | Floor         // round toward -∞
    | ToIncrement   // round to a step multiple (requires Decimal.roundToIncrement)

// Exact decimal arithmetic
Decimal.add(a: Decimal, b: Decimal): Decimal
Decimal.sub(a: Decimal, b: Decimal): Decimal
Decimal.mul(a: Decimal, b: Decimal): Decimal
Decimal.eq(a: Decimal, b: Decimal): Bool
Decimal.lt(a: Decimal, b: Decimal): Bool
Decimal.gt(a: Decimal, b: Decimal): Bool
Decimal.lte(a: Decimal, b: Decimal): Bool
Decimal.gte(a: Decimal, b: Decimal): Bool
Decimal.abs(d: Decimal): Decimal
Decimal.negate(d: Decimal): Decimal
Decimal.toInt(d: Decimal): Int
Decimal.fromInt(n: Int): Decimal
Decimal.toText(d: Decimal): Text

// Rounding (explicit mode mandatory)
Decimal.round(d: Decimal, places: Int, mode: RoundingMode): Decimal
Decimal.roundToIncrement(d: Decimal, step: Decimal, mode: RoundingMode): Decimal
Decimal.divRound(a: Decimal, b: Decimal, places: Int, mode: RoundingMode): Decimal

// Money helpers (fixed at 2 decimal places)
Money.fromCents(cents: Int): Decimal
Money.toCents(m: Decimal): Int
Money.fromDecimal(d: Decimal, mode: RoundingMode): Decimal
```

`Decimal.divRound` computes `a / b` and rounds the result to `places` decimal places in a single operation, avoiding intermediate precision loss.

### Stdlib.Http

```
import Stdlib.Http

// Client
Http.get(url: Text): HttpResponse [io]
Http.post(url: Text, body: Text, contentType: Text): HttpResponse [io]
Http.put(url: Text, body: Text, contentType: Text): HttpResponse [io]
Http.delete(url: Text): HttpResponse [io]

// Response accessors
HttpResponse.status(r: HttpResponse): Int
HttpResponse.body(r: HttpResponse): Text
HttpResponse.contentType(r: HttpResponse): Text
HttpResponse.ok(r: HttpResponse): Bool         // true if 2xx

// Server
Http.serve(port: Int, handler: fn(HttpRequest): HttpResponse): Unit [io]
Http.respond(status: Int, body: Text, contentType: Text): HttpResponse
Http.ok(body: Text, contentType: Text): HttpResponse
Http.notFound(body: Text): HttpResponse
Http.badRequest(body: Text): HttpResponse
Http.serverError(body: Text): HttpResponse

// Request accessors
HttpRequest.method(r: HttpRequest): Text
HttpRequest.path(r: HttpRequest): Text
HttpRequest.query(r: HttpRequest): Text    // raw query string
HttpRequest.body(r: HttpRequest): Text
HttpRequest.header(r: HttpRequest, name: Text): Text   // "" if absent
HttpRequest.headers(r: HttpRequest): List<List<Text>>  // [[name, value], ...]
```

### Stdlib.Json

```
import Stdlib.Json

// Construct
Json.parse(text: Text): JsonValue
Json.stringify(value: JsonValue): Text
Json.null(): JsonValue
Json.bool(b: Bool): JsonValue
Json.int(i: Int): JsonValue
Json.float(f: Float): JsonValue
Json.string(s: Text): JsonValue
Json.array(): JsonValue
Json.object(): JsonValue

// Inspect
JsonValue.isNull(v: JsonValue): Bool
JsonValue.isBool(v: JsonValue): Bool
JsonValue.isInt(v: JsonValue): Bool
JsonValue.isFloat(v: JsonValue): Bool
JsonValue.isString(v: JsonValue): Bool
JsonValue.isArray(v: JsonValue): Bool
JsonValue.isObject(v: JsonValue): Bool

// Extract (safe — return zero/empty on type mismatch)
JsonValue.asBool(v: JsonValue): Bool
JsonValue.asInt(v: JsonValue): Int
JsonValue.asFloat(v: JsonValue): Float
JsonValue.asText(v: JsonValue): Text
JsonValue.len(v: JsonValue): Int
JsonValue.get(v: JsonValue, i: Int): JsonValue       // array index
JsonValue.field(v: JsonValue, key: Text): JsonValue  // object field

// Mutate (returns new value — JsonValue is persistent)
JsonValue.push(arr: JsonValue, item: JsonValue): JsonValue
JsonValue.set(obj: JsonValue, key: Text, value: JsonValue): JsonValue
```

### Stdlib.Math

```
import Stdlib.Math

Math.pi(): Float
Math.e(): Float
Math.sin(x: Float): Float
Math.cos(x: Float): Float
Math.tan(x: Float): Float
Math.asin(x: Float): Float
Math.acos(x: Float): Float
Math.atan(x: Float): Float
Math.atan2(y: Float, x: Float): Float
Math.log(x: Float): Float      // natural log
Math.log2(x: Float): Float
Math.log10(x: Float): Float
Math.exp(x: Float): Float
Math.pow(x: Float, y: Float): Float
Math.hypot(a: Float, b: Float): Float
Math.clamp(x: Float, lo: Float, hi: Float): Float
Math.clampInt(x: Int, lo: Int, hi: Int): Int
Math.sign(x: Float): Float
Math.signInt(x: Int): Int
Math.trunc(x: Float): Float
Math.random(): Float            // [0.0, 1.0) — seeded from system time
```

### Stdlib.Crypto

```
import Stdlib.Crypto

Crypto.sha256(s: Text): Text        // lowercase hex, 64 chars
Crypto.md5(s: Text): Text           // lowercase hex, 32 chars
Crypto.base64Encode(s: Text): Text
Crypto.base64Decode(s: Text): Text  // empty string on invalid input
```

### Stdlib.Db

Direct database access via libpq (PostgreSQL). For schema-validated access, use `certo db pull` to generate typed row types and `dbQueryTyped`.

```
import Stdlib.Db

dbConnect(connstr: Text): Int [io]    // returns handle; 0 on failure
dbClose(conn: Int): Unit [io]
dbError(conn: Int): Text              // last error message

dbServerVersion(conn: Int): Int [io]  // e.g. 150004 for PG 15.0.4
dbVersionString(conn: Int): Text [io]

dbNull(): Text                        // SQL NULL sentinel for parameters

// Execute INSERT / UPDATE / DELETE — returns rows affected, -1 on error
dbExec(conn: Int, sql: Text, params: List<Text>): Int [io]

// SELECT — rows as List<List<Text?>>
dbQuery(conn: Int, sql: Text, params: List<Text>): List<List<Text?>> [io]

// SELECT with typed mapper — return type inferred from mapper function
dbQueryTyped<T: DbRow>(conn: Int, sql: Text, params: List<Text>,
                        mapper: fn(List<Text?>): T): List<T> [io]
```

Parameters are positional (`$1`, `$2`, ...) and always passed as `List<Text>`. Use `dbNull()` for SQL NULL values.

---

## 14. CLI Reference

### certo build

```sh
certo build src/main.cto               # compile to a.out / a.exe
certo build src/main.cto -o myapp      # specify output name
certo build src/main.cto --dll         # compile to shared library (.so / .dll)
certo build src/main.cto --watch       # rebuild on file changes
```

Produces a native executable via the C backend (Clang required on PATH).

### certo run

```sh
certo run src/main.cto                 # build and run immediately
certo run src/main.cto -- arg1 arg2   # pass arguments to the program
```

### certo check

```sh
certo check src/main.cto              # type-check without producing output
```

Runs the full pipeline (parse → resolve → type check) and reports errors. Does not invoke Clang.

### certo fmt

```sh
certo fmt src/main.cto                # format in place
certo fmt src/                        # format a directory
```

### certo test

```sh
certo test src/main.cto               # run test { } declarations
```

`test "name" { body }` declarations are collected and executed. A non-zero exit from the body or a failed `assert` fails the test.

### certo lint

```sh
certo lint src/main.cto
```

Runs style checks beyond what type checking catches.

### certo bench

```sh
certo bench src/main.cto
```

Runs `property "name" { body }` declarations as property-based benchmarks.

### certo new

```sh
certo new myapp                       # scaffold a new project
```

Creates a directory with `src/main.cto`, a `.gitignore`, and a basic project structure.

---

## 15. Database Schema Tools

### certo db pull

Introspects a live PostgreSQL database and writes Certo `type` declarations that match the current schema:

```sh
certo db pull                          # writes db/schema.cto
certo db pull -o path/to/output.cto
certo db pull --schema myschema        # default: public
```

Requires `DATABASE_URL` in the environment or a `.env` file. Uses `psql` (must be on PATH).

The output file is ordinary Certo source — import it explicitly if you want those declarations to be part of your build:

```
import db.schema
```

### certo db diff

Compares the `type` declarations in your source against the live database and reports drift:

```sh
certo db diff src/main.cto            # human-readable output
certo db diff src/main.cto --json     # machine-readable JSON
```

Drift categories reported:

| Kind | Meaning |
|------|---------|
| `MISSING_TABLE` | Declared as a `type` in code, absent in the DB |
| `MISSING_COLUMN` | Field declared in code, column absent in DB |
| `TYPE_MISMATCH` | Declared Certo type doesn't match the PostgreSQL column type |
| `NULLABLE_DRIFT` | Optionality (`?`) in code doesn't match `NOT NULL` in DB |
| `EXTRA_COLUMN` | Column present in DB, not declared in code |
| `EXTRA_TABLE` | Table present in DB, no corresponding `type` in code |

Exit code is `0` when in sync, `1` when any drift is found (suitable as a CI gate).

JSON output schema:

```json
{
  "in_sync": false,
  "tables_ok": 3,
  "issues": [
    { "kind": "MISSING_COLUMN", "table": "Order", "column": "notes", "detail": "Text?" },
    { "kind": "TYPE_MISMATCH",  "table": "Invoice", "column": "amount", "detail": "code=Int db=Decimal" }
  ]
}
```

### PostgreSQL → Certo Type Mapping

| PostgreSQL types | Certo type |
|-----------------|-----------|
| `integer`, `int4`, `bigint`, `int8`, `smallint`, `serial`, `bigserial` | `Int` |
| `text`, `varchar`, `char`, `name`, `citext` | `Text` |
| `boolean`, `bool` | `Bool` |
| `real`, `float4`, `double precision`, `float8` | `Float` |
| `numeric`, `decimal`, `money` | `Decimal` |
| `uuid` | `UUID` |
| `date`, `timestamp`, `timestamptz` | `Int` (Unix seconds) |
| `json`, `jsonb`, `bytea` | `Text` |
| anything else | PascalCase of the PostgreSQL type name |

### certo db drift-check

```sh
certo db drift-check db/schema.cto
```

Compares a previously pulled snapshot file against the live database. Useful in CI when you do not want to re-introspect from source on every run.

---

## Appendix: Compiler Pipeline

The compiler processes source through these stages in order:

| Stage | Crate | Description |
|-------|-------|-------------|
| Lex + Parse | `certo-parser` | Tokenise and build AST |
| Resolve | `certo-resolve` | Resolve names, imports, stdlib |
| Type check | `certo-typeck` | Hindley-Milner inference + unification |
| HIR lowering | `certo-hir` | Desugar to high-level IR |
| MIR lowering | `certo-mir` | Lower to basic-block IR |
| C codegen | `certo-codegen` | Emit C source |
| Compile | Clang | C → native binary |

Alternative backends (`certo-llvm` for LLVM IR, `certo-wasm` for WebAssembly IR) operate on the MIR rather than emitting C.
