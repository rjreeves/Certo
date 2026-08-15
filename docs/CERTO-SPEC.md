# Certo Language Specification v0.1

> *"Correct by construction"*
>
> Complete Reference — June 2026  
> certo-lang.org | github.com/certo-lang

---

# 1. Overview and Philosophy

Certo is a statically typed, compiled, general-purpose programming language for business applications with native database integration. The name derives from the Latin word for "certain" — reflecting the core promise: if your Certo program compiles, it is correct with respect to types, database schema, state transitions, and error handling.

## 1.1 The Problem Certo Solves

Business application development in 2026 suffers from a consistent set of problems that existing languages handle poorly:

- Runtime type errors from database schema mismatches discovered in production

- Null pointer exceptions from unhandled absent values

- Unhandled error cases causing silent data corruption

- Invalid state transitions in business workflows

- Currency and decimal precision errors from floating point arithmetic

- AI-generated code that looks correct but contains subtle schema mistakes

Certo eliminates all of these at compile time. They are not runtime exceptions — they are compilation failures.

## 1.2 Design Principles

| **Principle** | **Meaning in Practice** |
| --- | --- |
| Correct by construction | Illegal states cannot be represented in the type system |
| Schema as source of truth | Compiler reads live DB schema — column mismatches are compile errors |
| No implicit failures | All fallible operations return Result<T,E> — no exceptions |
| Null does not exist | Absent values use Option<T> — compiler enforces unwrapping |
| One right way | Standard library provides correct implementations — not options |
| Readable by default | Code should read like the business domain, not like the compiler |
| AI-output verifiable | Strong types catch AI code generation errors deterministically |

## 1.3 Language Characteristics

| **Property** | **Value** |
| --- | --- |
| Paradigm | Functional-first, expression-oriented, with objects for modeling |
| Type system | Hindley-Milner with ADTs, row types, traits, effects |
| Memory model | Garbage collected — tuned for throughput not latency |
| Concurrency | Coroutine-based async/await with structured concurrency |
| Compilation | Ahead-of-time — LLVM backend, WASM secondary target |
| Interop | C FFI, WASM component model, REST/gRPC code generation |
| File extension | .cto |
| CLI command | certo |

## 1.4 Quick Example — End to End

```
// billing.cto module Billing import Stdlib.{ DB, Result, Money, Timestamp } import MyApp.Models.{ Invoice, Customer, Order } type BillingError =     │ CustomerNotFound(id: UUID)     │ OrderAlreadyInvoiced(orderId: UUID)     │ InsufficientCredit(available: Money, required: Money) async fn generateInvoice(     orderId: UUID ): Result<Invoice, BillingError> =     db.transaction {         let order    = db.orders.find(orderId)                        │> require(OrderNotFound(orderId))         let customer = db.customers.find(order.customerId)                        │> require(CustomerNotFound(order.customerId))         guard !order.isInvoiced else             Err(OrderAlreadyInvoiced(orderId))         guard customer.creditLimit >= order.total else             Err(InsufficientCredit(customer.creditLimit, order.total))         let invoice = Invoice {             id:         UUID.new(),             orderId:    orderId,             customerId: customer.id,             amount:     order.total,             issuedAt:   Timestamp.now(),             dueAt:      Timestamp.now() + Duration.days(30)         }         db.invoices.insert(invoice)         db.orders.update(orderId, { isInvoiced: true })         Ok(invoice)     } // Every type verified at compile time. // Every column name verified against live schema. // Every error case must be handled by caller. // Transaction rolls back automatically on any Err.
```

# 2. Lexical Structure

## 2.1 Source Files

Certo source files use UTF-8 encoding and carry the .cto extension. Each file begins with a module declaration. The file name must match the module name in kebab-case (my-module.cto → module MyModule).

```
// src/orders/processing.cto module Orders.Processing import Stdlib.Collections.{ List, Map } import Stdlib.Result import MyApp.Models.Order
```

## 2.2 Naming Conventions

| **Category** | **Convention** | **Example** |
| --- | --- | --- |
| Values and functions | camelCase | processOrder, userId, totalAmount |
| Types and modules | PascalCase | OrderStatus, BillingService |
| Constants | UPPER_SNAKE_CASE | MAX_RETRIES, DEFAULT_PAGE_SIZE |
| Type parameters | Single uppercase letter or PascalCase | T, E, Key, Value |
| Trait names | PascalCase adjective | Serializable, Comparable, DbModel |
| File names | kebab-case | order-processing.cto, user-model.cto |

## 2.3 Keywords

```
fn         type       let        val        var match      if         then       else       when async      await      parallel   do         return import     module     export     pub        priv trait      impl       for        where      as statemachine  validator  migration  view     test property   dbTest     true       false      unit in         is         not        and        or guard      require    ensure     defer      with
```

## 2.4 Literals

| **Literal Type** | **Syntax** | **Examples** |
| --- | --- | --- |
| Integer | Decimal, hex, binary, octal | 42  0xFF  0b1010  0o77 |
| Float | Decimal with dot or exponent | 3.14  1.0e-5  6.022e23 |
| Decimal | d"..." suffix for exact | d"19.99"  d"0.001" |
| String | Double-quoted UTF-8 | "Hello"  "世界"  "line\n" |
| Interpolated | f"..." prefix | f"Total: {amount}" |
| Multiline | Triple-quoted | """\nMultiple\nLines\n""" |
| Boolean | true / false | true  false |
| UUID | uuid"..." literal | uuid"550e8400-e29b-41d4..." |
| Unit | () | ()  — no meaningful value |
| List | Square brackets | [1, 2, 3]  [] |
| Tuple | Parentheses, comma | (1, "hello", true) |
| Record | Braces with labels | { name: "Alice", age: 30 } |

## 2.5 Operators — Full Reference

| **Operator** | **Type** | **Meaning** | **Precedence** |
| --- | --- | --- | --- |
| │> | Infix | Pipe left value to right function | 1 (lowest) |
| or | Infix | Logical or (short-circuit) | 2 |
| and | Infix | Logical and (short-circuit) | 3 |
| not | Prefix | Logical negation | 4 |
| == != < > <= >= | Infix | Comparison | 5 |
| .. ... | Infix | Range (inclusive / exclusive) | 6 |
| + - | Infix | Addition, subtraction | 7 |
| * / % | Infix | Multiplication, division, modulo | 8 |
| ** | Infix | Exponentiation | 9 |
| - not | Prefix | Unary negation, not | 10 |
| . | Postfix | Field access | 11 |
| ?. | Postfix | Safe field access (Option) | 11 |
| () | Postfix | Function application | 12 (highest) |

# 3. Declarations

## 3.1 Values

```
// Immutable binding — val and let are aliases val name: Text = "Alice" val name = "Alice"         // Type inferred let name = "Alice"         // let is an alias for val // Mutable binding — explicit var counter: Int = 0 counter = counter + 1 // Destructuring val (first, second) = (1, 2) val { name, email } = user val [head, ...tail] = items
```

## 3.2 Functions

```
// Basic function fn add(a: Int, b: Int): Int = a + b // Multi-line with block body fn processOrder(order: Order): Result<Invoice, OrderError> = {     val validated = validate(order)     val invoice   = createInvoice(validated)     Ok(invoice) } // Generic function fn first<T>(list: List<T>): Option<T> =     if list.isEmpty() then None else Some(list.head) // Function with default arguments fn paginate<T>(query: Query<T>, page: Int = 0, size: Int = 20): Page<T> // Named arguments at call site paginate(db.users, page: 2, size: 50) // Higher-order functions fn apply<A, B>(f: A => B, value: A): B = f(value) // Lambda expressions — three equivalent forms List.map(xs, (x) => x * 2)                  // arrow lambda List.map(xs, fn(x: Int): Int = x * 2)        // fn expression List.map(xs) { x => x * 2 }                 // trailing lambda // Trailing lambda — last arg moved outside parens List.fold(xs, 0) { acc, x => acc + x }      // two-param trailing lambda List.forEach(xs) { x => println(x) }        // single-arg call, no parens needed // Async function async fn fetchUser(id: UUID): Result<User, DbError> // Function with effects declared fn calculateTax(amount: Money): Money  [pure] fn saveOrder(order: Order): Result<OrderId, DbError>  [db.write]
```

## 3.3 Type Declarations

```
// Type alias type UserId = UUID type Callback<T> = T => Unit // Record type (product type) type Address = {     street:  Text,     city:    Text,     country: Text,     zip:     BoundedText(10) } // Sum type (union / enum) type Shape =     │ Circle(radius: Float)     │ Rectangle(width: Float, height: Float)     │ Triangle(base: Float, height: Float) // Recursive type type Tree<T> =     │ Leaf     │ Node(value: T, left: Tree<T>, right: Tree<T>) // Newtype — semantic wrapper type Email    = Email(BoundedText(255)) type CustomerId = CustomerId(UUID)
```

## 3.4 Pattern Matching

```
// Match on sum type — exhaustive, compiler-enforced fn area(shape: Shape): Float =     match shape {         Circle(r)       => Math.PI * r * r         Rectangle(w, h) => w * h         Triangle(b, h)  => 0.5 * b * h     } // Match with guards fn classify(score: Int): Text =     match score {         s if s >= 90 => "Excellent"         s if s >= 70 => "Good"         s if s >= 50 => "Pass"         _            => "Fail"     } // Match on record fields fn greet(user: User): Text =     match user {         { name, status: Active }   => f"Hello, {name}!"         { name, status: Inactive } => f"Account inactive: {name}"         { name, status: Banned }   => "Access denied"     } // Nested pattern matching match result {     Ok(Some(user)) => process(user)     Ok(None)       => handleEmpty()     Err(e)         => handleError(e) }
```

## 3.5 Modules and Imports

```
// Module declaration — must be first line module MyApp.Orders.Processing // Import entire module import Stdlib.DateTime // Import specific names import Stdlib.Collections.{ List, Map, Set } // Import with alias import MyApp.Models.Order as O // Re-export pub import Stdlib.Result.{ Ok, Err, Result } // Conditional import (platform-specific) import when [target = "wasm"] Stdlib.WASM.{ Memory }
```

# 4. Type System — Complete Reference

## 4.1 Primitive Types

| **Type** | **Size** | **Range / Notes** | **Default?** |
| --- | --- | --- | --- |
| Int | 64-bit | -9,223,372,036,854,775,808 to 9,223,372,036,854,775,807 | Yes |
| Int8 | 8-bit | -128 to 127 |  |
| Int16 | 16-bit | -32,768 to 32,767 |  |
| Int32 | 32-bit | -2,147,483,648 to 2,147,483,647 |  |
| Int64 | 64-bit | Same as Int |  |
| UInt | 64-bit | 0 to 18,446,744,073,709,551,615 |  |
| Float | 64-bit | IEEE 754 double — for scientific use only | Yes |
| Float32 | 32-bit | IEEE 754 single |  |
| Decimal(p,s) | Variable | Exact decimal — always use for money |  |
| Bool | 1-bit | true or false — no truthy/falsy coercion | Yes |
| Char | 4 bytes | Unicode scalar value (not a byte) |  |
| Unit | 0 bytes | The type of functions with no return value | Yes |

## 4.2 Algebraic Data Types

Algebraic data types (ADTs) are the primary tool for domain modeling in Certo. They come in two forms: product types (records) and sum types (enums/unions).

### Product Types

```
// All fields required unless marked optional with ? type Invoice = {     id:         UUID,     customerId: CustomerId,     amount:     Money,     issuedAt:   Timestamp,     dueAt:      Timestamp,     paidAt:     Timestamp?,      // Optional     notes:      Text?,           // Optional     // Computed properties — never stored in DB     computed isPaid:     Bool      = paidAt.isSome()     computed isOverdue:  Bool      = Timestamp.now() > dueAt and not isPaid     computed daysUntilDue: Int     = (dueAt - Timestamp.now()).days }
```

### Sum Types

```
// Variants can carry data type NotificationChannel =     │ Email(address: Email)     │ SMS(phoneNumber: PhoneNumber)     │ Webhook(url: URL, secret: Text)     │ PushNotification(deviceToken: Text, platform: MobilePlatform) // Usage — compiler enforces all variants are handled fn send(notification: Notification, channel: NotificationChannel): Result<Unit, SendError> =     match channel {         Email(addr)              => sendEmail(addr, notification)         SMS(phone)               => sendSMS(phone, notification)         Webhook(url, secret)     => sendWebhook(url, secret, notification)         PushNotification(t, p)   => sendPush(t, p, notification)     }
```

## 4.3 Generics and Constraints

```
// Unconstrained generic fn identity<T>(x: T): T = x // Constrained by trait fn serialize<T: Serializable>(value: T): Json = value.toJson() // Multiple constraints fn saveAndLog<T: DbModel + Serializable + Loggable>(entity: T): Result<T, DbError> // Constrained by record shape (row polymorphism) fn getName<R: { name: Text }>(record: R): Text = record.name // Higher-kinded — for advanced stdlib use trait Functor<F<_>> {     fn map<A, B>(fa: F<A>, f: A => B): F<B> }
```

## 4.4 Type Inference Rules

Certo uses Algorithm W (Damas-Hindley-Milner) extended with local type inference for generic functions. The following rules govern inference:

- Literals infer their most specific type: 42 → Int, 3.14 → Float, d"19.99" → Decimal

- Function return types are inferred from the body expression

- Generic type arguments are inferred from usage at call sites

- Match arm types must unify — the compiler reports the conflicting types if they do not

- Pipeline |> preserves type information through each step

- Recursive functions require an explicit return type annotation

## 4.5 Subtyping and Coercion

Certo has minimal subtyping. There are no implicit coercions between numeric types. Conversions are always explicit and named:

```
// Explicit numeric conversion val n: Int   = 42 val f: Float = n.toFloat()      // Explicit val d: Decimal = n.toDecimal()  // Explicit // These are compile errors in Certo val wrong: Float  = 42          // Error: Int is not Float val wrong2: Text  = 42          // Error: Int is not Text // Semantic type coercion is also explicit val id: UserId  = UserId(uuid"550e8400...") val raw: UUID   = id.unwrap()   // Explicit unwrap
```

## 4.6 Effect Types

Functions declare their computational effects as part of their type. The compiler uses effects to enforce transaction boundaries, prevent pure function contamination, and enable automatic parallelization.

| **Effect** | **Meaning** | **Restrictions** |
| --- | --- | --- |
| [pure] | No side effects, deterministic | Cannot call any IO or DB function |
| [db.read] | Reads from database | Must have DB connection in scope |
| [db.write] | Writes to database | Should be inside transaction |
| [io] | General IO (file, network, etc.) | Cannot be called from pure context |
| [fallible] | May fail with external errors | Return type must be Result |
| [async] | Suspends execution | Must be called with await |
| [unsafe] | Bypasses type system | Requires explicit unsafe block |

# 5. Error Handling

Certo has no exceptions. All errors are values. This makes error handling visible, composable, and compiler-enforced. Every function that can fail says so in its type signature.

## 5.1 The Result Type

```
// Built into the language — not a library type Result<T, E> =     │ Ok(T)    // Success carrying value T     │ Err(E)   // Failure carrying error E // Pattern match to handle both cases match db.users.find(id) {     Ok(user) => render(user)     Err(e)   => handleError(e) } // Propagate errors up the call stack with ? fn processUser(id: UUID): Result<Report, AppError> = {     val user   = db.users.find(id)?          // Returns Err early if not found     val orders = db.orders.forUser(id)?       // Same     Ok(buildReport(user, orders)) }
```

## 5.2 Railway-Oriented Programming

```
// Chain fallible operations cleanly with │> and flatMap fn checkout(     cartId:   UUID,     userId:   UUID,     cardToken: Text ): Result<Receipt, CheckoutError> =     db.carts.find(cartId)     │> flatMap(validateCart)     │> flatMap(cart => applyDiscounts(cart, userId))     │> flatMap(cart => chargeCard(cart.total, cardToken))     │> flatMap(receipt => reserveInventory(cartId, receipt))     │> flatMap(reservation => confirmOrder(reservation))     │> map(generateReceipt)     │> mapErr(CheckoutError.from)    // Normalize error type
```

## 5.3 Defining Error Types

```
// Error types are sum types — model your error domain type OrderError =     │ NotFound(id: UUID)     │ AlreadyProcessed(orderId: UUID)     │ InsufficientInventory(productId: UUID, requested: Int, available: Int)     │ PaymentDeclined(code: Text, message: Text)     │ InvalidStatus(current: OrderStatus, expected: List<OrderStatus>)     │ Unauthorized(userId: UUID, action: Text) // Error hierarchy via composition type AppError =     │ OrderErr(OrderError)     │ BillingErr(BillingError)     │ AuthErr(AuthError)     │ DbErr(DbError)     │ UnexpectedErr(message: Text)
```

## 5.4 Error Handling Patterns

| **Pattern** | **When to Use** | **Syntax** |
| --- | --- | --- |
| Propagate with ? | Error should bubble to caller | db.find(id)? |
| match | Different handling per error type | match result { Ok(v) => ... Err(e) => ... } |
| getOrElse | Provide a default value | result.getOrElse(defaultValue) |
| recover | Convert error to success | result.recover(e => fallbackValue) |
| mapErr | Transform error type | result.mapErr(AppError.from) |
| flatMap | Chain dependent operations | result.flatMap(v => nextOp(v)) |
| Result.all | Require all to succeed | Result.all([r1, r2, r3]) |
| Result.allSettled | Collect all results | Result.allSettled([r1, r2, r3]) |

## 5.5 Guard Clauses

```
// guard — early return if condition fails fn processRefund(order: Order, amount: Money): Result<Refund, RefundError> = {     guard order.status == Fulfilled else         Err(RefundError.OrderNotFulfilled)     guard amount <= order.total else         Err(RefundError.AmountExceedsTotal(order.total))     guard not order.isRefunded else         Err(RefundError.AlreadyRefunded)     // All guards passed — proceed     issueRefund(order, amount) }
```

## 5.6 Panic — Last Resort

Certo has a panic mechanism for truly unrecoverable situations — programmer errors, not runtime conditions. Panics are never used for business logic.

```
// panic — for programmer errors only fn head<T>(list: List<T>): T =     match list.first() {         Some(v) => v         None    => panic("head called on empty list — this is a bug")     } // unreachable — marks code that should never execute fn process(status: Status): Text =     match status {         Active   => "active"         Inactive => "inactive"         // Compiler guarantees exhaustiveness above         // No default arm needed     }
```

# 6. Database Integration

Database integration is a first-class language feature, not a library. The compiler maintains a live connection to your database schema during compilation and verifies all queries, column references, join conditions, and type mappings statically.

## 6.1 Schema Connection and Verification

```
# certo.toml — database configuration [database] schema     = "postgresql://localhost/myapp_dev" migrations = "db/migrations/" schema-sync = true   # Fail compile if schema out of sync # What happens at compile time: # 1. Compiler connects to the schema URL # 2. Reads all table definitions from information_schema # 3. Infers Certo types for every column # 4. Validates every db.* reference in your code # 5. Reports mismatches as compile errors — not runtime errors
```

```
**Compile-Time Schema Error Example** // Developer writes: // db.users.where(.emal == "alice@example.com") // Compiler reports: // error[E0412]: field `emal` does not exist on type `users` //    → src/api/users.cto:42:23 //    │ //    42 │     db.users.where(.emal == email) //    │                       ^^^^ not found //    │ //    = help: did you mean `email`?
```

## 6.2 Query DSL — Full Reference

### Filtering

```
db.orders     │> where(.status == Pending)     │> where(.total > Money(100))     │> where(.createdAt > Timestamp.now() - Duration.days(7))     │> where(.customerId.in([id1, id2, id3]))     │> where(.notes.isNotNull())     │> where(.email.like("%@company.com"))     │> where(.tags.contains("premium"))       // Array contains     │> where(.deliverySlot.overlaps(range))   // Range overlap
```

### Sorting and Pagination

```
db.products     │> orderBy(.price, Asc)     │> orderBy(.name, Desc)      // Secondary sort     │> limit(20)     │> offset(page * 20)     // Or use built-in pagination:     │> paginate(page: 2, size: 20)  // Returns Page<Product>
```

### Joins

```
// Inner join db.orders     │> join db.customers on (.customerId == .id)     │> select(.orderNumber, .customerName, .total) // Left join db.users     │> leftJoin db.subscriptions on (.id == .userId)     │> select(.name, .email, subscriptionTier: .tier ?? "free") // Self join db.employees as e     │> leftJoin db.employees as m on (e.managerId == m.id)     │> select(name: e.name, managerName: m.name)
```

### Aggregations

```
db.orders     │> where(.status == Fulfilled)     │> groupBy(.customerId)     │> select({         customerId: .customerId,         orderCount: count(),         totalSpend: sum(.total),         avgOrder:   avg(.total),         firstOrder: min(.createdAt),         lastOrder:  max(.createdAt)     })     │> having(.totalSpend > Money(1000))     │> orderBy(.totalSpend, Desc)
```

### Mutations

```
// Insert — returns inserted record db.users.insert(User {     id:        UUID.new(),     name:      "Alice",     email:     Email("alice@example.com"),     createdAt: Timestamp.now() }) // Insert many — single round trip db.products.insertMany(newProducts) // Update — returns updated record db.orders.update(orderId, { status: Shipped, shippedAt: Timestamp.now() }) // Upsert — insert or update on conflict db.prices.upsert(newPrice, conflictOn: .productId) // Delete db.sessions.deleteWhere(.expiresAt < Timestamp.now())
```

## 6.3 Transactions

```
// Transactions are first-class — composable and typed async fn transferStock(     from: WarehouseId,     to:   WarehouseId,     sku:  SKU,     qty:  Int ): Result<StockTransfer, InventoryError> =     db.transaction {         val source = db.inventory                      .where(.warehouse == from)                      .where(.sku == sku)                      .first()                      │> require(InventoryError.NotFound(sku))         guard source.quantity >= qty else             Err(InventoryError.InsufficientStock(source.quantity, qty))         db.inventory.update(source.id, { quantity: source.quantity - qty })         db.inventory.upsert(             { warehouse: to, sku: sku, quantity: qty },             conflictOn: (.warehouse, .sku),             onConflict: { quantity: .quantity + qty }         )         val transfer = StockTransfer { from, to, sku, qty, at: Timestamp.now() }         db.stockTransfers.insert(transfer)         Ok(transfer)     }     // Automatically rolls back on any Err result     // Automatically commits on Ok result
```

## 6.4 Raw SQL Escape Hatch

```
// When the query DSL is insufficient — use raw SQL // Parameters are always sanitized — no SQL injection possible val results = db.raw<Order>(     sql: """         SELECT o.*, c.name as customer_name         FROM orders o         JOIN customers c ON c.id = o.customer_id         WHERE o.created_at > $1         AND c.region = ANY($2)     """,     params: [cutoffDate, regions] ) // Return type must be specified — compiler cannot verify raw SQL
```

## 6.5 Migrations — Full Reference

A `migration` declaration has a name, an optional `description`, and `up` / `down`
blocks containing one or more schema operations. Operations are keyword-led blocks.

### Operations

| Operation | Form |
|---|---|
| Create a table | `createTable NAME { COLUMN, … }` |
| Alter a table | `alterTable NAME { ALTEROP … }` |
| Drop a table | `dropTable NAME` |
| Create an index | `createIndex NAME on TABLE [COL, …]` |
| Drop an index | `dropIndex NAME` |
| Raw SQL escape hatch | `rawSql "…"` |

A `NAME` is a bare identifier or a string literal.

### Columns

```
COLUMN   := NAME : TYPE MODIFIER*
MODIFIER := primaryKey | unique | nullable | default EXPR
```

Columns are `NOT NULL` by default; add `nullable` to allow nulls.

### Alter operations

```
ALTEROP := addColumn COLUMN
         | dropColumn NAME
         | foreignKey COLUMN references TABLE (onDelete ACTION)?
ACTION  := cascade | setNull | restrict | noAction
```

### Example

```
migration "add_product_categories" {
    description = "Add category hierarchy to products"

    up {
        createTable categories {
            id:        UUID  primaryKey
            name:      Text  unique
            parentId:  UUID  nullable
            sortOrder: Int32 default 0
        }

        alterTable products {
            addColumn categoryId: UUID nullable
            foreignKey categoryId references categories onDelete setNull
        }

        createIndex products_category_idx on products [categoryId]
    }

    down {
        alterTable products { dropColumn categoryId }
        dropTable categories
    }
}
```

Each operation lowers directly to SQL DDL (`createTable` → `CREATE TABLE`, etc.).
The compiler validates every operation against the program's `type` declarations:
table and column names must match a declared type, column types must agree, and
`alterTable` / `dropTable` may only target a table created by an earlier
migration (errors E0505–E0507). Use `rawSql "…"` only when the structured
operations are insufficient — raw SQL is not schema-checked.

# 7. Concurrency and Async

> **Implementation status (2026-06-22):** `spawn` and `await` run on **real OS
> threads** (Win32 threads on Windows, pthreads on POSIX), with measured speedup
> on parallel CPU-bound work. The higher-level structured-concurrency features in
> this section (cancellation scopes, channels, `withTimeout`, supervised
> background jobs) are still a design target. The everyday `spawn` / `await` /
> `parallel {}` surface syntax is stable and works as written below. See
> [LIMITATIONS.md §1](LIMITATIONS.md).

Certo uses structured concurrency — all async operations are scoped, cancellable, and composable. There are no raw threads. The runtime manages a pool of OS threads internally, multiplexing coroutines across them.

## 7.1 Async Model

```
// async functions return immediately with a future async fn fetchUserProfile(id: UUID): Result<Profile, ApiError> // await suspends until the future resolves async fn buildDashboard(userId: UUID): Result<Dashboard, AppError> = {     val profile = await fetchUserProfile(userId)     val orders  = await db.orders.forUser(userId)     val balance = await billingService.getBalance(userId)     Ok(Dashboard(profile, orders, balance)) } // await propagates errors with ? async fn buildDashboard(userId: UUID): Result<Dashboard, AppError> = {     val profile = await fetchUserProfile(userId)?     val orders  = await db.orders.forUser(userId)?     Ok(Dashboard(profile, orders)) }
```

## 7.2 Parallel Execution

```
// parallel{} — run independent tasks concurrently // Compiler verifies tasks do not share mutable state async fn generateReport(month: Date): Result<Report, ReportError> = {     // All three run at the same time     val (revenue, expenses, headcount) = await parallel {         calculateRevenue(month),         calculateExpenses(month),         getHeadcount(month)     }     // Waits for all three — fails fast if any fails     Ok(Report {         revenue:   revenue?,         expenses:  expenses?,         headcount: headcount?,         profit:    revenue? - expenses?     }) } // parallel with timeout val results = await parallel(timeout: Duration.seconds(5)) {     fetchFromServiceA(),     fetchFromServiceB(), }
```

## 7.3 Structured Concurrency

```
// All tasks spawned inside a scope are cancelled when scope exits async fn processWithTimeout(orderId: UUID): Result<Receipt, Error> =     withTimeout(Duration.seconds(30)) {         processOrder(orderId)     } // Background tasks — fire and forget with supervision fn startBackgroundJobs(): Unit =     spawn {         every(Duration.minutes(5))  { cleanExpiredSessions() }         every(Duration.hours(1))    { generateHourlyReports() }         every(Duration.days(1))     { archiveOldRecords() }     } // Channels — communicate between coroutines val channel = Channel<Order>(capacity: 100) spawn { orderProducer(channel) } spawn { orderConsumer(channel) }
```

## 7.4 Resource Management

```
// use — automatically closes resources when scope exits async fn processFile(path: Path): Result<Report, IOError> =     use file = await File.open(path) {         val content = await file.readAll()         Ok(parseReport(content))     }  // file.close() called automatically here // defer — run on scope exit regardless of success/failure async fn withAudit(action: Text, f: () => Result<T, E>): Result<T, E> = {     val start = Timestamp.now()     defer { db.auditLog.insert(AuditEntry(action, start, Timestamp.now())) }     f() }
```

# 8. Domain Modeling Patterns

## 8.1 Making Illegal States Unrepresentable

The most important pattern in Certo domain modeling. Design your types so that invalid data cannot exist — not just cannot be created by correct code, but literally cannot be represented.

```
// BAD — anything can go wrong at runtime type Order = {     status:      Text,       // "pending"? "Pending"? "PENDING"?     amount:      Float,      // Float for money — precision errors     customerEmail: Text,     // Could be "not-an-email"     approvedBy:  Text?,      // What does null mean here exactly? } // GOOD — illegal states are unrepresentable type OrderStatus =     │ Pending     │ Approved(by: UserId, at: Timestamp)     │ Rejected(reason: Text, by: UserId)     │ Fulfilled(trackingId: TrackingId, at: Timestamp)     │ Cancelled(reason: Text, by: UserId, at: Timestamp) type Order = {     id:       OrderId,     amount:   Money,            // Exact decimal — never Float     customer: Email,            // Validated at construction     status:   OrderStatus,      // Typed enum — not a string } // Now "approved with no approver" is literally unrepresentable. // "Pending but with a tracking ID" is literally unrepresentable. // The wrong state cannot compile.
```

## 8.2 State Machines — Full Syntax

```
statemachine SubscriptionLifecycle {     states:         Trial,         Active,         PastDue,         Paused,         Cancelled,         Expired     transitions:         Trial     → Active    : activate(paymentMethod: PaymentMethod)         Trial     → Cancelled : cancel(reason: CancelReason)         Active    → PastDue   : markPastDue(invoiceId: InvoiceId)         Active    → Paused    : pause(until: Date)         Active    → Cancelled : cancel(reason: CancelReason)         PastDue   → Active    : resolvePayment(receiptId: ReceiptId)         PastDue   → Cancelled : cancel(reason: CancelReason)         PastDue   → Expired   : expire()         Paused    → Active    : resume()         Paused    → Cancelled : cancel(reason: CancelReason)     // on_enter hooks — run when entering a state     on_enter PastDue:   sendPaymentFailureEmail()     on_enter Cancelled: sendCancellationEmail()     on_enter Active:    sendWelcomeEmail()     // Invariants — always true in a given state     invariant Active:   self.paymentMethod.isSome()     invariant Expired:  self.endDate < Date.today() }
```

## 8.3 Smart Constructors

```
// Prevent invalid values at the boundary type Email = priv Email(Text)    // Private constructor // Only way to create an Email is through the validated constructor fn Email.new(raw: Text): Result<Email, ValidationError> =     if raw.contains("@") and raw.length > 3     then Ok(Email(raw))     else Err(ValidationError("email", f"Invalid email: {raw}")) // Usage val email = Email.new("alice@example.com")?  // Validated // val bad = Email("bad")  // Compile error — constructor is private
```

## 8.4 Value Objects

```
// Value objects — equality by value, not by reference @valueObject type Money = {     amount:   Decimal(19,4),     currency: Currency } // Equality is structural Money(d"10.00", USD) == Money(d"10.00", USD)  // true Money(d"10.00", USD) == Money(d"10.01", USD)  // false // Arithmetic preserves currency fn Money.add(a: Money, b: Money): Result<Money, CurrencyMismatch> =     if a.currency == b.currency     then Ok(Money(a.amount + b.amount, a.currency))     else Err(CurrencyMismatch(a.currency, b.currency))
```

## 8.5 Aggregate Roots

```
// Aggregates — consistency boundaries for domain objects @aggregate type ShoppingCart = {     id:       CartId,     userId:   UserId,     items:    List<CartItem>,     coupon:   Coupon?,     status:   CartStatus     // Computed values     computed subtotal:  Money = items.sumBy(.lineTotal)     computed discount:  Money = coupon.map(.discount(subtotal)) ?? Money.zero(USD)     computed total:     Money = subtotal - discount     computed itemCount: Int   = items.sumBy(.quantity)     // Domain methods — maintain invariants     fn addItem(self, item: Product, qty: Int): Result<ShoppingCart, CartError> = {         guard self.status == Open else Err(CartError.CartClosed)         guard qty > 0 else Err(CartError.InvalidQuantity)         Ok(self.with(items: self.items.upsert(CartItem(item, qty), on: .productId)))     }     fn applyCoupon(self, coupon: Coupon): Result<ShoppingCart, CartError> = {         guard coupon.isValid(self) else Err(CartError.CouponInvalid)         Ok(self.with(coupon: Some(coupon)))     } }
```

# 9. Standard Library Reference

## 9.1 Stdlib.Core

| **Function** | **Signature** | **Description** |
| --- | --- | --- |
| identity | fn<T>(x: T): T | Returns its argument unchanged |
| const | fn<A,B>(a: A): B => A | Returns a function that always returns a |
| compose | fn<A,B,C>(f: B=>C, g: A=>B): A=>C | Function composition |
| flip | fn<A,B,C>(f: A=>B=>C): B=>A=>C | Flip first two arguments |
| todo | fn(): Nothing | Marks unimplemented code — compile warning |
| unreachable | fn(): Nothing | Marks logically impossible branches |
| panic | fn(msg: Text): Nothing | Crash with message — programmer error only |

## 9.2 Stdlib.Collections

| **Function** | **Signature** | **Description** |
| --- | --- | --- |
| List.empty | fn<T>(): List<T> | Empty list |
| List.of | fn<T>(...T): List<T> | List from arguments |
| list.map | fn<A,B>(A=>B): List<B> | Transform each element |
| list.filter | fn(T=>Bool): List<T> | Keep matching elements |
| list.flatMap | fn<B>(T=>List<B>): List<B> | Map then flatten one level |
| list.reduce | fn<B>(B, (B,T)=>B): B | Left fold with initial value |
| list.find | fn(T=>Bool): Option<T> | First matching element |
| list.partition | fn(T=>Bool): (List<T>, List<T>) | Split by predicate |
| list.groupBy | fn<K>(T=>K): Map<K, List<T>> | Group by key function |
| list.sortBy | fn<K: Ord>(T=>K): List<T> | Sort by key |
| list.distinct | fn(): List<T> | Remove duplicates |
| list.zip | fn<U>(List<U>): List<(T,U)> | Pair with another list |
| list.chunked | fn(Int): List<List<T>> | Split into chunks of size n |
| list.sumBy | fn<N: Numeric>(T=>N): N | Sum a numeric projection |
| list.minBy | fn<K: Ord>(T=>K): Option<T> | Element with minimum key |
| list.maxBy | fn<K: Ord>(T=>K): Option<T> | Element with maximum key |

## 9.3 Stdlib.Text

| **Function** | **Description** |
| --- | --- |
| text.length() | Character count (not byte count) |
| text.byteLength() | UTF-8 byte count |
| text.trim() | Remove leading and trailing whitespace |
| text.split(sep) | Split into list on separator |
| text.contains(sub) | Case-sensitive containment check |
| text.startsWith(prefix) | Prefix check |
| text.toUppercase(locale?) | Locale-aware uppercase conversion |
| text.matches(regex) | Full regex match — returns Bool |
| text.find(regex) | First match — returns Option<Match> |
| text.replace(regex, replacement) | Replace all matches |
| text.toInt() | Parse as Int — returns Result<Int, ParseError> |
| text.toDecimal() | Parse as Decimal — returns Result<Decimal, ParseError> |
| Text.join(sep, list) | Join list of Text with separator |

## 9.4 Stdlib.DateTime

| **Type/Function** | **Description** |
| --- | --- |
| Timestamp.now() | Current UTC instant |
| Timestamp.of(y,m,d,h,min,s,tz) | Construct from components |
| Timestamp.parse(text) | Parse ISO 8601 — Result<Timestamp, ParseError> |
| timestamp.inTimezone(tz) | Convert for display purposes |
| timestamp.format(pattern, tz) | Format with explicit timezone |
| Date.today(tz) | Current date in given timezone |
| Date.of(year, month, day) | Construct from components |
| date + duration | Produces Date |
| timestamp - timestamp | Produces Duration |
| Duration.days(n) | Construct duration of n days |
| Duration.hours(n) | Construct duration of n hours |
| Timezone("Europe/London") | IANA timezone by name |

## 9.5 Stdlib.Test

```
// Unit test test "total includes tax" {     val items = [CartItem(price: Money(d"10.00", USD), qty: 2)]     val total = calculateTotal(items, taxRate: d"0.10")     expect(total).toBe(Money(d"22.00", USD)) } // Property-based test — runs 100 random examples by default property "total is never negative" {     forAll(List<CartItem>) { items =>         calculateTotal(items) >= Money.zero(USD)     } } // DB integration test — auto-rollback after test dbTest "user creation persists" {     val user = db.users.insert(testUser)     val found = db.users.find(user.id)     expect(found).toBeSome()     expect(found?.name).toBe(testUser.name) } // Async test asyncTest "payment processing succeeds" {     val result = await processPayment(testOrder, testCard)     expect(result).toBeOk() }
```

# 10. UI Compiler

The Certo UI compiler generates type-safe web interfaces from your data models and business logic. It is optional — Certo works equally well as a pure backend language. The UI compiler targets Htmx-over-HTTP as the primary output with React as a secondary target.

## 10.1 Schema-Driven Generation

The fastest path to a working UI. Annotate a type with @ui.generate and the compiler produces a complete CRUD interface: list view with sorting and filtering, detail view, create and edit forms with validation.

```
@ui.generate(Product) {     title:  "Products"     list: {         columns:    [name, sku, price, stockQuantity, status],         sortable:   [name, price, stockQuantity],         filterable: [status, categoryId],         searchable: [name, sku, description]     }     detail: {         sections: [             { title: "Product Info",  fields: [name, sku, description, categoryId] },             { title: "Pricing",       fields: [price, compareAtPrice, costPrice] },             { title: "Inventory",     fields: [stockQuantity, lowStockThreshold] }         ]     }     form: {         create: [name, sku, price, categoryId, description]         edit:   [name, price, stockQuantity, status, description]     }     permissions: {         view:   [Admin, StockManager, Viewer],         create: [Admin, StockManager],         edit:   [Admin, StockManager],         delete: [Admin]     } }
```

## 10.2 Declarative View Syntax

```
view OrderDashboard {     // Reactive queries — auto-refresh when data changes     live val pendingOrders  = db.orders.where(.status == Pending).count()     live val todayRevenue   = db.orders.where(.today).sumBy(.total)     live val lowStock       = db.products.where(.stockQty < 10)     layout: Column {         // Metric cards row         Row {             MetricCard("Pending",        pendingOrders)             MetricCard("Today Revenue",  todayRevenue,   format: Currency)             MetricCard("Low Stock",      lowStock.count, alert: lowStock.count > 0)         }         // Orders table         Table(db.orders.recent(50)) {             column("Order #",   .orderNumber)             column("Customer",  .customerName)             column("Amount",    .total,     format: Currency)             column("Status",    .status,    badge: statusColor)             column("Date",      .createdAt, format: RelativeTime)             action("View",      o => navigate(OrderDetail(o.id)))         }     } }
```

## 10.3 Form Syntax

```
form ProductForm {     target: Product     field name {         label:       "Product Name"         placeholder: "Enter product name"         // Validation rules auto-derived from BoundedText(200) type     }     field price {         label:  "Price"         type:   CurrencyInput(USD)         // Min(0) auto-derived from Money type constraints     }     field categoryId {         label:    "Category"         type:     Select         options:  db.categories.select(.id, .name)   // Loaded from DB     }     field description {         label: "Description"         type:  RichText         rows:  6     }     onSubmit:  createProduct    // Tied to business function     onSuccess: navigate(ProductList) }
```

## 10.4 UI Compilation Targets

| **Target** | **Output** | **Phase** | **Best For** |
| --- | --- | --- | --- |
| Htmx | Server-rendered HTML + htmx attributes | 1 | Business web apps, admin panels |
| React | TypeScript + React components | 2 | Teams already on React |
| PWA | Progressive Web App with offline support | 2 | Mobile business users |
| Flutter | Flutter widget tree | 3 | Native mobile requirements |

# 11. Toolchain

## 11.1 Compiler Pipeline

| **Stage** | **Input** | **Output** | **Key Work** |
| --- | --- | --- | --- |
| Lexing | Source text | Token stream | Tokenize, handle Unicode, strip comments |
| Parsing | Tokens | AST | Grammar rules, error recovery, source spans |
| Name Resolution | AST | Resolved AST | Bind identifiers, module imports, scope chains |
| Type Inference | Resolved AST | Typed AST | Algorithm W, constraint generation, unification |
| Schema Validation | Typed AST | Verified AST | Connect to DB, verify column types and names |
| HIR Lowering | Verified AST | HIR | Desugar syntax, normalize patterns |
| MIR Lowering | HIR | MIR (SSA) | Build CFG, SSA form, explicit drops |
| Optimization | MIR | Optimized MIR | DCE, inlining, constant folding, query pushdown |
| Codegen | Optimized MIR | LLVM IR / C / WASM | Target-specific emission |
| Linking | LLVM IR | Binary | Link stdlib, produce executable |

## 11.2 CLI Reference

| **Command** | **Options** | **Description** |
| --- | --- | --- |
| certo new <name> | --template default│api│lib│cli│fullstack | Scaffold new project |
| certo build | --release  --target native│wasm│jvm | Compile project |
| certo run | --watch  --port 8080 | Build and run, hot reload |
| certo check | --strict  --explain E0412 | Type check only |
| certo test | --filter <pattern>  --coverage  --watch | Run tests |
| certo fmt | --check  --diff | Format source files |
| certo db migrate | --dry-run  --step 1 | Apply migrations |
| certo db rollback | --step 1 | Reverse migrations |
| certo db status |  | Show migration state |
| certo db pull | --url <dsn> | Import schema from DB |
| certo add <pkg> | --version 1.2.0  --dev | Add dependency |
| certo audit | --fix  --level high | Security audit |
| certo repl | --connect <dsn> | Interactive session |
| certo doc | --serve  --port 4000 | Generate documentation |
| certo generate | model│api│migration <name> | Code scaffolding |

## 11.3 Project Structure

```
my-app/ ├── certo.toml              # Project manifest and config ├── certo.lock              # Dependency lock file — always commit ├── src/ │   ├── main.cto            # Entry point │   ├── models/             # Domain types │   │   ├── user.cto │   │   └── order.cto │   ├── services/           # Business logic │   │   ├── billing.cto │   │   └── shipping.cto │   ├── api/                # HTTP handlers │   │   └── routes.cto │   └── ui/                 # UI views (optional) │       └── views/ ├── db/ │   ├── migrations/         # Database migrations │   │   ├── 001_create_users.cto │   │   └── 002_add_orders.cto │   └── seeds/              # Development seed data │       └── development.cto └── tests/     ├── unit/               # Unit tests     └── integration/        # Integration tests
```

## 11.4 certo.toml — Full Schema

```
[project] name    = "billing-system" version = "1.2.0" edition = "2026" authors = ["Alice Smith <alice@example.com>"] license = "MIT" [build] target     = "native"      # native │ wasm │ jvm output     = "dist/" entry      = "src/main.cto" [database] schema     = "postgresql://localhost/billing_dev" migrations = "db/migrations/" seeds      = "db/seeds/" [server] port    = 8080 host    = "0.0.0.0" [dependencies] stripe      = "4.2.0" sendgrid    = "2.1.0" pdf-render  = "1.0.3" [dev-dependencies] test-fixtures = "1.0.0" [features] strict-nulls     = true    # Warn on any unsafe null operations query-logging    = true    # Log all DB queries in dev mode schema-sync      = true    # Fail compile on schema mismatch effect-checking  = true    # Enforce effect type annotations [targets.production] optimize    = true strip-debug = true schema      = "${DATABASE_URL}"
```

# 12. Interoperability

## 12.1 C FFI

Certo can call C libraries through its Foreign Function Interface. FFI calls are marked unsafe — the compiler cannot verify their behavior.

```
// Declare an external C function extern "C" {     fn strlen(s: *Byte): UInt     fn malloc(size: UInt): *Byte     fn free(ptr: *Byte): Unit } // Call it in an unsafe block fn rawStringLength(s: Text): UInt =     unsafe { strlen(s.toCString()) } // Wrap unsafe in a safe API — preferred pattern fn safeLength(s: Text): UInt = rawStringLength(s)
```

## 12.2 REST API Client Generation

```
// Generate a type-safe client from OpenAPI spec @apiClient("https://api.stripe.com/openapi.json") module Stripe // Compiler downloads spec and generates: // — Typed request/response models // — Async client functions // — Error types matching API error codes // Generated usage looks like: async fn chargeCard(amount: Money, token: Text): Result<Charge, StripeError> =     Stripe.charges.create({         amount:   amount.toCents(),         currency: "usd",         source:   token     })
```

## 12.3 Calling Certo from Other Languages

```
// Export functions for use from Python, Node.js, etc. @export("certo_process_order") pub fn processOrder(orderJson: Text): Text =     orderJson     │> Json.decode<OrderRequest>     │> flatMap(processOrderInternal)     │> map(Json.encode)     │> getOrElse("""{"error":"Processing failed"}""") // Called from Python: // import certo // result = certo.process_order(order_json) // WASM target — runs in browser or inside PostgreSQL // certo build --target wasm // Creates: dist/my-app.wasm
```

## 12.4 Gradual Adoption

Certo is designed for gradual adoption. You do not need to rewrite your entire application. Start with a single module, service, or data layer.

| **Adoption Pattern** | **Description** | **Good Starting Point** |
| --- | --- | --- |
| New service | Write new microservices in Certo alongside existing services | Best option |
| Data layer only | Use Certo for DB operations, call from Python/Node | Low risk |
| Migration tooling | Use Certo migrations alongside existing app | Zero risk |
| UI layer | Use Certo UI compiler, keep existing backend | Front-end teams |
| Full greenfield | New project entirely in Certo | Best results |

# 13. Security Model

## 13.1 SQL Injection — Impossible by Design

SQL injection is architecturally impossible in Certo. The query DSL never constructs SQL strings. All values are parameterized at the type level. Even the raw SQL escape hatch always uses parameterized queries.

```
// This looks like string interpolation — it is NOT // The compiler generates parameterized queries always db.users.where(.email == userInput)    // Safe — parameterized // Raw SQL also uses parameters — never string concat db.raw<User>(     sql: "SELECT * FROM users WHERE email = $1",     params: [userInput]    // Always parameterized ) // This is a compile error — no string SQL allowed db.raw<User>(sql: f"SELECT * WHERE email = '{userInput}'") // Error: string interpolation in SQL context is forbidden
```

## 13.2 Secrets Management

```
// Secrets are a first-class type — never logged, never serialized type Secret<T> = priv Secret(T) // Load from environment — not from code val dbPassword = Env.require("DB_PASSWORD") │> map(Secret.wrap) // Secret values cannot be logged Log.info("Connecting", { password: dbPassword })  // Compile error // Error: Secret<Text> is not Loggable // Secret values cannot be serialized to JSON Json.encode(dbPassword)  // Compile error // Error: Secret<Text> is not Serializable // Must explicitly unwrap to use the value connect(host: dbHost, password: dbPassword.expose())
```

## 13.3 Authorization Patterns

```
// Role-based access using phantom types type Permission<T> = priv Permission(T) type Admin type User type Viewer // Functions that require specific roles fn deleteUser(id: UUID, _auth: Permission<Admin>): Result<Unit, DbError> fn viewReport(_auth: Permission<Admin │ Viewer>): Result<Report, DbError> // Caller must prove they have permission // The Permission value can only be obtained from the auth middleware fn handleDeleteRequest(req: Request): Response =     match req.auth.asAdmin() {         Some(perm) => deleteUser(req.userId, perm) │> toResponse         None       => Response.forbidden()     }
```

# 14. Implementation Roadmap

## Phase 1 — Bootstrap (Months 1–6)

```
**Milestone: "Hello World" compiles and runs** Month 1:  Lexer and parser for core syntax. AST data structures. Month 2:  Name resolution, basic scope analysis. Month 3:  Type checker — primitives, records, ADTs. Month 4:  C transpiler backend. First programs execute. Month 5:  Stdlib.Core, Stdlib.Text, Stdlib.Collections. Month 6:  CLI (build/run/check), formatter, basic LSP errors.
```

## Phase 2 — Language Complete (Months 7–12)

```
**Milestone: First real application runs against PostgreSQL** Month 7:  Generics and type inference (Algorithm W). Month 8:  Traits and impl blocks. Stdlib.Option, Stdlib.Result. Month 9:  DB module. Schema connection. Query DSL. Month 10: Async/await. Coroutine runtime. Stdlib.IO. Month 11: Stdlib.DateTime, Stdlib.Money. Migration tool. Month 12: Test runner. REPL. Full LSP (autocomplete, hover types).
```

## Phase 3 — Production Ready (Months 13–24)

```
**Milestone: One company runs Certo in production** Month 13: LLVM backend (replaces C transpiler). Month 14: Schema-aware LSP. Column autocomplete. Query plan preview. Month 15: Package registry. certo.toml dependency resolution. Month 16: UI compiler — schema-driven generation, Htmx output. Month 17: WASM target. PostgreSQL WASM extension support. Month 18: Performance profiler. Debugger. Docs generator. Month 19: State machine verification. Effect checking. Month 20: React UI export. PWA support. Month 21: Security audit of compiler and stdlib. Month 22: C FFI. REST client generation. Month 23: Self-hosting — parser rewritten in Certo. Month 24: Self-hosting — full compiler rewritten in Certo.
```

## 14.1 Build Order Rationale

The C transpiler comes first because it is the fastest path to a working compiler. Once the language is validated by real use, the LLVM backend provides production-grade performance without rebuilding the frontend. The UI compiler comes after the language is stable — UI design requires stable semantics to build against.

## 14.2 Key Risk Factors

| **Risk** | **Severity** | **Mitigation** |
| --- | --- | --- |
| Type inference performance | High | Implement incremental type checking from month 8 |
| Schema sync latency in large codebases | Medium | Cache schema, invalidate on migration |
| Adoption without ecosystem | High | Python/Node interop from day one — gradual adoption |
| LLVM complexity | Medium | C transpiler first — validate language before LLVM |
| AI code tools ignoring new syntax | High | Publish grammar for tree-sitter by month 6 |
| Developer familiarity curve | Medium | Invest in error messages from day one (Elm-quality) |

# 15. Positioning and Market Strategy

## 15.1 Target Market

- Regulated industries — finance, healthcare, legal, government — where runtime correctness is a compliance requirement, not a preference

- Business application teams of 5–50 developers building internal tools, ERPs, billing systems, and workflow automation

- Organizations using AI-assisted code generation who need a verification layer between AI output and production

- Teams who have experienced production incidents from database schema drift, null pointer exceptions, or unhandled error cases

## 15.2 Differentiation Matrix

| **Feature** | **Certo** | **Python** | **Kotlin** | **TypeScript** | **Go** |
| --- | --- | --- | --- | --- | --- |
| Compile-time schema validation | ✓ | ✗ | ✗ | ✗ | ✗ |
| Null safety by design | ✓ | ✗ | ✓ | ✓ | ✗ |
| ADTs + exhaustive matching | ✓ | ✗ | ✓ | partial | ✗ |
| Result types — no exceptions | ✓ | ✗ | ✗ | ✗ | partial |
| State machine enforcement | ✓ | ✗ | ✗ | ✗ | ✗ |
| Money as first-class type | ✓ | ✗ | ✗ | ✗ | ✗ |
| Effect tracking | ✓ | ✗ | ✗ | ✗ | ✗ |
| UI generation from schema | ✓ | ✗ | ✗ | ✗ | ✗ |
| AI output verification | ✓ | ✗ | ✗ | partial | ✗ |

## 15.3 The AI Positioning

In 2026, AI tools generate large amounts of application code. This code is probabilistically correct — it passes code review and tests, but contains subtle errors that emerge in production. The most common AI code generation failures in business applications are:

- Schema mismatches — referencing columns that do not exist or have different types

- Missing error cases — AI generates happy-path code that misses edge cases

- Invalid state transitions — business rules not encoded in the AI's training context

- Currency errors — AI uses Float for monetary values

Certo's compiler catches all four categories deterministically. The positioning is not "write code without AI" — it is "use AI to write Certo, then let Certo verify the AI."

## 15.4 Identity

| **Asset** | **Value** |
| --- | --- |
| Language name | Certo |
| CLI command | certo |
| File extension | .cto |
| Primary tagline | "Correct by construction" |
| Technical tagline | "If it compiles, it is correct" |
| Market tagline | "The verification layer for AI-generated business code" |
| Domain to register | certo-lang.org |
| GitHub org to register | github.com/certo-lang |
| Social handle | @certo_lang |

## 15.5 Adoption Strategy

- Phase 1 — Tool first: Ship the migration tool standalone. Zero adoption risk. Developers use certo db migrate without adopting the language.

- Phase 2 — One module: Target teams building one new service. Certo service alongside existing Python/Node. Interop via REST or FFI.

- Phase 3 — One company: Find one regulated-industry company willing to run Certo in production. That case study changes the conversation.

- Phase 4 — AI angle: Publish the demo of Copilot writing Certo and the compiler catching schema errors. Developer community shares this.

- Phase 5 — Ecosystem: Package registry, community libraries, conference talks. Language adoption follows community adoption.

# Appendix A: Grammar (EBNF Subset)

Partial grammar for the Certo parser. Full grammar available at certo-lang.org/grammar.

```
program       ::= module_decl import* declaration* module_decl   ::= "module" module_path module_path   ::= IDENT ("." IDENT)* declaration   ::= fn_decl │ type_decl │ val_decl │ trait_decl                 │ impl_decl │ statemachine_decl │ validator_decl fn_decl       ::= "async"? "fn" IDENT type_params? "(" params ")"                   ":" type effect_ann? "=" expr type_decl     ::= "type" IDENT type_params? "=" type_body type_body     ::= record_type │ sum_type │ type_alias record_type   ::= "{" (field_decl ",")* computed_decl* "}" sum_type      ::= ("│" IDENT variant_payload?)+ expr          ::= let_expr │ match_expr │ if_expr │ pipeline_expr pipeline_expr ::= apply_expr ("│>" apply_expr)* apply_expr    ::= atom_expr arg_list* match_expr    ::= "match" expr "{" match_arm+ "}" match_arm     ::= pattern ("if" expr)? "=>" expr pattern       ::= IDENT │ "_" │ literal │ record_pat │ tuple_pat                 │ ctor_pat │ or_pat │ as_pat type          ::= primitive │ named_type │ option_type │ result_type                 │ list_type │ tuple_type │ fn_type │ generic_type option_type   ::= type "?" effect_ann    ::= "[" effect ("," effect)* "]" effect        ::= "pure" │ "db.read" │ "db.write" │ "io"                 │ "async" │ "fallible" │ "unsafe"
```

# Appendix B: Error Code Reference

| **Code** | **Category** | **Description** |
| --- | --- | --- |
| E0001 | Syntax | Unexpected token |
| E0002 | Syntax | Unclosed delimiter |
| E0003 | Syntax | Invalid expression |
| E0100 | Names | Undefined identifier |
| E0101 | Names | Ambiguous import |
| E0102 | Names | Duplicate definition in scope |
| E0200 | Types | Type mismatch |
| E0201 | Types | Cannot unify types |
| E0202 | Types | Infinite type (occurs check failed) |
| E0203 | Types | Missing type annotation on recursive function |
| E0300 | Patterns | Non-exhaustive match — missing variants |
| E0301 | Patterns | Unreachable pattern arm |
| E0302 | Patterns | Binding not used in all arms |
| E0400 | Schema | Table not found in schema |
| E0401 | Schema | Column not found on table |
| E0402 | Schema | Type mismatch between Certo type and DB column type |
| E0403 | Schema | Join condition type mismatch |
| E0404 | Schema | Schema connection failed |
| E0500 | Effects | DB write outside transaction |
| E0501 | Effects | IO in pure context |
| E0502 | Effects | Async function called without await |
| E0600 | Safety | Unsafe code outside unsafe block |
| E0601 | Safety | Secret type used in loggable context |
| E0602 | Safety | SQL string interpolation — injection risk |
| W0001 | Warning | Possible integer overflow |
| W0002 | Warning | Float used where Decimal expected |
| W0003 | Warning | DB write not in transaction |
| W0004 | Warning | Large query without limit |

# Appendix C: Style Guide

## C.1 Code Organization

- One type per file for complex domain types

- Group related functions in the same module as their primary type

- Keep modules under 300 lines — split if larger

- Tests live alongside the code they test, in the same file or a parallel tests/ structure

## C.2 Naming

- Functions that return Result should NOT be named tryX or getX — the return type communicates fallibility

- Boolean functions use is, has, can, should prefix: isActive, hasPermission

- Async functions are not named with Async suffix — the return type communicates async

- Type parameters: T for generic, E for error, K for key, V for value

## C.3 Error Handling

- Define domain-specific error types — never use Text as an error type

- Use ? propagation for errors that should bubble up

- Use match for errors that require different handling per variant

- Never use panic for business logic — only for programmer errors

## C.4 Database

- Always use transactions for multi-step writes

- Name queries descriptively in code — use let to give them names

- Keep business logic separate from query construction

- Always add limit to queries that could return large result sets

## C.5 Formatting — Enforced by certo fmt

```
// 4-space indentation // Opening brace on same line // Trailing commas in multi-line structures // Align => in match arms when they fit on screen // Correct: match status {     Active    => doActive()     Inactive  => doInactive()     Banned    => doBanned() } // Max line length: 100 characters // Formatter wraps function arguments when they exceed this
```