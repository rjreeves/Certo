# How to Program with Certo

> A practical guide to writing Certo programs, from first principles to
> advanced patterns. Each section introduces a feature simply, then shows
> how it is used in realistic code.

---

## Table of Contents

1. [Your First Program](#1-your-first-program)
2. [Variables and Bindings](#2-variables-and-bindings)
3. [Functions](#3-functions)
4. [Types — Records and Sums](#4-types--records-and-sums)
5. [Option — Handling Absence](#5-option--handling-absence)
6. [Result — Handling Failure](#6-result--handling-failure)
7. [Collections — List and Map](#7-collections--list-and-map)
8. [Pattern Matching](#8-pattern-matching)
9. [Loops and Iteration](#9-loops-and-iteration)
10. [Text](#10-text)
11. [Exact Arithmetic with Decimal](#11-exact-arithmetic-with-decimal)
12. [Dates and Times](#12-dates-and-times)
13. [HTTP — Client and Server](#13-http--client-and-server)
14. [JSON](#14-json)
15. [Database Access](#15-database-access)
16. [Concurrency with parallel](#16-concurrency-with-parallel)
17. [State Machines](#17-state-machines)
18. [Validators](#18-validators)
19. [Migrations](#19-migrations)
20. [Traits and Implementations](#20-traits-and-implementations)
21. [Testing](#21-testing)
22. [Modules and Imports](#22-modules-and-imports)

---

## 1. Your First Program

Every Certo file begins with a `module` declaration. `fn main` is the entry point.

```
module Hello

fn main(): Unit [io] = println("Hello, world!")
```

Compile and run:

```sh
certo build hello.cto -o hello
./hello
```

A slightly more interesting version — read a name from the command line:

```
module Hello

fn main(): Unit [io] = {
    val name = arg(1) ?? "stranger"
    println("Hello, " ++ name ++ "!")
}
```

`arg(1)` returns `Text?` (the first command-line argument, or `None`). The `??` operator supplies a default when the value is absent.

---

## 2. Variables and Bindings

### Immutable bindings — val and let

`val` and `let` are identical. Both declare an immutable binding. Once assigned, the value cannot change.

```
module Bindings

fn example(): Int = {
    val x = 10
    let y = 20      // same as val
    x + y
}
```

### Mutable bindings — var

Use `var` when you need to reassign:

```
fn countVowels(text: Text): Int = {
    var count = 0
    for ch in Text.split(text, "") {
        if ch == "a" or ch == "e" or ch == "i" or ch == "o" or ch == "u" {
            count = count + 1
        }
    }
    count
}
```

### Blocks are expressions

A block's value is its last expression. You can use this to compute a value in multiple steps and then use it inline:

```
fn describe(n: Int): Text = {
    val category = {
        if n < 0 then "negative"
        else if n == 0 then "zero"
        else "positive"
    }
    "The number " ++ intToText(n) ++ " is " ++ category
}
```

### Module-level bindings

At the top level of a module, `val` declares a constant:

```
module Config

val maxRetries: Int = 3
val defaultTimeout: Int = 30
val apiBase = "https://api.example.com"
```

---

## 3. Functions

### Basic syntax

```
module Functions

fn add(a: Int, b: Int): Int = a + b

fn greet(name: Text): Text = "Hello, " ++ name ++ "!"

fn square(n: Int): Int = n * n
```

### Block bodies

When a function needs more than one expression:

```
fn hypotenuse(a: Float, b: Float): Float = {
    val a2 = a * a
    val b2 = b * b
    sqrt(a2 + b2)
}
```

### Public vs private

```
pub fn exported(n: Int): Int = n * 2   // visible to importers

fn helper(n: Int): Int = n + 1         // private to this module
```

### Generic functions

A type parameter in angle brackets makes a function work over any type:

```
fn identity<T>(x: T): T = x

fn swap<A, B>(pair: (A, B)): (B, A) = {
    val (a, b) = pair
    (b, a)
}
```

### Lambdas and higher-order functions

```
fn applyTwice(f: Int => Int, x: Int): Int = f(f(x))

fn main(): Unit [io] = {
    val triple = (n: Int) => n * 3
    println(intToText(applyTwice(triple, 2)))   // 18
}
```

### Trailing lambda syntax

When the last argument to a function is a lambda, you can write it outside the parentheses. For a single-argument call, the parentheses can be dropped entirely:

```
// These three are identical:
val doubled = List.map(numbers, (x) => x * 2)
val doubled = List.map(numbers) { x => x * 2 }

// Two-parameter trailing lambda
val total = List.fold(numbers, 0) { acc, x => acc + x }

// Single-argument, no parens
List.forEach(names) { name => println(name) }
```

### Guard clauses

`guard` exits a function early when a precondition is not met:

```
fn safeDivide(a: Int, b: Int): Result<Int, Text> = {
    guard b != 0 else Err("cannot divide by zero")
    Ok(a / b)
}
```

### Defer

`defer` runs an expression when the enclosing block exits, regardless of how it exits. Useful for cleanup:

```
fn processFile(path: Text): Result<Int, Text> = {
    val conn = dbConnect("postgresql://localhost/mydb")
    defer dbClose(conn)

    val rows = dbQuery(conn, "SELECT count(*) FROM events", [])
    Ok(parseInt(List.getOrPanic(List.getOrPanic(rows, 0), 0) ?? "0") ?? 0)
}
```

The connection is always closed, even if an early return or error occurs.

### Advanced: function composition via pipeline

The `|>` operator pipes a value through a series of transformations. Each step receives the previous result as its first argument:

```
fn processOrders(orders: List<Order>): List<Text> =
    orders
    |> List.filter((o) => o.total > Decimal.fromInt(100))
    |> List.sort((a, b) => if Decimal.gt(a.total, b.total) then -1 else 1)
    |> List.map((o) => o.id ++ ": " ++ Decimal.toText(o.total))
```

---

## 4. Types — Records and Sums

### Record types

A record type is a named collection of typed fields:

```
module Domain

type Point = {
    x: Float,
    y: Float
}

type User = {
    id:    Int,
    name:  Text,
    email: Text,
    age:   Int?    // optional — may be absent
}
```

### Constructing records

```
fn makePoint(x: Float, y: Float): Point =
    Point { x: x, y: y }

fn makeUser(id: Int, name: Text, email: Text): User =
    User { id: id, name: name, email: email, age: () }
```

### Field access

```
fn greetUser(u: User): Text = "Hello, " ++ u.name ++ "!"

fn distance(a: Point, b: Point): Float = {
    val dx = a.x - b.x
    val dy = a.y - b.y
    sqrt(dx * dx + dy * dy)
}
```

### Record update

Create a modified copy without changing the original:

```
fn birthday(u: User): User = { u with age: (u.age ?? 0) + 1 }
```

### Sum types (enums)

A sum type is a value that is exactly one of several named variants:

```
type Direction = North | South | East | West

type Shape =
    | Circle(radius: Float)
    | Rectangle(width: Float, height: Float)
    | Triangle(base: Float, height: Float)
```

Unit variants (no fields) are constants. Variants with fields are constructor functions.

### Using sum types

```
fn area(shape: Shape): Float =
    match shape {
        Circle(r)       => Math.pi() * r * r
        Rectangle(w, h) => w * h
        Triangle(b, h)  => 0.5 * b * h
    }

fn move(dir: Direction, steps: Int): (Int, Int) =
    match dir {
        North => (0,  steps)
        South => (0, -steps)
        East  => ( steps, 0)
        West  => (-steps, 0)
    }
```

### Advanced: modeling a domain

A realistic domain model using records and sums together:

```
module Orders

type OrderStatus = | Draft | Submitted | Fulfilled | Cancelled

type Address = {
    street:  Text,
    city:    Text,
    country: Text
}

type LineItem = {
    productId:   Int,
    description: Text,
    quantity:    Int,
    unitPrice:   Decimal
}

type Order = {
    id:          Int,
    customerId:  Int,
    status:      OrderStatus,
    shippingTo:  Address,
    lines:       List<LineItem>,
    placedAt:    Int?,
    notes:       Text?
}

fn orderTotal(order: Order): Decimal =
    List.fold(order.lines, Decimal.fromInt(0), (acc, line) =>
        Decimal.add(acc,
            Decimal.mul(Decimal.fromInt(line.quantity), line.unitPrice)))

fn isFulfilled(order: Order): Bool =
    match order.status {
        Fulfilled => true
        _         => false
    }
```

---

## 5. Option — Handling Absence

`Option<T>` (written `T?`) represents a value that might not be present. There is no `null` in Certo — absence is always explicit.

### Basic usage

```
fn findPositive(xs: List<Int>): Int? =
    List.find(xs, (x) => x > 0)

fn main(): Unit [io] = {
    val numbers = [1, -2, 3, -4]
    match findPositive(numbers) {
        Some(n) => println("Found: " ++ intToText(n))
        None    => println("No positive numbers")
    }
}
```

### The ?? operator — default values

```
fn displayName(name: Text?): Text = name ?? "Anonymous"

fn getPort(envVar: Text?): Int =
    parseInt(envVar ?? "8080") ?? 8080
```

### Chaining optional operations

Use `match` to chain operations that might fail:

```
fn getUserEmail(userId: Int): Text? = {
    val user = findUser(userId)
    match user {
        None    => ()          // None propagates
        Some(u) => u.email     // u.email is Text (not optional)
    }
}
```

### Converting between Option and Result

```
fn requireUser(id: Int): Result<User, Text> =
    match findUser(id) {
        Some(u) => Ok(u)
        None    => Err("user " ++ intToText(id) ++ " not found")
    }
```

### Advanced: safe pipeline over optional data

```
fn summarise(userId: Int): Text = {
    val user    = findUser(userId)
    val name    = match user { Some(u) => u.name None => "unknown" }
    val orders  = match user { Some(u) => userOrderCount(u.id) None => 0 }
    name ++ " has " ++ intToText(orders) ++ " orders"
}
```

---

## 6. Result — Handling Failure

`Result<T, E>` is either `Ok(value)` or `Err(error)`. Every function that can fail returns a `Result` — there are no exceptions to catch.

### Simple example

```
fn divide(a: Int, b: Int): Result<Int, Text> =
    if b == 0 then Err("division by zero") else Ok(a / b)

fn main(): Unit [io] = {
    match divide(10, 2) {
        Ok(result) => println(intToText(result))
        Err(msg)   => eprintln("Error: " ++ msg)
    }
}
```

### The ? operator — propagate errors up

`?` unwraps `Ok(v)` to `v`, or returns the `Err` immediately from the current function. The enclosing function must return a `Result`.

```
fn parseAndDouble(s: Text): Result<Int, Text> = {
    val n = parseInt(s) match {
        Some(n) => n
        None    => return Err("not a number: " ++ s)
    }
    Ok(n * 2)
}

fn compute(a: Text, b: Text): Result<Int, Text> = {
    val x = parseAndDouble(a)?   // returns Err early if parseAndDouble fails
    val y = parseAndDouble(b)?
    Ok(x + y)
}
```

### Defining your own error types

Model your error domain as a sum type:

```
type AppError =
    | NotFound(resource: Text, id: Int)
    | Forbidden(userId: Int, action: Text)
    | InvalidInput(field: Text, reason: Text)
    | DatabaseError(message: Text)

fn findActiveUser(id: Int): Result<User, AppError> = {
    val user = match findUser(id) {
        None    => return Err(AppError.NotFound("user", id))
        Some(u) => u
    }
    if not user.active then
        Err(AppError.Forbidden(id, "login"))
    else
        Ok(user)
}
```

### Advanced: a multi-step operation with full error handling

```
fn checkout(userId: Int, cartId: Int, cardToken: Text): Result<Receipt, AppError> = {
    val user = findActiveUser(userId)?
    val cart = findCart(cartId)?

    guard List.len(cart.items) > 0
        else Err(AppError.InvalidInput("cart", "cart is empty"))

    guard Decimal.gte(user.creditLimit, cart.total)
        else Err(AppError.Forbidden(userId, "insufficient credit"))

    val receipt = chargeCard(cardToken, cart.total)?
    Ok(receipt)
}

fn main(): Unit [io] = {
    match checkout(42, 7, "tok_visa") {
        Ok(receipt)  => println("Charged: " ++ Decimal.toText(receipt.amount))
        Err(NotFound(r, id))    => eprintln(r ++ " " ++ intToText(id) ++ " not found")
        Err(Forbidden(_, msg))  => eprintln("Forbidden: " ++ msg)
        Err(InvalidInput(f, r)) => eprintln("Bad " ++ f ++ ": " ++ r)
        Err(DatabaseError(msg)) => eprintln("DB error: " ++ msg)
    }
}
```

---

## 7. Collections — List and Map

### List basics

```
import Stdlib.Collections

fn main(): Unit [io] = {
    val nums = [1, 2, 3, 4, 5]

    println(intToText(List.len(nums)))          // 5
    println(intToText(List.getOrPanic(nums, 0))) // 1

    val doubled = List.map(nums, (x) => x * 2)  // [2,4,6,8,10]
    val evens   = List.filter(nums, (x) => x % 2 == 0)   // [2,4]
    val total   = List.fold(nums, 0, (acc, x) => acc + x) // 15

    for n in doubled {
        println(intToText(n))
    }
}
```

### Building lists

```
fn buildList(n: Int): List<Int> = {
    var result: List<Int> = []
    for i in range(0, n) {
        result = List.push(result, i * i)
    }
    result
}

// Or using map over range:
fn buildList2(n: Int): List<Int> =
    List.map(range(0, n), (i) => i * i)
```

### Common list patterns

```
import Stdlib.Collections

// Find the first element matching a predicate
val first = List.find(users, (u) => u.role == "admin")

// Check if any / all elements satisfy a condition
val anyPending   = List.any(orders, (o) => o.status == Pending)
val allConfirmed = List.all(orders, (o) => o.confirmed)

// Sort — comparator returns negative, 0, or positive
val sorted = List.sort(users, (a, b) =>
    if a.name < b.name then -1
    else if a.name > b.name then 1
    else 0)

// Pair up two lists
val pairs = List.zip([1, 2, 3], ["a", "b", "c"])
// pairs = [(1,"a"), (2,"b"), (3,"c")]

// Concatenate
val combined = List.concat([1, 2], [3, 4])    // [1,2,3,4]
```

### Map basics

```
fn wordCount(words: List<Text>): Map<Text, Int> = {
    var counts: Map<Text, Int> = Map.empty()
    for word in words {
        val current = Map.get(counts, word) ?? 0
        counts = Map.insert(counts, word, current + 1)
    }
    counts
}

fn main(): Unit [io] = {
    val counts = wordCount(["the", "cat", "sat", "on", "the", "mat"])
    val theCount = Map.get(counts, "the") ?? 0
    println("'the' appears " ++ intToText(theCount) ++ " times")
}
```

### Advanced: grouping and aggregation

```
import Stdlib.Collections

fn groupByStatus(orders: List<Order>): Map<Text, List<Order>> = {
    var groups: Map<Text, List<Order>> = Map.empty()
    for order in orders {
        val key = match order.status {
            Draft     => "draft"
            Submitted => "submitted"
            Fulfilled => "fulfilled"
            Cancelled => "cancelled"
        }
        val existing = Map.get(groups, key) ?? []
        groups = Map.insert(groups, key, List.push(existing, order))
    }
    groups
}

fn topNByTotal(orders: List<Order>, n: Int): List<Order> =
    orders
    |> List.sort((a, b) =>
        if Decimal.gt(a.total, b.total) then -1 else 1)
    |> List.slice(0, n)
```

---

## 8. Pattern Matching

`match` is the primary way to inspect sum types, options, and results. It is exhaustive — the compiler requires every case to be handled.

### Matching on values

```
fn dayName(n: Int): Text =
    match n {
        0 => "Sunday"
        1 => "Monday"
        2 => "Tuesday"
        3 => "Wednesday"
        4 => "Thursday"
        5 => "Friday"
        6 => "Saturday"
        _ => "Unknown"
    }
```

### Guards in match arms

```
fn grade(score: Int): Text =
    match score {
        s if s >= 90 => "A"
        s if s >= 80 => "B"
        s if s >= 70 => "C"
        s if s >= 60 => "D"
        _            => "F"
    }
```

### Binding variables in patterns

```
fn describeList(xs: List<Int>): Text =
    match List.first(xs) {
        None    => "empty list"
        Some(n) => "starts with " ++ intToText(n)
    }
```

### Nested patterns

```
fn processResult(r: Result<Int?, Text>): Text =
    match r {
        Ok(Some(n)) => "got: " ++ intToText(n)
        Ok(None)    => "present but empty"
        Err(msg)    => "error: " ++ msg
    }
```

### Advanced: matching on sum type fields

```
type Notification =
    | Email(address: Text, subject: Text)
    | SMS(phone: Text)
    | Push(deviceId: Text, title: Text, body: Text)

fn summarise(n: Notification): Text =
    match n {
        Email(addr, subj)       => "Email to " ++ addr ++ ": " ++ subj
        SMS(phone)              => "SMS to " ++ phone
        Push(_, title, _)       => "Push: " ++ title
    }

fn destination(n: Notification): Text =
    match n {
        Email(addr, _)  => addr
        SMS(phone)      => phone
        Push(dev, _, _) => dev
    }
```

---

## 9. Loops and Iteration

### For loops

`for` iterates over any `List<T>`:

```
fn printNumbers(n: Int): Unit [io] = {
    for i in range(1, n + 1) {
        println(intToText(i))
    }
}
```

### While loops

```
fn fibonacci(n: Int): Int = {
    if n <= 1 then return n
    var a = 0
    var b = 1
    var i = 2
    while i <= n {
        val next = a + b
        a = b
        b = next
        i = i + 1
    }
    b
}
```

### Functional iteration with List

Prefer `List.map`, `List.filter`, and `List.fold` when you are transforming data rather than performing side effects. They compose cleanly with `|>`:

```
fn summariseUsers(users: List<User>): Text = {
    val active = List.filter(users, (u) => u.active)
    val names  = List.map(active, (u) => u.name)
    Text.join(names, ", ")
}
```

### Advanced: accumulating results across a list

Collect errors without stopping early:

```
fn validateAll(items: List<Int>): Result<List<Int>, List<Text>> = {
    var results: List<Int>  = []
    var errors:  List<Text> = []
    for item in items {
        if item < 0 then
            errors = List.push(errors, "negative value: " ++ intToText(item))
        else
            results = List.push(results, item)
    }
    if List.len(errors) > 0 then Err(errors) else Ok(results)
}
```

---

## 10. Text

### Building text

```
import Stdlib.Text

fn fullName(first: Text, last: Text): Text = first ++ " " ++ last

fn csv(values: List<Text>): Text = Text.join(values, ",")

fn repeat(s: Text, n: Int): Text = Text.repeat(s, n)
```

### Searching and testing

```
fn isValidEmail(s: Text): Bool =
    Text.contains(s, "@") and Text.contains(s, ".")

fn startsWithHttp(url: Text): Bool =
    Text.startsWith(url, "http://") or Text.startsWith(url, "https://")
```

### Splitting and transforming

```
fn words(sentence: Text): List<Text> =
    Text.split(Text.trim(sentence), " ")

fn titleCase(s: Text): Text = {
    val parts = Text.split(s, " ")
    val cased = List.map(parts, (w) => {
        if Text.len(w) == 0 then w
        else Text.toUpper(Text.slice(w, 0, 1)) ++ Text.slice(w, 1, Text.len(w))
    })
    Text.join(cased, " ")
}
```

### Advanced: simple template substitution

```
fn render(template: Text, vars: Map<Text, Text>): Text = {
    var result = template
    val keys = Map.keys(vars)
    for key in keys {
        val value = Map.get(vars, key) ?? ""
        result = Text.replace(result, "{{" ++ key ++ "}}", value)
    }
    result
}

fn main(): Unit [io] = {
    var vars: Map<Text, Text> = Map.empty()
    vars = Map.insert(vars, "name", "Alice")
    vars = Map.insert(vars, "role", "admin")
    println(render("Hello {{name}}, you are an {{role}}.", vars))
}
```

---

## 11. Exact Arithmetic with Decimal

Never use `Float` for money or any quantity where rounding matters. Use `Decimal`.

### Basic arithmetic

```
import Stdlib.Money

fn main(): Unit [io] = {
    val price   = Decimal.fromInt(19)   // 19
    val tax     = Decimal.fromInt(2)    // approximation — see below for proper way
    val total   = Decimal.add(price, tax)
    println(Decimal.toText(total))      // "21"
}
```

### Working with cents

The cleanest way to handle money is to store it in cents (integers) and convert at display time:

```
fn formatMoney(cents: Int): Text = {
    val dollars = cents / 100
    val c       = absInt(cents % 100)
    intToText(dollars) ++ "." ++ (if c < 10 then "0" else "") ++ intToText(c)
}

fn addTax(priceCents: Int, taxRatePct: Int): Int =
    priceCents + (priceCents * taxRatePct / 100)
```

### Rounding with an explicit mode

Every rounding operation requires you to choose a `RoundingMode`. There is no silent default:

```
import Stdlib.Money

fn applyDiscount(price: Decimal, pct: Int): Decimal = {
    val factor     = Decimal.sub(Decimal.fromInt(1),
                         Decimal.divRound(Decimal.fromInt(pct), Decimal.fromInt(100), 4, HalfUp))
    val discounted = Decimal.mul(price, factor)
    Decimal.round(discounted, 2, HalfUp)
}
```

### Advanced: split a bill evenly (no penny lost)

```
import Stdlib.Money

fn splitBill(totalCents: Int, ways: Int): List<Int> = {
    val base      = totalCents / ways
    val remainder = totalCents % ways
    var shares: List<Int> = []
    for i in range(0, ways) {
        val extra = if i < remainder then 1 else 0
        shares = List.push(shares, base + extra)
    }
    shares
}

fn main(): Unit [io] = {
    val shares = splitBill(1000, 3)   // $10.00 split 3 ways
    for s in shares {
        println(formatMoney(s))       // 334, 333, 333
    }
}
```

---

## 12. Dates and Times

Certo represents dates and times as Unix timestamps (integers). All conversion and arithmetic is explicit.

### Getting the current time

```
import Stdlib.DateTime

fn main(): Unit [io] = {
    val now   = DateTime.now()
    val today = Date.today()
    println(DateTime.toIso(now))
    println(Date.format(today, "%Y-%m-%d"))
}
```

### Arithmetic on dates

```
import Stdlib.DateTime

fn dueDate(invoicedAt: Int, termsDays: Int): Int =
    DateTime.addDays(invoicedAt, termsDays)

fn isOverdue(dueAt: Int): Bool =
    DateTime.before(dueAt, DateTime.now())

fn daysBetween(a: Int, b: Int): Int =
    absInt(DateTime.diffDays(a, b))
```

### Formatting and parsing

```
import Stdlib.DateTime

fn formatForDisplay(ts: Int): Text =
    DateTime.format(ts, "%d %b %Y at %H:%M")

fn parseUserDate(s: Text): Int? =
    match DateTime.parseIso(s) {
        Ok(ts) => Some(ts)
        Err(_) => None
    }
```

### Advanced: age calculation

```
import Stdlib.DateTime

fn ageInYears(birthdateTs: Int): Int = {
    val now      = DateTime.now()
    val years    = DateTime.year(now) - DateTime.year(birthdateTs)
    val hadBday  = DateTime.month(now) > DateTime.month(birthdateTs)
                or (DateTime.month(now) == DateTime.month(birthdateTs)
                    and DateTime.day(now) >= DateTime.day(birthdateTs))
    if hadBday then years else years - 1
}
```

---

## 13. HTTP — Client and Server

### Making HTTP requests

```
import Stdlib.Http
import Stdlib.Json

fn fetchUserName(userId: Int): Text = {
    val url  = "https://api.example.com/users/" ++ intToText(userId)
    val resp = Http.get(url)
    if HttpResponse.ok(resp) then {
        val body = HttpResponse.body(resp)
        val json = Json.parse(body)
        JsonValue.asText(JsonValue.field(json, "name"))
    } else {
        "unknown"
    }
}
```

### Building a simple HTTP server

```
import Stdlib.Http

fn handleRequest(req: HttpRequest): HttpResponse = {
    val path = HttpRequest.path(req)
    match path {
        "/" =>
            Http.ok("<h1>Hello!</h1>", "text/html")
        "/health" =>
            Http.ok("{\"status\":\"ok\"}", "application/json")
        _ =>
            Http.notFound("Not found")
    }
}

fn main(): Unit [io] =
    Http.serve(8080, handleRequest)
```

### Reading request data

```
import Stdlib.Http

fn apiHandler(req: HttpRequest): HttpResponse = {
    val method      = HttpRequest.method(req)
    val path        = HttpRequest.path(req)
    val body        = HttpRequest.body(req)
    val authHeader  = HttpRequest.header(req, "Authorization")

    if authHeader == "" then
        return Http.respond(401, "Unauthorized", "text/plain")

    match method {
        "GET"  => Http.ok("[]", "application/json")
        "POST" => Http.ok(body, "application/json")
        _      => Http.respond(405, "Method Not Allowed", "text/plain")
    }
}
```

### Advanced: a JSON REST endpoint

```
import Stdlib.Http
import Stdlib.Json

fn getUsersJson(): Text = {
    val arr = Json.array()
    // In real code, load from DB; here we build a static example
    var result = arr
    val user1 = Json.object() |> JsonValue.set("id", Json.int(1))
                              |> JsonValue.set("name", Json.string("Alice"))
    result = JsonValue.push(result, user1)
    Json.stringify(result)
}

fn router(req: HttpRequest): HttpResponse = {
    val path   = HttpRequest.path(req)
    val method = HttpRequest.method(req)
    match (method, path) {
        ("GET",  "/api/users") =>
            Http.ok(getUsersJson(), "application/json")
        ("POST", "/api/users") => {
            val body = HttpRequest.body(req)
            val json = Json.parse(body)
            val name = JsonValue.asText(JsonValue.field(json, "name"))
            if name == "" then
                Http.badRequest("{\"error\":\"name required\"}")
            else
                Http.ok("{\"created\":true}", "application/json")
        }
        _ => Http.notFound("{\"error\":\"not found\"}")
    }
}

fn main(): Unit [io] = Http.serve(8080, router)
```

---

## 14. JSON

### Parsing and reading

```
import Stdlib.Json

fn parseConfig(src: Text): (Text, Int) = {
    val json = Json.parse(src)
    val host = JsonValue.asText(JsonValue.field(json, "host"))
    val port = JsonValue.asInt(JsonValue.field(json, "port"))
    (host, port)
}
```

### Building JSON

```
import Stdlib.Json

fn userToJson(id: Int, name: Text, active: Bool): Text = {
    val obj = Json.object()
              |> JsonValue.set("id",     Json.int(id))
              |> JsonValue.set("name",   Json.string(name))
              |> JsonValue.set("active", Json.bool(active))
    Json.stringify(obj)
}
```

### Building a JSON array

```
import Stdlib.Json

fn idsToJson(ids: List<Int>): Text = {
    var arr = Json.array()
    for id in ids {
        arr = JsonValue.push(arr, Json.int(id))
    }
    Json.stringify(arr)
}
```

### Advanced: mapping a list of records to JSON

```
import Stdlib.Json

fn usersToJson(users: List<User>): Text = {
    var arr = Json.array()
    for u in users {
        val obj = Json.object()
                  |> JsonValue.set("id",    Json.int(u.id))
                  |> JsonValue.set("name",  Json.string(u.name))
                  |> JsonValue.set("email", Json.string(u.email))
        arr = JsonValue.push(arr, obj)
    }
    Json.stringify(arr)
}
```

---

## 15. Database Access

Certo's database layer uses raw SQL with positional parameters (`$1`, `$2`, ...) and a connection handle returned by `dbConnect`.

### Connecting

```
import Stdlib.Db

fn openDb(): Int = {
    val conn = dbConnect("postgresql://localhost/myapp")
    if conn == 0 then
        panic("database connection failed: " ++ dbError(0))
    conn
}
```

### Running queries

```
import Stdlib.Db

fn getUserName(conn: Int, id: Int): Text? = {
    val rows = dbQuery(conn,
        "SELECT name FROM users WHERE id = $1",
        [intToText(id)])
    match List.first(rows) {
        None    => None
        Some(r) => List.getOrPanic(r, 0)
    }
}
```

### Inserting and updating

```
import Stdlib.Db

fn createUser(conn: Int, name: Text, email: Text): Int = {
    dbExec(conn,
        "INSERT INTO users (name, email) VALUES ($1, $2)",
        [name, email])
}

fn deactivateUser(conn: Int, id: Int): Int =
    dbExec(conn,
        "UPDATE users SET active = false WHERE id = $1",
        [intToText(id)])
```

### Typed queries with dbQueryTyped

Use `dbQueryTyped` when you have a row mapper function. The return type is inferred from the mapper — the compiler verifies it:

```
import Stdlib.Db

type User = { id: Int, name: Text, email: Text }

fn userFromRow(row: List<Text?>): User =
    User {
        id:    parseInt(List.getOrPanic(row, 0) ?? "0") ?? 0,
        name:  List.getOrPanic(row, 1) ?? "",
        email: List.getOrPanic(row, 2) ?? ""
    }

fn getActiveUsers(conn: Int): List<User> =
    dbQueryTyped(conn,
        "SELECT id, name, email FROM users WHERE active = $1",
        ["true"],
        userFromRow)
```

### Advanced: a full CRUD module

```
module Users

import Stdlib.Db

type User = {
    id:     Int,
    name:   Text,
    email:  Text,
    active: Bool
}

fn fromRow(row: List<Text?>): User =
    User {
        id:     parseInt(List.getOrPanic(row, 0) ?? "0") ?? 0,
        name:   List.getOrPanic(row, 1) ?? "",
        email:  List.getOrPanic(row, 2) ?? "",
        active: List.getOrPanic(row, 3) == Some("t")
    }

pub fn findById(conn: Int, id: Int): User? =
    List.first(dbQueryTyped(conn,
        "SELECT id, name, email, active FROM users WHERE id = $1",
        [intToText(id)], fromRow))

pub fn findAll(conn: Int): List<User> =
    dbQueryTyped(conn,
        "SELECT id, name, email, active FROM users ORDER BY name",
        [], fromRow)

pub fn create(conn: Int, name: Text, email: Text): Result<Unit, Text> = {
    val affected = dbExec(conn,
        "INSERT INTO users (name, email, active) VALUES ($1, $2, true)",
        [name, email])
    if affected < 0 then Err(dbError(conn)) else Ok(())
}

pub fn deactivate(conn: Int, id: Int): Result<Unit, Text> = {
    val affected = dbExec(conn,
        "UPDATE users SET active = false WHERE id = $1",
        [intToText(id)])
    if affected < 0 then Err(dbError(conn)) else Ok(())
}
```

---

## 16. Concurrency with parallel

`parallel { expr1, expr2, ... }` runs multiple function calls simultaneously on separate OS threads and returns a tuple of their results. Use it when independent operations can safely run at the same time.

### Simple parallel fetch

```
import Stdlib.Http

fn fetchBoth(url1: Text, url2: Text): (Text, Text) = {
    val results = parallel {
        Http.get(url1) |> HttpResponse.body,
        Http.get(url2) |> HttpResponse.body
    }
    results
}
```

### Parallel database queries

```
import Stdlib.Db

fn loadDashboard(conn: Int, userId: Int): (List<Order>, List<Invoice>, User?) = {
    parallel {
        getOrdersForUser(conn, userId),
        getInvoicesForUser(conn, userId),
        findById(conn, userId)
    }
}
```

### Handling partial failures

`parallel` returns a tuple. Inspect each value individually:

```
fn fetchWithFallback(primaryUrl: Text, fallbackUrl: Text): Text = {
    val results = parallel {
        Http.get(primaryUrl),
        Http.get(fallbackUrl)
    }
    val (primary, fallback) = results
    if HttpResponse.ok(primary) then
        HttpResponse.body(primary)
    else
        HttpResponse.body(fallback)
}
```

### Advanced: fan-out enrichment

Fetch related data for a list of items concurrently — use `spawn` directly when the number of items is dynamic:

```
import Stdlib.Http

fn enrichOrder(order: Order): Order = {
    val resp = Http.get("/api/customers/" ++ intToText(order.customerId))
    val name = Json.parse(HttpResponse.body(resp))
               |> JsonValue.field("name")
               |> JsonValue.asText
    { order with customerName: name }
}

// For a fixed set of concurrent operations use parallel {}:
fn loadTopThree(ids: List<Int>): (Order, Order, Order) = {
    parallel {
        loadOrder(List.getOrPanic(ids, 0)),
        loadOrder(List.getOrPanic(ids, 1)),
        loadOrder(List.getOrPanic(ids, 2))
    }
}
```

> **Note:** each branch of `parallel {}` must be a direct function call.
> Arguments are passed as 64-bit integers through the thread boundary, so
> pointer and integer arguments work correctly. Avoid passing `Decimal`
> values directly across the boundary — convert to `Int` (cents) first.

---

## 17. State Machines

State machines enforce that an entity can only move between states in legal ways. The type checker — not runtime code — rejects invalid transitions.

### Declaring a state machine

```
module Orders

statemachine Order {
    states: Draft, Submitted, Fulfilled, Cancelled

    transitions:
        Draft -> Submitted  : submit(note: Text)
        Submitted -> Fulfilled  : fulfil(trackingNumber: Text)
        [Draft, Submitted] -> Cancelled : cancel(reason: Text)
}
```

### Using the generated API

```
fn main(): Unit [io] = {
    // Order_new() returns Order<Draft>
    val draft = Order_new()

    // submit only accepts Order<Draft>
    val submitted = Order_submit(draft, "approved by manager")

    // fulfil only accepts Order<Submitted>
    val fulfilled = Order_fulfil(submitted, "TRACK-9876")

    println("order fulfilled")
}
```

The type checker prevents this from compiling:

```
val draft = Order_new()
val wrong = Order_fulfil(draft, "TRACK-0")  // Error: expected Order<Submitted>
```

### Multi-from transitions

A transition that is valid from more than one state:

```
// [Draft, Submitted] -> Cancelled : cancel  means:
// Order_cancel accepts Order<Draft> OR Order<Submitted>
val draft     = Order_new()
val cancelled = Order_cancel(draft, "customer request")

val submitted = Order_submit(Order_new(), "ready")
val cancelled2 = Order_cancel(submitted, "out of stock")
```

### Loading from a database (unknown state)

When you load an order from a database, you do not know its state at compile time. Use the `assertX` functions to recover a state-typed value:

```
fn processSubmitted(conn: Int, orderId: Int): Result<Unit, Text> = {
    val row = dbQuery(conn, "SELECT * FROM orders WHERE id = $1",
                      [intToText(orderId)])
    // ... deserialise to Order<S> with unknown state ...
    val order = loadOrder(row)

    match Order_assertSubmitted(order) {
        None    => Err("order is not in Submitted state")
        Some(s) => {
            val _ = Order_fulfil(s, "TRACK-AUTO")
            Ok(())
        }
    }
}
```

### State predicates and accessors

```
fn describeOrder(order: Order<S>): Text = {
    val stateName = match Order_state(order) {
        Order_Draft     => "draft"
        Order_Submitted => "submitted"
        Order_Fulfilled => "fulfilled"
        Order_Cancelled => "cancelled"
    }
    "order is " ++ stateName
}

// Boolean predicates
val isDraft = Order_isDraft(order)
```

### Advanced: a complete order lifecycle

```
module OrderLifecycle

statemachine Order {
    states: Draft, Submitted, PaymentPending, Confirmed, Shipped, Cancelled

    transitions:
        Draft -> Submitted               : submit(customerId: Int, total: Decimal)
        Submitted -> PaymentPending      : requestPayment(amount: Decimal)
        PaymentPending -> Confirmed      : confirmPayment(transactionId: Text)
        Confirmed -> Shipped             : ship(trackingNumber: Text, carrier: Text)
        [Draft, Submitted] -> Cancelled  : cancel(reason: Text)
        PaymentPending -> Cancelled      : cancelPayment(reason: Text)
}

fn processNewOrder(customerId: Int, total: Decimal): Order<Shipped> = {
    val draft     = Order_new()
    val submitted = Order_submit(draft, customerId, total)
    val pending   = Order_requestPayment(submitted, total)
    val confirmed = Order_confirmPayment(pending, "txn_" ++ intToText(customerId))
    Order_ship(confirmed, "TRACK-001", "FedEx")
}
```

---

## 18. Validators

Validators check business rules against an entity and collect all violations. They compile to both a runtime function and (optionally) a PostgreSQL trigger.

### A simple validator

```
module Pricing

type Product = { id: Int, name: Text, price: Decimal, stock: Int }

type PriceError =
    | NegativePrice(price: Decimal)
    | StockNegative(stock: Int)
    | NameEmpty

validator ProductValidator for Product errors PriceError {
    rule nonNegativePrice {
        require Decimal.gte(product.price, Decimal.fromInt(0))
        else PriceError.NegativePrice(product.price)
    }

    rule positiveStock {
        require product.stock >= 0
        else PriceError.StockNegative(product.stock)
    }

    rule nameRequired {
        require Text.len(product.name) > 0
        else PriceError.NameEmpty
    }
}
```

### Calling the validator

```
fn main(): Unit [io] = {
    val product = Product {
        id: 1, name: "", price: Decimal.fromInt(-5), stock: -1
    }
    val ctx    = ProductValidator_context {}
    val errors = ProductValidator_validate(product, ctx)

    if List.len(errors) == 0 then
        println("product is valid")
    else {
        for err in errors {
            val msg = match err {
                NegativePrice(p) => "price is negative: " ++ Decimal.toText(p)
                StockNegative(s) => "stock is negative: " ++ intToText(s)
                NameEmpty        => "name is required"
            }
            eprintln(msg)
        }
    }
}
```

### Ordered rules (after)

Use `after` to run a rule only when its dependency has passed:

```
validator InvoiceValidator for Invoice errors InvoiceError {
    rule amountPositive {
        require Decimal.gt(invoice.amount, Decimal.fromInt(0))
        else InvoiceError.ZeroAmount
    }

    rule taxReasonable {
        after amountPositive        // only runs if amountPositive passed
        require Decimal.lte(invoice.tax,
            Decimal.mul(invoice.amount, Decimal.fromInt(2)))
        else InvoiceError.TaxExceedsDouble
    }
}
```

### Context — loading related data

Sometimes a rule needs data from outside the entity. Declare it in `context`:

```
type CreditError = | CreditExceeded(limit: Decimal, required: Decimal)

validator OrderCreditCheck for Order errors CreditError {
    context {
        customer: Customer loaded by db.customers.find(order.customerId)
    }

    rule withinCreditLimit {
        require Decimal.gte(customer.creditLimit, order.total)
        else CreditError.CreditExceeded(customer.creditLimit, order.total)
    }
}
```

When a `loaded by` expression is present, the compiler generates a `validateWithDb` variant that fetches the context automatically:

```
fn checkOrder(conn: Int, order: Order): List<CreditError> =
    OrderCreditCheck_validateWithDb(conn, order)
```

### Triggers — enforce rules at the database level

```
validator StockConstraint for LineItem errors StockError {
    trigger on insert
    trigger on update when status != "cancelled"

    rule positiveQty {
        require lineItem.quantity > 0
        else StockError.ZeroQuantity
    }
}
```

`certo build` emits the corresponding `CREATE TRIGGER` SQL alongside the C output.

---

## 19. Migrations

Migrations describe schema changes in Certo source. The compiler validates them against your `type` declarations.

### Creating a table

```
migration "create users table" {
    up {
        createTable users {
            id:        Int        primary key
            name:      Text
            email:     Text       unique
            active:    Bool
            createdAt: Int
            bio:       Text?
        }
    }
    down {
        dropTable users
    }
}
```

### Adding a column

```
migration "add users.role column" {
    up {
        alterTable users {
            addColumn role: Text default "member"
        }
    }
    down {
        alterTable users {
            dropColumn role
        }
    }
}
```

### Foreign keys and indexes

```
migration "create orders table" {
    up {
        createTable orders {
            id:         Int       primary key
            userId:     Int       references users(id) on delete cascade
            total:      Decimal
            status:     Text
            createdAt:  Int
        }
        createIndex idx_orders_user on orders (userId)
        createIndex idx_orders_status on orders (status)
    }
    down {
        dropIndex idx_orders_status
        dropIndex idx_orders_user
        dropTable orders
    }
}
```

### Raw SQL for complex cases

```
migration "add full text search index" {
    up {
        rawSql "CREATE INDEX idx_products_fts ON products USING gin(to_tsvector('english', name || ' ' || description))"
    }
    down {
        rawSql "DROP INDEX IF EXISTS idx_products_fts"
    }
}
```

### Running migrations

```sh
certo migrate status          # see what is applied and what is pending
certo migrate apply           # apply all pending migrations in order
certo migrate rollback        # roll back the last applied migration
```

---

## 20. Traits and Implementations

Traits define a named set of operations. `impl` provides those operations for a specific type.

### Defining and implementing a trait

```
module Display

trait Displayable {
    fn display(self): Text
}

type Color = | Red | Green | Blue

impl Displayable for Color {
    fn display(self): Text =
        match self {
            Red   => "red"
            Green => "green"
            Blue  => "blue"
        }
}
```

### Using traits as constraints

```
fn printAll<T: Displayable>(items: List<T>): Unit [io] = {
    for item in items {
        println(item.display())
    }
}
```

### Multiple methods in a trait

```
trait Comparable {
    fn lessThan(self, other: Self): Bool
    fn equals(self, other: Self): Bool
    fn greaterThan(self, other: Self): Bool
}

type Version = { major: Int, minor: Int, patch: Int }

impl Comparable for Version {
    fn lessThan(self, other: Version): Bool =
        self.major < other.major
        or (self.major == other.major and self.minor < other.minor)
        or (self.major == other.major and self.minor == other.minor
            and self.patch < other.patch)

    fn equals(self, other: Version): Bool =
        self.major == other.major
        and self.minor == other.minor
        and self.patch == other.patch

    fn greaterThan(self, other: Version): Bool =
        not self.lessThan(other) and not self.equals(other)
}
```

---

## 21. Testing

### Unit tests

`test` declarations are collected by `certo test` and executed:

```
module MathTests

test "add works" {
    assert(2 + 2 == 4, "2 + 2 should be 4")
}

test "safeDivide returns Err on zero" {
    match safeDivide(10, 0) {
        Err(_) => ()            // expected
        Ok(_)  => assert(false, "should have returned Err")
    }
}

test "list fold computes sum" {
    val xs  = [1, 2, 3, 4, 5]
    val sum = List.fold(xs, 0, (acc, x) => acc + x)
    assert(sum == 15, "sum should be 15")
}
```

### Testing sum types

```
test "shape area circle" {
    val c      = Circle(2.0)
    val result = area(c)
    val diff   = absFloat(result - 12.566370614359172)
    assert(diff < 0.0001, "circle area should be pi*4")
}

test "order state machine happy path" {
    val draft     = Order_new()
    val submitted = Order_submit(draft, "ready")
    assert(Order_isSubmitted(submitted), "should be submitted after submit")
    assert(not Order_isDraft(submitted), "should not be draft after submit")
}
```

### Testing validators

```
test "validator rejects negative price" {
    val p      = Product { id: 1, name: "Widget", price: Decimal.fromInt(-1), stock: 10 }
    val errors = ProductValidator_validate(p, ProductValidator_context {})
    assert(List.len(errors) > 0, "should have validation errors")
    assert(List.any(errors, (e) => match e { NegativePrice(_) => true _ => false }),
           "should report negative price")
}

test "validator passes valid product" {
    val p      = Product { id: 2, name: "Gadget", price: Decimal.fromInt(9), stock: 5 }
    val errors = ProductValidator_validate(p, ProductValidator_context {})
    assert(List.len(errors) == 0, "valid product should have no errors")
}
```

### Running tests

```sh
certo test src/main.cto
certo test src/           # test all .cto files in a directory
```

---

## 22. Modules and Imports

### One module per file

Every `.cto` file declares exactly one module:

```
module MyApp.Orders.Processing
```

The module name is independent of the file path — it is a namespace, not a file locator.

### Importing stdlib

```
import Stdlib.Text
import Stdlib.Collections
import Stdlib.Http
import Stdlib.Json
import Stdlib.DateTime
import Stdlib.Money
import Stdlib.Math
import Stdlib.Crypto
import Stdlib.Db
```

### Importing your own modules

Pass all files that form the program to `certo build`:

```sh
certo build src/main.cto src/orders.cto src/users.cto -o myapp
```

Within a file, import by module name:

```
module MyApp.Main

import MyApp.Orders
import MyApp.Users
```

### Public and private

Names without `pub` are private to the module. Names with `pub` are visible to importers:

```
module MyApp.Orders

pub type Order    = { id: Int, total: Decimal }   // visible outside
pub fn create(): Order = { id: 1, total: Decimal.fromInt(0) }

fn validate(o: Order): Bool = o.total > Decimal.fromInt(0)  // private
```

### Organising a project

A typical layout for a non-trivial Certo application:

```
src/
    main.cto         // module MyApp — entry point, wires things together
    domain.cto       // module MyApp.Domain — types
    orders.cto       // module MyApp.Orders — order logic
    users.cto        // module MyApp.Users — user logic
    api.cto          // module MyApp.Api — HTTP handlers
db/
    schema.cto       // generated by certo db pull
    migrations/
        001-create-users.cto
        002-create-orders.cto
```

Build the whole program:

```sh
certo build src/main.cto src/domain.cto src/orders.cto \
            src/users.cto src/api.cto \
            db/migrations/001-create-users.cto \
            db/migrations/002-create-orders.cto \
            -o myapp
```

---

## Putting It Together — A Complete Example

A small HTTP API that stores and retrieves users from PostgreSQL, showing
types, error handling, database access, JSON, and HTTP together:

```
module App

import Stdlib.Http
import Stdlib.Json
import Stdlib.Db

// ── Domain ────────────────────────────────────────────────────────────

type User = { id: Int, name: Text, email: Text }

type AppError =
    | DbError(msg: Text)
    | NotFound(id: Int)
    | BadRequest(reason: Text)

fn userFromRow(row: List<Text?>): User =
    User {
        id:    parseInt(List.getOrPanic(row, 0) ?? "0") ?? 0,
        name:  List.getOrPanic(row, 1) ?? "",
        email: List.getOrPanic(row, 2) ?? ""
    }

// ── Data access ───────────────────────────────────────────────────────

fn getAllUsers(conn: Int): Result<List<User>, AppError> = {
    val users = dbQueryTyped(conn,
        "SELECT id, name, email FROM users ORDER BY id",
        [], userFromRow)
    Ok(users)
}

fn getUserById(conn: Int, id: Int): Result<User, AppError> = {
    val users = dbQueryTyped(conn,
        "SELECT id, name, email FROM users WHERE id = $1",
        [intToText(id)], userFromRow)
    match List.first(users) {
        None    => Err(AppError.NotFound(id))
        Some(u) => Ok(u)
    }
}

fn createUser(conn: Int, name: Text, email: Text): Result<Unit, AppError> = {
    guard Text.len(name) > 0
        else Err(AppError.BadRequest("name is required"))
    guard Text.contains(email, "@")
        else Err(AppError.BadRequest("email must contain @"))
    val affected = dbExec(conn,
        "INSERT INTO users (name, email) VALUES ($1, $2)",
        [name, email])
    if affected < 0 then Err(AppError.DbError(dbError(conn))) else Ok(())
}

// ── Serialisation ─────────────────────────────────────────────────────

fn userToJson(u: User): JsonValue =
    Json.object()
    |> JsonValue.set("id",    Json.int(u.id))
    |> JsonValue.set("name",  Json.string(u.name))
    |> JsonValue.set("email", Json.string(u.email))

fn usersToJson(users: List<User>): Text = {
    var arr = Json.array()
    for u in users {
        arr = JsonValue.push(arr, userToJson(u))
    }
    Json.stringify(arr)
}

fn errorToJson(err: AppError): Text = {
    val msg = match err {
        DbError(m)     => m
        NotFound(id)   => "user " ++ intToText(id) ++ " not found"
        BadRequest(r)  => r
    }
    Json.stringify(Json.object() |> JsonValue.set("error", Json.string(msg)))
}

fn errorStatus(err: AppError): Int =
    match err {
        DbError(_)    => 500
        NotFound(_)   => 404
        BadRequest(_) => 400
    }

// ── HTTP layer ────────────────────────────────────────────────────────

fn handleError(err: AppError): HttpResponse =
    Http.respond(errorStatus(err), errorToJson(err), "application/json")

fn router(conn: Int, req: HttpRequest): HttpResponse = {
    val method = HttpRequest.method(req)
    val path   = HttpRequest.path(req)

    match (method, path) {
        ("GET", "/users") =>
            match getAllUsers(conn) {
                Ok(users) => Http.ok(usersToJson(users), "application/json")
                Err(e)    => handleError(e)
            }

        ("POST", "/users") => {
            val body  = HttpRequest.body(req)
            val json  = Json.parse(body)
            val name  = JsonValue.asText(JsonValue.field(json, "name"))
            val email = JsonValue.asText(JsonValue.field(json, "email"))
            match createUser(conn, name, email) {
                Ok(())  => Http.respond(201, "{\"created\":true}", "application/json")
                Err(e)  => handleError(e)
            }
        }

        _ => Http.notFound(errorToJson(AppError.NotFound(0)))
    }
}

fn main(): Unit [io] = {
    val conn = dbConnect("postgresql://localhost/myapp")
    if conn == 0 then panic("could not connect to database")
    defer dbClose(conn)
    println("listening on :8080")
    Http.serve(8080, (req) => router(conn, req))
}
```

This program handles two routes, validates input, propagates errors as typed values,
serialises responses as JSON, and always closes the database connection on exit.
